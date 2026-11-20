//! Declarations written by name only (`SwiftData.ModelContext.fetch(_:)`, `Todos.Todo.title { get }`): the whole
//! declaration comes from the module's symbol graph, rewritten in apus's syntax (modules and kinds spelled out). It is
//! then mangled by apus like any other, and the symbol checked against the one the graph names (`verify`).

use crate::graphs::{self, Fragment, Module, Symbol};

/// A declaration by name: the module, the enclosing types, the member's title as the graph spells it (`fetch(_:)`,
/// `title`, `init(title:)`), the accessor written (`{ get }`/`{ set }`), and whether written `static`.
struct ShortForm<'a> {
    module: &'a str,
    context: Vec<&'a str>,
    title: String,
    accessor: Option<&'a str>,
}

fn short_form(text: &str) -> Option<ShortForm<'_>> {
    let text = text.trim();
    let (head, accessor) = match text.split_once('{') {
        Some((head, rest)) => {
            let accessor = rest.strip_suffix('}')?.trim();
            (head.trim_end(), Some(matches!(accessor, "get" | "set").then_some(accessor)?))
        }
        None => (text, None),
    };
    let (path, labels) = match head.split_once('(') {
        Some((path, rest)) => (path, Some(rest.strip_suffix(')')?)),
        None => (head, None),
    };
    let identifier = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') && !s.starts_with(|c: char| c.is_ascii_digit());
    if let Some(labels) = labels {
        // `label:label:` only: a full declaration has types after its colons.
        if !labels.split_terminator(':').all(identifier) || !(labels.is_empty() || labels.ends_with(':')) {
            return None;
        }
    }
    let segments: Vec<&str> = path.split('.').collect();
    if segments.len() < 2 || !segments.iter().all(|s| identifier(s)) {
        return None;
    }
    let member = segments[segments.len() - 1];
    let title = match labels {
        Some(labels) => format!("{member}({labels})"),
        None => member.to_string(),
    };
    Some(ShortForm { module: segments[0], context: segments[1..segments.len() - 1].to_vec(), title, accessor })
}

/// The full declaration `text` stands for when it is a declaration by name of a member of a module with a symbol
/// graph: `Ok(None)` when it is not one (a full declaration, or a module without a graph), `Err` when the graph has no
/// such member, several, or one apus cannot write.
pub fn expand(text: &str) -> Result<Option<String>, String> {
    let Some(short) = short_form(text) else { return Ok(None) };
    let Some(module) = graphs::graphs().module(short.module) else { return Ok(None) };
    let mut path: Vec<&str> = short.context.clone();
    path.push(&short.title);
    let callable = |s: &&Symbol| {
        matches!(
            s.kind.as_str(),
            "swift.func" | "swift.method" | "swift.type.method" | "swift.init" | "swift.property" | "swift.type.property" | "swift.subscript" | "swift.type.subscript"
        )
    };
    let found: Vec<&Symbol> = module.find(&path).filter(callable).collect();
    let symbol = match found.as_slice() {
        [one] => *one,
        [] => return Err(format!("`{text}`: {} has no `{}`", short.module, path.join("."))),
        several => {
            let all: Vec<String> = several.iter().map(|s| declaration_text(&s.fragments)).collect();
            return Err(format!("`{text}` names several declarations: write the one meant in full ({})", all.join("; ")));
        }
    };
    let property = matches!(symbol.kind.as_str(), "swift.property" | "swift.type.property");
    let subscript = symbol.kind.contains("subscript");
    if (property || subscript) != short.accessor.is_some() {
        return Err(format!("`{text}`: a property or subscript is written with `{{ get }}` or `{{ set }}`, anything else without"));
    }
    rewrite(short.module, module, &short.context, symbol, short.accessor).map(Some).map_err(|e| format!("`{text}`: {e}"))
}

fn declaration_text(fragments: &[Fragment]) -> String {
    fragments.iter().map(|f| f.spelling.as_str()).collect()
}

