//! Procedural macros of `apus`. Use them through that crate.

use proc_macro::{Delimiter, Group, Literal, Punct, Spacing, TokenStream, TokenTree};

/// The tokens with invisible groups opened: a literal forwarded through `macro_rules!` arrives wrapped in one.
fn flatten(input: TokenStream) -> Vec<TokenTree> {
    let mut out = Vec::new();
    for token in input {
        match token {
            TokenTree::Group(group) if group.delimiter() == Delimiter::None => out.extend(flatten(group.stream())),
            other => out.push(other),
        }
    }
    out
}

fn compile_error(message: &str) -> TokenStream {
    format!("::core::compile_error!({message:?})").parse().expect("a compile_error! invocation")
}

/// A compile error where an item is expected (`swift!` expands to items).
fn item_compile_error(message: &str) -> TokenStream {
    format!("::core::compile_error! {{ {message:?} }}").parse().expect("a compile_error! invocation")
}

/// The text of a plain or raw string literal token.
fn string_literal(token: &TokenTree) -> Option<String> {
    let TokenTree::Literal(literal) = token else { return None };
    let text = literal.to_string();
    if let Some(body) = text.strip_prefix('"').and_then(|t| t.strip_suffix('"')) {
        // Declarations contain no characters that need escaping; `\` followed by a newline continues a line.
        let mut out = String::new();
        let mut chars = body.chars().peekable();
        while let Some(c) = chars.next() {
            match c {
                '\\' if chars.peek() == Some(&'\n') => {
                    while chars.peek().is_some_and(|c| c.is_whitespace()) {
                        chars.next();
                    }
                }
                '\\' => return None,
                c => out.push(c),
            }
        }
        return Some(out);
    }
    let raw = text.strip_prefix('r')?;
    let hashes = raw.len() - raw.trim_start_matches('#').len();
    let body = raw[hashes..].strip_prefix('"')?;
    Some(body[..body.len().checked_sub(hashes + 1)?].to_string())
}

/// Expands to the mangled symbol (a string literal) of a Swift declaration:
/// `mangle!("Greeter.greet(_: String, times: Int) async throws -> String")`.
///
/// A second string literal is appended to the symbol (`"Tu"` gives an async function pointer symbol).
#[proc_macro]
pub fn mangle(input: TokenStream) -> TokenStream {
    let tokens = flatten(input);
    let (declaration, suffix) = match tokens.as_slice() {
        [declaration] => (string_literal(declaration), Some(String::new())),
        [declaration, TokenTree::Punct(comma), suffix] if comma.as_char() == ',' => (string_literal(declaration), string_literal(suffix)),
        _ => (None, None),
    };
    let (Some(declaration), Some(suffix)) = (declaration, suffix) else {
        return compile_error("expected a Swift declaration as a string literal, e.g. mangle!(\"Module.function(_: Int) -> Int\")");
    };
    match apus_mangle::mangle(&declaration) {
        Ok(symbol) => TokenTree::Literal(Literal::string(&format!("{symbol}{suffix}"))).into(),
        Err(error) => compile_error(&format!("cannot mangle `{declaration}`: {error}")),
    }
}

/// Expands to the mangled name of a declaration's result type as an `Option<&'static str>`:
/// `Some("SaySSG")` for `-> [String]`, `None` when the result depends on a generic parameter.
#[proc_macro]
pub fn result_type(input: TokenStream) -> TokenStream {
    let tokens = flatten(input);
    let [declaration] = tokens.as_slice() else {
        return compile_error("expected a Swift declaration as a string literal");
    };
    let Some(declaration) = string_literal(declaration) else {
        return compile_error("expected a Swift declaration as a string literal");
    };
    match apus_mangle::result_type(&declaration) {
        Ok(Some(name)) => format!("::core::option::Option::Some({name:?})").parse().expect("an Option expression"),
        Ok(None) => "::core::option::Option::<&'static str>::None".parse().expect("an Option expression"),
        Err(error) => compile_error(&format!("cannot mangle `{declaration}`: {error}")),
    }
}

