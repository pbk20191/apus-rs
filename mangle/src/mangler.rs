//! The mangling itself. The buffer-level rules (identifiers with word substitutions, the substitution table and
//! substitution merging) are ported from the Swift compiler: lib/Basic/Mangler.cpp and
//! include/swift/Demangling/ManglingUtils.h @ swift-6.4.2-RELEASE. The entity and type rules follow
//! lib/AST/ASTMangler.cpp and docs/ABI/Mangling.rst, and are checked against swiftc by tests/corpus.rs.

use crate::ast::*;
use crate::names;
use std::collections::HashMap;

/// What can be referred back to with an `A` substitution. Identifiers and entities share one numbering.
#[derive(PartialEq, Eq, Hash)]
enum Key {
    Identifier(String),
    Type(Ty),
}

const MAX_WORDS: usize = 26;
const MAX_REPEAT_COUNT: usize = 2048;

#[derive(Default)]
struct Mangler {
    buffer: String,
    substitutions: HashMap<Key, usize>,
    /// Words of the identifiers mangled so far, for word substitutions.
    words: Vec<String>,
    // SubstitutionMerging
    last_subst_position: usize,
    last_subst_size: usize,
    last_num_substs: usize,
    last_subst_is_standard: bool,
}

/// `INDEX`: 0 is `_`, n is `<n-1>_`.
fn index(n: usize) -> String {
    if n == 0 { "_".into() } else { format!("{}_", n - 1) }
}

fn is_word_start(ch: u8) -> bool {
    !ch.is_ascii_digit() && ch != b'_' && ch != 0
}

fn is_word_end(ch: u8, previous: u8) -> bool {
    ch == b'_' || ch == 0 || (!previous.is_ascii_uppercase() && ch.is_ascii_uppercase())
}

impl Mangler {
    fn push(&mut self, operator: &str) {
        self.buffer.push_str(operator);
    }

    fn add_substitution(&mut self, key: Key) {
        let next = self.substitutions.len();
        self.substitutions.entry(key).or_insert(next);
    }

    fn try_substitution(&mut self, key: &Key) -> bool {
        match self.substitutions.get(key).copied() {
            Some(index) => {
                self.mangle_substitution(index);
                true
            }
            None => false,
        }
    }

    /// SubstitutionMerging::tryMergeSubst
    fn try_merge(&mut self, subst: &str, is_standard: bool) -> bool {
        let size = self.buffer.len();
        if self.last_num_substs > 0
            && self.last_num_substs < MAX_REPEAT_COUNT
            && size == self.last_subst_position + self.last_subst_size
            && self.last_subst_is_standard == is_standard
        {
            let last = self.buffer[size - self.last_subst_size..].trim_start_matches(|c: char| c.is_ascii_digit()).to_string();
            if last != subst && !is_standard {
                // Merge with a different 'A' substitution: 'AB' -> 'AbC'.
                self.last_subst_position = size;
                self.last_num_substs = 1;
                let last_char = last.chars().last().expect("a substitution");
                self.buffer.truncate(size - 1);
                self.buffer.push(last_char.to_ascii_lowercase());
                self.buffer.push_str(subst);
                self.last_subst_size = 1;
                return true;
            }
            if last == subst {
                // Merge with the same substitution: 'AB' -> 'A2B', 'S3i' -> 'S4i'.
                self.last_num_substs += 1;
                self.buffer.truncate(self.last_subst_position);
                self.buffer.push_str(&self.last_num_substs.to_string());
                self.buffer.push_str(subst);
                self.last_subst_size = self.buffer.len() - self.last_subst_position;
                return true;
            }
        }
        self.last_subst_position = size + 1;
        self.last_subst_size = subst.len();
        self.last_num_substs = 1;
        self.last_subst_is_standard = is_standard;
        false
    }

    /// Mangler::mangleSubstitution
    fn mangle_substitution(&mut self, substitution: usize) {
        if substitution >= 26 {
            self.push("A");
            self.push(&index(substitution - 26));
            return;
        }
        let subst = ((b'A' + substitution as u8) as char).to_string();
        if !self.try_merge(&subst, false) {
            self.push("A");
            self.push(&subst);
        }
    }

    fn standard_substitution(&mut self, subst: &str) {
        if !self.try_merge(subst, true) {
            self.push("S");
            self.push(subst);
        }
    }