/// `symbol` in apus's syntax: `[modifiers] Module.Context(kind)<P>.member<own generics>(label: Type, …) … where …`.
fn rewrite(module_name: &str, module: &Module, context: &[&str], symbol: &Symbol, accessor: Option<&str>) -> Result<String, String> {
    // The enclosing types, each with its kind and generic parameters.
    let mut prefix = module_name.to_string();
    let mut context_params: Vec<String> = Vec::new();
    for i in 0..context.len() {
        let enclosing = module.find(&context[..=i]).find(|s| graphs::type_kind(&s.kind).is_some()).ok_or_else(|| format!("no type `{}`", context[..=i].join(".")))?;
        let kind = kind_mark(graphs::type_kind(&enclosing.kind).expect("a type"));
        let own: Vec<String> = enclosing.params.iter().map(|(n, _)| n.clone()).filter(|n| !context_params.contains(n)).collect();
        prefix.push('.');
        prefix.push_str(context[i]);
        prefix.push_str(kind);
        if !own.is_empty() {
            prefix.push_str(&format!("<{}>", own.join(", ")));
        }
        context_params.extend(own);
    }
    let all_params: Vec<&str> = symbol.params.iter().map(|(n, _)| n.as_str()).collect();
    let own_params: Vec<&str> = all_params.iter().copied().filter(|n| !context_params.iter().any(|c| c == n)).collect();
    // Requirements on the enclosing type's parameters (or a protocol's `Self`): the type's own are part of its
    // declaration, an extension's go in its `(extension in M where …)`, and the member's own in its `where`.
    let enclosing = module.find(context).find(|s| graphs::type_kind(&s.kind).is_some());
    let protocol = enclosing.is_some_and(|e| e.kind == "swift.protocol");
    let declared = |c: &(String, String, String, Option<String>)| {
        enclosing.is_some_and(|e| e.constraints.contains(c))
            // A protocol member's `Self: P`, P the protocol itself.
            || (protocol && c.1 == "Self" && c.0 == "conformance" && Some(c.2.as_str()) == context.last().copied())
    };
    let extension_constraints: Vec<_> =
        symbol.extension.iter().flat_map(|e| e.constraints.iter()).filter(|c| !declared(c)).collect();
    let member_constraints: Vec<_> = symbol
        .constraints
        .iter()
        .filter(|c| !own_params.contains(&root(&c.1)) && !declared(c) && !extension_constraints.contains(c))
        .collect();
    let mut extension = String::new();
    if let Some(e) = &symbol.extension {
        // Swift mangles an extension's member as the type's own unless the extension is another module's, adds
        // requirements, or extends a protocol.
        if protocol || e.extended_module != module_name || !extension_constraints.is_empty() {
            let requirements: Vec<String> = extension_constraints
                .iter()
                .map(|(kind, lhs, rhs, usr)| requirement(kind, lhs, rhs, usr.as_deref(), &context_params_str(&context_params), module_name))
                .collect::<Result<_, _>>()?;
            extension = match requirements.is_empty() {
                true => format!("(extension in {module_name}) "),
                false => format!("(extension in {module_name} where {}) ", requirements.join(", ")),
            };
        }
    }

    // The declaration's text, fragment by fragment: labels without parameter names, types with modules and kinds.
    let fragments = &symbol.fragments;
    let mut modifiers = Vec::new();
    let mut body = String::new();
    let mut i = 0;
    // Modifiers and attributes, up to the introducer (`func`, `init`, `var`, `subscript`).
    while i < fragments.len() {
        let f = &fragments[i];
        match (f.kind.as_str(), f.spelling.as_str()) {
            ("keyword", "func" | "var" | "let") => {
                i += 1;
                break;
            }
            ("keyword", "init" | "subscript") => break,
            ("keyword", "mutating" | "nonisolated" | "consuming" | "borrowing" | "static") => modifiers.push(f.spelling.clone()),
            ("keyword", "class") => modifiers.push("static".into()),
            ("keyword", "final" | "open" | "public" | "override" | "required" | "convenience" | "nonmutating") => {}
            ("attribute", "@discardableResult" | "@preconcurrency") => {}
            ("attribute", "@MainActor") => modifiers.push("@MainActor".into()),
            ("text", t) if t.trim().is_empty() => {}
            (_, other) => return Err(format!("`{other}` in `{}`: write the declaration in full", declaration_text(fragments))),
        }
        i += 1;
    }
    let mut own_generics_skipped = false;
    while i < fragments.len() {
        let f = &fragments[i];
        match f.kind.as_str() {
            // The requirements are written from `swiftGenerics` (below).
            "keyword" if f.spelling == "where" => break,
            "externalParam" => {
                body.push_str(&f.spelling);
                // ` name` after the label: dropped.
                if fragments.get(i + 1).is_some_and(|n| n.kind == "text" && n.spelling == " ") && fragments.get(i + 2).is_some_and(|n| n.kind == "internalParam") {
                    i += 2;
                }
            }
            // A subscript's parameter without a label.
            "internalParam" => body.push('_'),
            "genericParameter" if !own_generics_skipped => {
                // `<T, U>` after the name: written from `swiftGenerics` instead, with the requirements.
                let open = body.rfind('<').ok_or("unexpected generic parameter")?;
                body.truncate(open);
                while i < fragments.len() && !(fragments[i].kind == "text" && fragments[i].spelling.starts_with('>')) {
                    i += 1;
                }
                let rest = fragments.get(i).map(|f| &f.spelling[1..]).unwrap_or("");
                body.push_str(&generic_clause(&own_params, &symbol.constraints, module_name)?);
                body.push_str(rest);
                own_generics_skipped = true;
            }
            // `Outer.Inner`: the graph gives each name its own fragment; an inner one is written with its kind only.
            "typeIdentifier" if i >= 2 && fragments[i - 1].spelling == "." && fragments[i - 2].kind == "typeIdentifier" => {
                body.push_str(&nested_name(f)?)
            }
            "typeIdentifier" => body.push_str(&type_name(f, &all_params, module_name)?),
            _ => body.push_str(&f.spelling),
        }
        i += 1;
    }
    // Requirements that are not a generic parameter's own conformance or superclass stay in a `where` clause, with
    // those the member adds to its type's parameters.
    let rest: Vec<String> = symbol
        .constraints
        .iter()
        .filter(|(kind, lhs, _, _)| {
            own_params.contains(&root(lhs)) && (lhs.contains('.') || !matches!(kind.as_str(), "conformance" | "superclass"))
        })
        .chain(member_constraints.iter().copied())
        .map(|(kind, lhs, rhs, usr)| requirement(kind, lhs, rhs, usr.as_deref(), &all_params, module_name))
        .collect::<Result<_, _>>()?;
    let mut body = body.trim().to_string();
    // A property: `name: Type { get … }`, with the accessor written.
    if let Some(accessor) = accessor {
        let effects = match body.find(" {") {
            Some(at) => {
                // `{ get async throws set }`: the getter's effects only.
                let accessors = body[at..].trim_matches(|c| c == ' ' || c == '{' || c == '}');
                let effects = accessors.split_whitespace().filter(|w| matches!(*w, "async" | "throws")).collect::<Vec<_>>().join(" ");
                body.truncate(at);
                effects
            }
            None => String::new(),
        };
        body = match accessor {
            "get" if effects.is_empty() => format!("{body} {{ get }}"),
            "get" => format!("{body} {{ get {effects} }}"),
            _ => format!("{body} {{ set }}"),
        };
    }
    // A failable initializer returns an optional.
    if let Some(after) = body.strip_prefix("init?") {
        body = format!("init{after} -> {prefix}?");
    }
    if !rest.is_empty() {
        body = format!("{body} where {}", rest.join(", "));
    }
    let modifiers = if modifiers.is_empty() { String::new() } else { format!("{} ", modifiers.join(" ")) };
    Ok(format!("{modifiers}{extension}{prefix}.{body}"))
}