/// Expands to the declaration's opaque type descriptor symbol as an `Option<&'static str>`: `Some("…QOMQ")` for a
/// `some P` result, else `None`.
#[doc(hidden)]
#[proc_macro]
pub fn __opaque_descriptor(input: TokenStream) -> TokenStream {
    let tokens = flatten(input);
    let Some(declaration) = tokens.first().and_then(string_literal) else {
        return compile_error("expected a Swift declaration as a string literal");
    };
    match apus_mangle::opaque_descriptor(&declaration) {
        Ok(Some(symbol)) => format!("::core::option::Option::Some({symbol:?})").parse().expect("an Option expression"),
        Ok(None) => "::core::option::Option::<&'static str>::None".parse().expect("an Option expression"),
        Err(error) => compile_error(&format!("cannot mangle `{declaration}`: {error}")),
    }
}

/// Expands to `true` if the declaration consumes `self` (`consuming func`), else `false`.
#[doc(hidden)]
#[proc_macro]
pub fn __consumes_self(input: TokenStream) -> TokenStream {
    let tokens = flatten(input);
    let Some(declaration) = tokens.first().and_then(string_literal) else {
        return compile_error("expected a Swift declaration as a string literal");
    };
    match apus_mangle::Declaration::parse(&declaration) {
        Ok(parsed) => (if parsed.consumes_self() { "true" } else { "false" }).parse().expect("a bool"),
        Err(error) => compile_error(&format!("cannot mangle `{declaration}`: {error}")),
    }
}

/// Expands to `true` if the declaration's result is exactly the protocol's `Self`, else `false`.
#[doc(hidden)]
#[proc_macro]
pub fn __returns_self(input: TokenStream) -> TokenStream {
    let tokens = flatten(input);
    let Some(declaration) = tokens.first().and_then(string_literal) else {
        return compile_error("expected a Swift declaration as a string literal");
    };
    match apus_mangle::Declaration::parse(&declaration) {
        Ok(parsed) => (if parsed.returns_self() { "true" } else { "false" }).parse().expect("a bool"),
        Err(error) => compile_error(&format!("cannot mangle `{declaration}`: {error}")),
    }
}

