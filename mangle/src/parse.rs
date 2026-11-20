//! Parses the declaration syntax: Swift's own, with the kind of each non-standard type spelled out.
//!
//! ```text
//! [static] [(extension in Module [where …])] Module.Type(kind)[<T, …>]….name[<U: P>]([label|_]: Type, …)
//!     [async] [throws] [-> Type] [where …]
//! [static] [(extension in Module [where …])] Module.Type(kind)[<T, …>]….name: Type { get }
//! ```
//! Types: `Int`, `String`, … (standard library names), `[T]`, `[K: V]`, `T?`, `(A, b: B)`, `Any`, `AnyObject`,
//! `any Proto`, `Name(struct|class|enum)` (in the declaration's module), `Module.Name(kind)`,
//! `Outer(kind).Inner(kind)`, `Name(kind)<Args>`, generic parameters and `T.Associated`.

use crate::ast::*;
use crate::names;

pub struct Parser<'a> {
    text: &'a str,
    position: usize,
    module: String,
    /// The module a bare name belongs to: the extension's when there is one, else the declaration's.
    home: String,
    generics: Vec<(String, GenericParam)>,
    /// `T.Value(of P)`: the protocol that declares an associated type.
    associated: Vec<((GenericParam, String), Protocol)>,
    /// The constrained extensions a type in the signature may name, with the types they extend: the one an
    /// enclosing type is declared in, and the declaration's own (for a type declared beside it).
    known_extensions: Vec<(String, Vec<(String, Kind)>, ContextExtension)>,
    /// Kinds of names written without one, and whether protocols are class-bound (a symbol graph's).
    resolver: Option<&'a dyn crate::KindResolver>,
}

type Parsed<T> = Result<T, String>;

/// What a `T: …` item requires.
enum Constraint {
    Protocol(Protocol),
    Superclass(Ty),
    AnyObject,
}

impl Constraint {
    fn on_param(self, param: GenericParam) -> Requirement {
        match self {
            Constraint::Protocol(protocol) => Requirement::Conforms(Subject::Param(param), protocol),
            Constraint::Superclass(ty) => Requirement::Superclass(Subject::Param(param), ty),
            Constraint::AnyObject => Requirement::AnyObject(Subject::Param(param)),
        }
    }

    fn on(self, subject: Subject) -> Parsed<Requirement> {
        match (self, subject) {
            (Constraint::Protocol(protocol), subject) => Ok(Requirement::Conforms(subject, protocol)),
            (constraint, Subject::Param(param)) => Ok(constraint.on_param(param)),
            (_, Subject::Member(_, name)) => Err(format!("a class constraint on an associated type (`{name}`) is not supported")),
        }
    }
}

/// One dotted segment of the declaration's path, with what was written after its name.
struct Segment {
    name: String,
    kind: Option<Kind>,
    /// `<T: P, U>`: parameter names with their inline constraints.
    generics: Option<Vec<(String, Vec<Constraint>)>>,
    /// `(extension in Module [where …])` before it: its module and where its `where` clause starts.
    extension: Option<(String, Option<usize>)>,
}