fn context_params_str(params: &[String]) -> Vec<&str> {
    params.iter().map(String::as_str).collect()
}

fn root(lhs: &str) -> &str {
    lhs.split('.').next().unwrap_or(lhs)
}

fn kind_mark(kind: apus_mangle::TypeKind) -> &'static str {
    use apus_mangle::TypeKind::*;
    match kind {
        Struct => "(struct)",
        Class => "(class)",
        Actor => "(actor)",
        Enum => "(enum)",
        Protocol => "(protocol)",
    }
}

/// `<T: P, U>`: the own generic parameters, each with its first conformance or superclass.
fn generic_clause(own: &[&str], constraints: &[(String, String, String, Option<String>)], module: &str) -> Result<String, String> {
    if own.is_empty() {
        return Ok(String::new());
    }
    let mut items = Vec::new();
    for param in own {
        let bounds: Vec<String> = constraints
            .iter()
            .filter(|(kind, lhs, _, _)| lhs == param && (kind == "conformance" || kind == "superclass"))
            .map(|(_, _, rhs, usr)| named(rhs, usr.as_deref(), module))
            .collect::<Result<_, _>>()?;
        items.push(if bounds.is_empty() { param.to_string() } else { format!("{param}: {}", bounds.join(" & ")) });
    }
    Ok(format!("<{}>", items.join(", ")))
}