/// Picks how `swift!` calls a declaration, then expands `PATH! { binding how KIND rest… }`. Input:
/// `PATH ; binding how async|sync rest…`, `rest` ending with the declaration. A sync declaration that must run on
/// an actor (an actor's sync member, `@MainActor`) becomes `hop SelfActor` / `hop Main` when declared `async fn`
/// in Rust, and a compile error when declared `fn`: Rust would call it off its actor.
#[doc(hidden)]
#[proc_macro]
pub fn __swift_route(input: TokenStream) -> TokenStream {
    let tokens: Vec<TokenTree> = input.into_iter().collect();
    let Some(split) = tokens.iter().position(|t| matches!(t, TokenTree::Punct(p) if p.as_char() == ';')) else {
        return item_compile_error("__swift_route: expected `PATH ; …`");
    };
    let (path, rest) = (&tokens[..split], &tokens[split + 1..]);
    let [TokenTree::Ident(binding), TokenTree::Ident(how), TokenTree::Ident(asyncness), tail @ ..] = rest else {
        return item_compile_error("__swift_route: expected `binding how async|sync …`");
    };
    let declaration = tail.last().and_then(|t| flatten(t.clone().into()).first().and_then(string_literal));
    // A declared call's declaration, parsed once for every question below.
    let declared = match (how.to_string().as_str(), declaration) {
        ("declared", Some(declaration)) => match apus_mangle::Declaration::parse(&declaration) {
            Ok(parsed) => Some((declaration, parsed)),
            Err(error) => return item_compile_error(&format!("cannot mangle `{declaration}`: {error}")),
        },
        _ => None,
    };
    // The Rust signature's `throws`: a bracket group holding just that word.
    let rust_throws = tail.iter().any(|t| matches!(t, TokenTree::Group(g) if g.delimiter() == Delimiter::Bracket && g.stream().to_string() == "throws"));
    // The Rust signature's `async` and `throws` must be the declaration's: an async callee called the sync way, or a
    // thrown error left unread, is undefined behavior. A sync member that must run on its actor is `async` in Rust
    // (apus hops there); a typed-throws declaration takes an `ErrorSlot` instead of `throws` (checked below).
    if let Some((declaration, parsed)) = &declared {
        let rust_async = asyncness.to_string() == "async";
        let hops = parsed.isolation() != apus_mangle::Isolation::None;
        if parsed.is_async() && !rust_async {
            return item_compile_error(&format!("`{declaration}` is async: declare it `async fn`"));
        }
        if !parsed.is_async() && rust_async && !hops {
            return item_compile_error(&format!("`{declaration}` is synchronous: declare it `fn`"));
        }
        if parsed.throws_any_error() != rust_throws && !matches!(parsed.typed_throws(), Ok(true)) {
            return item_compile_error(&format!(
                "`{declaration}` {}: {} `throws` after the Rust signature",
                if rust_throws { "does not throw" } else { "throws" },
                if rust_throws { "remove" } else { "write" }
            ));
        }
    }
    // `&mut` borrows a Rust value for the call: only a struct's setter or mutating method takes `self` that way,
    // `inout` parameters take one, and only a synchronous call may hold the borrow.
    let receiver = rust_receiver(tail);
    let mutable = |text: &str| text.replace(' ', "").starts_with("&mut");
    let receiver_mut = receiver.as_deref().is_some_and(mutable);
    let argument_types = rust_argument_types(tail);
    if (receiver_mut || argument_types.iter().any(|t| mutable(t))) && asyncness.to_string() != "sync" {
        return item_compile_error("a `&mut` (`inout`) argument or `self` needs a synchronous `fn`: an async call would outlive the borrow");
    }
    if let Some((declaration, parsed)) = &declared {
        if parsed.mutates_self() && !receiver_mut {
            return item_compile_error(&format!("`{declaration}` changes `self` in place: declare `self: &mut SwiftValue`"));
        }
        if receiver_mut && !parsed.mutates_self() {
            return item_compile_error(&format!("`{declaration}` is not a struct's setter or `mutating` method: `self` takes no `&mut`"));
        }
        for (i, (inout, ty)) in parsed.inout_params().iter().zip(&argument_types).enumerate() {
            if *inout != mutable(ty) {
                return item_compile_error(&format!(
                    "`{declaration}`: parameter {} is {}`inout`, so its Rust type {} `&mut SwiftValue`",
                    i + 1,
                    if *inout { "" } else { "not " },
                    if *inout { "must be" } else { "cannot be" }
                ));
            }
        }
    }
    // A static member or initializer gets its type as `self`: a class's metadata, nothing for a struct or enum.
    if let Some((declaration, parsed)) = &declared {
        match parsed.type_self() {
            apus_mangle::TypeSelf::Thick if !receiver.as_deref().is_some_and(|r| r.ends_with("SwiftType")) => {
                return item_compile_error(&format!("`{declaration}` takes its class as `self`: declare `self: &SwiftType` first"));
            }
            apus_mangle::TypeSelf::Conformance if !receiver.as_deref().is_some_and(|r| r.ends_with("OpenedType")) => {
                return item_compile_error(&format!(
                    "`{declaration}` is called on a conforming type: declare `self: &OpenedType` first (`SwiftType::conforming`)"
                ));
            }
            apus_mangle::TypeSelf::Thin if receiver.is_some() => {
                return item_compile_error(&format!("`{declaration}` belongs to a struct or enum, which passes no `self`: declare no receiver"));
            }
            _ => {}
        }
    }
    // The Rust signature's `ErrorSlot` argument (an identifier anywhere in its groups).
    fn mentions(stream: TokenStream, name: &str) -> bool {
        stream.into_iter().any(|t| match t {
            TokenTree::Ident(i) => i.to_string() == name,
            TokenTree::Group(g) => mentions(g.stream(), name),
            _ => false,
        })
    }
    let rust_error_slot = tail.iter().any(|t| matches!(t, TokenTree::Group(g) if mentions(g.stream(), "ErrorSlot")));
    if let Some((declaration, parsed)) = &declared {
        if let Err(error) = parsed.check_callable() {
            return item_compile_error(&format!("`{declaration}`: {error}"));
        }
        match parsed.typed_throws() {
            Ok(false) if rust_error_slot => {
                return item_compile_error(&format!("`{declaration}` has no typed throws: it takes no `ErrorSlot`"))
            }
            Ok(true) if rust_throws => {
                return item_compile_error(&format!(
                    "`{declaration}` has typed throws: declare it without `throws` and pass an `ErrorSlot` after its generic arguments; its error comes back as `CallError::Typed`"
                ))
            }
            Ok(_) => {} // a typed-throws slot is passed explicitly, or filled automatically (checked below)
            Err(error) => return item_compile_error(&format!("`{declaration}`: {error}")),
        }
    }
    // Automatic generic arguments: the Rust signature lists exactly the Swift declaration's formal parameters.
    let rust_args = rust_argument_names(tail);
    let mut generic = TokenStream::new();
    let mut automatic = false;
    if let Some((declaration, parsed)) = &declared {
        let formal = parsed.formal_count();
        let explicit_slot = rust_error_slot;
        match parsed.generic_plan() {
            Ok(Some(plan)) if rust_args.len() == plan.formal_count && !explicit_slot => {
                if parsed.nonsending() {
                    return item_compile_error(&format!("`{declaration}`: automatic generic arguments with nonisolated(nonsending): pass them explicitly"));
                }
                // The argument identifiers themselves (not text): they must resolve to the function's own parameters.
                let name = |i: usize| TokenTree::Ident(rust_args[i].clone());
                let bracket = |tokens: Vec<TokenTree>| TokenTree::Group(Group::new(Delimiter::Bracket, tokens.into_iter().collect()));
                // Each source as `function argument`: the function of `apus::swift` that gives the type it holds.
                let function = |i: usize| TokenTree::Ident(proc_macro::Ident::new(plan.type_functions[i], proc_macro::Span::call_site()));
                let source = |i: usize| vec![function(i), name(i)];
                let paren = |tokens: Vec<TokenTree>| TokenTree::Group(Group::new(Delimiter::Parenthesis, tokens.into_iter().collect()));
                let metadata = plan.metadata_from.iter().map(|&i| paren(source(i))).collect();
                let witnesses = plan
                    .witnesses
                    .iter()
                    .zip(&plan.witness_layouts)
                    .map(|((i, p), layout)| {
                        let mut entry = source(*i);
                        entry.push(TokenTree::Literal(Literal::string(p)));
                        entry.push(TokenTree::Ident(proc_macro::Ident::new(&format!("{layout:?}"), proc_macro::Span::call_site())));
                        paren(entry)
                    })
                    .collect();
                let slot = plan.error_slot_from.map(source).unwrap_or_default();
                let result = plan.result_from.map(source).unwrap_or_default();
                let auto = TokenTree::Ident(proc_macro::Ident::new("auto", proc_macro::Span::call_site()));
                generic = [auto, bracket(metadata), bracket(witnesses), bracket(slot), bracket(result)].into_iter().collect();
                automatic = true;
            }
            Err(error) if rust_args.len() == formal => return item_compile_error(&format!("`{declaration}`: {error}")),
            _ => {}
        }
    }
    // Function-typed parameters: the shape the closure argument must have, checked when the call is made.
    if let Some((declaration, parsed)) = &declared {
        match parsed.closure_shapes() {
            Ok(shapes) if shapes.iter().any(Option::is_some) => {
                if rust_args.len() < shapes.len() {
                    return item_compile_error(&format!("`{declaration}`: the Rust signature lists fewer arguments than the declaration"));
                }
                let pairs: TokenStream = shapes
                    .iter()
                    .enumerate()
                    .filter_map(|(i, shape)| shape.as_ref().map(|shape| (i, shape)))
                    .map(|(i, shape)| {
                        let pair = vec![TokenTree::Ident(rust_args[i].clone()), TokenTree::Literal(Literal::string(shape))];
                        TokenTree::Group(Group::new(Delimiter::Parenthesis, pair.into_iter().collect()))
                    })
                    .collect();
                generic.extend([
                    TokenTree::Ident(proc_macro::Ident::new("closures", proc_macro::Span::call_site())),
                    TokenTree::Group(Group::new(Delimiter::Bracket, pairs)),
                ]);
            }
            Ok(_) => {}
            Err(error) => return item_compile_error(&format!("`{declaration}`: {error}")),
        }
    }
    // Arguments the callee consumes (an initializer's): passed +1, the call keeping no reference of its own.
    if let Some((declaration, parsed)) = &declared {
        let consumed: Vec<TokenTree> = parsed
            .consumed()
            .iter()
            .zip(&rust_args)
            .filter(|(consumes, _)| **consumes)
            .map(|(_, name)| TokenTree::Ident(name.clone()))
            .collect();
        if !consumed.is_empty() {
            if parsed.nonsending() {
                return item_compile_error(&format!("`{declaration}`: nonisolated(nonsending) with consumed arguments is not supported"));
            }
            let mut front: TokenStream = [
                TokenTree::Ident(proc_macro::Ident::new("consumed", proc_macro::Span::call_site())),
                TokenTree::Group(Group::new(Delimiter::Bracket, consumed.into_iter().collect())),
            ]
            .into_iter()
            .collect();
            front.extend(generic);
            generic = front;
        }
    }
    // Arguments Swift passes as one pointer where a value would go by address: a class-bound generic parameter's
    // (a class reference) and a metatype (a `SwiftType`). Wrapped so that another Rust type does not compile. An
    // `any P` too: in registers when its protocols are class-bound, which its metadata tells at run time.
    if let Some((declaration, parsed)) = &declared {
        let kinds = match parsed.argument_kinds() {
            Ok(kinds) => kinds,
            Err(error) => return item_compile_error(&format!("`{declaration}`: {error}")),
        };
        let wrapped: Vec<TokenTree> = kinds
            .iter()
            .zip(&rust_args)
            .filter_map(|(kind, name)| {
                let (wrapper, shape) = match kind.as_ref()? {
                    apus_mangle::ArgumentKind::ClassBound => ("ClassBound", None),
                    apus_mangle::ArgumentKind::Metatype => ("Metatype", None),
                    apus_mangle::ArgumentKind::Existential => ("Existential", None),
                    apus_mangle::ArgumentKind::Error => ("ErrorArg", None),
                    apus_mangle::ArgumentKind::CFunction(shape) => ("CFunctionArg", Some(shape)),
                    apus_mangle::ArgumentKind::Block(shape) => ("BlockArg", Some(shape)),
                };
                let mut tokens = vec![TokenTree::Ident(name.clone()), TokenTree::Ident(proc_macro::Ident::new(wrapper, proc_macro::Span::call_site()))];
                tokens.extend(shape.map(|shape| TokenTree::Literal(Literal::string(shape))));
                Some(TokenTree::Group(Group::new(Delimiter::Parenthesis, tokens.into_iter().collect())))
            })
            .collect();
        if !wrapped.is_empty() {
            let mut front: TokenStream = [
                TokenTree::Ident(proc_macro::Ident::new("wrapped", proc_macro::Span::call_site())),
                TokenTree::Group(Group::new(Delimiter::Bracket, wrapped.into_iter().collect())),
            ]
            .into_iter()
            .collect();
            front.extend(generic);
            generic = front;
        }
        let ret = rust_return(tail);
        let reference = ["SwiftObject", "Retained"].iter().any(|t| ret.contains(t)) && !ret.starts_with("Option");
        match parsed.result_kind() {
            Some(apus_mangle::ResultKind::ClassReference) if !reference => {
                return item_compile_error(&format!(
                    "`{declaration}` returns a class-bound generic parameter's value, a +1 reference: declare `-> SwiftObject` (or `Retained<T>`)"
                ))
            }
            Some(apus_mangle::ResultKind::Metatype) if ret != "SwiftType" => {
                return item_compile_error(&format!("`{declaration}` returns a metatype, the type's metadata: declare `-> SwiftType`"))
            }
            // `DirectValue` takes the box as an opaque value of type `any Error`.
            Some(apus_mangle::ResultKind::Error) if !ret.contains("SwiftError") && !ret.contains("DirectValue") => {
                return item_compile_error(&format!(
                    "`{declaration}` returns an `any Error`, an error box (+1): declare `-> SwiftError` (`Option<SwiftError>` if optional)"
                ))
            }
            Some(apus_mangle::ResultKind::CFunction) if !ret.contains("extern \"C\" fn") => {
                return item_compile_error(&format!(
                    "`{declaration}` returns a C function pointer: declare `-> extern \"C\" fn(…)` (`Option<…>` if optional)"
                ))
            }
            Some(apus_mangle::ResultKind::Block) if !ret.contains("RcBlock") => {
                return item_compile_error(&format!(
                    "`{declaration}` returns an Objective-C block (+1): declare `-> RcBlock<dyn Fn(…)>` (feature `block2`; `Option<…>` if optional)"
                ))
            }
            _ => {}
        }
    }
    // A member of a generic struct or enum: an instance member takes `Self`'s metadata (from the receiver) right after
    // its formal arguments; a static member or initializer takes the type's generic arguments, passed explicitly.
    if let Some((declaration, parsed)) = &declared {
        let formal = parsed.formal_count();
        match parsed.value_context() {
            apus_mangle::ValueContext::SelfMetadata => {
                let Some(this) = rust_receiver_name(tail) else {
                    return item_compile_error(&format!("`{declaration}` is an instance member: declare its receiver, `self: &SwiftValue`"));
                };
                if parsed.nonsending() {
                    return item_compile_error(&format!("`{declaration}`: nonisolated(nonsending) members of generic types are not supported"));
                }
                if rust_args.len() < formal {
                    return item_compile_error(&format!("`{declaration}`: the Rust signature lists fewer arguments than the declaration"));
                }
                let names = |range: std::ops::Range<usize>| -> TokenStream {
                    let mut tokens = Vec::new();
                    for (n, i) in range.enumerate() {
                        if n > 0 {
                            tokens.push(TokenTree::Punct(proc_macro::Punct::new(',', proc_macro::Spacing::Alone)));
                        }
                        tokens.push(TokenTree::Ident(rust_args[i].clone()));
                    }
                    tokens.into_iter().collect()
                };
                let mut front: TokenStream = [
                    TokenTree::Ident(proc_macro::Ident::new("selftype", proc_macro::Span::call_site())),
                    TokenTree::Ident(this),
                    TokenTree::Group(Group::new(Delimiter::Bracket, names(0..formal))),
                    TokenTree::Group(Group::new(Delimiter::Bracket, names(formal..rust_args.len()))),
                    TokenTree::Group(Group::new(
                        Delimiter::Bracket,
                        parsed.result_from_self().map(|n| TokenTree::Literal(Literal::usize_unsuffixed(n))).into_iter().collect(),
                    )),
                ]
                .into_iter()
                .collect();
                front.extend(generic);
                generic = front;
            }
            apus_mangle::ValueContext::GenericArguments if rust_args.len() == formal => {
                return item_compile_error(&format!(
                    "`{declaration}` belongs to a generic struct or enum: after its arguments, pass the type's generic arguments (each parameter's `SwiftType`, then witness tables)"
                ))
            }
            _ => {}
        }
    }
    // A typed-throws declaration whose error slot is neither passed nor filled automatically.
    if let Some((declaration, parsed)) = &declared {
        if parsed.typed_throws().unwrap_or(false) && !rust_error_slot && !automatic {
            return item_compile_error(&format!(
                "`{declaration}` has typed throws: pass an `ErrorSlot` argument after its generic arguments, where the callee writes its error"
            ));
        }
    }
    let hop = match declared.as_ref().map(|(d, parsed)| (parsed.isolation(), d)) {
        Some((apus_mangle::Isolation::Actor, d)) => Some(("SelfActor", d)),
        Some((apus_mangle::Isolation::MainActor, d)) => Some(("Main", d)),
        _ => None,
    };
    let nonsending = declared.as_ref().is_some_and(|(_, parsed)| parsed.nonsending());
    if nonsending && asyncness.to_string() != "async" {
        return item_compile_error("a `nonisolated(nonsending)` function is async: declare it `async fn`");
    }
    let kind = match (asyncness.to_string().as_str(), hop) {
        ("async", None) if nonsending => "nonsending".to_string(),
        ("async", Some((target, _))) => format!("hop {target}"),
        ("sync", Some((_, d))) => {
            return item_compile_error(&format!(
                "`{d}` must run on its actor: declare it `async fn`, and apus hops to the actor for the call"
            ))
        }
        (other, None) => other.to_string(),
        (other, _) => other.to_string(),
    };
    // Built from the token trees themselves: `$crate` in `path` does not survive a trip through text.
    let mut body: Vec<TokenTree> = vec![TokenTree::Ident(binding.clone()), TokenTree::Ident(how.clone())];
    body.extend(kind.parse::<TokenStream>().expect("kind tokens"));
    let (spec, before) = tail.split_last().expect("a declaration");
    body.extend(before.iter().cloned());
    body.push(TokenTree::Group(Group::new(Delimiter::Bracket, generic)));
    body.push(spec.clone());
    let mut out: Vec<TokenTree> = path.to_vec();
    out.push(TokenTree::Punct(Punct::new('!', Spacing::Alone)));
    out.push(TokenTree::Group(Group::new(Delimiter::Brace, body.into_iter().collect())));
    out.into_iter().collect()
}

