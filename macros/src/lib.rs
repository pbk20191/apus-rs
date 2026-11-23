//! Procedural macros of `apus`. Use them through that crate.

mod graphs;
mod sdk;
mod short;

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

/// A declaration literal's text, a declaration by name (`SwiftData.ModelContext.fetch(_:)`) written in full from the
/// module's symbol graph (`__swift_route` reports one that cannot be).
fn declaration_literal(token: &TokenTree) -> Option<String> {
    let text = string_literal(token)?;
    Some(short::expand(&text).ok().flatten().unwrap_or(text))
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
    graphs::graphs();
    let tokens = flatten(input);
    let (declaration, suffix) = match tokens.as_slice() {
        [declaration] => (declaration_literal(declaration), Some(String::new())),
        [declaration, TokenTree::Punct(comma), suffix] if comma.as_char() == ',' => (declaration_literal(declaration), string_literal(suffix)),
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
    graphs::graphs();
    let tokens = flatten(input);
    let [declaration] = tokens.as_slice() else {
        return compile_error("expected a Swift declaration as a string literal");
    };
    let Some(declaration) = declaration_literal(declaration) else {
        return compile_error("expected a Swift declaration as a string literal");
    };
    match apus_mangle::result_type(&declaration) {
        Ok(Some(name)) => format!("::core::option::Option::Some({name:?})").parse().expect("an Option expression"),
        Ok(None) => "::core::option::Option::<&'static str>::None".parse().expect("an Option expression"),
        Err(error) => compile_error(&format!("cannot mangle `{declaration}`: {error}")),
    }
}

/// The one string literal of a macro's input.
fn single_literal(input: TokenStream) -> Option<String> {
    match flatten(input).as_slice() {
        [literal] => string_literal(literal),
        _ => None,
    }
}

/// Expands to the mangled name (a string literal, no `$s`) of a concrete Swift type, written as in a declaration, for
/// `SwiftType::from_mangled_name`:
///
/// ```ignore
/// assert_eq!(apus::swift_type!("Todos.Todo(class)"), "5Todos4TodoC");
/// assert_eq!(apus::swift_type!("[String]"), "SaySSG");
/// // let todo = apus::swift::SwiftType::from_mangled_name(apus::swift_type!("Todos.Todo(class)"));
/// ```
#[proc_macro]
pub fn swift_type(input: TokenStream) -> TokenStream {
    graphs::graphs();
    let Some(ty) = single_literal(input) else {
        return compile_error("expected a Swift type as a string literal, e.g. swift_type!(\"Todos.Todo(class)\")");
    };
    match apus_mangle::type_name(&ty) {
        Ok(name) => TokenTree::Literal(Literal::string(&name)).into(),
        Err(error) => compile_error(&format!("cannot mangle {error}")),
    }
}

/// `swift!`'s typed wrappers: `(class|struct|enum "Module.Type(kind)")` expands to the tuple (mangled name, qualified
/// name without generic arguments, whether a class), or a compile error if the type is not a concrete one of that kind.
#[doc(hidden)]
#[proc_macro]
pub fn __swift_wrapper_names(input: TokenStream) -> TokenStream {
    graphs::graphs();
    let tokens = flatten(input);
    let [TokenTree::Ident(kind), literal] = tokens.as_slice() else {
        return compile_error("__swift_wrapper_names: expected `class|struct|enum \"Module.Type(kind)\"`");
    };
    let Some(ty) = string_literal(literal) else { return compile_error("expected the Swift type as a string literal") };
    match apus_mangle::wrapper_type(&kind.to_string(), &ty) {
        Ok((mangled, name)) => {
            let items: TokenStream = [
                TokenTree::Literal(Literal::string(&mangled)),
                TokenTree::Punct(proc_macro::Punct::new(',', proc_macro::Spacing::Alone)),
                TokenTree::Literal(Literal::string(&name)),
                TokenTree::Punct(proc_macro::Punct::new(',', proc_macro::Spacing::Alone)),
                TokenTree::Ident(proc_macro::Ident::new(if kind.to_string() == "class" { "true" } else { "false" }, proc_macro::Span::call_site())),
            ]
            .into_iter()
            .collect();
            TokenTree::Group(Group::new(Delimiter::Parenthesis, items)).into()
        }
        Err(error) => compile_error(&format!("swift! {kind} wrapper: {error}")),
    }
}

/// Expands to a protocol's descriptor symbol (a string literal), for `SwiftProtocol::new`:
///
/// ```ignore
/// assert_eq!(apus::swift_protocol!("SwiftData.PersistentModel"), "$s9SwiftData15PersistentModelMp");
/// static MODEL: apus::swift::SwiftProtocol = apus::swift::SwiftProtocol::new(apus::swift_protocol!("Swift.Hashable"));
/// ```
#[proc_macro]
pub fn swift_protocol(input: TokenStream) -> TokenStream {
    graphs::graphs();
    let Some(protocol) = single_literal(input) else {
        return compile_error("expected a Swift protocol as a string literal, e.g. swift_protocol!(\"SwiftData.PersistentModel\")");
    };
    match apus_mangle::protocol_descriptor_of(&protocol) {
        Ok(symbol) => TokenTree::Literal(Literal::string(&symbol)).into(),
        Err(error) => compile_error(&format!("cannot mangle {error}")),
    }
}

/// Expands to the declaration's opaque type descriptor symbol as an `Option<&'static str>`: `Some("…QOMQ")` for a
/// `some P` result, else `None`.
#[doc(hidden)]
#[proc_macro]
pub fn __opaque_descriptor(input: TokenStream) -> TokenStream {
    graphs::graphs();
    let tokens = flatten(input);
    let Some(declaration) = tokens.first().and_then(declaration_literal) else {
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
    graphs::graphs();
    let tokens = flatten(input);
    let Some(declaration) = tokens.first().and_then(declaration_literal) else {
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
    graphs::graphs();
    let tokens = flatten(input);
    let Some(declaration) = tokens.first().and_then(declaration_literal) else {
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
    graphs::graphs();
    let tokens: Vec<TokenTree> = input.into_iter().collect();
    let Some(split) = tokens.iter().position(|t| matches!(t, TokenTree::Punct(p) if p.as_char() == ';')) else {
        return item_compile_error("__swift_route: expected `PATH ; …`");
    };
    let (path, rest) = (&tokens[..split], &tokens[split + 1..]);
    let [TokenTree::Ident(binding), TokenTree::Ident(how), TokenTree::Ident(asyncness), TokenTree::Group(bindings), tail @ ..] = rest else {
        return item_compile_error("__swift_route: expected `binding how async|sync [bindings] …`");
    };
    // `T = "Module.Type(kind)"` after the declaration: generic parameters bound to concrete types.
    let mut bound: Vec<(String, String)> = Vec::new();
    for item in flatten(bindings.stream()).split(|t| matches!(t, TokenTree::Punct(p) if p.as_char() == ',')) {
        match item {
            [] => {}
            [TokenTree::Ident(name), TokenTree::Punct(eq), ty] if eq.as_char() == '=' && string_literal(ty).is_some() => {
                bound.push((name.to_string(), string_literal(ty).expect("checked")));
            }
            _ => return item_compile_error("expected generic parameters bound as `T = \"Module.Type(kind)\"` after the declaration"),
        }
    }
    let written = tail.last().and_then(|t| flatten(t.clone().into()).first().and_then(string_literal));
    // A declaration by name: written in full from the module's symbol graph.
    let expanded = match written.as_deref().map(short::expand) {
        Some(Err(error)) => return item_compile_error(&error),
        Some(Ok(full)) => full,
        None => None,
    };
    let declaration = expanded.clone().or(written.clone());
    // A declared call's declaration, parsed once for every question below.
    let declared = match (how.to_string().as_str(), declaration) {
        ("declared", Some(declaration)) => match apus_mangle::Declaration::parse(&declaration) {
            Ok(parsed) => Some((declaration, parsed)),
            Err(error) => return item_compile_error(&format!("cannot mangle `{declaration}`: {error}")),
        },
        _ => None,
    };
    // The symbol of a declaration written from a graph must be the one the graph names.
    if let (Some((declaration, parsed)), Some(_), Some(written)) = (&declared, &expanded, &written) {
        let symbol = parsed.symbol().map_err(|e| e.to_string());
        let expected = short::expected_symbol(written);
        // A type that moved modules is mangled as defined (`@_originallyDefinedIn`), which the graph's USR is not:
        // the symbol the SDK exports is the one meant.
        let exported = symbol.as_ref().is_ok_and(|s| exported_by_sdk(parsed, s));
        if symbol.as_ref().ok() != expected.as_ref() && !exported {
            return item_compile_error(&format!(
                "`{written}`, written as `{declaration}`, mangles to {symbol:?}, not to {expected:?} as its symbol graph says: write the declaration in full"
            ));
        }
    }
    // A generic declaration written by name is specialized: every generic parameter bound.
    if let (Some((declaration, parsed)), Some(written)) = (&declared, &expanded.as_ref().and(written.as_ref())) {
        let names = parsed.generic_parameter_names();
        if bound.is_empty() && !names.is_empty() {
            let wanted: Vec<String> = names.iter().map(|n| format!("{n} = \"Module.Type\"")).collect();
            return item_compile_error(&format!(
                "`{written}` (`{declaration}`) is generic: a declaration by name binds every generic parameter, `{}`",
                wanted.join(", ")
            ));
        }
    }
    let concrete = match (&declared, bound.is_empty()) {
        (_, true) => None,
        (None, false) => return item_compile_error("binding generic parameters (`T = \"…\"`) needs a Swift declaration: #[swift = \"…\"] or #[swift_linked = \"…\"]"),
        (Some((declaration, parsed)), false) => {
            let pairs: Vec<(&str, &str)> = bound.iter().map(|(n, t)| (n.as_str(), t.as_str())).collect();
            match parsed.bind(&pairs) {
                Ok(plan) => Some(plan),
                Err(error) => return item_compile_error(&format!("`{declaration}`: {error}")),
            }
        }
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
    // A static member or initializer gets its type as `self`: a class's metadata, nothing for a struct or enum. A
    // generic class's, with its parameters bound, is the class specialized.
    let bound_self = concrete.as_ref().and_then(|plan| plan.self_type.clone());
    if let Some((declaration, parsed)) = &declared {
        match parsed.type_self() {
            // Without `self`, the declared class's own metadata (`classself`, added last below).
            apus_mangle::TypeSelf::Thick if receiver.is_none() && parsed.context_type_name().is_none() && bound_self.is_none() => {
                return item_compile_error(&format!(
                    "`{declaration}` belongs to a generic class: declare `self: &SwiftType` first, the specialized class's metadata"
                ));
            }
            apus_mangle::TypeSelf::Thick if receiver.as_deref().is_some_and(|r| !r.ends_with("SwiftType")) => {
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
        let callable = if concrete.is_some() { parsed.check_bound_callable() } else { parsed.check_callable() };
        if let Err(error) = callable {
            return item_compile_error(&format!("`{declaration}`: {error}"));
        }
        // The symbol a member of an SDK module is called through must be one the module exports.
        if std::env::var_os("APUS_NO_SDK_CHECK").is_none() {
            if let Err(error) = check_sdk_symbol(declaration, parsed) {
                return item_compile_error(&error);
            }
        }
        // A `SwiftValue` result is read by address; these come in registers.
        let rust_result = rust_return(tail);
        let last_segment = rust_result.rsplit([':', ' ']).next().unwrap_or_default();
        if parsed.result_in_registers() && last_segment == "SwiftValue" {
            return item_compile_error(&format!(
                "`{declaration}` returns an array, dictionary, set, string or class instance, in registers, not by address: \
                 declare `-> DirectValue` (or `SwiftObject` for an object)"
            ));
        }
        // A C value goes by swiftcall's lowering of its C layout (registers, up to four); not as a `SwiftValue`.
        let (imported_args, imported_result) = parsed.imported_values();
        let rust_types = rust_argument_types(tail);
        let by_address = |ty: &str| ty.rsplit([':', ' ', '&']).next() == Some("SwiftValue");
        let imported_arg = (rust_types.len() == imported_args.len())
            .then(|| imported_args.iter().zip(&rust_types).position(|(imported, ty)| *imported && by_address(ty)))
            .flatten();
        if imported_arg.is_some() || (imported_result && last_segment == "SwiftValue") {
            return item_compile_error(&format!(
                "`{declaration}`: a value of a type imported from C is passed and returned as its C layout, not as a \
                 `SwiftValue`: declare its Rust type (an objc2 crate's: `CGRect`, `NSCalendarUnit`, `&NSString` for a typed string)"
            ));
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
            Ok(Some(plan)) if rust_args.len() == plan.formal_count && !explicit_slot && concrete.is_none() => {
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
            Err(error) if rust_args.len() == formal && concrete.is_none() => return item_compile_error(&format!("`{declaration}`: {error}")),
            _ => {}
        }
    }
    // Bound generic parameters: the bound types' metadata and witness tables, found by name when the call is made;
    // the result's type with the bindings substituted. A value of type `T` goes by address whatever `T` is bound to.
    if let (Some((declaration, parsed)), Some(plan)) = (&declared, &concrete) {
        if rust_args.len() != parsed.formal_count() {
            return item_compile_error(&format!(
                "`{declaration}`: with its generic parameters bound, the Rust signature lists exactly the declaration's parameters"
            ));
        }
        if parsed.nonsending() {
            return item_compile_error(&format!("`{declaration}`: bound generic parameters with nonisolated(nonsending): pass them explicitly"));
        }
        let by_address = |ty: &str| ty.replace(' ', "").starts_with('&') && !ty.replace(' ', "").starts_with("&mut") && ty.ends_with("SwiftValue");
        for (i, (address, ty)) in plan.by_address.iter().zip(&argument_types).enumerate() {
            if *address && !by_address(ty) {
                return item_compile_error(&format!(
                    "`{declaration}`: parameter {} is a generic parameter's value, which goes by address whatever type is bound: declare it `&SwiftValue`",
                    i + 1
                ));
            }
        }
        if plan.result_by_address && !rust_return(tail).ends_with("SwiftValue") {
            return item_compile_error(&format!(
                "`{declaration}` returns a generic parameter's value, by address whatever type is bound: declare `-> SwiftValue`"
            ));
        }
        // Everything the call looks up, once each, by index: the bound types' and checked types' names, and the
        // protocols.
        let mut names: Vec<String> = Vec::new();
        let mut protocols: Vec<String> = Vec::new();
        fn index(table: &mut Vec<String>, item: &str) -> TokenTree {
            let i = table.iter().position(|t| t == item).unwrap_or_else(|| {
                table.push(item.to_string());
                table.len() - 1
            });
            TokenTree::Literal(Literal::usize_unsuffixed(i))
        }
        let literal = |text: &str| TokenTree::Literal(Literal::string(text));
        let bracket = |tokens: Vec<TokenTree>| TokenTree::Group(Group::new(Delimiter::Bracket, tokens.into_iter().collect()));
        let paren = |tokens: Vec<TokenTree>| TokenTree::Group(Group::new(Delimiter::Parenthesis, tokens.into_iter().collect()));
        let ident = |text: &str| TokenTree::Ident(proc_macro::Ident::new(text, proc_macro::Span::call_site()));
        // `[type]`, or `[type (protocol "…TL" "…Tl")…]` for an associated type, a step for each name of its path.
        let subject = |on: &apus_mangle::BoundType, names: &mut Vec<String>, protocols: &mut Vec<String>| match on {
            apus_mangle::BoundType::Named(name) => bracket(vec![index(names, name)]),
            apus_mangle::BoundType::Associated { ty, path } => {
                let mut tokens = vec![index(names, ty)];
                for step in path {
                    tokens.push(paren(vec![index(protocols, &step.protocol), literal(&step.base), literal(&step.descriptor)]));
                }
                bracket(tokens)
            }
        };
        let types = plan.types.iter().map(|t| index(&mut names, t)).collect();
        let witnesses = plan
            .witnesses
            .iter()
            .map(|(on, protocol, layout)| {
                let on = subject(on, &mut names, &mut protocols);
                paren(vec![on, index(&mut protocols, protocol), ident(&format!("{layout:?}"))])
            })
            .collect();
        let superclasses =
            plan.superclasses.iter().map(|(ty, class)| paren(vec![index(&mut names, ty), index(&mut names, class)])).collect();
        let same_types = plan
            .same_types
            .iter()
            .map(|(on, expected)| {
                let on = subject(on, &mut names, &mut protocols);
                paren(vec![on, index(&mut names, expected)])
            })
            .collect();
        let error = plan.error_type.iter().map(|e| index(&mut names, e)).collect();
        // Arguments whose type names a bound parameter, checked against that type when the call is made: a value by
        // its type, a class reference by its dynamic class (as automatic generic arguments would read them).
        let kinds = parsed.argument_kinds().unwrap_or_default();
        let checks = plan
            .argument_types
            .iter()
            .enumerate()
            .filter_map(|(i, ty)| {
                let ty = ty.as_ref()?;
                let function = match (kinds.get(i), argument_types.get(i)) {
                    (Some(Some(apus_mangle::ArgumentKind::ClassBound)), _) => "__object_type_of",
                    (Some(None), Some(rust)) if by_address(rust) => "__type_of",
                    _ => return None,
                };
                Some(paren(vec![TokenTree::Ident(rust_args[i].clone()), ident(function), index(&mut names, ty)]))
            })
            .collect();
        generic = [
            ident("concrete"),
            bracket(names.iter().map(|n| literal(n)).collect()),
            bracket(protocols.iter().map(|p| literal(p)).collect()),
            bracket(types),
            bracket(witnesses),
            bracket(superclasses),
            bracket(same_types),
            bracket(error),
            bracket(plan.result.iter().map(|r| literal(r)).collect()),
            bracket(checks),
        ]
        .into_iter()
        .collect();
        automatic = true;
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
                let (optional, plain) = (String::from("?"), String::new());
                let (wrapper, shape) = match kind.as_ref()? {
                    apus_mangle::ArgumentKind::CValue(true) => ("CValueArg", Some(&optional)),
                    apus_mangle::ArgumentKind::CValue(false) => ("CValueArg", Some(&plain)),
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
            // A typed string (`NS_TYPED_ENUM`) comes back as its object (+1); any other C value as its C layout. An
            // optional one comes with its tag (by address when that makes five registers): a typed string's only.
            Some(apus_mangle::ResultKind::CValue(optional)) => {
                let as_layout = ret.rsplit("::").next().is_some_and(|last| last.starts_with("CValue"));
                if !ret.contains("Retained") && (optional || !as_layout) {
                    return item_compile_error(&format!(
                        "`{declaration}` returns a value of a type imported from C, as its C layout: declare `-> CValue<T>`, \
                         T its Rust type (an objc2 crate's), or `-> Retained<T>` for a typed string's object (`Option<…>` if \
                         optional; an optional C struct, enum or option set is not supported)"
                    ));
                }
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
            apus_mangle::ValueContext::GenericArguments if rust_args.len() == formal && concrete.is_none() => {
                // Only the parameters no `where` fixes to a type are generic arguments.
                let free = parsed.generic_parameter_names().join(", ");
                return item_compile_error(&format!(
                    "`{declaration}` belongs to a generic struct or enum: after its arguments, pass its generic arguments (the `SwiftType` of {free}, then witness tables)"
                ));
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
    // Outermost marker: `__swift_call!` reads it before the others.
    if let Some((_, parsed)) = &declared {
        if parsed.type_self() == apus_mangle::TypeSelf::Thick && receiver.is_none() {
            let name = bound_self.clone().or_else(|| parsed.context_type_name()).expect("checked above");
            let mut front: TokenStream = [
                TokenTree::Ident(proc_macro::Ident::new("classself", proc_macro::Span::call_site())),
                TokenTree::Literal(Literal::string(&name)),
            ]
            .into_iter()
            .collect();
            front.extend(generic);
            generic = front;
        }
    }
    // A type-erased receiver (`SwiftObject`, `SwiftValue`) is checked against the declaration's type when the call is
    // made: `receiver this instance|exactly|named "name"`, outermost.
    if let (Some((_, parsed)), Some(ty), Some(this)) = (&declared, &receiver, rust_receiver_name(tail)) {
        let erased = matches!(ty.rsplit([':', ' ', '&']).next(), Some("SwiftObject" | "SwiftValue"));
        let check = concrete.as_ref().map_or_else(|| parsed.receiver_check(), |plan| plan.receiver.clone());
        if let (true, Some(check)) = (erased, check) {
            let (how, name) = match check {
                apus_mangle::ReceiverCheck::Instance(name) => ("Instance", name),
                apus_mangle::ReceiverCheck::Exactly(name) => ("Exactly", name),
                apus_mangle::ReceiverCheck::Named(name) => ("Named", name),
            };
            let mut front: TokenStream = [
                TokenTree::Ident(proc_macro::Ident::new("receiver", proc_macro::Span::call_site())),
                TokenTree::Ident(this),
                TokenTree::Ident(proc_macro::Ident::new(how, proc_macro::Span::call_site())),
                TokenTree::Literal(Literal::string(&name)),
            ]
            .into_iter()
            .collect();
            front.extend(generic);
            generic = front;
        }
    }
    // Typed wrappers as the receiver, arguments or result: checked against the declaration's types at compile time
    // (`typed [(value who (Type) "mangled" "name" class) (result (Type) …)]`, outermost).
    if let Some((_, parsed)) = &declared {
        let mut entries: Vec<TokenTree> = Vec::new();
        let mut entry = |role: &str, who: Option<proc_macro::Ident>, ty: Vec<TokenTree>, nominal: (String, String, bool)| {
            let mut tokens = vec![TokenTree::Ident(proc_macro::Ident::new(role, proc_macro::Span::call_site()))];
            tokens.extend(who.map(TokenTree::Ident));
            tokens.push(TokenTree::Group(Group::new(Delimiter::Parenthesis, ty.into_iter().collect())));
            tokens.push(TokenTree::Literal(Literal::string(&nominal.0)));
            tokens.push(TokenTree::Literal(Literal::string(&nominal.1)));
            tokens.push(TokenTree::Ident(proc_macro::Ident::new(if nominal.2 { "true" } else { "false" }, proc_macro::Span::call_site())));
            entries.push(TokenTree::Group(Group::new(Delimiter::Parenthesis, tokens.into_iter().collect())));
        };
        let of = |n: &apus_mangle::NominalRef| (n.mangled.clone().unwrap_or_default(), n.name.clone(), n.class);
        let check = concrete.as_ref().map_or_else(|| parsed.receiver_check(), |plan| plan.receiver.clone());
        if let (Some(ty), Some(who), Some(check)) = (rust_type_tokens(tail, 2).into_iter().next(), rust_receiver_name(tail), check) {
            if let Some(ty) = wrapper_type(ty.1, false, false) {
                let nominal = match check {
                    apus_mangle::ReceiverCheck::Instance(m) => (m, String::new(), true),
                    apus_mangle::ReceiverCheck::Exactly(m) => (m, String::new(), false),
                    apus_mangle::ReceiverCheck::Named(n) => (String::new(), n, false),
                };
                entry("value", Some(who), ty, nominal);
            }
        }
        let params = concrete.as_ref().map_or_else(|| parsed.nominal_params(), |plan| plan.nominal_params.clone());
        for ((who, ty), nominal) in rust_type_tokens(tail, 3).into_iter().zip(params) {
            if let (Some(n), Some(who)) = (nominal.filter(|n| !n.optional && !n.array), who) {
                if let Some(ty) = wrapper_type(ty, false, false) {
                    entry("value", Some(who), ty, of(&n));
                }
            }
        }
        let result = concrete.as_ref().map_or_else(|| parsed.nominal_result(), |plan| plan.nominal_result.clone());
        if let (Some(n), Some((_, ty))) = (result, rust_type_tokens(tail, 5).into_iter().next()) {
            if let Some(ty) = wrapper_type(ty, n.optional, n.array) {
                entry("result", None, ty, of(&n));
            }
        }
        if !entries.is_empty() {
            let mut front: TokenStream = [
                TokenTree::Ident(proc_macro::Ident::new("typed", proc_macro::Span::call_site())),
                TokenTree::Group(Group::new(Delimiter::Bracket, entries.into_iter().collect())),
            ]
            .into_iter()
            .collect();
            front.extend(generic);
            generic = front;
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
    // A sync call of integers, `bool`, `f64` and borrowed class instances with a result in registers, no receiver and
    // nothing generic, consumed, checked or wrapped: made with no call record (`SwiftFn::__call_scalar`).
    let bare = |t: &str| t.replace(' ', "").rsplit("::").next().unwrap_or_default().to_string();
    let scalar = |t: &str| {
        let t = t.replace(' ', "");
        matches!(t.as_str(), "i64" | "i32" | "isize" | "u64" | "u32" | "usize" | "bool" | "f64")
            || t.strip_prefix('&').is_some_and(|object| !object.starts_with("mut") && bare(object) == "SwiftObject")
    };
    let in_registers = |t: &str| {
        let t = t.replace(' ', "");
        matches!(t.as_str(), "" | "()" | "i64" | "i32" | "u64" | "bool" | "f64" | "String" | "Option<i64>" | "Option<String>")
            || bare(&t) == "SwiftObject"
            || t.strip_prefix("Option<").and_then(|t| t.strip_suffix('>')).is_some_and(|t| bare(t) == "SwiftObject")
    };
    let kind = if kind == "sync"
        && generic.is_empty()
        && rust_receiver(tail).is_none()
        && rust_argument_types(tail).iter().all(|t| scalar(t))
        && in_registers(&rust_return(tail))
    {
        "scalar".to_string()
    } else {
        kind
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

/// Whether an SDK module the declaration names exports `symbol`.
fn exported_by_sdk(parsed: &apus_mangle::Declaration, symbol: &str) -> bool {
    parsed.binary_modules().iter().filter_map(|m| sdk::exported_symbols(m)).any(|set| set.contains(symbol))
}

/// Whether the SDK module declaring `declaration` exports the symbol apus calls it through (its dispatch thunk or its
/// own symbol; an async function's `…Tu` pointer). `Ok` when no module the declaration names is in the SDK.
fn check_sdk_symbol(declaration: &str, parsed: &apus_mangle::Declaration) -> Result<(), String> {
    let modules = parsed.binary_modules();
    let exported: Vec<_> = modules.iter().filter_map(|m| sdk::exported_symbols(m)).collect();
    if exported.is_empty() {
        return Ok(());
    }
    let suffix = if parsed.is_async() { "Tu" } else { "" };
    let symbol = apus_mangle::call_symbol(declaration).map_err(|e| e.to_string())?;
    let thunk = apus_mangle::dispatch_thunk(declaration).map_err(|e| e.to_string())?;
    let found = std::iter::once(&symbol).chain(thunk.as_ref()).any(|s| exported.iter().any(|set| set.contains(&format!("{s}{suffix}"))));
    if found {
        return Ok(());
    }
    Err(format!(
        "`{declaration}`: the SDK's {} does not export `{symbol}{suffix}`: check the declaration's names, labels, types \
         and kinds against the framework's interface (APUS_NO_SDK_CHECK=1 skips this check)",
        modules.join(" / ")
    ))
}

/// The Rust types in the signature's `n`th bracket group (2: the receiver, 3: the arguments, 5: the result), each
/// with the name it is bound to, flattened.
fn rust_type_tokens(tail: &[TokenTree], n: usize) -> Vec<(Option<proc_macro::Ident>, Vec<TokenTree>)> {
    let Some(group) = tail.iter().filter_map(|t| match t {
        TokenTree::Group(g) if g.delimiter() == Delimiter::Bracket => Some(g),
        _ => None,
    }).nth(n) else { return Vec::new() };
    let tokens = flatten(group.stream());
    if n == 5 {
        return if tokens.is_empty() { Vec::new() } else { vec![(None, tokens)] };
    }
    let mut out = Vec::new();
    let (mut name, mut ty, mut angle) = (None, Vec::new(), 0i32);
    let mut i = 0;
    while i < tokens.len() {
        match &tokens[i] {
            TokenTree::Ident(ident) if name.is_none() && matches!(tokens.get(i + 1), Some(TokenTree::Punct(p)) if p.as_char() == ':') => {
                name = Some(ident.clone());
                i += 1; // the `:`
            }
            TokenTree::Punct(p) if p.as_char() == ',' && angle == 0 => {
                out.push((name.take(), std::mem::take(&mut ty)));
            }
            token => {
                if let TokenTree::Punct(p) = token {
                    angle += match p.as_char() { '<' => 1, '>' => -1, _ => 0 };
                }
                ty.push(token.clone());
            }
        }
        i += 1;
    }
    if !ty.is_empty() {
        out.push((name, ty));
    }
    out
}

/// The wrapper type a Rust type names, if it may be one: `&`/`&mut` left out, and `Option<…>` (`optional`) or
/// `Vec<…>` (`array`) where the Swift type is written so; `None` for a type-erased or plain Rust type.
fn wrapper_type(ty: Vec<TokenTree>, optional: bool, array: bool) -> Option<Vec<TokenTree>> {
    let mut ty: &[TokenTree] = &ty;
    while let [TokenTree::Punct(p), rest @ ..] = ty {
        if p.as_char() != '&' {
            break;
        }
        ty = rest;
    }
    if let [TokenTree::Ident(m), rest @ ..] = ty {
        if m.to_string() == "mut" {
            ty = rest;
        }
    }
    if optional || array {
        let outer = if optional { "Option" } else { "Vec" };
        match ty {
            [TokenTree::Ident(o), TokenTree::Punct(lt), inner @ .., TokenTree::Punct(gt)]
                if o.to_string() == outer && lt.as_char() == '<' && gt.as_char() == '>' => ty = inner,
            _ => return None,
        }
    }
    let last = ty.iter().rev().find_map(|t| match t {
        TokenTree::Ident(i) => Some(i.to_string()),
        _ => None,
    })?;
    const NOT_WRAPPERS: &[&str] = &[
        "SwiftObject", "SwiftValue", "DirectValue", "Retained", "Existential", "SwiftType", "OpenedType", "Opened",
        "SwiftError", "SwiftClosure", "WitnessTable", "ErrorSlot", "Isolation", "String", "str", "bool", "i8", "i16",
        "i32", "i64", "isize", "u8", "u16", "u32", "u64", "usize", "f32", "f64",
    ];
    let generic = ty.iter().any(|t| matches!(t, TokenTree::Punct(p) if p.as_char() == '<'));
    (!generic && !NOT_WRAPPERS.contains(&last.as_str())).then(|| ty.to_vec())
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
    graphs::graphs();
    let tokens = flatten(input);
    let Some(declaration) = (match tokens.as_slice() {
        [declaration] => declaration_literal(declaration),
        _ => None,
    }) else {
        return compile_error("expected a Swift declaration as a string literal");
    };
    // `("…Tj", "…Tq", "…Mn", class_side)`, empty for what is not a class's own member.
    match apus_mangle::class_method(&declaration) {
        Ok(method) => {
            let method = method.unwrap_or(apus_mangle::ClassMethod { thunk: String::new(), method: String::new(), class: String::new(), class_side: false });
            format!("({:?}, {:?}, {:?}, {})", method.thunk, method.method, method.class, method.class_side).parse().unwrap()
        }
        Err(error) => compile_error(&format!("cannot mangle `{declaration}`: {error}")),
    }
}

/// Like `mangle!`, with the symbol apus calls: a protocol requirement's dispatch thunk (`…Tj`).
#[doc(hidden)]
#[proc_macro]
pub fn __call_symbol(input: TokenStream) -> TokenStream {
    graphs::graphs();
    let tokens = flatten(input);
    let (declaration, suffix) = match tokens.as_slice() {
        [declaration] => (declaration_literal(declaration), Some(String::new())),
        [declaration, TokenTree::Punct(comma), suffix] if comma.as_char() == ',' => (declaration_literal(declaration), string_literal(suffix)),
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