fn requirement(kind: &str, lhs: &str, rhs: &str, usr: Option<&str>, params: &[&str], module: &str) -> Result<String, String> {
    let rhs = if params.contains(&root(rhs)) { rhs.to_string() } else { named(rhs, usr, module)? };
    Ok(match kind {
        "sameType" => format!("{lhs} == {rhs}"),
        _ => format!("{lhs}: {rhs}"),
    })
}

/// A type or protocol named by the graph, as apus writes it: the standard library's bare, another's with its module
/// and (for a type) its kind.
fn named(spelling: &str, usr: Option<&str>, module: &str) -> Result<String, String> {
    let Some(usr) = usr else { return Ok(spelling.to_string()) };
    let owner = graphs::usr_module(usr).ok_or_else(|| format!("`{spelling}`: unknown type"))?;
    // A standard library type with a substitution (`s:Si`, `s:SS`) apus knows by its bare name; one spelled out
    // (`s:s7DecoderP`, `s:s6HasherV`) is written with its module, or it would be taken for the declaring module's.
    let spelled_out = usr.strip_prefix("s:s").is_some_and(|rest| rest.starts_with(|c: char| c.is_ascii_digit()));
    if (owner == "Swift" && !spelled_out) || owner == "_Concurrency" {
        return Ok(spelling.to_string());
    }
    let kind = match usr.chars().last() {
        Some('V') => "(struct)",
        Some('C') => "(class)",
        Some('O') => "(enum)",
        Some('P') => "",
        _ => return Err(format!("`{spelling}` ({usr}): write the declaration in full")),
    };
    if owner == "__C" {
        return Err(format!("`{spelling}` is an imported C or Objective-C type: write the declaration in full"));
    }
    let _ = module; // a bare name would belong to `module`; written with its own module whatever it is
    Ok(format!("{owner}.{spelling}{kind}"))
}

/// A type nested in the one before it (`Inner` of `Outer.Inner`): its name and kind.
fn nested_name(f: &Fragment) -> Result<String, String> {
    let kind = match f.precise.as_deref().and_then(|usr| usr.chars().last()) {
        Some('V') => "(struct)",
        Some('C') => "(class)",
        Some('O') => "(enum)",
        _ => return Err(format!("`{}` ({:?}): write the declaration in full", f.spelling, f.precise)),
    };
    Ok(format!("{}{kind}", f.spelling))
}

fn type_name(f: &Fragment, params: &[&str], module: &str) -> Result<String, String> {
    if params.contains(&f.spelling.as_str()) || f.spelling == "Self" {
        return Ok(f.spelling.clone());
    }
    named(&f.spelling, f.precise.as_deref(), module)
}

