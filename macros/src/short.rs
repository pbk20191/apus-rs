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
    // The type's own requirements are part of its declaration; others on its parameters come from an extension.
    let context_constraints: Vec<_> = symbol.constraints.iter().filter(|(_, lhs, _, _)| !own_params.contains(&root(lhs))).collect();
    if !context_constraints.is_empty() {
        let enclosing = module.find(context).find(|s| graphs::type_kind(&s.kind).is_some());
        let declared = |c: &&(String, String, String, Option<String>)| enclosing.is_some_and(|e| e.constraints.contains(c));
        if !context_constraints.iter().all(declared) {
            return Err("a member of a constrained extension: write the declaration in full, with its `(extension in …)`".into());
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
            "typeIdentifier" => body.push_str(&type_name(f, &all_params, module_name)?),
            _ => body.push_str(&f.spelling),
        }
        i += 1;
    }
    // Requirements that are not a generic parameter's own conformance or superclass stay in a `where` clause.
    let rest: Vec<String> = symbol
        .constraints
        .iter()
        .filter(|(kind, lhs, _, _)| {
            own_params.contains(&root(lhs)) && (lhs.contains('.') || !matches!(kind.as_str(), "conformance" | "superclass"))
        })
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
    Ok(format!("{modifiers}{prefix}.{body}"))
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
    if owner == "Swift" || owner == "_Concurrency" {
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
    let mangled = format!("$s{}", symbol.usr.strip_prefix("s:")?);
    let accessor = if short.accessor == Some("set") { 's' } else { 'g' };
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
