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
    /// A `@preconcurrency` declaration: its symbol leaves out marker protocols.
    strip_markers: bool,
    /// How many protocols the signature requires of each generic parameter, and the protocols written for
    /// associated types (`T.Value(of P)`).
    protocol_counts: HashMap<GenericParam, usize>,
    /// The protocols (not marker protocols) the declaration requires of each associated type (`T.A: Q`).
    member_protocols: HashMap<(GenericParam, String), std::collections::HashSet<(String, String)>>,
    associated: HashMap<(GenericParam, String), Protocol>,
}

fn is_sendable(protocol: &Protocol) -> bool {
    protocol.module == "Swift" && protocol.name == "Sendable"
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
    /// `extension`: the context extension of a path element, mangled after its parent when that element is in `path`.
    fn nominal(&mut self, module: &str, path: &[(String, Kind)], extension: Option<&ContextExtension>) -> Result<(), String> {
        let extension = extension.filter(|extension| extension.at < path.len());
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
        let key = Key::Type(Ty::Nominal {
            module: module.to_string(),
            path: path.to_vec(),
            args: Vec::new(),
            split: Vec::new(),
            extension: extension.cloned().map(Box::new),
        });
        if self.try_substitution(&key) {
            return Ok(());
        }
        if path.len() == 1 {
            self.module(&crate::original_module(module, name).unwrap_or_else(|| module.to_string()))?;
        } else {
            self.nominal(module, &path[..path.len() - 1], extension)?;
        }
        if let Some(extension) = extension.filter(|extension| extension.at == path.len() - 1) {
            // ASTMangler::appendExtension: the extended type (just mangled), the module, the new requirements, `E`.
            self.module(&extension.module)?;
            if !extension.requirements.is_empty() {
                self.requirements(&extension.requirements)?;
                self.push(signature_end(extension.extended_params, extension.extended_constrained));
            }
            self.push("E");
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
        self.module(&crate::original_module(&protocol.module, &protocol.name).unwrap_or_else(|| protocol.module.clone()))?;
        self.identifier(&protocol.name)
    }

    fn ty(&mut self, ty: &Ty) -> Result<(), String> {
        match ty {
            Ty::Nominal { module, path, args, extension, .. } if args.is_empty() => self.nominal(module, path, extension.as_deref()),
            Ty::Nominal { module, path, args, split, extension } => {
                let key = Key::Type(ty.clone());
                if self.try_substitution(&key) {
                    return Ok(());
                }
                self.nominal(module, path, extension.as_deref())?;
                // One argument list per type level, outermost first (`y`, then `_` before each next level), the
                // arguments at the level that declares them: `PE.Pair<Int, String>` is `y_SiSSG`,
                // `Outer<Int>.E<String>` is `ySi_SSG`.
                let mut rest = args.as_slice();
                for l in 0..path.len() {
                    self.push(if l == 0 { "y" } else { "_" });
                    let count = if split.is_empty() { if l + 1 == path.len() { rest.len() } else { 0 } } else { split[l] };
                    let (level, after) = rest.split_at(count.min(rest.len()));
                    for arg in level {
                        self.ty(arg)?;
                    }
                    rest = after;
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
            // docs/ABI/Mangling.rst: `type-list 'QP'` (a pack); the list is a tuple's (`Si_SS`, empty `y`). Not a
            // substitution entry.
            Ty::Pack(elements) => {
                if elements.is_empty() {
                    self.push("y");
                }
                for (i, element) in elements.iter().enumerate() {
                    self.ty(element)?;
                    if i == 0 {
                        self.push("_");
                    }
                }
                self.push("QP");
                Ok(())
            }
            // docs/ABI/Mangling.rst: `type 'm'` (metatype); `type 'Xp'` for an existential's (`Any.Type`: `ypXp`).
            // Not a substitution entry (`T.Type` twice is `xmxm`).
            Ty::Metatype(instance) => {
                self.ty(instance)?;
                self.push(if matches!(**instance, Ty::Any | Ty::AnyObject | Ty::Existential(_)) { "Xp" } else { "m" });
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
            // The declaration's own opaque result type (only results keep `some P`: see the parser).
            Ty::Opaque(_) => {
                self.push("Qr");
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
                let nested = self.associated_path(*param, name)?;
                if *param == (GenericParam { depth: 0, index: 0 }) {
                    self.push(if nested { "QZ" } else { "Qz" });
                } else {
                    self.push(if nested { "QY" } else { "Qy" });
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

    /// ASTMangler::appendAssocType: an associated type's names, each with its protocol where needed, and `_` after
    /// the first when there are several (`assoc-type-list`); whether there are.
    fn associated_path(&mut self, param: GenericParam, path: &str) -> Result<bool, String> {
        let mut end = 0;
        for (i, name) in path.split('.').enumerate() {
            end += name.len() + usize::from(i > 0);
            self.identifier(name)?;
            self.associated_protocol(param, &path[..end])?;
            if i == 0 && path.contains('.') {
                self.push("_");
            }
        }
        Ok(path.contains('.'))
    }

    /// ASTMangler::appendRequirement
    /// ASTMangler::appendAssociatedTypeName: an associated type of a parameter that the signature requires to
    /// conform to more than one protocol is followed by the protocol that declares it.
    fn associated_protocol(&mut self, param: GenericParam, name: &str) -> Result<(), String> {
        let several = match name.rsplit_once('.') {
            None => self.protocol_counts.get(&param).copied().unwrap_or(0) >= 2,
            // A nested one's base (`T.A` of `T.A.B`): the protocols the declaration requires of it, and the one that
            // declares `B` (written `(of P)`; its own requirement signature's are not known here).
            Some((base, _)) => {
                let mut protocols = self.member_protocols.get(&(param, base.to_string())).cloned().unwrap_or_default();
                if let Some(protocol) = self.associated.get(&(param, name.to_string())) {
                    protocols.insert((protocol.module.clone(), protocol.name.clone()));
                }
                protocols.len() >= 2
            }
        };
        if !several {
            return Ok(());
        }
        let protocol = self.associated.get(&(param, name.to_string())).cloned().ok_or_else(|| {
            format!("`{name}` belongs to a parameter with several protocols: write the one that declares it, `.{name}(of P)`")
        })?;
        self.nominal(&protocol.module, &[(protocol.name.clone(), Kind::Protocol)], None)
    }

    /// A signature's requirements in canonical order; under `@preconcurrency`, without the marker protocol
    /// `Sendable` (ASTMangler's AllowMarkerProtocols).
    fn requirements(&mut self, requirements: &[Requirement]) -> Result<(), String> {
        for requirement in sorted_requirements(requirements) {
            if self.strip_markers && matches!(&requirement, Requirement::Conforms(_, protocol) if is_sendable(protocol)) {
                continue;
            }
            self.requirement(&requirement)?;
        }
        Ok(())
    }

    fn requirement(&mut self, requirement: &Requirement) -> Result<(), String> {
        let (subject, plain, with_substitution, on_associated_type, on_nested_associated_type) = match requirement {
            Requirement::Conforms(subject, protocol) => {
                self.protocol(protocol)?;
                (subject, "R", "RQ", "Rp", "RP")
            }
            Requirement::SameType(subject, ty) => {
                self.ty(ty)?;
                (subject, "Rs", "RS", "Rt", "RT")
            }
            // docs/ABI/Mangling.rst: `type 'Rb' GENERIC-PARAM-INDEX` (base class); `'Rl' GENERIC-PARAM-INDEX
            // LAYOUT-CONSTRAINT` with `C` for a class. The parser allows them on generic parameters only.
            Requirement::Superclass(Subject::Param(param), ty) => {
                self.ty(ty)?;
                self.push("Rb");
                self.generic_param_index(*param);
                return Ok(());
            }
            Requirement::AnyObject(Subject::Param(param)) => {
                self.push("Rl");
                self.generic_param_index(*param);
                self.push("C");
                return Ok(());
            }
            Requirement::Superclass(Subject::Member(..), _) | Requirement::AnyObject(Subject::Member(..)) => {
                return Err("a class constraint on an associated type is not supported".into());
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
                    let nested = self.associated_path(*param, name)?;
                    self.add_substitution(key);
                    self.push(if nested { on_nested_associated_type } else { on_associated_type });
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
            .map(|ty| Param { label: None, ty: ty.clone(), inout: false, variadic: false, isolated: false, ownership: None, compile_time_const: false })
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
            self.nominal("_Concurrency", &[("MainActor".into(), Kind::Class)], None)?;
            self.push("Yc");
        }
        if function.sending_result {
            self.push("YT");
        }
        let escaping = !in_parameter || function.escaping;
        self.push(match (function.convention, function.autoclosure, escaping) {
            (Convention::C, ..) => "XC",
            (Convention::Block, ..) => "XB",
            (Convention::Swift, true, _) if !in_parameter => return Err("`@autoclosure` is only for a parameter".into()),
            (Convention::Swift, true, false) => "XK",
            (Convention::Swift, true, true) => "XA",
            (Convention::Swift, false, false) => "XE",
            (Convention::Swift, false, true) => "c",
        });
        Ok(())
    }

    fn parameters(&mut self, params: &[Param]) -> Result<(), String> {
        let element = |m: &mut Mangler, p: &Param| -> Result<(), String> {
            match &p.ty {
                Ty::Function(function) => m.function(function, true)?,
                ty => m.ty(ty)?,
            }
            if p.compile_time_const {
                m.push("Yt");
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
            // Same subject: a layout, then a superclass, before protocols (observed from swiftc: `RlzC…Rz`,
            // `…CRbz…Rz`).
            Requirement::AnyObject(subject) => (subject, -2, Default::default()),
            Requirement::Superclass(subject, _) => (subject, -1, Default::default()),
            Requirement::Conforms(subject, protocol) => (subject, 0, (protocol.module.clone(), protocol.name.clone())),
            Requirement::SameType(subject, _) => (subject, 1, Default::default()),
        };
        let subject = match subject {
            Subject::Param(param) => (0, *param, String::new()),
            // A shorter path first (`T.C` before `T.A.B`: compareDependentTypes compares bases first).
            Subject::Member(param, name) => (1 + name.matches('.').count(), *param, name.clone()),
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
        Ty::Metatype(instance) => Ty::Metatype(Box::new(canonical(instance, same_types))),
        Ty::Pack(elements) => Ty::Pack(elements.iter().map(|element| canonical(element, same_types)).collect()),
        Ty::Tuple(elements) => Ty::Tuple(elements.iter().map(|(label, element)| (label.clone(), canonical(element, same_types))).collect()),
        Ty::Nominal { module, path, args, split, extension } => Ty::Nominal {
            module: module.clone(),
            path: path.clone(),
            args: args.iter().map(|arg| canonical(arg, same_types)).collect(),
            split: split.clone(),
            extension: extension.clone(),
        },
        Ty::Function(function) => Ty::Function(Box::new(FunctionTy {
            params: function.params.iter().map(|param| canonical(param, same_types)).collect(),
            result: canonical(&function.result, same_types),
            sendable: function.sendable && !same_types.strip_concurrency,
            main_actor: function.main_actor && !same_types.strip_concurrency,
            ..(**function).clone()
        })),
        Ty::Existential(protocol) if same_types.strip_concurrency && is_sendable(protocol) => Ty::Any,
        Ty::Any | Ty::AnyObject | Ty::Existential(_) | Ty::Opaque(_) => ty.clone(),
    }
}

fn all_same_types(decl: &Decl) -> SameTypes {
    let mut map = same_types(
        decl.extension
            .iter()
            .flat_map(|e| e.requirements.iter())
            .chain(decl.context_extension.iter().flat_map(|e| e.requirements.iter()))
            .chain(decl.requirements.iter()),
    );
    map.strip_concurrency = decl.preconcurrency;
    map
}

/// How a generic signature that adds no parameters to its context ends (ASTMangler::gatherGenericSignatureParts):
/// with a lone unconstrained context parameter the whole signature is mangled (`l`); otherwise only the new
/// requirements are, after an empty parameter count (`rl`).
fn contextual_signature_end(decl: &Decl, inside_constrained_extension: bool) -> &'static str {
    signature_end(decl.context_params, decl.context_is_constrained || inside_constrained_extension)
}

fn signature_end(context_params: usize, context_is_constrained: bool) -> &'static str {
    if context_params == 1 && !context_is_constrained { "l" } else { "rl" }
}

/// A protocol's descriptor symbol (`$sSHMp` for Hashable).
pub(crate) fn protocol_descriptor(protocol: &Protocol) -> Result<String, String> {
    let mut m = Mangler { buffer: "$s".into(), ..Default::default() };
    m.protocol(protocol)?;
    m.push("Mp");
    Ok(m.buffer)
}

/// The protocols (not marker protocols) the declaration requires of each associated type.
fn member_protocols(decl: &Decl) -> HashMap<(GenericParam, String), std::collections::HashSet<(String, String)>> {
    let mut protocols: HashMap<_, std::collections::HashSet<_>> = HashMap::new();
    for requirement in all_requirements(decl) {
        if let Requirement::Conforms(Subject::Member(param, name), protocol) = requirement {
            if !protocol.is_marker() {
                protocols.entry((*param, name.clone())).or_default().insert((protocol.module.clone(), protocol.name.clone()));
            }
        }
    }
    protocols
}

/// Every requirement of the declaration's generic signature: its context's, its extensions', its own.
fn all_requirements(decl: &Decl) -> impl Iterator<Item = &Requirement> {
    decl.context_requirements
        .iter()
        .chain(decl.context_extension.iter().flat_map(|e| e.requirements.iter()))
        .chain(decl.extension.iter().flat_map(|e| e.requirements.iter()))
        .chain(decl.requirements.iter())
}

/// The protocols (not marker protocols) the declaration's whole generic signature requires of each parameter.
fn protocol_counts(decl: &Decl) -> HashMap<GenericParam, usize> {
    let mut conformances = std::collections::HashSet::new();
    if let Some((_, Kind::Protocol)) = decl.context.last() {
        conformances.insert((GenericParam { depth: 0, index: 0 }, None));
    }
    for requirement in all_requirements(decl) {
        match requirement {
            Requirement::Conforms(Subject::Param(param), protocol) if !protocol.is_marker() => {
                conformances.insert((*param, Some(protocol.clone())));
            }
            _ => {}
        }
    }
    let mut counts = HashMap::new();
    for (param, _) in conformances {
        *counts.entry(param).or_insert(0) += 1;
    }
    counts
}

pub fn mangle(decl: &Decl) -> Result<String, String> {
    let same_types = all_same_types(decl);
    let mut m = Mangler {
        buffer: "$s".into(),
        strip_markers: decl.preconcurrency,
        protocol_counts: protocol_counts(decl),
        member_protocols: member_protocols(decl),
        associated: decl.associated_protocols.iter().cloned().collect(),
        ..Default::default()
    };
    if decl.context.is_empty() {
        m.module(&decl.module)?;
    } else {
        m.nominal(&decl.module, &decl.context, decl.context_extension.as_ref())?;
    }
    let mut in_constrained_extension = false;
    if let Some(extension) = &decl.extension {
        // ASTMangler::appendExtension: the type, the extension's module, the constraints it adds, `E`.
        m.module(&extension.module)?;
        if !extension.requirements.is_empty() {
            m.requirements(&extension.requirements)?;
            m.push(contextual_signature_end(decl, false));
            in_constrained_extension = true;
        }
        m.push("E");
    }
    let is_init = decl.name == "init";
    m.in_initializer = is_init;
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
                .map(|p| Param { label: p.label.clone(), ty: canonical(&p.ty, &same_types), inout: p.inout, variadic: p.variadic, isolated: p.isolated, ownership: p.ownership, compile_time_const: p.compile_time_const })
                .collect();
            m.parameters(&params)?;
            m.push("c");
            if decl.generic_params > 0 {
                m.requirements(&decl.requirements)?;
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
            let this = canonical(&decl.context_type(), &same_types);
            m.push("y");
            m.ty(&this)?;
            if !payload.is_empty() {
                // `(Payload) -> Enum`: the payload as the function's parameters, labels included.
                let params: Vec<Param> = payload
                    .iter()
                    .map(|(label, ty)| Param { label: label.clone(), ty: ty.clone(), inout: false, variadic: false, isolated: false, ownership: None, compile_time_const: false })
                    .collect();
                m.parameters(&params)?;
                m.push("c");
            }
            m.ty(&this)?;
            m.push("m");
            if decl.context_params > 0 {
                m.requirements(&decl.requirements)?;
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
                .map(|p| Param { label: p.label.clone(), ty: canonical(&p.ty, &same_types), inout: p.inout, variadic: p.variadic, isolated: p.isolated, ownership: p.ownership, compile_time_const: p.compile_time_const })
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
                    m.requirements(&decl.requirements)?;
                    if decl.generic_params > 1 {
                        m.push("r");
                        m.push(&index(decl.generic_params - 1));
                    }
                    m.push("lu");
                } else if !decl.requirements.is_empty() {
                    // A `where` clause on the type's parameters only (`init() where V == Bool`: `…ycSbRszlufC`).
                    m.requirements(&decl.requirements)?;
                    m.push(contextual_signature_end(decl, in_constrained_extension));
                    m.push("u");
                }
                m.push("fC");
                return Ok(m.buffer);
            }
            if decl.generic_params > 0 {
                m.requirements(&decl.requirements)?;
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
                m.requirements(&decl.requirements)?;
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
        Ty::Param(_) | Ty::Member(..) | Ty::Opaque(_) => false,
        Ty::Optional(wrapped) | Ty::Metatype(wrapped) => is_concrete(wrapped),
        Ty::Pack(elements) => elements.iter().all(is_concrete),
        Ty::Tuple(elements) => elements.iter().all(|(_, element)| is_concrete(element)),
        Ty::Nominal { args, .. } => args.iter().all(is_concrete),
        Ty::Function(function) => function.params.iter().all(is_concrete) && is_concrete(&function.result),
        Ty::Any | Ty::AnyObject | Ty::Existential(_) => true,
    }
}

/// The mangled name of `ty`, a type in `decl`'s signature, with generic parameters bound to `bound`; `None` if it is
/// still not concrete (an associated type).
pub(crate) fn bound_type_name(decl: &Decl, ty: &Ty, bound: &[(GenericParam, Ty)]) -> Result<Option<String>, String> {
    let ty = bound_type(decl, ty, bound);
    if !is_concrete(&ty) {
        return Ok(None);
    }
    type_name(&ty).map(Some)
}

/// `ty` with the bound generic parameters (and the declaration's same-type requirements) substituted.
pub(crate) fn bound_type(decl: &Decl, ty: &Ty, bound: &[(GenericParam, Ty)]) -> Ty {
    let mut same_types = all_same_types(decl);
    for (param, concrete) in bound {
        same_types.params.insert(*param, concrete.clone());
    }
    // A same-type requirement may name a parameter (`S.Element == T`): bound too.
    for member in same_types.members.values_mut() {
        if let Ty::Param(param) = member {
            if let Some((_, concrete)) = bound.iter().find(|(p, _)| p == param) {
                *member = concrete.clone();
            }
        }
    }
    canonical(ty, &same_types)
}

/// The symbols that find an associated type of a conformance to `protocol` (`swift_getAssociatedTypeWitness`): the
/// protocol's requirement base descriptor (`$sSTTL`) and the associated type's descriptor (`$s7ElementSTTl`).
pub(crate) fn associated_type_symbols(protocol: &Protocol, name: &str) -> Result<(String, String), String> {
    let mut base = Mangler { buffer: "$s".into(), ..Default::default() };
    base.protocol(protocol)?;
    base.push("TL");
    let mut descriptor = Mangler { buffer: "$s".into(), ..Default::default() };
    descriptor.identifier(name)?;
    descriptor.protocol(protocol)?;
    descriptor.push("Tl");
    Ok((base.buffer, descriptor.buffer))
}

pub(crate) fn is_concrete_type(ty: &Ty) -> bool {
    is_concrete(ty)
}

/// A type's mangled name, as the runtime resolves type names (no `$s`).
pub(crate) fn type_name(ty: &Ty) -> Result<String, String> {
    let mut m = Mangler::default();
    m.ty(ty)?;
    Ok(m.buffer)
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