/// The symbol a declaration by name's graph entry names, as the module exports it: its USR (`s:` + mangled name) with
/// the accessor's entity (`vp` → `vg`/`vs`, `ip` → `ig`/`is`) and an initializer's allocating entry (`fc` → `fC`).
/// `None` when `text` is not a declaration by name.
pub fn expected_symbol(text: &str) -> Option<String> {
    let short = short_form(text)?;
    let module = graphs::graphs().module(short.module)?;
    let mut path: Vec<&str> = short.context.clone();
    path.push(&short.title);
    let symbol = module.find(&path).find(|s| s.kind != "swift.typealias" && graphs::type_kind(&s.kind).is_none())?;
    usr_symbol(&symbol.usr, short.accessor)
}

/// The symbol a graph entry's USR names (see [`expected_symbol`]).
fn usr_symbol(usr: &str, accessor: Option<&str>) -> Option<String> {
    let mangled = format!("$s{}", usr.strip_prefix("s:")?);
    let accessor = if accessor == Some("set") { 's' } else { 'g' };
    let replace_entity = |m: &str, from: &str, to: String| m.strip_suffix(from).map(|base| format!("{base}{to}"));
    Some(
        replace_entity(&mangled, "vpZ", format!("v{accessor}Z"))
            .or_else(|| replace_entity(&mangled, "vp", format!("v{accessor}")))
            .or_else(|| replace_entity(&mangled, "ipZ", format!("i{accessor}Z")))
            .or_else(|| replace_entity(&mangled, "ip", format!("i{accessor}")))
            .or_else(|| replace_entity(&mangled, "fc", "fC".into()))
            .unwrap_or(mangled),
    )
}

/// How much of a framework `swift!` can call: every function, initializer, property getter and subscript getter in
/// the SDK's symbol graphs of the modules in `APUS_COVERAGE` (default: a few Swift-only frameworks), rewritten as
/// a declaration, mangled, checked as `swift!` checks it, and found among the module's exported symbols; the
/// reasons the others are refused, most common first.
/// `APUS_COVERAGE=StoreKit,SwiftData cargo test -p apus-macros coverage -- --ignored --nocapture`
#[cfg(test)]
mod tests {
    /// A protocol extension's member, from the SDK's graph: `(extension in …)`, as its symbol says.
    #[test]
    fn extension_members() {
        let text = "SwiftData.ModelActor.modelContext { get }";
        let expanded = super::expand(text).unwrap().unwrap();
        assert_eq!(expanded, "(extension in SwiftData) SwiftData.ModelActor(protocol).modelContext: SwiftData.ModelContext(class) { get }");
        let symbol = apus_mangle::Declaration::parse(&expanded).unwrap().symbol().unwrap();
        assert_eq!(Some(symbol), super::expected_symbol(text));
    }
    /// A nested type in a signature: its module once, each enclosing type with its kind.
    #[test]
    fn nested_types() {
        let text = "StoreKit.Product.SubscriptionInfo.RenewalInfo.AdvancedCommerceInfo.items { get }";
        let expanded = super::expand(text).unwrap().unwrap();
        assert!(expanded.ends_with(
            "items: [StoreKit.Product(struct).SubscriptionInfo(struct).RenewalInfo(struct).AdvancedCommerceInfo(struct).Item(struct)] { get }"
        ), "{expanded}");
        let symbol = apus_mangle::Declaration::parse(&expanded).unwrap().symbol().unwrap();
        assert_eq!(Some(symbol), super::expected_symbol(text));
    }
}

#[cfg(test)]
mod coverage {
    use super::{graphs, rewrite, usr_symbol};
    use std::collections::BTreeMap;

    /// A refusal's message with what names one declaration (`…` in backticks, numbers) taken out, to group them.
    fn reason(stage: &str, message: &str) -> String {
        let mut out = String::new();
        let mut quoted = false;
        for c in message.chars() {
            match c {
                '`' if quoted => {
                    quoted = false;
                    out.push_str("`…`");
                }
                '`' => quoted = true,
                _ if quoted => {}
                c if c.is_ascii_digit() => {}
                c => out.push(c),
            }
        }
        format!("{stage}: {}", out.chars().take(150).collect::<String>())
    }