    /// Mangler::appendIdentifier
    fn identifier(&mut self, name: &str) -> Result<(), String> {
        if !name.is_ascii() {
            return Err(format!("non-ASCII identifier `{name}` is not supported"));
        }
        let key = Key::Identifier(name.to_string());
        if self.try_substitution(&key) {
            return Ok(());
        }
        self.add_substitution(key);
        self.mangle_identifier(name);
        Ok(())
    }

    /// mangleIdentifier (ManglingUtils.h): the identifier's length and text, with words that already occurred
    /// replaced by a letter.
    fn mangle_identifier(&mut self, ident: &str) {
        let bytes = ident.as_bytes();
        let mut replacements: Vec<(usize, Option<usize>)> = Vec::new(); // (position, word index)
        let mut word_start: Option<usize> = None;
        for position in 0..=bytes.len() {
            let ch = bytes.get(position).copied().unwrap_or(0);
            if let Some(start) = word_start {
                if is_word_end(ch, bytes[position - 1]) {
                    let word = &ident[start..position];
                    let known = self.words.iter().position(|w| w == word);
                    if let Some(word_index) = known {
                        replacements.push((start, Some(word_index)));
                    } else if word.len() >= 2 && self.words.len() < MAX_WORDS {
                        self.words.push(word.to_string());
                    }
                    word_start = None;
                }
            }
            if word_start.is_none() && is_word_start(ch) {
                word_start = Some(position);
            }
        }
        if !replacements.is_empty() {
            self.buffer.push('0');
        }
        let count = replacements.len();
        replacements.push((bytes.len(), None)); // the dummy word at the end
        let mut position = 0;
        for (i, &(at, word)) in replacements.iter().enumerate() {
            if position < at {
                self.buffer.push_str(&(at - position).to_string());
                self.buffer.push_str(&ident[position..at]);
                position = at;
            }
            if let Some(word_index) = word {
                position += self.words[word_index].len();
                if i + 1 < count {
                    self.buffer.push((b'a' + word_index as u8) as char);
                } else {
                    // The last word substitution is a capital letter.
                    self.buffer.push((b'A' + word_index as u8) as char);
                    if position == bytes.len() {
                        self.buffer.push('0');
                    }
                }
            }
        }
    }

    fn module(&mut self, module: &str) -> Result<(), String> {
        match module {
            "Swift" => self.push("s"),
            "_Concurrency" => self.push("Sc"),
            "__C" => self.push("So"),
            _ => self.identifier(module)?,
        }
        Ok(())
    }

    /// ASTMangler::appendAnyGenericType for a non-generic (or unbound) nominal type.
    fn nominal(&mut self, module: &str, path: &[(String, Kind)]) -> Result<(), String> {
        let (name, kind) = path.last().ok_or("empty type path")?;
        if path.len() == 1 {
            let standard = match module {
                "Swift" => names::standard_type(name),
                "_Concurrency" => names::concurrency_type(name),
                _ => None,
            };
            if let Some((subst, _)) = standard {
                self.standard_substitution(subst);
                return Ok(());
            }
        }
        let key = Key::Type(Ty::Nominal { module: module.to_string(), path: path.to_vec(), args: Vec::new() });
        if self.try_substitution(&key) {
            return Ok(());
        }
        if path.len() == 1 {
            self.module(module)?;
        } else {
            self.nominal(module, &path[..path.len() - 1])?;
        }
        self.identifier(name)?;
        self.buffer.push(kind.operator());
        self.add_substitution(key);
        Ok(())
    }

    fn protocol(&mut self, protocol: &Protocol) -> Result<(), String> {
        let standard = match protocol.module.as_str() {
            "Swift" => names::standard_protocol(&protocol.name),
            "_Concurrency" => names::concurrency_protocol(&protocol.name),
            _ => None,
        };
        if let Some(subst) = standard {
            self.standard_substitution(subst);
            return Ok(());
        }
        // A protocol is not an entry of its own: only its module and name identifiers are substituted.
        self.module(&protocol.module)?;
        self.identifier(&protocol.name)
    }

