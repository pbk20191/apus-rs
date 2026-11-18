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
    // A static member or initializer gets its type as `self`: a class's metadata, nothing for a struct or enum.
    if let Some((declaration, parsed)) = &declared {
        let receiver = rust_receiver(tail);
        match parsed.type_self() {
            apus_mangle::TypeSelf::Thick if !receiver.as_deref().is_some_and(|r| r.ends_with("SwiftType")) => {
                return item_compile_error(&format!("`{declaration}` takes its class as `self`: declare `self: &SwiftType` first"));
            }
            apus_mangle::TypeSelf::Thin if receiver.is_some() => {
                return item_compile_error(&format!("`{declaration}` belongs to a struct or enum, which passes no `self`: declare no receiver"));
            }
            _ => {}
        }
    }
    // The Rust signature's `throws`: a bracket group holding just that word.
    let rust_throws = tail.iter().any(|t| matches!(t, TokenTree::Group(g) if g.delimiter() == Delimiter::Bracket && g.stream().to_string() == "throws"));
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
                let metadata = plan.metadata_from.iter().map(|&i| name(i)).collect();
                let witnesses = plan
                    .witnesses
                    .iter()
                    .map(|(i, p)| {
                        let pair = vec![name(*i), TokenTree::Literal(Literal::string(p))];
                        TokenTree::Group(Group::new(Delimiter::Parenthesis, pair.into_iter().collect()))
                    })
                    .collect();
                let slot = plan.error_slot_from.map(name).into_iter().collect();
                let result = plan.result_from.map(name).into_iter().collect();
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

/// The Rust signature's receiver type as text (the third bracket group, `this : & Type`), if it has one.
fn rust_receiver(tail: &[TokenTree]) -> Option<String> {
    let receiver = tail.iter().filter_map(|t| match t {
        TokenTree::Group(g) if g.delimiter() == Delimiter::Bracket => Some(g),
        _ => None,
    }).nth(2)?;
    let text: String = flatten(receiver.stream()).iter().map(|t| t.to_string()).collect::<Vec<_>>().join(" ");
    (!text.trim().is_empty()).then(|| text.trim().to_string())
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
