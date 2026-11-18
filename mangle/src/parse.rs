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
}

type Parsed<T> = Result<T, String>;

/// One dotted segment of the declaration's path, with what was written after its name.
struct Segment {
    name: String,
    kind: Option<Kind>,
    /// `<T: P, U>`: parameter names with their inline constraints.
    generics: Option<Vec<(String, Vec<Protocol>)>>,
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
        ];
        for (text, kind) in kinds {
            if self.rest().starts_with(text) {
                self.position += text.len();
                return Some(kind);
            }
        }
        None
    }

    fn generic_param(&self, name: &str) -> Option<GenericParam> {
        self.generics.iter().find(|(known, _)| known == name).map(|(_, param)| *param)
    }

    fn protocol(&mut self) -> Parsed<Protocol> {
        let first = self.identifier()?;
        if self.eat_path_dot() {
            return Ok(Protocol { module: first, name: self.identifier()? });
        }
        let module = if names::standard_protocol(&first).is_some() || names::swift_protocol(&first) {
            "Swift"
        } else if names::concurrency_protocol(&first).is_some() {
            "_Concurrency"
        } else {
            &self.home
        };
        Ok(Protocol { module: module.to_string(), name: first })
    }

    fn ty(&mut self) -> Parsed<Ty> {
        let mut ty = self.primary_type()?;
        while self.eat("?") {
            ty = Ty::Optional(Box::new(ty));
        }
        if self.rest().trim_start().starts_with("->") {
            return Err(self.error("function types are not supported"));
        }
        Ok(ty)
    }

    fn primary_type(&mut self) -> Parsed<Ty> {
        let (mut escaping, mut sendable, mut main_actor) = (false, false, false);
        loop {
            if self.eat("@escaping") {
                escaping = true;
            } else if self.eat("@Sendable") {
                sendable = true;
            } else if self.eat("@MainActor") {
                main_actor = true;
            } else {
                break;
            }
        }
        let attributed = escaping || sendable || main_actor;
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
                let params = elements.into_iter().map(|(_, ty)| ty).collect();
                let result = self.ty()?;
                return Ok(Ty::Function(Box::new(FunctionTy { params, result, is_async, throws, sendable, main_actor, escaping })));
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
            return Ok(Ty::Existential(self.protocol()?));
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
            if self.eat_path_dot() {
                return Ok(Ty::Member(param, self.identifier()?));
            }
            return Ok(Ty::Param(param));
        }
        let mut segments = vec![(first, self.kind_suffix())];
        while self.eat_path_dot() {
            let name = self.identifier()?;
            segments.push((name, self.kind_suffix()));
        }
        let args = if self.eat_here('<') {
            let mut args = vec![self.ty()?];
            while self.eat(",") {
                args.push(self.ty()?);
            }
            self.expect(">")?;
            args
        } else {
            Vec::new()
        };

        // A first segment without a kind, followed by more, is a module name.
        let (module, path) = if segments.len() > 1 && segments[0].1.is_none() {
            (segments[0].0.clone(), &segments[1..])
        } else {
            (self.home.clone(), &segments[..])
        };
        // A lone name without a kind must be one the standard library defines.
        if let [(name, None)] = path {
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
                        return Ok(Ty::Nominal { module: known_module.into(), path: vec![(name.clone(), kind)], args });
                    }
                }
            }
            return Err(format!("unknown type `{name}`: write `{name}(struct)`, `{name}(class)` or `{name}(enum)`"));
        }
        let mut resolved = Vec::new();
        for (name, kind) in path {
            let kind = kind.ok_or_else(|| needs_kind(name))?;
            resolved.push((name.clone(), kind));
        }
        Ok(Ty::Nominal { module, path: resolved, args })
    }

    fn subject(&mut self) -> Parsed<Subject> {
        let name = self.identifier()?;
        let param = self.generic_param(&name).ok_or_else(|| format!("`{name}` is not a generic parameter"))?;
        if self.eat_path_dot() {
            return Ok(Subject::Member(param, self.identifier()?));
        }
        Ok(Subject::Param(param))
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
                for protocol in self.protocol_list()? {
                    into.push(Requirement::Conforms(subject.clone(), protocol));
                }
            }
            if !self.eat(",") {
                return Ok(());
            }
        }
    }

    /// `<T: P & Q, U>` (the `<` already eaten): names with their inline constraints.
    fn generic_clause(&mut self) -> Parsed<Vec<(String, Vec<Protocol>)>> {
        let mut params = Vec::new();
        loop {
            let name = self.identifier()?;
            let constraints = if self.eat(":") { self.protocol_list()? } else { Vec::new() };
            params.push((name, constraints));
            if !self.eat(",") {
                break;
            }
        }
        self.expect(">")?;
        Ok(params)
    }

    fn segment(&mut self) -> Parsed<Segment> {
        let name = self.identifier()?;
        let kind = self.kind_suffix();
        let generics = if self.eat_here('<') { Some(self.generic_clause()?) } else { None };
        Ok(Segment { name, kind, generics })
    }

    /// Registers generic parameters at `depth` and returns the requirements written inline.
    fn declare_generics(&mut self, clause: Vec<(String, Vec<Protocol>)>, depth: usize) -> Vec<Requirement> {
        let mut requirements = Vec::new();
        for (index, (name, constraints)) in clause.into_iter().enumerate() {
            let param = GenericParam { depth, index };
            self.generics.push((name, param));
            requirements.extend(constraints.into_iter().map(|protocol| Requirement::Conforms(Subject::Param(param), protocol)));
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
        loop {
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

        // `(extension in Module [where …])`: its `where` clause is parsed once the generic parameters are known.
        let mut extension: Option<(String, Option<usize>)> = None;
        self.skip_whitespace();
        if self.rest().starts_with('(') {
            self.position += 1;
            if !(self.eat_keyword("extension") && self.eat_keyword("in")) {
                return Err(self.error("expected `(extension in Module)`"));
            }
            let module = self.identifier()?;
            let where_at = if self.eat_keyword("where") { Some(self.position) } else { None };
            let close = self.rest().find(')').ok_or_else(|| self.error("expected `)`"))?;
            self.position += close + 1;
            extension = Some((module, where_at));
        }

        self.skip_whitespace();
        let mut segments = vec![self.segment()?];
        if segments[0].kind.is_some() || segments[0].generics.is_some() {
            return Err("expected `Module.name…`: a declaration starts with its module".into());
        }
        self.module = segments[0].name.clone();
        self.home = extension.as_ref().map_or_else(|| self.module.clone(), |(module, _)| module.clone());
        while self.eat_path_dot() {
            segments.push(self.segment()?);
        }
        let operator = self.operator_name();
        if let Some(name) = &operator {
            segments.push(Segment { name: name.clone(), kind: None, generics: None });
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

        // The enclosing types.
        let mut context = Vec::new();
        let mut context_params = 0;
        let mut context_is_constrained = false;
        let lone_context = segments.len() == 2;
        for segment in segments.drain(1..) {
            let kind = match segment.kind {
                Some(kind) => kind,
                None if lone_context => standard_kind(&self.module, &segment.name).ok_or_else(|| needs_kind(&segment.name))?,
                None => return Err(needs_kind(&segment.name)),
            };
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
                let declared = self.declare_generics(clause, 0);
                context_is_constrained = !declared.is_empty() || standard_type_is_constrained(&self.module, &segment.name);
            }
            context.push((segment.name, kind));
        }

        // The declaration's own generic parameters sit one level below the enclosing type's.
        let own_depth = usize::from(context_params > 0);
        let (generic_params, mut requirements) = match member.generics {
            Some(clause) => (clause.len(), self.declare_generics(clause, own_depth)),
            None => (0, Vec::new()),
        };

        let extension = match extension {
            Some((module, where_at)) => {
                let mut extension_requirements = Vec::new();
                if let Some(at) = where_at {
                    let resume = std::mem::replace(&mut self.position, at);
                    self.where_clause(&mut extension_requirements)?;
                    if !self.eat(")") {
                        return Err(self.error("unexpected text in the extension's `where` clause"));
                    }
                    self.position = resume;
                }
                Some(Extension { module, requirements: extension_requirements })
            }
            None => None,
        };

        let mut accessor_effects = (false, false);
        let signature = if is_case {
            if extension.is_some() || generic_params > 0 || context.last().is_none_or(|(_, kind)| *kind != Kind::Enum) {
                return Err("expected `case Module.Type(enum).name[(payload…)]`".into());
            }
            if context_is_constrained {
                return Err("cases of an enum with constrained generic parameters are not supported".into());
            }
            // The payload's labels are not part of the symbol.
            let mut payload = Vec::new();
            if self.eat("(") {
                while !self.eat(")") {
                    if !payload.is_empty() {
                        self.expect(",")?;
                    }
                    let start = self.position;
                    if !(self.identifier().is_ok() && self.eat(":")) {
                        self.position = start;
                    }
                    // A payload is stored: a function in it escapes.
                    payload.push(match self.ty()? {
                        Ty::Function(mut function) => {
                            function.escaping = true;
                            Ty::Function(function)
                        }
                        ty => ty,
                    });
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
                let inout = self.eat_keyword("inout");
                let isolated = self.eat_keyword("isolated");
                let ownership = [("consuming", Ownership::Consuming), ("borrowing", Ownership::Borrowing), ("__owned", Ownership::Owned), ("__shared", Ownership::Shared)]
                    .into_iter()
                    .find(|(word, _)| self.eat_keyword(word))
                    .map(|(_, ownership)| ownership);
                let ty = self.ty()?;
                let variadic = self.eat("...");
                // A default value changes nothing in the symbol; `#isolation` is the only one understood.
                if self.eat("=") && !self.eat("#isolation") {
                    return Err(self.error("only `= #isolation` is understood as a default value"));
                }
                params.push(Param { label, ty, inout, variadic, isolated, ownership });
            }
            let is_async = self.eat_keyword("async");
            let mut throws = self.eat_keyword("throws");
            // `throws(E)`; `throws(any Error)` is plain `throws`, `throws(Never)` no `throws` at all.
            let typed_error = if throws && self.eat("(") {
                let ty = self.ty()?;
                self.expect(")")?;
                match ty {
                    Ty::Existential(ref p) if p.name == "Error" && p.module == "Swift" => None,
                    Ty::Nominal { ref module, ref path, ref args } if module == "Swift" && args.is_empty() && matches!(path.as_slice(), [(name, _)] if name == "Never") => {
                        throws = false;
                        None
                    }
                    ty => Some(ty),
                }
            } else {
                None
            };
            let result = if self.eat("->") {
                self.ty()?
            } else if member.name == "init" {
                // An initializer returns the type itself (written out only for a failable one: `-> T?`).
                let args = (0..context_params).map(|index| Ty::Param(GenericParam { depth: 0, index })).collect();
                Ty::Nominal { module: self.module.clone(), path: context.clone(), args }
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
        if mutating && (is_static || !matches!(signature, Signature::Function { .. }) || context.last().is_none_or(|(_, kind)| !matches!(kind, Kind::Struct | Kind::Enum))) {
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
        if member.name == "init" {
            match context.last() {
                None => return Err("`init` belongs to a type: `Module.Type(kind).init(…)`".into()),
                Some((_, Kind::Protocol)) => return Err("protocol initializers are not supported".into()),
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
        Ok(Decl {
            module: self.module.clone(),
            context,
            context_params,
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
    Ty::Nominal { module: "Swift".into(), path: vec![(name.into(), Kind::Struct)], args }
}

/// The characters of an ASCII operator name.
const OPERATOR_CHARS: &str = "/=-+!*%<>&|^~?.";

pub fn parse(text: &str) -> Parsed<Decl> {
    Parser { text, position: 0, module: String::new(), home: String::new(), generics: Vec::new() }.declaration()
}