    fn ty(&mut self, ty: &Ty) -> Result<(), String> {
        match ty {
            Ty::Nominal { module, path, args } if args.is_empty() => self.nominal(module, path),
            Ty::Nominal { module, path, args } => {
                let key = Key::Type(ty.clone());
                if self.try_substitution(&key) {
                    return Ok(());
                }
                self.nominal(module, path)?;
                self.push("y");
                for arg in args {
                    self.ty(arg)?;
                }
                self.push("G");
                self.add_substitution(key);
                Ok(())
            }
            Ty::Optional(wrapped) => {
                let key = Key::Type(ty.clone());
                if self.try_substitution(&key) {
                    return Ok(());
                }
                self.ty(wrapped)?;
                self.push("Sg");
                self.add_substitution(key);
                Ok(())
            }
            // The empty tuple as a type; a `()` result is just `y` (see `mangle`).
            Ty::Tuple(elements) if elements.is_empty() => {
                self.push("yt");
                Ok(())
            }
            // Tuples and existentials are mangled in full every time; they are not substitution entries.
            Ty::Tuple(elements) => {
                for (i, (label, element)) in elements.iter().enumerate() {
                    self.ty(element)?;
                    if let Some(label) = label {
                        self.identifier(label)?;
                    }
                    if i == 0 {
                        self.push("_");
                    }
                }
                self.push("t");
                Ok(())
            }
            Ty::Any => {
                self.push("yp");
                Ok(())
            }
            Ty::AnyObject => {
                self.push("yXl");
                Ok(())
            }
            Ty::Existential(protocol) => {
                self.protocol(protocol)?;
                self.push("_p");
                Ok(())
            }
            Ty::Param(0) => {
                self.push("x");
                Ok(())
            }
            Ty::Param(n) => {
                self.push("q");
                self.push(&index(n - 1));
                Ok(())
            }
            Ty::Member(param, name) => {
                let key = Key::Type(ty.clone());
                if self.try_substitution(&key) {
                    return Ok(());
                }
                self.identifier(name)?;
                if *param == 0 {
                    self.push("Qz");
                } else {
                    self.push("Qy");
                    self.push(&index(param - 1));
                }
                self.add_substitution(key);
                Ok(())
            }
        }
    }

    /// The generic parameter a requirement is about: `z` for the first, an index for the others.
    fn generic_param_index(&mut self, param: usize) {
        if param == 0 {
            self.push("z");
        } else {
            self.push(&index(param - 1));
        }
    }

    /// ASTMangler::appendRequirement
    fn requirement(&mut self, requirement: &Requirement) -> Result<(), String> {
        let (subject, plain, with_substitution, on_associated_type) = match requirement {
            Requirement::Conforms(subject, protocol) => {
                self.protocol(protocol)?;
                (subject, "R", "RQ", "Rp")
            }
            Requirement::SameType(subject, ty) => {
                self.ty(ty)?;
                (subject, "Rs", "RS", "Rt")
            }
        };
        match subject {
            Subject::Param(param) => {
                self.push(plain);
                self.generic_param_index(*param);
            }
            Subject::Member(param, name) => {
                let key = Key::Type(Ty::Member(*param, name.clone()));
                if self.try_substitution(&key) {
                    self.push(with_substitution);
                } else {
                    self.identifier(name)?;
                    self.add_substitution(key);
                    self.push(on_associated_type);
                    self.generic_param_index(*param);
                }
            }
        }
        Ok(())
    }

    /// ASTMangler::appendFunctionInputType
    fn parameters(&mut self, params: &[Param]) -> Result<(), String> {
        let element = |m: &mut Mangler, p: &Param| -> Result<(), String> {
            m.ty(&p.ty)?;
            if p.inout {
                m.push("z");
            }
            if p.variadic {
                m.push("d");
            }
            Ok(())
        };
        match params {
            [] => self.push("y"),
            // A sole unlabeled, non-variadic, non-tuple parameter is mangled as its type alone.
            [only] if only.label.is_none() && !only.variadic && !matches!(only.ty, Ty::Tuple(_)) => element(self, only)?,
            _ => {
                for (i, param) in params.iter().enumerate() {
                    element(self, param)?;
                    if i == 0 {
                        self.push("_");
                    }
                }
                self.push("t");
            }
        }
        Ok(())
    }
}

