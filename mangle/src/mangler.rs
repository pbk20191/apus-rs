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
    /// Mangling an initializer's parameters, which are consumed by default.
    in_initializer: bool,
    /// The declaration's generic enclosing type nested `n` levels deep: its bound form ends with `n` `_` (one per
    /// non-generic level below the generic one).
    nested_generic: Option<(Vec<(String, Kind)>, usize)>,
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
            // The concurrency library's ABI name is Swift (`Sc` only prefixes its abbreviated types).
            "Swift" | "_Concurrency" => self.push("s"),
            "__C" => self.push("So"),
            _ => self.identifier(module)?,
        }
        Ok(())
    }

    /// ASTMangler::appendAnyGenericType for a non-generic (or unbound) nominal type.
    fn nominal(&mut self, module: &str, path: &[(String, Kind)]) -> Result<(), String> {
        let (name, kind) = path.last().ok_or("empty type path")?;
        if path.len() == 1 {
            let standard = match (module, kind) {
                ("Swift", Kind::Protocol) => names::standard_protocol(name).map(|subst| (subst, *kind)),
                ("_Concurrency", Kind::Protocol) => names::concurrency_protocol(name).map(|subst| (subst, *kind)),
                ("Swift", _) => names::standard_type(name),
                ("_Concurrency", _) => names::concurrency_type(name),
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
                if let Some((nested, levels)) = &self.nested_generic {
                    if nested == path {
                        self.push(&"_".repeat(*levels));
                    }
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
            Ty::Param(GenericParam { depth: 0, index: 0 }) => {
                self.push("x");
                Ok(())
            }
            Ty::Param(param) => {
                self.push("q");
                self.param_index(*param);
                Ok(())
            }
            // Function types are mangled in full every time too.
            Ty::Function(function) => self.function(function, false),
            Ty::Member(param, name) => {
                let key = Key::Type(ty.clone());
                if self.try_substitution(&key) {
                    return Ok(());
                }
                self.identifier(name)?;
                if *param == (GenericParam { depth: 0, index: 0 }) {
                    self.push("Qz");
                } else {
                    self.push("Qy");
                    self.param_index(*param);
                }
                self.add_substitution(key);
                Ok(())
            }
        }
    }

    /// A generic parameter other than the first: its index at depth 0, `d<depth><index>` deeper.
    fn param_index(&mut self, param: GenericParam) {
        if param.depth == 0 {
            self.push(&index(param.index - 1));
        } else {
            self.push("d");
            self.push(&index(param.depth - 1));
            self.push(&index(param.index));
        }
    }

    /// The generic parameter a requirement is about: `z` for the first, else as `param_index`.
    fn generic_param_index(&mut self, param: GenericParam) {
        if param == (GenericParam { depth: 0, index: 0 }) {
            self.push("z");
        } else {
            self.param_index(param);
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
    /// `result params [Ya] [Yb] [K] [ScMYc] (c | XE)`; `in_parameter`: the closure is a parameter, so it escapes only
    /// with `@escaping`.
    fn function(&mut self, function: &FunctionTy, in_parameter: bool) -> Result<(), String> {
        if function.result == Ty::Tuple(Vec::new()) {
            self.push("y");
        } else {
            self.ty(&function.result)?;
        }
        let params: Vec<Param> = function
            .params
            .iter()
            .map(|ty| Param { label: None, ty: ty.clone(), inout: false, variadic: false, isolated: false, ownership: None })
            .collect();
        self.parameters(&params)?;
        if function.is_async {
            self.push("Ya");
        }
        if function.sendable {
            self.push("Yb");
        }
        if function.throws {
            self.push("K");
        }
        if function.main_actor {
            self.nominal("_Concurrency", &[("MainActor".into(), Kind::Class)])?;
            self.push("Yc");
        }
        self.push(if in_parameter && !function.escaping { "XE" } else { "c" });
        Ok(())
    }

    fn parameters(&mut self, params: &[Param]) -> Result<(), String> {
        let element = |m: &mut Mangler, p: &Param| -> Result<(), String> {
            match &p.ty {
                Ty::Function(function) => m.function(function, true)?,
                ty => m.ty(ty)?,
            }
            if p.isolated {
                m.push("Yi");
            }
            // `consuming`/`borrowing` show only when not the default (an initializer's parameters are consumed, a
            // function's borrowed): `n`, `h`. `__owned`/`__shared` always show.
            match (p.ownership, m.in_initializer) {
                (Some(Ownership::Consuming), false) | (Some(Ownership::Owned), _) => m.push("n"),
                (Some(Ownership::Borrowing), true) | (Some(Ownership::Shared), _) => m.push("h"),
                _ => {}
            }
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
pub(crate) fn sorted_requirements(requirements: &[Requirement]) -> Vec<Requirement> {
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
fn same_types<'a>(requirements: impl Iterator<Item = &'a Requirement>) -> SameTypes {
    let mut map = SameTypes::default();
    for requirement in requirements {
        // A generic parameter fixed to a concrete type.
        if let Requirement::SameType(Subject::Param(param), ty) = requirement {
            if !matches!(ty, Ty::Member(..) | Ty::Param(_)) {
                map.params.insert(*param, ty.clone());
                continue;
            }
        }
        match requirement {
            Requirement::SameType(Subject::Member(param, name), ty) if !matches!(ty, Ty::Member(..)) => {
                map.members.insert((*param, name.clone()), ty.clone());
            }
            Requirement::SameType(Subject::Param(param), Ty::Member(other, name)) => {
                map.members.insert((*other, name.clone()), Ty::Param(*param));
            }
            // `T.A == U.B`: the later one is spelled as the earlier one.
            Requirement::SameType(Subject::Member(param, name), Ty::Member(other, other_name)) => {
                map.members.insert((*other, other_name.clone()), Ty::Member(*param, name.clone()));
            }
            _ => {}
        }
    }
    map
}

/// What same-type requirements pin down: the signature is mangled with these replaced.
#[derive(Default)]
struct SameTypes {
    params: HashMap<GenericParam, Ty>,
    members: HashMap<(GenericParam, String), Ty>,
    /// A `@preconcurrency` declaration: function types drop `@Sendable` and global actors in its symbol.
    strip_concurrency: bool,
}

fn canonical(ty: &Ty, same_types: &SameTypes) -> Ty {
    match ty {
        Ty::Param(param) => same_types.params.get(param).cloned().unwrap_or_else(|| ty.clone()),
        Ty::Member(param, name) => same_types.members.get(&(*param, name.clone())).cloned().unwrap_or_else(|| ty.clone()),
        Ty::Optional(wrapped) => Ty::Optional(Box::new(canonical(wrapped, same_types))),
        Ty::Tuple(elements) => Ty::Tuple(elements.iter().map(|(label, element)| (label.clone(), canonical(element, same_types))).collect()),
        Ty::Nominal { module, path, args } => {
            Ty::Nominal { module: module.clone(), path: path.clone(), args: args.iter().map(|arg| canonical(arg, same_types)).collect() }
        }
        Ty::Function(function) => Ty::Function(Box::new(FunctionTy {
            params: function.params.iter().map(|param| canonical(param, same_types)).collect(),
            result: canonical(&function.result, same_types),
            sendable: function.sendable && !same_types.strip_concurrency,
            main_actor: function.main_actor && !same_types.strip_concurrency,
            ..(**function).clone()
        })),
        Ty::Any | Ty::AnyObject | Ty::Existential(_) => ty.clone(),
    }
}

fn all_same_types(decl: &Decl) -> SameTypes {
    let mut map = same_types(decl.extension.iter().flat_map(|e| e.requirements.iter()).chain(decl.requirements.iter()));
    map.strip_concurrency = decl.preconcurrency;
    map
}

/// How a generic signature that adds no parameters to its context ends (ASTMangler::gatherGenericSignatureParts):
/// with a lone unconstrained context parameter the whole signature is mangled (`l`); otherwise only the new
/// requirements are, after an empty parameter count (`rl`).
fn contextual_signature_end(decl: &Decl, inside_constrained_extension: bool) -> &'static str {
    if decl.context_params == 1 && !decl.context_is_constrained && !inside_constrained_extension { "l" } else { "rl" }
}

/// A protocol's descriptor symbol (`$sSHMp` for Hashable).
pub(crate) fn protocol_descriptor(protocol: &Protocol) -> Result<String, String> {
    let mut m = Mangler { buffer: "$s".into(), ..Default::default() };
    m.protocol(protocol)?;
    m.push("Mp");
    Ok(m.buffer)
}

pub fn mangle(decl: &Decl) -> Result<String, String> {
    let same_types = all_same_types(decl);
    let mut m = Mangler { buffer: "$s".into(), ..Default::default() };
    if decl.context.is_empty() {
        m.module(&decl.module)?;
    } else {
        m.nominal(&decl.module, &decl.context)?;
    }
    let mut in_constrained_extension = false;
    if let Some(extension) = &decl.extension {
        // ASTMangler::appendExtension: the type, the extension's module, the constraints it adds, `E`.
        m.module(&extension.module)?;
        if !extension.requirements.is_empty() {
            for requirement in sorted_requirements(&extension.requirements) {
                m.requirement(&requirement)?;
            }
            m.push(contextual_signature_end(decl, false));
            in_constrained_extension = true;
        }
        m.push("E");
    }
    let is_init = decl.name == "init";
    m.in_initializer = is_init;
    m.nested_generic = decl.context_generic_level.map(|level| (decl.context.clone(), decl.context.len() - 1 - level));
    if let Some(fixity) = decl.operator {
        // Each operator character as a letter (docs/ABI/Mangling.rst: operator-char), then the fixity.
        let letters: String = decl
            .name
            .chars()
            .map(|c| match c {
                '&' => 'a', '@' => 'c', '/' => 'd', '=' => 'e', '>' => 'g', '<' => 'l', '*' => 'm', '!' => 'n',
                '|' => 'o', '+' => 'p', '?' => 'q', '%' => 'r', '-' => 's', '~' => 't', '^' => 'x', '.' => 'z',
                other => other,
            })
            .collect();
        // An operator name is an identifier for substitutions.
        let key = Key::Identifier(decl.name.clone());
        if !m.try_substitution(&key) {
            m.add_substitution(key);
            m.push(&format!("{}{letters}", letters.len()));
        }
        m.push(fixity);
    } else if !is_init && !matches!(decl.signature, Signature::Subscript { .. }) {
        m.identifier(&decl.name)?;
    }
    match &decl.signature {
        Signature::Getter(ty) => {
            m.ty(&canonical(ty, &same_types))?;
            m.push("vg");
        }
        Signature::Setter(ty) => {
            m.ty(&canonical(ty, &same_types))?;
            m.push("vs");
        }
        Signature::Subscript { params, result, setter } => {
            // Like an initializer: labels, the function type (`c`), generic (`…lu`), then the accessor.
            if params.iter().any(|p| p.label.is_some()) {
                for param in params {
                    match &param.label {
                        Some(label) => m.identifier(label)?,
                        None => m.push("_"),
                    }
                }
            } else {
                m.push("y");
            }
            m.ty(&canonical(result, &same_types))?;
            let params: Vec<Param> = params
                .iter()
                .map(|p| Param { label: p.label.clone(), ty: canonical(&p.ty, &same_types), inout: p.inout, variadic: p.variadic, isolated: p.isolated, ownership: p.ownership })
                .collect();
            m.parameters(&params)?;
            m.push("c");
            if decl.generic_params > 0 {
                for requirement in sorted_requirements(&decl.requirements) {
                    m.requirement(&requirement)?;
                }
                if decl.generic_params > 1 {
                    m.push("r");
                    m.push(&index(decl.generic_params - 1));
                }
                m.push("lu");
            }
            m.push(if *setter { "is" } else { "ig" });
        }
        Signature::Case(payload) => {
            // The case as a function: `(Enum.Type) -> Enum`, or `(Enum.Type) -> (Payload) -> Enum`, in the enum's
            // generic signature; then the tag symbol's `WC`.
            let path: Vec<(String, Kind)> = decl.context.clone();
            let args = (0..decl.context_params).map(|index| Ty::Param(GenericParam { depth: 0, index })).collect();
            let this = Ty::Nominal { module: decl.module.clone(), path, args };
            m.push("y");
            m.ty(&this)?;
            if !payload.is_empty() {
                // `(Payload) -> Enum`: the payload as the function's parameters, labels included.
                let params: Vec<Param> = payload
                    .iter()
                    .map(|(label, ty)| Param { label: label.clone(), ty: ty.clone(), inout: false, variadic: false, isolated: false, ownership: None })
                    .collect();
                m.parameters(&params)?;
                m.push("c");
            }
            m.ty(&this)?;
            m.push("m");
            if decl.context_params > 0 {
                if decl.context_params > 1 {
                    m.push("r");
                    m.push(&index(decl.context_params - 1));
                }
                m.push("l");
            }
            m.push("FWC");
            return Ok(m.buffer);
        }
        Signature::Function { params, result, is_async, throws, typed_error } => {
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
                .map(|p| Param { label: p.label.clone(), ty: canonical(&p.ty, &same_types), inout: p.inout, variadic: p.variadic, isolated: p.isolated, ownership: p.ownership })
                .collect();
            m.parameters(&params)?;
            if *is_async {
                m.push("Ya");
            }
            match typed_error {
                Some(error) => {
                    m.ty(&canonical(error, &same_types))?;
                    m.push("YK");
                }
                None if *throws => m.push("K"),
                None => {}
            }
            if is_init {
                // The allocating initializer: a function type (`c`), generic (`…lu`) when it has parameters of its
                // own, then `fC`.
                m.push("c");
                if decl.generic_params > 0 {
                    for requirement in sorted_requirements(&decl.requirements) {
                        m.requirement(&requirement)?;
                    }
                    if decl.generic_params > 1 {
                        m.push("r");
                        m.push(&index(decl.generic_params - 1));
                    }
                    m.push("lu");
                }
                m.push("fC");
                return Ok(m.buffer);
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
            } else if !decl.requirements.is_empty() {
                // A `where` clause on a member that adds no parameters of its own.
                if decl.context_params == 0 {
                    return Err("a `where` clause needs generic parameters".into());
                }
                for requirement in sorted_requirements(&decl.requirements) {
                    m.requirement(&requirement)?;
                }
                m.push(contextual_signature_end(decl, in_constrained_extension));
            }
            m.push("F");
        }
    }
    if decl.is_static {
        m.push("Z");
    }
    Ok(m.buffer)
}

fn is_concrete(ty: &Ty) -> bool {
    match ty {
        Ty::Param(_) | Ty::Member(..) => false,
        Ty::Optional(wrapped) => is_concrete(wrapped),
        Ty::Tuple(elements) => elements.iter().all(|(_, element)| is_concrete(element)),
        Ty::Nominal { args, .. } => args.iter().all(is_concrete),
        Ty::Function(function) => function.params.iter().all(is_concrete) && is_concrete(&function.result),
        Ty::Any | Ty::AnyObject | Ty::Existential(_) => true,
    }
}

/// The mangled name of the declaration's result type (a function's result, a getter's type), as the runtime
/// resolves type names; `None` when it depends on a generic parameter.
pub fn result_type(decl: &Decl) -> Result<Option<String>, String> {
    let same_types = all_same_types(decl);
    let ty = match &decl.signature {
        Signature::Function { result, .. } => canonical(result, &same_types),
        Signature::Getter(ty) | Signature::Subscript { result: ty, setter: false, .. } => canonical(ty, &same_types),
        Signature::Setter(_) | Signature::Subscript { setter: true, .. } => Ty::Tuple(Vec::new()),
        Signature::Case(_) => return Err("an enum case has no result type".into()),
    };
    if !is_concrete(&ty) {
        return Ok(None);
    }
    let mut m = Mangler::default();
    m.ty(&ty)?;
    Ok(Some(m.buffer))
}