    #[test]
    #[ignore]
    fn coverage() {
        graphs::graphs();
        let modules = std::env::var("APUS_COVERAGE").unwrap_or_else(|_| "StoreKit,SwiftData,MusicKit,WeatherKit,AppIntents,SwiftUI".into());
        for name in modules.split(',') {
            let Some(module) = graphs::graphs().module(name) else {
                println!("{name}: no symbol graph");
                continue;
            };
            let exported = crate::sdk::exported_symbols(name);
            let mut reasons: BTreeMap<String, (usize, String)> = BTreeMap::new();
            let (mut total, mut callable, mut generic) = (0, 0, 0);
            for symbol in &module.symbols {
                let accessor = match symbol.kind.as_str() {
                    "swift.func" | "swift.method" | "swift.type.method" | "swift.init" => None,
                    "swift.property" | "swift.type.property" | "swift.subscript" | "swift.type.subscript" => Some("get"),
                    _ => continue,
                };
                // A protocol extension's member shown on a conforming type: called through the protocol.
                if symbol.usr.contains("::SYNTHESIZED::") {
                    continue;
                }
                total += 1;
                let path: Vec<&str> = symbol.path.iter().map(String::as_str).collect();
                let outcome = (|| {
                    let declaration = rewrite(name, module, &path[..path.len() - 1], symbol, accessor).map_err(|e| reason("rewrite", &e))?;
                    let parsed = apus_mangle::Declaration::parse_resolving(&declaration, graphs::graphs()).map_err(|e| {
                        if std::env::var_os("APUS_COVERAGE_DETAIL").is_some() {
                            println!("PARSE {declaration}\n  {e}");
                        }
                        reason("parse", &e.to_string())
                    })?;
                    let mangled = parsed.symbol().map_err(|e| reason("mangle", &e.to_string()))?;
                    if let Some(expected) = usr_symbol(&symbol.usr, accessor).filter(|expected| *expected != mangled) {
                        if std::env::var_os("APUS_COVERAGE_DETAIL").is_some() {
                            println!("DIFF {declaration}\n  want {expected}\n  got  {mangled}");
                        }
                        return Err("mangle: differs from the symbol graph's USR".to_string());
                    }
                    let bound = !parsed.generic_parameter_names().is_empty();
                    let checked = if bound { parsed.check_bound_callable() } else { parsed.check_callable() };
                    checked.map_err(|e| reason("refused", &e.to_string()))?;
                    let call = apus_mangle::call_symbol(&declaration).map_err(|e| reason("mangle", &e.to_string()))?;
                    let thunk = apus_mangle::dispatch_thunk(&declaration).ok().flatten();
                    let suffix = if parsed.is_async() { "Tu" } else { "" };
                    if let Some(exported) = exported {
                        if !std::iter::once(&call).chain(thunk.as_ref()).any(|s| exported.contains(&format!("{s}{suffix}"))) {
                            return Err("not exported (inlinable, or a protocol's default implementation)".to_string());
                        }
                    }
                    Ok((bound, declaration))
                })();
                match outcome {
                    Ok((bound, _)) => {
                        callable += 1;
                        generic += usize::from(bound);
                    }
                    Err(why) => {
                        let entry = reasons.entry(why).or_insert((0, path.join(".")));
                        entry.0 += 1;
                    }
                }
            }
            println!("\n{name}: {callable}/{total} callable ({generic} of them bound generics)");
            let mut sorted: Vec<_> = reasons.into_iter().collect();
            sorted.sort_by(|a, b| b.1 .0.cmp(&a.1 .0));
            for (why, (count, example)) in sorted.iter().take(25) {
                println!("  {count:5}  {why}   e.g. {example}");
            }
        }
    }
}
