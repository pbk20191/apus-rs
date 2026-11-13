//! Parses the declaration syntax: Swift's own, with the kind of each non-standard type spelled out.
//!
//! ```text
//! [static] Module.Type(kind)….name[<T: P, U>]([label|_]: Type, …) [async] [throws] [-> Type] [where …]
//! [static] Module.Type(kind)….name: Type { get }
//! ```
//! Types: `Int`, `String`, … (standard library names), `[T]`, `[K: V]`, `T?`, `(A, b: B)`, `Any`, `AnyObject`,
//! `any Proto`, `Name(struct|class|enum)` (in the declaration's module), `Module.Name(kind)`, `Outer(kind).Inner(kind)`,
//! `Name(kind)<Args>`, generic parameters and `T.Associated`.

use crate::ast::*;
use crate::names;

pub struct Parser<'a> {
    text: &'a str,
    position: usize,
    module: String,
    generics: Vec<String>,
}

type Parsed<T> = Result<T, String>;

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

    /// `(struct)`, `(class)`, `(enum)` or `(actor)` right after a type name.
    fn kind_suffix(&mut self) -> Option<Kind> {
        for (text, kind) in [("(struct)", Kind::Struct), ("(class)", Kind::Class), ("(actor)", Kind::Class), ("(enum)", Kind::Enum)] {
            if self.rest().starts_with(text) {
                self.position += text.len();
                return Some(kind);
            }
        }
        None
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
            &self.module
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
        if let Some(param) = self.generics.iter().position(|g| *g == first) {
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
            (self.module.clone(), &segments[..])
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
            let kind = kind.ok_or_else(|| format!("`{name}` needs its kind: `{name}(struct)`, `{name}(class)` or `{name}(enum)`"))?;
            resolved.push((name.clone(), kind));
        }
        Ok(Ty::Nominal { module, path: resolved, args })
    }

    fn subject(&mut self) -> Parsed<Subject> {
        let name = self.identifier()?;
        let param = self.generics.iter().position(|g| *g == name).ok_or_else(|| format!("`{name}` is not a generic parameter"))?;
        if self.eat_path_dot() {
            return Ok(Subject::Member(param, self.identifier()?));
        }
        Ok(Subject::Param(param))
    }

    /// `: P & Q` after a subject.
    fn conformances(&mut self, subject: &Subject, into: &mut Vec<Requirement>) -> Parsed<()> {
        loop {
            into.push(Requirement::Conforms(subject.clone(), self.protocol()?));
            if !self.eat("&") {
                return Ok(());
            }
        }
    }

    fn where_clause(&mut self, into: &mut Vec<Requirement>) -> Parsed<()> {
        loop {
            let subject = self.subject()?;
            if self.eat("==") {
                let ty = self.ty()?;
                // Canonical form anchors a same-type requirement on the generic parameter.
                into.push(match (subject, ty) {
                    (Subject::Member(param, name), Ty::Param(other)) => Requirement::SameType(Subject::Param(other), Ty::Member(param, name)),
                    (Subject::Member(param, name), Ty::Member(other, other_name)) if other < param => {
                        Requirement::SameType(Subject::Member(other, other_name), Ty::Member(param, name))
                    }
                    (subject, ty) => Requirement::SameType(subject, ty),
                });
            } else {
                self.expect(":")?;
                self.conformances(&subject, into)?;
            }
            if !self.eat(",") {
                return Ok(());
            }
        }
    }

    fn declaration(&mut self) -> Parsed<Decl> {
        let is_static = self.eat_keyword("static");
        self.skip_whitespace();
        let mut segments = vec![(self.identifier()?, self.kind_suffix())];
        while self.eat_path_dot() {
            let name = self.identifier()?;
            segments.push((name, self.kind_suffix()));
        }
        if segments.len() < 2 {
            return Err("expected `Module.name…`".into());
        }
        let (name, name_kind) = segments.pop().unwrap();
        if name_kind.is_some() || segments[0].1.is_some() {
            return Err("expected `Module.Type(kind).name…`: the module and the declaration's name take no kind".into());
        }
        self.module = segments[0].0.clone();
        let mut context = Vec::new();
        for (type_name, kind) in &segments[1..] {
            let kind = kind.ok_or_else(|| format!("`{type_name}` needs its kind: `{type_name}(struct)`, `{type_name}(class)` or `{type_name}(enum)`"))?;
            context.push((type_name.clone(), kind));
        }

        let mut requirements = Vec::new();
        if self.eat_here('<') {
            let mut inline: Vec<(usize, usize)> = Vec::new(); // (parameter, where its constraints start)
            loop {
                let param = self.identifier()?;
                self.generics.push(param);
                if self.eat(":") {
                    inline.push((self.generics.len() - 1, self.position));
                    // Skip now, parse once every parameter name is known.
                    let mut skipped = Vec::new();
                    self.conformances(&Subject::Param(0), &mut skipped)?;
                }
                if !self.eat(",") {
                    break;
                }
            }
            self.expect(">")?;
            let after = self.position;
            for (param, at) in inline {
                self.position = at;
                self.conformances(&Subject::Param(param), &mut requirements)?;
            }
            self.position = after;
        }

        let signature = if self.eat("(") {
            let mut params = Vec::new();
            while !self.eat(")") {
                if !params.is_empty() {
                    self.expect(",")?;
                }
                let label = if self.eat_keyword("_") { None } else { Some(self.identifier()?) };
                self.expect(":")?;
                let inout = self.eat_keyword("inout");
                let ty = self.ty()?;
                let variadic = self.eat("...");
                params.push(Param { label, ty, inout, variadic });
            }
            let is_async = self.eat_keyword("async");
            let throws = self.eat_keyword("throws");
            let result = if self.eat("->") { self.ty()? } else { Ty::Tuple(Vec::new()) };
            Signature::Function { params, result, is_async, throws }
        } else if self.eat(":") {
            let ty = self.ty()?;
            self.expect("{")?;
            if !self.eat_keyword("get") {
                return Err(self.error("expected `get`"));
            }
            self.expect("}")?;
            Signature::Getter(ty)
        } else {
            return Err(self.error("expected `(` or `:` after the declaration's name"));
        };
        if self.eat_keyword("where") {
            self.where_clause(&mut requirements)?;
        }
        self.skip_whitespace();
        if !self.rest().is_empty() {
            return Err(self.error("unexpected text"));
        }
        Ok(Decl { module: self.module.clone(), context, name, is_static, generic_params: self.generics.len(), requirements, signature })
    }
}

fn swift_generic(name: &str, args: Vec<Ty>) -> Ty {
    Ty::Nominal { module: "Swift".into(), path: vec![(name.into(), Kind::Struct)], args }
}

pub fn parse(text: &str) -> Parsed<Decl> {
    Parser { text, position: 0, module: String::new(), generics: Vec::new() }.declaration()
}