/// Canonical requirement order: by subject (generic parameters, then their associated types), conformances
/// before same-type requirements, protocols by module and name.
fn sorted_requirements(requirements: &[Requirement]) -> Vec<Requirement> {
    let mut sorted = requirements.to_vec();
    sorted.sort_by_key(|requirement| {
        let (subject, kind, protocol) = match requirement {
            Requirement::Conforms(subject, protocol) => (subject, 0, (protocol.module.clone(), protocol.name.clone())),
            Requirement::SameType(subject, _) => (subject, 1, Default::default()),
        };
        let subject = match subject {
            Subject::Param(param) => (0, *param, String::new()),
            Subject::Member(param, name) => (1, *param, name.clone()),
        };
        (subject, kind, protocol)
    });
    sorted
}

/// Associated types that a same-type requirement pins down: the signature is mangled with them replaced
/// (`S.Element == Int` makes a parameter of type `S.Element` an `Int`).
fn same_types(requirements: &[Requirement]) -> HashMap<(usize, String), Ty> {
    let mut map = HashMap::new();
    for requirement in requirements {
        match requirement {
            Requirement::SameType(Subject::Member(param, name), ty) if !matches!(ty, Ty::Member(..)) => {
                map.insert((*param, name.clone()), ty.clone());
            }
            Requirement::SameType(Subject::Param(param), Ty::Member(other, name)) => {
                map.insert((*other, name.clone()), Ty::Param(*param));
            }
            // `T.A == U.B`: the later one is spelled as the earlier one.
            Requirement::SameType(Subject::Member(param, name), Ty::Member(other, other_name)) => {
                map.insert((*other, other_name.clone()), Ty::Member(*param, name.clone()));
            }
            _ => {}
        }
    }
    map
}

fn canonical(ty: &Ty, same_types: &HashMap<(usize, String), Ty>) -> Ty {
    match ty {
        Ty::Member(param, name) => same_types.get(&(*param, name.clone())).cloned().unwrap_or_else(|| ty.clone()),
        Ty::Optional(wrapped) => Ty::Optional(Box::new(canonical(wrapped, same_types))),
        Ty::Tuple(elements) => Ty::Tuple(elements.iter().map(|(label, element)| (label.clone(), canonical(element, same_types))).collect()),
        Ty::Nominal { module, path, args } => {
            Ty::Nominal { module: module.clone(), path: path.clone(), args: args.iter().map(|arg| canonical(arg, same_types)).collect() }
        }
        Ty::Any | Ty::AnyObject | Ty::Existential(_) | Ty::Param(_) => ty.clone(),
    }
}

pub fn mangle(decl: &Decl) -> Result<String, String> {
    let same_types = same_types(&decl.requirements);
    let mut m = Mangler { buffer: "$s".into(), ..Default::default() };
    if decl.context.is_empty() {
        m.module(&decl.module)?;
    } else {
        m.nominal(&decl.module, &decl.context)?;
    }
    m.identifier(&decl.name)?;
    match &decl.signature {
        Signature::Getter(ty) => {
            m.ty(&canonical(ty, &same_types))?;
            m.push("vg");
        }
        Signature::Function { params, result, is_async, throws } => {
            // Argument labels: nothing without parameters, `y` when none is labeled, else one entry each.
            if params.iter().any(|p| p.label.is_some()) {
                for param in params {
                    match &param.label {
                        Some(label) => m.identifier(label)?,
                        None => m.push("_"),
                    }
                }
            } else if !params.is_empty() {
                m.push("y");
            }
            let result = canonical(result, &same_types);
            if result == Ty::Tuple(Vec::new()) {
                m.push("y");
            } else {
                m.ty(&result)?;
            }
            let params: Vec<Param> = params
                .iter()
                .map(|p| Param { label: p.label.clone(), ty: canonical(&p.ty, &same_types), inout: p.inout, variadic: p.variadic })
                .collect();
            m.parameters(&params)?;
            if *is_async {
                m.push("Ya");
            }
            if *throws {
                m.push("K");
            }
            if decl.generic_params > 0 {
                for requirement in sorted_requirements(&decl.requirements) {
                    m.requirement(&requirement)?;
                }
                if decl.generic_params > 1 {
                    m.push("r");
                    m.push(&index(decl.generic_params - 1));
                }
                m.push("l");
            }
            m.push("F");
        }
    }
    if decl.is_static {
        m.push("Z");
    }
    Ok(m.buffer)
}