impl<'a> Parser<'a> {
    fn rest(&self) -> &'a str {
        &self.text[self.position..]
    }

    fn skip_whitespace(&mut self) {
        self.position += self.rest().len() - self.rest().trim_start().len();
    }

    fn eat(&mut self, token: &str) -> bool {
        self.skip_whitespace();
        if self.rest().starts_with(token) {
            self.position += token.len();
            true
        } else {
            false
        }
    }

    /// Eats `word` only as a whole word.
    fn eat_keyword(&mut self, word: &str) -> bool {
        self.skip_whitespace();
        let rest = self.rest();
        let ends = rest[word.len().min(rest.len())..].chars().next().map_or(true, |c| !(c.is_alphanumeric() || c == '_'));
        if rest.starts_with(word) && ends {
            self.position += word.len();
            true
        } else {
            false
        }
    }

    fn expect(&mut self, token: &str) -> Parsed<()> {
        if self.eat(token) { Ok(()) } else { Err(self.error(&format!("expected `{token}`"))) }
    }

    /// Eats `ch` only if it comes right here, with no whitespace before it.
    fn eat_here(&mut self, ch: char) -> bool {
        if self.rest().starts_with(ch) {
            self.position += ch.len_utf8();
            true
        } else {
            false
        }
    }

    /// Eats a `.` that continues a dotted path (not the `...` of a variadic parameter).
    fn eat_path_dot(&mut self) -> bool {
        let mut chars = self.rest().chars();
        if chars.next() == Some('.') && chars.next().is_some_and(|c| c.is_alphabetic() || c == '_') {
            self.position += 1;
            true
        } else {
            false
        }
    }

    /// A `.` before a path segment declared in an extension: `Outer.(extension in M) Inner`.
    fn eat_extension_dot(&mut self) -> bool {
        if self.rest().starts_with(".(extension ") {
            self.position += 1;
            true
        } else {
            false
        }
    }

    fn error(&self, what: &str) -> String {
        let rest = self.rest().trim_start();
        if rest.is_empty() { format!("{what} at the end") } else { format!("{what} at `{rest}`") }
    }

    fn peek_identifier(&mut self) -> Option<&'a str> {
        self.skip_whitespace();
        let rest = self.rest();
        let first = rest.chars().next()?;
        if !(first.is_alphabetic() || first == '_') {
            return None;
        }
        let end = rest.find(|c: char| !(c.is_alphanumeric() || c == '_')).unwrap_or(rest.len());
        Some(&rest[..end])
    }

    fn identifier(&mut self) -> Parsed<String> {
        let name = self.peek_identifier().ok_or_else(|| self.error("expected a name"))?;
        self.position += name.len();
        Ok(name.to_string())
    }

    /// `(struct)`, `(class)`, `(enum)`, `(actor)` or `(protocol)` right after a type name.
    fn kind_suffix(&mut self) -> Option<Kind> {
        let kinds = [
            ("(struct)", Kind::Struct),
            ("(class)", Kind::Class),
            ("(actor)", Kind::Actor),
            ("(enum)", Kind::Enum),
            ("(protocol)", Kind::Protocol),
            ("(typedef struct)", Kind::TypedefStruct),
            ("(typedef class)", Kind::TypedefClass),
        ];
        for (text, kind) in kinds {
            if self.rest().starts_with(text) {
                self.position += text.len();
                return Some(kind);
            }
        }
        None
    }

    /// The kind of a type named without one, from the resolver.
    fn resolve(&self, module: &str, path: &[&str]) -> Option<Kind> {
        Some(match self.resolver?.kind(module, path)? {
            crate::TypeKind::Struct => Kind::Struct,
            crate::TypeKind::Class => Kind::Class,
            crate::TypeKind::Actor => Kind::Actor,
            crate::TypeKind::Enum => Kind::Enum,
            crate::TypeKind::Protocol => Kind::Protocol,
        })
    }

    fn generic_param(&self, name: &str) -> Option<GenericParam> {
        self.generics.iter().find(|(known, _)| known == name).map(|(_, param)| *param)
    }

    fn protocol(&mut self) -> Parsed<Protocol> {
        let mut protocol = self.protocol_name()?;
        // `(AnyObject)`: a class-bound protocol (`protocol P: AnyObject`), whose conforming types are classes.
        protocol.class_bound = self.eat("(AnyObject)");
        if !protocol.class_bound && protocol.module != "Swift" && protocol.module != "_Concurrency" {
            protocol.class_bound = self.resolver.and_then(|r| r.class_bound(&protocol.module, &protocol.name)).unwrap_or(false);
        }
        Ok(protocol)
    }

    fn protocol_name(&mut self) -> Parsed<Protocol> {
        let first = self.identifier()?;
        // `any P.Type` is `(any P).Type`.
        if !self.at_metatype_suffix() && self.eat_path_dot() {
            return Ok(Protocol { module: first, name: self.identifier()?, class_bound: false });
        }
        let module = if names::standard_protocol(&first).is_some() || names::swift_protocol(&first) {
            "Swift"
        } else if names::concurrency_protocol(&first).is_some() {
            "_Concurrency"
        } else {
            &self.home
        };
        Ok(Protocol { module: module.to_string(), name: first, class_bound: false })
    }

    /// `.Type` or `.Protocol` next: a metatype of the type before it, not a member type.
    fn at_metatype_suffix(&self) -> bool {
        let rest = self.rest();
        [".Type", ".Protocol"].iter().any(|suffix| {
            rest.starts_with(suffix) && !rest[suffix.len()..].starts_with(|c: char| c.is_alphanumeric() || c == '_' || c == '(')
        })
    }

    fn ty(&mut self) -> Parsed<Ty> {
        let mut ty = self.primary_type()?;
        while self.at_metatype_suffix() {
            if self.eat(".Protocol") {
                return Err(self.error("protocol metatypes (`P.Protocol`) are not supported"));
            }
            self.position += ".Type".len();
            ty = Ty::Metatype(Box::new(ty));
        }
        while self.eat("?") {
            ty = Ty::Optional(Box::new(ty));
        }
        if self.rest().trim_start().starts_with("->") {
            return Err(self.error("function types are not supported"));
        }
        Ok(ty)
    }

    fn primary_type(&mut self) -> Parsed<Ty> {
        let (mut escaping, mut sendable, mut main_actor, mut autoclosure) = (false, false, false, false);
        let mut convention = None;
        loop {
            if self.eat("@escaping") {
                escaping = true;
            } else if self.eat("@Sendable") {
                sendable = true;
            } else if self.eat("@MainActor") {
                main_actor = true;
            } else if self.eat("@autoclosure") {
                autoclosure = true;
            } else if self.eat("@convention") {
                self.expect("(")?;
                let name = self.identifier()?;
                convention = Some(match name.as_str() {
                    "swift" => Convention::Swift,
                    "c" => Convention::C,
                    "block" => Convention::Block,
                    "thin" => return Err("`@convention(thin)` is not supported: no public API takes one".into()),
                    _ => return Err(format!("unknown function convention `{name}`: write `c`, `block` or `swift`")),
                });
                self.expect(")")?;
            } else {
                break;
            }
        }
        let attributed = escaping || sendable || main_actor || autoclosure || convention.is_some();
        let convention = convention.unwrap_or(Convention::Swift);
        if attributed && !self.rest().trim_start().starts_with('(') {
            return Err(self.error("expected a function type after its attributes"));
        }
        if self.eat("[") {
            let element = self.ty()?;
            let ty = if self.eat(":") {
                let value = self.ty()?;
                swift_generic("Dictionary", vec![element, value])
            } else {
                swift_generic("Array", vec![element])
            };
            self.expect("]")?;
            return Ok(ty);
        }
        if self.eat("(") {
            let mut elements = Vec::new();
            while !self.eat(")") {
                if !elements.is_empty() {
                    self.expect(",")?;
                }
                let checkpoint = self.position;
                let label = match self.peek_identifier() {
                    Some(name) => {
                        self.position += name.len();
                        if self.eat(":") { Some(name.to_string()) } else { None }
                    }
                    None => None,
                };
                if label.is_none() {
                    self.position = checkpoint;
                }
                if self.eat_keyword("sending") {
                    return Err("a function type's `sending` parameter is not supported: Swift hands it over +1".into());
                }
                elements.push((label, self.ty()?));
            }
            let is_async = self.eat_keyword("async");
            let throws = self.eat_keyword("throws");
            if throws && self.rest().starts_with('(') {
                return Err(self.error("typed throws in a function type is not supported"));
            }
            if is_async || throws || self.eat("->") {
                if (is_async || throws) && !self.eat("->") {
                    return Err(self.error("expected `->`"));
                }
                if elements.iter().any(|(label, _)| label.is_some()) {
                    return Err("a function type's parameters take no labels".into());
                }
                if convention != Convention::Swift && (is_async || throws) {
                    return Err("a C function or block type cannot be async or throw".into());
                }
                if convention != Convention::Swift && autoclosure {
                    return Err("an `@autoclosure` is a Swift closure".into());
                }
                let params = elements.into_iter().map(|(_, ty)| ty).collect();
                let sending_result = self.eat_keyword("sending");
                let result = self.ty()?;
                return Ok(Ty::Function(Box::new(FunctionTy {
                    params,
                    result,
                    is_async,
                    throws,
                    sendable,
                    main_actor,
                    escaping,
                    autoclosure,
                    convention,
                    sending_result,
                })));
            }
            if attributed {
                return Err(self.error("expected a function type after its attributes"));
            }
            // `(T)` is just `T`.
            if let [(None, _)] = elements.as_slice() {
                return Ok(elements.pop().unwrap().1);
            }
            return Ok(Ty::Tuple(elements));
        }
        if self.eat_keyword("any") {
            return Ok(Ty::Existential(self.unmarked(Self::protocol)?));
        }
        if self.eat_keyword("some") {
            return Ok(Ty::Opaque(self.unmarked(Self::protocol_list)?));
        }
        if self.eat_keyword("AnyObject") {
            return Ok(Ty::AnyObject);
        }
        if self.eat_keyword("Any") {
            return Ok(Ty::Any);
        }
        self.named_type()
    }

    fn named_type(&mut self) -> Parsed<Ty> {
        let first = self.identifier()?;
        if let Some(param) = self.generic_param(&first) {
            if !self.at_metatype_suffix() && self.eat_path_dot() {
                let name = self.identifier()?;
                self.associated_protocol(param, &name)?;
                return Ok(Ty::Member(param, name));
            }
            return Ok(Ty::Param(param));
        }
        let kind = self.kind_suffix();
        let mut segments = vec![(first, kind, self.type_arguments()?)];
        // A segment declared in another module's extension: `Swift.Duration(struct).(extension in Foundation) …`.
        let mut extension = None;
        loop {
            if self.eat_extension_dot() {
                if extension.is_some() {
                    return Err(self.error("only one type of a path may be declared in an extension"));
                }
                let (module, where_at) = self.extension_marker()?.expect("`.(extension` was seen");
                let requirements = self.extension_requirements(where_at)?;
                extension = Some((segments.len(), module, requirements));
            } else if self.at_metatype_suffix() || !self.eat_path_dot() {
                break;
            }
            let name = self.identifier()?;
            let kind = self.kind_suffix();
            segments.push((name, kind, self.type_arguments()?));
        }

        // A first segment without a kind, followed by more, is a module name.
        let (module, path) = if segments.len() > 1 && segments[0].1.is_none() && segments[0].2.is_empty() {
            (segments[0].0.clone(), &segments[1..])
        } else {
            (self.home.clone(), &segments[..])
        };
        // `Outer<A>.Inner<B>`: the arguments of every level, outermost first, and how many each takes.
        let args: Vec<Ty> = path.iter().flat_map(|(_, _, args)| args.iter().cloned()).collect();
        let counts: Vec<usize> = path.iter().map(|(_, _, args)| args.len()).collect();
        let split = if counts[..counts.len() - 1].iter().all(|&count| count == 0) { Vec::new() } else { counts };
        // A lone name without a kind must be one the standard library defines.
        if let [(name, None, _)] = path {
            let explicit_module = segments.len() > 1;
            if name == "Void" && !explicit_module {
                return Ok(Ty::Tuple(Vec::new()));
            }
            let known = [
                ("Swift", names::standard_type(name).map(|(_, kind)| kind).or(names::swift_type(name))),
                ("_Concurrency", names::concurrency_type(name).map(|(_, kind)| kind)),
            ];
            for (known_module, kind) in known {
                if let Some(kind) = kind {
                    if !explicit_module || module == known_module {
                        if name == "Optional" && args.len() == 1 {
                            return Ok(Ty::Optional(Box::new(args.into_iter().next().unwrap())));
                        }
                        return Ok(Ty::Nominal { module: known_module.into(), path: vec![(name.clone(), kind)], args, split, extension: None });
                    }
                }
            }
            if let Some(kind) = self.resolve(&module, &[name.as_str()]) {
                return Ok(Ty::Nominal { module, path: vec![(name.clone(), kind)], args, split, extension: None });
            }
            return Err(format!("unknown type `{name}`: write `{name}(struct)`, `{name}(class)` or `{name}(enum)`"));
        }
        let mut resolved = Vec::new();
        for (i, (name, kind, _)) in path.iter().enumerate() {
            let names: Vec<&str> = path[..=i].iter().map(|(n, _, _)| n.as_str()).collect();
            let kind = kind.or_else(|| self.resolve(&module, &names)).ok_or_else(|| needs_kind(name))?;
            if matches!(kind, Kind::TypedefStruct | Kind::TypedefClass) && (module != "__C" || path.len() != 1) {
                return Err(format!("`{name}`: a typedef kind names an imported C type, `__C.{name}(typedef …)`"));
            }
            resolved.push((name.clone(), kind));
        }
        let extension = match extension {
            Some((at, extension_module, requirements)) => {
                let at = at - (segments.len() - path.len());
                if at == 0 {
                    return Err(self.error("an extension needs a type to extend"));
                }
                if requirements.is_empty() {
                    Some(Box::new(ContextExtension {
                        at,
                        module: extension_module,
                        requirements,
                        extended_params: 0,
                        extended_constrained: false,
                    }))
                } else {
                    // A constrained one is the declaration's own context extension, written the same way: its
                    // requirements are in terms of the enclosing type's parameters, which only the context declares.
                    let known = self.known_extensions.iter().find(|(known_module, extended, known)| {
                        *known_module == module
                            && known.at == at
                            && known.module == extension_module
                            && known.requirements == requirements
                            && extended.get(..at) == resolved.get(..at)
                    });
                    match known {
                        Some((_, _, known)) => Some(Box::new(known.clone())),
                        None => {
                            return Err("a constrained extension in a type's path must be the declaration's own \
                                        enclosing one"
                                .into())
                        }
                    }
                }
            }
            None => None,
        };
        Ok(Ty::Nominal { module, path: resolved, args, split, extension })
    }

    /// `<A, B>` right after a type's name (and kind), or none.
    fn type_arguments(&mut self) -> Parsed<Vec<Ty>> {
        if !self.eat_here('<') {
            return Ok(Vec::new());
        }
        let mut args = vec![self.type_argument()?];
        while self.eat(",") {
            args.push(self.type_argument()?);
        }
        self.expect(">")?;
        Ok(args)
    }

    /// A type argument, or a pack of them, `Pack{A, B}` (for a variadic generic type's `each` parameter).
    fn type_argument(&mut self) -> Parsed<Ty> {
        self.skip_whitespace();
        if !self.rest().starts_with("Pack{") {
            return self.ty();
        }
        self.position += "Pack{".len();
        let mut elements = Vec::new();
        if !self.eat("}") {
            elements.push(self.ty()?);
            while self.eat(",") {
                elements.push(self.ty()?);
            }
            self.expect("}")?;
        }
        Ok(Ty::Pack(elements))
    }

    fn subject(&mut self) -> Parsed<Subject> {
        let name = self.identifier()?;
        let param = self.generic_param(&name).ok_or_else(|| format!("`{name}` is not a generic parameter"))?;
        if self.eat_path_dot() {
            let name = self.identifier()?;
            self.associated_protocol(param, &name)?;
            return Ok(Subject::Member(param, name));
        }
        Ok(Subject::Param(param))
    }

    /// `(of P)` after an associated type's name: the protocol that declares it, which its symbol names when the
    /// parameter has several protocols.
    fn associated_protocol(&mut self, param: GenericParam, name: &str) -> Parsed<()> {
        if !self.rest().starts_with("(of ") {
            return Ok(());
        }
        self.position += 1;
        self.eat_keyword("of");
        let protocol = self.protocol()?;
        self.expect(")")?;
        let key = (param, name.to_string());
        match self.associated.iter().find(|(known, _)| *known == key) {
            Some((_, known)) if *known != protocol => Err(format!("`{name}` is declared by one protocol")),
            Some(_) => Ok(()),
            None => {
                self.associated.push((key, protocol));
                Ok(())
            }
        }
    }

    /// Protocols of an existential or opaque type, where Swift's lowering of a class-bound one is not supported.
    fn unmarked<T>(&mut self, parse: fn(&mut Self) -> Parsed<T>) -> Parsed<T> {
        let start = self.position;
        let parsed = parse(self)?;
        if self.text[start..self.position].contains("(AnyObject)") {
            return Err(self.error("a class-bound protocol `(AnyObject)` is supported only as a generic constraint"));
        }
        Ok(parsed)
    }

    /// `P & Q`
    fn protocol_list(&mut self) -> Parsed<Vec<Protocol>> {
        let mut protocols = vec![self.protocol()?];
        while self.eat("&") {
            protocols.push(self.protocol()?);
        }
        Ok(protocols)
    }

    /// The requirements of a `where` clause, up to its end.
    fn where_clause(&mut self, into: &mut Vec<Requirement>) -> Parsed<()> {
        loop {
            let subject = self.subject()?;
            if self.eat("==") {
                let ty = self.ty()?;
                // Canonical form anchors a same-type requirement on the earlier, shorter type.
                into.push(match (subject, ty) {
                    (Subject::Member(param, name), Ty::Param(other)) => Requirement::SameType(Subject::Param(other), Ty::Member(param, name)),
                    (Subject::Member(param, name), Ty::Member(other, other_name)) if other < param => {
                        Requirement::SameType(Subject::Member(other, other_name), Ty::Member(param, name))
                    }
                    (subject, ty) => Requirement::SameType(subject, ty),
                });
            } else {
                self.expect(":")?;
                for constraint in self.constraint_list()? {
                    into.push(constraint.on(subject.clone())?);
                }
            }
            if !self.eat(",") {
                return Ok(());
            }
        }
    }

    /// `<T: P & Q, U>` (the `<` already eaten): names with their inline constraints.
    /// `P & Q`, where an item may also be a superclass (`Base(class)`, written with its class kind) or `AnyObject`.
    fn constraint_list(&mut self) -> Parsed<Vec<Constraint>> {
        let mut constraints = Vec::new();
        loop {
            self.skip_whitespace();
            // The item's text, up to the next `&`, `,` or closing `>`/`)` outside brackets.
            let mut depth = 0i32;
            let end = self
                .rest()
                .char_indices()
                .find(|&(_, c)| {
                    depth += match c { '<' | '(' | '[' => 1, '>' | ')' | ']' => -1, _ => 0 };
                    depth < 0 || (depth == 0 && matches!(c, '&' | ','))
                })
                .map_or(self.rest().len(), |(i, _)| i);
            let item = &self.rest()[..end];
            constraints.push(if self.eat_keyword("AnyObject") || self.eat("Swift.AnyObject") {
                Constraint::AnyObject
            } else if item.contains("(class)") || item.contains("(typedef class)") || item.contains("(actor)") {
                Constraint::Superclass(self.ty()?)
            } else {
                Constraint::Protocol(self.protocol()?)
            });
            if !self.eat("&") {
                return Ok(constraints);
            }
        }
    }

    fn generic_clause(&mut self) -> Parsed<Vec<(String, Vec<Constraint>)>> {
        let mut params = Vec::new();
        loop {
            let name = self.identifier()?;
            let constraints = if self.eat(":") { self.constraint_list()? } else { Vec::new() };
            params.push((name, constraints));
            if !self.eat(",") {
                break;
            }
        }
        self.expect(">")?;
        Ok(params)
    }

    fn segment(&mut self) -> Parsed<Segment> {
        let extension = self.extension_marker()?;
        let name = self.identifier()?;
        let kind = self.kind_suffix();
        let generics = if self.eat_here('<') { Some(self.generic_clause()?) } else { None };
        Ok(Segment { name, kind, generics, extension })
    }

    /// `(extension in Module [where …])`: its `where` clause is parsed once the generic parameters are known.
    fn extension_marker(&mut self) -> Parsed<Option<(String, Option<usize>)>> {
        self.skip_whitespace();
        if !self.rest().starts_with('(') {
            return Ok(None);
        }
        self.position += 1;
        if !(self.eat_keyword("extension") && self.eat_keyword("in")) {
            return Err(self.error("expected `(extension in Module)`"));
        }
        let module = self.identifier()?;
        let where_at = if self.eat_keyword("where") { Some(self.position) } else { None };
        // The `)` that closes the extension, past any in its `where` clause (`T == Big(struct)`).
        let mut depth = 0;
        let close = self
            .rest()
            .char_indices()
            .find(|&(_, c)| {
                depth += match c { '(' => 1, ')' => -1, _ => 0 };
                depth < 0
            })
            .map(|(i, _)| i)
            .ok_or_else(|| self.error("expected `)`"))?;
        self.position += close + 1;
        Ok(Some((module, where_at)))
    }

    /// The requirements of an extension's `where` clause that starts at `at`.
    fn extension_requirements(&mut self, at: Option<usize>) -> Parsed<Vec<Requirement>> {
        let mut requirements = Vec::new();
        if let Some(at) = at {
            let resume = std::mem::replace(&mut self.position, at);
            self.where_clause(&mut requirements)?;
            if !self.eat(")") {
                return Err(self.error("unexpected text in the extension's `where` clause"));
            }
            self.position = resume;
        }
        Ok(requirements)
    }

    /// Registers generic parameters at `depth` and returns the requirements written inline.
    fn declare_generics(&mut self, clause: Vec<(String, Vec<Constraint>)>, depth: usize) -> Vec<Requirement> {
        let mut requirements = Vec::new();
        for (index, (name, constraints)) in clause.into_iter().enumerate() {
            let param = GenericParam { depth, index };
            self.generics.push((name, param));
            requirements.extend(constraints.into_iter().map(|constraint| constraint.on_param(param)));
        }
        requirements
    }

    /// `get [async] [throws] }` or `set }` after a `{`: whether it is the setter, and the getter's effects.
    fn accessor(&mut self) -> Parsed<(bool, bool, bool)> {
        let accessor = if self.eat_keyword("get") {
            let is_async = self.eat_keyword("async");
            (false, is_async, self.eat_keyword("throws"))
        } else if self.eat_keyword("set") {
            (true, false, false)
        } else {
            return Err(self.error("expected `get` or `set`"));
        };
        self.expect("}")?;
        Ok(accessor)
    }

    /// An operator's name after the `.` before it: `+`, `==`, `..<`.
    fn operator_name(&mut self) -> Option<String> {
        let rest = self.rest();
        let rest = rest.strip_prefix('.')?;
        let length = rest.find(|c: char| !OPERATOR_CHARS.contains(c)).unwrap_or(rest.len());
        if length == 0 {
            return None;
        }
        let name = rest[..length].to_string();
        self.position += 1 + length;
        Some(name)
    }

    fn declaration(&mut self) -> Parsed<Decl> {
        // Isolation attributes: they change no symbol, only how apus calls the declaration.
        let mut main_actor = false;
        let mut nonisolated = false;
        let mut nonsending = false;
        let mut mutating = false;
        let mut fixity = None;
        let mut consumes_self = false;
        let mut preconcurrency = false;
        loop {
            if self.eat("@preconcurrency") {
                preconcurrency = true;
                continue;
            }
            if self.eat_keyword("mutating") {
                mutating = true;
            } else if self.eat_keyword("consuming") || self.eat_keyword("__consuming") {
                consumes_self = true;
            } else if self.eat_keyword("borrowing") {
                // The default for `self`.
            } else if self.eat_keyword("prefix") {
                fixity = Some("op");
            } else if self.eat_keyword("postfix") {
                fixity = Some("oP");
            } else if self.eat("@MainActor") {
                main_actor = true;
            } else if self.eat("nonisolated(nonsending)") {
                nonsending = true;
            } else if self.eat_keyword("nonisolated") {
                nonisolated = true;
            } else {
                break;
            }
        }
        let is_case = self.eat_keyword("case");
        if is_case && (main_actor || nonisolated || nonsending) {
            return Err("an enum case takes no isolation attributes".into());
        }
        let is_static = !is_case && self.eat_keyword("static");
        // `static prefix …` as Swift writes it.
        if self.eat_keyword("prefix") {
            fixity = Some("op");
        } else if self.eat_keyword("postfix") {
            fixity = Some("oP");
        }

        // The declaration's own extension, before its module.
        let extension = self.extension_marker()?;

        self.skip_whitespace();
        let mut segments = vec![self.segment()?];
        if segments[0].kind.is_some() || segments[0].generics.is_some() || segments[0].extension.is_some() {
            return Err("expected `Module.name…`: a declaration starts with its module".into());
        }
        self.module = segments[0].name.clone();
        self.home = extension.as_ref().map_or_else(|| self.module.clone(), |(module, _)| module.clone());
        while self.eat_path_dot() || self.eat_extension_dot() {
            segments.push(self.segment()?);
        }
        let operator = self.operator_name();
        if let Some(name) = &operator {
            segments.push(Segment { name: name.clone(), kind: None, generics: None, extension: None });
        } else if fixity.is_some() {
            return Err("`prefix` and `postfix` mark an operator".into());
        }
        if segments.len() < 2 {
            return Err("expected `Module.name…`".into());
        }
        let member = segments.pop().unwrap();
        if member.kind.is_some() {
            return Err(format!("`{}` is the declaration's name: it takes no kind", member.name));
        }
        if member.extension.is_some() {
            return Err("a member's own extension goes before the declaration: `(extension in M) M.Type.member`".into());
        }

        // The enclosing types.
        let mut context = Vec::new();
        let mut context_params = 0;
        let mut context_generic_level = None;
        let mut context_is_constrained = false;
        let mut context_requirements = Vec::new();
        let mut context_extension = None;
        let lone_context = segments.len() == 2;
        for segment in segments.drain(1..) {
            if let Some((module, where_at)) = segment.extension {
                if context.is_empty() {
                    return Err(format!("`{}` has no enclosing type to extend", segment.name));
                }
                if context_extension.is_some() {
                    return Err("only one enclosing type may be declared in an extension".into());
                }
                let requirements = self.extension_requirements(where_at)?;
                context_extension = Some(ContextExtension {
                    at: context.len(),
                    module,
                    requirements,
                    extended_params: context_params,
                    extended_constrained: context_is_constrained,
                });
            }
            let names: Vec<&str> = context.iter().map(|(n, _): &(String, Kind)| n.as_str()).chain([segment.name.as_str()]).collect();
            let kind = match segment.kind {
                Some(kind) => kind,
                None if lone_context => standard_kind(&self.module, &segment.name)
                    .or_else(|| self.resolve(&self.module, &names))
                    .ok_or_else(|| needs_kind(&segment.name))?,
                None => self.resolve(&self.module, &names).ok_or_else(|| needs_kind(&segment.name))?,
            };
            if matches!(kind, Kind::TypedefStruct | Kind::TypedefClass) && (self.module != "__C" || !context.is_empty()) {
                return Err(format!("`{}`: a typedef kind names an imported C type, `__C.{}(typedef …)`", segment.name, segment.name));
            }
            if kind == Kind::Protocol {
                if segment.generics.is_some() {
                    return Err(format!("protocol `{}` takes no generic parameters; its conforming type is `Self`", segment.name));
                }
                self.generics.push(("Self".into(), GenericParam { depth: 0, index: 0 }));
                context_params = 1;
                context_is_constrained = true; // Self: Protocol
            } else if let Some(clause) = segment.generics {
                if context_params > 0 {
                    return Err("only one enclosing type may be generic".into());
                }
                context_params = clause.len();
                context_generic_level = Some(context.len());
                context_requirements = self.declare_generics(clause, 0);
                context_is_constrained =
                    !context_requirements.is_empty() || standard_type_is_constrained(&self.module, &segment.name);
            }
            context.push((segment.name, kind));
        }

        if let Some(extension) = &context_extension {
            self.known_extensions.push((self.module.clone(), context.clone(), extension.clone()));
        }

        // The declaration's own generic parameters sit one level below the enclosing type's.
        let own_depth = usize::from(context_params > 0);
        let (generic_params, mut requirements) = match member.generics {
            Some(clause) => (clause.len(), self.declare_generics(clause, own_depth)),
            None => (0, Vec::new()),
        };

        let extension = match extension {
            Some((module, where_at)) => {
                let requirements = self.extension_requirements(where_at)?;
                if !requirements.is_empty() && !context.is_empty() {
                    // A type declared in the same extension as the member: nested in the member's context.
                    self.known_extensions.push((
                        self.module.clone(),
                        context.clone(),
                        ContextExtension {
                            at: context.len(),
                            module: module.clone(),
                            requirements: requirements.clone(),
                            extended_params: context_params,
                            extended_constrained: context_is_constrained,
                        },
                    ));
                }
                Some(Extension { module, requirements })
            }
            None => None,
        };

        let mut accessor_effects = (false, false);
        let signature = if is_case {
            if extension.is_some() || generic_params > 0 || context.last().is_none_or(|(_, kind)| *kind != Kind::Enum) {
                return Err("expected `case Module.Type(enum).name[(payload…)]`".into());
            }
            // A case's symbol carries the enum's whole generic signature: its constraints must be written out.
            if context_is_constrained && context_requirements.is_empty() {
                return Err("write the enum's generic constraints: `Enum<T: P>.case`".into());
            }
            // The enum's signature: its own constraints and those of the extension it is declared in.
            requirements = context_requirements.clone();
            requirements.extend(context_extension.iter().flat_map(|e: &ContextExtension| e.requirements.iter().cloned()));
            let mut payload = Vec::new();
            if self.eat("(") {
                while !self.eat(")") {
                    if !payload.is_empty() {
                        self.expect(",")?;
                    }
                    let start = self.position;
                    let label = match self.identifier() {
                        Ok(label) if self.eat(":") => Some(label),
                        _ => {
                            self.position = start;
                            None
                        }
                    };
                    // A payload is stored: a function in it escapes.
                    payload.push((label, match self.ty()? {
                        Ty::Function(mut function) => {
                            function.escaping = true;
                            Ty::Function(function)
                        }
                        ty => ty,
                    }));
                }
            }
            Signature::Case(payload)
        } else if self.eat("(") {
            let mut params = Vec::new();
            while !self.eat(")") {
                if !params.is_empty() {
                    self.expect(",")?;
                }
                let label = if self.eat_keyword("_") { None } else { Some(self.identifier()?) };
                self.expect(":")?;
                let compile_time_const = self.eat_keyword("_const");
                let inout = self.eat_keyword("inout");
                let isolated = self.eat_keyword("isolated");
                let ownership = [
                    ("consuming", Ownership::Consuming),
                    ("borrowing", Ownership::Borrowing),
                    ("__owned", Ownership::Owned),
                    ("__shared", Ownership::Shared),
                    ("sending", Ownership::Owned),
                ]
                    .into_iter()
                    .find(|(word, _)| self.eat_keyword(word))
                    .map(|(_, ownership)| ownership);
                let ty = self.ty()?;
                let variadic = self.eat("...");
                // A default value changes nothing in the symbol; `#isolation` is the only one understood.
                if self.eat("=") && !self.eat("#isolation") {
                    return Err(self.error("only `= #isolation` is understood as a default value"));
                }
                params.push(Param { label, ty, inout, variadic, isolated, ownership, compile_time_const });
            }
            let is_async = self.eat_keyword("async");
            let mut throws = self.eat_keyword("throws");
            // `throws(E)`; `throws(any Error)` is plain `throws`, `throws(Never)` no `throws` at all.
            let typed_error = if throws && self.eat("(") {
                let ty = self.ty()?;
                self.expect(")")?;
                match ty {
                    Ty::Existential(ref p) if p.name == "Error" && p.module == "Swift" => None,
                    Ty::Nominal { ref module, ref path, ref args, .. } if module == "Swift" && args.is_empty() && matches!(path.as_slice(), [(name, _)] if name == "Never") => {
                        throws = false;
                        None
                    }
                    ty => Some(ty),
                }
            } else {
                None
            };
            let result = if self.eat("->") {
                // A declaration's `sending` result changes neither its symbol nor how it is returned.
                self.eat_keyword("sending");
                self.ty()?
            } else if member.name == "init" {
                // An initializer returns the type itself (written out only for a failable one: `-> T?`).
                let args = (0..context_params).map(|index| Ty::Param(GenericParam { depth: 0, index })).collect();
                if let Some((_, Kind::Protocol)) = context.last() {
                    // A protocol's initializer makes a `Self`.
                    Ty::Param(GenericParam { depth: 0, index: 0 })
                } else {
                    Ty::Nominal {
                        module: self.module.clone(),
                        path: context.clone(),
                        split: Ty::split(context.len(), context_generic_level.unwrap_or(context.len() - 1), context_params),
                        args,
                        extension: context_extension.clone().map(Box::new),
                    }
                }
            } else {
                Ty::Tuple(Vec::new())
            };
            Signature::Function { params, result, is_async, throws, typed_error }
        } else if self.eat(":") {
            let ty = self.ty()?;
            self.expect("{")?;
            let (setter, is_async, throws) = self.accessor()?;
            accessor_effects = (is_async, throws);
            if setter {
                Signature::Setter(ty)
            } else {
                Signature::Getter(ty)
            }
        } else {
            return Err(self.error("expected `(` or `:` after the declaration's name"));
        };
        if self.eat_keyword("where") {
            self.where_clause(&mut requirements)?;
        }
        let signature = match signature {
            Signature::Function { params, result, is_async, throws, .. } if member.name == "subscript" => {
                if is_async || throws {
                    return Err("a subscript's effects go on its getter: `subscript(…) -> T { get async throws }`".into());
                }
                if context.is_empty() {
                    return Err("a subscript belongs to a type: `Module.Type(kind).subscript(…) -> T { get }`".into());
                }
                self.expect("{")?;
                let (setter, is_async, throws) = self.accessor()?;
                accessor_effects = (is_async, throws);
                Signature::Subscript { params, result, setter }
            }
            signature => signature,
        };
        if mutating && (is_static || !matches!(signature, Signature::Function { .. }) || context.last().is_none_or(|(_, kind)| !matches!(kind, Kind::Struct | Kind::Enum | Kind::TypedefStruct))) {
            return Err("`mutating` marks a method of a struct or enum".into());
        }
        let operator = match (&operator, &signature) {
            (None, _) => None,
            (Some(_), Signature::Function { params, .. }) => match (fixity, params.len()) {
                (None, 2) => Some("oi"),
                (Some(fixity), 1) => Some(fixity),
                _ => return Err("an infix operator takes two parameters, a `prefix` or `postfix` one takes one".into()),
            },
            (Some(_), _) => return Err("an operator is a function".into()),
        };
        // `some P` parameters are anonymous generic parameters after the declared ones; a `some P` result is the
        // declaration's opaque result type, one at most.
        let own_depth = usize::from(context_params > 0);
        let mut generic_params = generic_params;
        let mut signature = signature;
        if let Signature::Function { params, .. } | Signature::Subscript { params, .. } = &mut signature {
            for param in params.iter_mut() {
                param.ty = anonymous_generics(&param.ty, own_depth, &mut generic_params, &mut requirements);
            }
        }
        let opaque_results = match &signature {
            Signature::Function { result, typed_error, .. } => {
                if typed_error.as_ref().is_some_and(|e| e.opaque_count() > 0) {
                    return Err("`some` is not supported in a typed throws error".into());
                }
                result.opaque_count()
            }
            Signature::Getter(ty) | Signature::Subscript { result: ty, setter: false, .. } => ty.opaque_count(),
            Signature::Setter(ty) | Signature::Subscript { result: ty, .. } => ty.opaque_count() * 2,
            Signature::Case(payload) => payload.iter().map(|(_, ty)| ty.opaque_count() * 2).sum(),
        };
        if opaque_results > 1 {
            return Err("only one `some P` result is supported (not in setters, cases or several places)".into());
        }
        if member.name == "init" {
            match context.last() {
                None => return Err("`init` belongs to a type: `Module.Type(kind).init(…)`".into()),
                _ if is_static => return Err("an initializer is not `static`".into()),
                _ if !matches!(signature, Signature::Function { .. }) => return Err("expected `init(…)`".into()),
                _ if generic_params == 0 && !requirements.is_empty() => {
                    return Err("a `where` clause on an initializer needs generic parameters of its own".into())
                }
                _ => {}
            }
        }
        self.skip_whitespace();
        if !self.rest().is_empty() {
            return Err(self.error("unexpected text"));
        }
        let mut generic_names: Vec<(usize, String)> =
            self.generics.iter().filter(|(name, param)| param.depth == own_depth && name != "Self").map(|(name, param)| (param.index, name.clone())).collect();
        generic_names.sort();
        let mut generic_names: Vec<String> = generic_names.into_iter().map(|(_, name)| name).collect();
        generic_names.resize(generic_params, "_".into());
        let mut context_names: Vec<(usize, String)> = match own_depth {
            0 => Vec::new(),
            _ => self.generics.iter().filter(|(name, param)| param.depth == 0 && name != "Self").map(|(name, param)| (param.index, name.clone())).collect(),
        };
        context_names.sort();
        Ok(Decl {
            generic_names,
            context_names: context_names.into_iter().map(|(_, name)| name).collect(),
            module: self.module.clone(),
            context,
            context_extension,
            context_params,
            context_generic_level,
            preconcurrency,
            context_is_constrained,
            extension,
            name: member.name,
            is_static,
            main_actor,
            nonisolated,
            nonsending,
            consumes_self,
            accessor_async: accessor_effects.0,
            accessor_throws: accessor_effects.1,
            operator,
            mutating,
            generic_params,
            context_requirements,
            associated_protocols: std::mem::take(&mut self.associated),
            requirements,
            signature,
        })
    }
}