/// The Rust signature's argument types as text, in order (the fourth bracket group: `name : Type , …`).
fn rust_argument_types(tail: &[TokenTree]) -> Vec<String> {
    let Some(args) = tail.iter().filter_map(|t| match t {
        TokenTree::Group(g) if g.delimiter() == Delimiter::Bracket => Some(g),
        _ => None,
    }).nth(3) else { return Vec::new() };
    let mut types = Vec::new();
    let (mut current, mut in_type, mut angle) = (Vec::new(), false, 0i32);
    let tokens = flatten(args.stream());
    for (i, token) in tokens.iter().enumerate() {
        match token {
            TokenTree::Punct(p) if p.as_char() == ':' && !in_type && !matches!(tokens.get(i + 1), Some(TokenTree::Punct(q)) if q.as_char() == ':') => in_type = true,
            TokenTree::Punct(p) if p.as_char() == ',' && angle == 0 && in_type => {
                types.push(current.join(" "));
                current.clear();
                in_type = false;
            }
            token if in_type => {
                if let TokenTree::Punct(p) = token {
                    angle += match p.as_char() { '<' => 1, '>' => -1, _ => 0 };
                }
                current.push(token.to_string());
            }
            _ => {}
        }
    }
    if in_type {
        types.push(current.join(" "));
    }
    types
}

