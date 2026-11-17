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
    let isolation = match (how.to_string().as_str(), declaration) {
        ("declared", Some(declaration)) => match apus_mangle::isolation(&declaration) {
            Ok(isolation) => Some((isolation, declaration)),
            Err(error) => return item_compile_error(&format!("cannot mangle `{declaration}`: {error}")),
        },
        _ => None,
    };
    let hop = match isolation {
        Some((apus_mangle::Isolation::Actor, d)) => Some(("SelfActor", d)),
        Some((apus_mangle::Isolation::MainActor, d)) => Some(("Main", d)),
        _ => None,
    };
    let kind = match (asyncness.to_string().as_str(), hop) {
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
    body.extend(tail.iter().cloned());
    let mut out: Vec<TokenTree> = path.to_vec();
    out.push(TokenTree::Punct(Punct::new('!', Spacing::Alone)));
    out.push(TokenTree::Group(Group::new(Delimiter::Brace, body.into_iter().collect())));
    out.into_iter().collect()
}