fn needs_kind(name: &str) -> String {
    format!("`{name}` needs its kind: `{name}(struct)`, `{name}(class)`, `{name}(enum)` or `{name}(protocol)`")
}

/// The kind of a standard library type or protocol named without one.
fn standard_kind(module: &str, name: &str) -> Option<Kind> {
    match module {
        "Swift" => names::standard_type(name)
            .map(|(_, kind)| kind)
            .or(names::swift_type(name))
            .or((names::standard_protocol(name).is_some() || names::swift_protocol(name)).then_some(Kind::Protocol)),
        "_Concurrency" => {
            names::concurrency_type(name).map(|(_, kind)| kind).or(names::concurrency_protocol(name).map(|_| Kind::Protocol))
        }
        _ => None,
    }
}

/// Single-parameter standard types whose declaration constrains the parameter (`Set<Element: Hashable>`).
fn standard_type_is_constrained(module: &str, name: &str) -> bool {
    module == "Swift" && matches!(name, "Set" | "Range" | "ClosedRange")
}

fn swift_generic(name: &str, args: Vec<Ty>) -> Ty {
    Ty::Nominal { module: "Swift".into(), path: vec![(name.into(), Kind::Struct)], args, split: Vec::new(), extension: None }
}

/// `ty` with each `some P` replaced by a new generic parameter at `depth`, conforming to its protocols.
fn anonymous_generics(ty: &Ty, depth: usize, count: &mut usize, requirements: &mut Vec<Requirement>) -> Ty {
    let mut again = |ty: &Ty| anonymous_generics(ty, depth, count, requirements);
    match ty {
        Ty::Opaque(protocols) => {
            let param = GenericParam { depth, index: *count };
            *count += 1;
            requirements.extend(protocols.iter().map(|p| Requirement::Conforms(Subject::Param(param), p.clone())));
            Ty::Param(param)
        }
        Ty::Optional(wrapped) => Ty::Optional(Box::new(again(wrapped))),
        Ty::Metatype(instance) => Ty::Metatype(Box::new(again(instance))),
        Ty::Pack(elements) => Ty::Pack(elements.iter().map(&mut again).collect()),
        Ty::Tuple(elements) => Ty::Tuple(elements.iter().map(|(label, e)| (label.clone(), again(e))).collect()),
        Ty::Nominal { module, path, args, split, extension } => Ty::Nominal {
            module: module.clone(),
            path: path.clone(),
            args: args.iter().map(again).collect(),
            split: split.clone(),
            extension: extension.clone(),
        },
        Ty::Function(f) => Ty::Function(Box::new(FunctionTy {
            params: f.params.iter().map(&mut again).collect(),
            result: again(&f.result),
            ..(**f).clone()
        })),
        other => other.clone(),
    }
}

/// The characters of an ASCII operator name.
const OPERATOR_CHARS: &str = "/=-+!*%<>&|^~?.";

pub fn parse(text: &str) -> Parsed<Decl> {
    parse_resolving(text, crate::kind_resolver())
}

pub fn parse_resolving<'a>(text: &'a str, resolver: Option<&'a dyn crate::KindResolver>) -> Parsed<Decl> {
    Parser {
        text,
        position: 0,
        module: String::new(),
        home: String::new(),
        generics: Vec::new(),
        associated: Vec::new(),
        known_extensions: Vec::new(),
        resolver,
    }
    .declaration()
}