/// The Rust signature's receiver type as text (the third bracket group, `this : & Type`; just `& Type`), if any.
fn rust_receiver(tail: &[TokenTree]) -> Option<String> {
    let receiver = tail.iter().filter_map(|t| match t {
        TokenTree::Group(g) if g.delimiter() == Delimiter::Bracket => Some(g),
        _ => None,
    }).nth(2)?;
    let text: String = flatten(receiver.stream()).iter().map(|t| t.to_string()).collect::<Vec<_>>().join(" ");
    let ty = text.split_once(':').map_or(text.as_str(), |(_, ty)| ty).trim();
    (!ty.is_empty()).then(|| ty.to_string())
}

/// The Rust signature's receiver name (`self`), if it has one.
fn rust_receiver_name(tail: &[TokenTree]) -> Option<proc_macro::Ident> {
    let receiver = tail.iter().filter_map(|t| match t {
        TokenTree::Group(g) if g.delimiter() == Delimiter::Bracket => Some(g),
        _ => None,
    }).nth(2)?;
    match receiver.stream().into_iter().next()? {
        TokenTree::Ident(this) => Some(this),
        _ => None,
    }
}

/// The Rust signature's return type as text (the sixth bracket group: docs, visibility, name… arguments, throws,
/// return), `""` when there is none.
fn rust_return(tail: &[TokenTree]) -> String {
    let Some(ret) = tail.iter().filter_map(|t| match t {
        TokenTree::Group(g) if g.delimiter() == Delimiter::Bracket => Some(g),
        _ => None,
    }).nth(5) else { return String::new() };
    flatten(ret.stream()).iter().map(|t| t.to_string()).collect::<Vec<_>>().join(" ").replace(" :: ", "::").trim().to_string()
}

/// The Rust signature's argument names: identifiers followed by a single `:` in its argument group (the fourth
/// bracket group: docs, visibility, receiver, arguments).
fn rust_argument_names(tail: &[TokenTree]) -> Vec<proc_macro::Ident> {
    let groups: Vec<&Group> = tail.iter().filter_map(|t| match t {
        TokenTree::Group(g) if g.delimiter() == Delimiter::Bracket => Some(g),
        _ => None,
    }).collect();
    let Some(args) = groups.get(3) else { return Vec::new() };
    let tokens: Vec<TokenTree> = flatten(args.stream());
    let mut names = Vec::new();
    for (i, token) in tokens.iter().enumerate() {
        let TokenTree::Ident(ident) = token else { continue };
        let colon = |t: Option<&TokenTree>| matches!(t, Some(TokenTree::Punct(p)) if p.as_char() == ':');
        let at_start = i == 0 || matches!(&tokens[i - 1], TokenTree::Punct(p) if p.as_char() == ',');
        if at_start && colon(tokens.get(i + 1)) && !colon(tokens.get(i + 2)) {
            names.push(ident.clone());
        }
    }
    names
}

/// A class member's dispatch thunk (`…Tj`), which apus tries before its own symbol; `""` for anything else.
#[doc(hidden)]
#[proc_macro]
pub fn __dispatch_thunk(input: TokenStream) -> TokenStream {
    let tokens = flatten(input);
    let Some(declaration) = (match tokens.as_slice() {
        [declaration] => string_literal(declaration),
        _ => None,
    }) else {
        return compile_error("expected a Swift declaration as a string literal");
    };
    match apus_mangle::dispatch_thunk(&declaration) {
        Ok(thunk) => TokenTree::Literal(Literal::string(&thunk.unwrap_or_default())).into(),
        Err(error) => compile_error(&format!("cannot mangle `{declaration}`: {error}")),
    }
}

/// Like `mangle!`, with the symbol apus calls: a protocol requirement's dispatch thunk (`…Tj`).
#[doc(hidden)]
#[proc_macro]
pub fn __call_symbol(input: TokenStream) -> TokenStream {
    let tokens = flatten(input);
    let (declaration, suffix) = match tokens.as_slice() {
        [declaration] => (string_literal(declaration), Some(String::new())),
        [declaration, TokenTree::Punct(comma), suffix] if comma.as_char() == ',' => (string_literal(declaration), string_literal(suffix)),
        _ => (None, None),
    };
    let (Some(declaration), Some(suffix)) = (declaration, suffix) else {
        return compile_error("expected a Swift declaration as a string literal");
    };
    match apus_mangle::call_symbol(&declaration) {
        Ok(symbol) => TokenTree::Literal(Literal::string(&format!("{symbol}{suffix}"))).into(),
        Err(error) => compile_error(&format!("cannot mangle `{declaration}`: {error}")),
    }
}
