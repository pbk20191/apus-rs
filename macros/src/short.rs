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
    // The context as a type (a failable initializer's result): without the requirements.
    let mut self_type = module_name.to_string();
    let mut context_params: Vec<String> = Vec::new();
    // An Objective-C class or C type the module extends (`RunLoop`, `Decimal`) is mangled by its C name, and what the
    // module declares in it as an extension's (`So9NSRunLoopC10FoundationE17SchedulerTimeTypeV`).
    // An imported type nested in another (`NSTextCheckingResult.CheckingType`) is in `__C` all the same.
    let mut imported_at = None;
    for i in 0..context.len() {
        let enclosing = module.find(&context[..=i]).find(|s| graphs::type_kind(&s.kind).is_some()).ok_or_else(|| format!("no type `{}`", context[..=i].join(".")))?;
        let kind = kind_mark(graphs::type_kind(&enclosing.kind).expect("a type"));
        // The requirements an enclosing type's extension adds to those of the type it extends.
        let outer = (i > 0).then(|| module.find(&context[..i]).find(|s| graphs::type_kind(&s.kind).is_some())).flatten();
        let added = |e: &graphs::Extension| -> Vec<(String, String, String, Option<String>)> {
            e.constraints.iter().filter(|c| !outer.is_some_and(|o| o.constraints.contains(c))).cloned().collect()
        };
        let own: Vec<String> = enclosing.params.iter().map(|(n, _)| n.clone()).filter(|n| !context_params.contains(n)).collect();
        let segment = match imported_name(&enclosing.usr, &enclosing.kind) {
            Some(name) => {
                prefix.clear();
                self_type.clear();
                imported_at = Some(i);
                name
            }
            None if enclosing.usr.starts_with("c:") => return Err(format!("`{}` ({}): write the declaration in full", context[..=i].join("."), enclosing.usr)),
            None if imported_at.is_some_and(|at| at + 1 == i) => format!(".(extension in {module_name}) {}{kind}", context[i]),
            // A type declared in a constrained extension of the one enclosing it (`extension Measurement where
            // UnitType: Dimension { struct AttributedStyle }`), or in another module's: marked with it.
            None if i > 0 && enclosing.extension.as_ref().is_some_and(|e| !added(e).is_empty() || e.extended_module != module_name) => {
                let e = enclosing.extension.as_ref().expect("an extension");
                let params = context_params_str(&context_params);
                let requirements: Vec<String> = added(e)
                    .into_iter()
                    .map(|(kind, lhs, rhs, usr)| requirement(&kind, &lhs, &rhs, usr.as_deref(), &params, module_name))
                    .collect::<Result<_, _>>()?;
                match requirements.is_empty() {
                    true => format!(".(extension in {module_name}) {}{kind}", context[i]),
                    false => format!(".(extension in {module_name} where {}) {}{kind}", requirements.join(", "), context[i]),
                }
            }
            None => format!(".{}{kind}", context[i]),
        };
        // The declaration's context carries the type's own requirements (`Form<Content: View>`): a member's signature
        // that fixes or constrains a parameter is mangled differently when the type is constrained.
        let own_refs: Vec<&str> = own.iter().map(String::as_str).collect();
        prefix.push_str(&format!("{segment}{}", generic_clause(&own_refs, &enclosing.constraints, module_name)?));
        self_type.push_str(&segment);
        if !own.is_empty() {
            self_type.push_str(&format!("<{}>", own.join(", ")));
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
        if protocol || e.extended_module != module_name || !extension_constraints.is_empty() || imported_at.is_some_and(|at| at + 1 == context.len()) {
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
    let mut outer = 0; // where the last type written starts
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
            ("attribute", "@discardableResult") => {}
            // Its symbol leaves out `@Sendable` and global actors in function types, and marker protocols.
            ("attribute", "@preconcurrency") => modifiers.insert(0, "@preconcurrency".into()),
            // The framework exports the function on the systems it names (`.tbd`-checked as any other); its body
            // offered for inlining, or for dynamic replacement, the symbol stays the one to call.
            (_, a) if a.trim_start().starts_with("@backDeployed") || a.trim_start().starts_with("@export(") => {}
            ("keyword" | "attribute", "dynamic") => {}
            ("attribute", "@MainActor") => modifiers.push("@MainActor".into()),
            // `@` then `MainActor` as a type.
            ("attribute", "@") if fragments.get(i + 1).is_some_and(|n| n.spelling == "MainActor") => {
                modifiers.push("@MainActor".into());
                i += 1;
            }
            ("attribute", "nonisolated") => modifiers.push("nonisolated".into()),
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
            // A parameter's result builder (`@ContentBuilder actions:`): a compile-time feature, not part of the type.
            "attribute" if f.spelling == "@" && fragments.get(i + 1).is_some_and(|n| n.kind == "attribute" && n.spelling.ends_with("Builder")) => {
                i += 1;
                if fragments.get(i + 1).is_some_and(|n| n.kind == "text" && n.spelling == " ") {
                    i += 1;
                }
            }
            "attribute" if f.spelling.starts_with('@') && f.spelling.ends_with("Builder") => {
                if fragments.get(i + 1).is_some_and(|n| n.kind == "text" && n.spelling == " ") {
                    i += 1;
                }
            }
            "typeIdentifier" => {
                if replaces_outer(fragments, i) {
                    body.truncate(outer);
                }
                outer = body.len();
                body.push_str(&type_fragment(fragments, i, &all_params, module_name, 0)?);
            }
            _ => body.push_str(&f.spelling),
        }
        i += 1;
    }
    // Requirements that are not a generic parameter's own conformance or superclass stay in a `where` clause, with
    // those the member adds to its type's parameters.
    let mut rest: Vec<String> = symbol
        .constraints
        .iter()
        .filter(|(kind, lhs, _, _)| {
            own_params.contains(&root(lhs)) && (lhs.contains('.') || !matches!(kind.as_str(), "conformance" | "superclass"))
        })
        .chain(member_constraints.iter().copied())
        .map(|(kind, lhs, rhs, usr)| requirement(kind, lhs, rhs, usr.as_deref(), &all_params, module_name))
        .collect::<Result<_, _>>()?;
    // `swiftGenerics` leaves out a layout requirement (`where Value: AnyObject`): read from the declaration's own
    // `where` clause.
    for layout in any_object_requirements(fragments) {
        if !rest.contains(&layout) {
            rest.push(layout);
        }
    }
    let mut body = packs(&strip_defaults(body.trim()));
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
        body = format!("init{after} -> {self_type}?");
    }
    if !rest.is_empty() {
        body = format!("{body} where {}", rest.join(", "));
    }
    let modifiers = if modifiers.is_empty() { String::new() } else { format!("{} ", modifiers.join(" ")) };
    let declaration = |body: &str| format!("{modifiers}{extension}{prefix}.{body}");
    let body = shared_parameters(&body, usr_symbol(&symbol.usr, accessor).as_deref(), &declaration);
    Ok(declaration(&body))
}

/// `body` with `__shared` written on the parameters its symbol marks so (`h`), which the graph's declaration leaves
/// out: those whose mark alone brings the symbol written closer to `expected`, which differs from it by `h`s only.
/// The symbol of the result is still compared with the graph's as any other.
fn shared_parameters(body: &str, expected: Option<&str>, declaration: &dyn Fn(&str) -> String) -> String {
    let symbol = |body: &str| apus_mangle::Declaration::parse_resolving(&declaration(body), graphs::graphs()).ok()?.symbol().ok();
    // `expected` is `symbol` with `h`s inserted.
    let marked = |symbol: &str, expected: &str| {
        let mut want = expected.chars().peekable();
        for c in symbol.chars() {
            while want.peek().is_some_and(|&w| w != c && w == 'h') {
                want.next();
            }
            if want.next() != Some(c) {
                return false;
            }
        }
        want.all(|w| w == 'h')
    };
    let Some(expected) = expected else { return body.to_string() };
    let Some(plain) = symbol(body) else { return body.to_string() };
    if plain == expected || !marked(&plain, expected) {
        return body.to_string();
    }
    let starts = parameter_types(body);
    let shared: Vec<usize> = starts
        .iter()
        .copied()
        .filter(|&at| {
            let one = format!("{}__shared {}", &body[..at], &body[at..]);
            symbol(&one).is_some_and(|s| s.len() > plain.len() && marked(&s, expected))
        })
        .collect();
    let mut out = body.to_string();
    for at in shared.into_iter().rev() {
        out.insert_str(at, "__shared ");
    }
    out
}

/// Where each parameter's type starts in `body` (`init(a: T, _: U)`): after its label's `: `, unless it already has
/// an ownership or `inout`.
fn parameter_types(body: &str) -> Vec<usize> {
    let mut starts = Vec::new();
    let (mut angle, mut depth, mut open) = (0usize, 0usize, None);
    let mut expecting = false;
    for (at, c) in body.char_indices() {
        match c {
            '<' if open.is_none() => angle += 1,
            '>' if open.is_none() && !body[..at].ends_with('-') => angle = angle.saturating_sub(1),
            '(' if open.is_none() && angle == 0 => {
                open = Some(at);
                expecting = true;
            }
            _ if open.is_none() => {}
            '(' | '[' | '<' => depth += 1,
            '>' if body[..at].ends_with('-') => {}
            ')' if depth == 0 => break,
            ')' | ']' | '>' => depth -= 1,
            ',' if depth == 0 => expecting = true,
            ':' if depth == 0 && expecting && body[at..].starts_with(": ") => {
                expecting = false;
                let start = at + 2;
                let rest = &body[start..];
                if !["inout ", "consuming ", "borrowing ", "__owned ", "__shared ", "sending "].iter().any(|o| rest.starts_with(o)) {
                    starts.push(start);
                }
            }
            _ => {}
        }
    }
    starts
}

/// The parameter list without default values (` = …`), which neither the symbol nor the call depends on; apus
/// reads `= #isolation` only, which makes the caller pass its isolation.
fn strip_defaults(body: &str) -> String {
    // The first `(` outside the generic clause (`f<T: M.P(protocol)>(…)`).
    let mut angle = 0usize;
    let open = body.char_indices().find(|&(at, c)| {
        match c {
            '<' => angle += 1,
            '>' if !body[..at].ends_with('-') => angle = angle.saturating_sub(1),
            '(' if angle == 0 => return true,
            _ => {}
        }
        false
    });
    let Some((open, _)) = open else { return body.to_string() };
    let mut out = String::from(&body[..=open]);
    let (mut depth, mut skipping) = (0usize, false);
    let chars: Vec<char> = body[open + 1..].chars().collect();
    let mut k = 0;
    while k < chars.len() {
        let c = chars[k];
        if depth == 0 && !skipping && c == ' ' && chars.get(k + 1) == Some(&'=') && chars.get(k + 2) == Some(&' ') {
            let rest: String = chars[k + 3..].iter().collect();
            if !rest.starts_with("#isolation") {
                skipping = true;
                k += 3;
                continue;
            }
        }
        match c {
            '(' | '[' | '<' => depth += 1,
            ')' | ']' | '>' if depth == 0 && c == ')' => {
                out.extend(&chars[k..]);
                return out;
            }
            ')' | ']' => depth -= 1,
            // `->` is no closing bracket.
            '>' if k > 0 && chars[k - 1] == '-' => {}
            '>' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => skipping = false,
            _ => {}
        }
        if !skipping {
            out.push(c);
        }
        k += 1;
    }
    out
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
    // An Objective-C class is mangled by its Objective-C name (`So10CLLocationC`), whatever Swift calls it.
    if let Some(class) = usr.strip_prefix("c:objc(cs)") {
        return Ok(format!("__C.{class}(class)"));
    }
    if usr == "s:s4Voida" {
        return Ok("()".into());
    }
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

/// The type the fragment at `i` names: a typealias as the type it stands for (Swift mangles that), a type nested in
/// the one before it (`Inner` of `Outer.Inner`, which the graph splits) with its kind only, any other with its module.
/// A typealias or C type nested in another (`Notification.Name`) is written whole: see [`replaces_outer`].
fn type_fragment(fragments: &[Fragment], i: usize, params: &[&str], module: &str, depth: usize) -> Result<String, String> {
    let f = &fragments[i];
    let nested = i >= 2 && fragments[i - 1].spelling.ends_with('.') && fragments[i - 2].kind == "typeIdentifier";
    let usr = f.precise.as_deref();
    if let Some(usr) = usr.filter(|usr| imported(usr)) {
        return imported_type(f, usr, params, module, depth);
    }
    match (usr.filter(|usr| swift_alias(usr)), nested) {
        (Some(usr), _) => {
            let declared = graphs::usr_module(usr).and_then(|m| graphs::graphs().module(&m));
            let alias = declared.and_then(|m| m.by_usr(usr, Some("swift.typealias")).next());
            let alias = alias.ok_or_else(|| format!("`{}` ({usr}): write the declaration in full", f.spelling))?;
            expand_alias(alias, f, params, module, depth)
        }
        (None, nested) => {
            // Only a type with generic arguments may be variadic (its module's graph is read then).
            let arguments = fragments.get(i + 1).is_some_and(|n| n.spelling.starts_with('<'));
            let mark = if arguments { variadic_mark(f) } else { Some("") };
            let mark = mark.ok_or_else(|| format!("`{}`: a variadic type with other parameters: write the declaration in full", f.spelling))?;
            let name = match (nested, nested_standard_type(fragments, i)) {
                (true, _) => nested_name(f, &fragments[i - 2], params, module)?,
                // The standard library type a nested type is in (`String` of `String.Index`), written whole: bare,
                // `String` would be read as the nested type's module.
                (false, Some(kind)) => format!("Swift.{}{kind}", f.spelling),
                (false, None) => type_name(f, params, module)?,
            };
            Ok(name + mark)
        }
    }
}

/// A Swift typealias's USR (`Void` is written `()`, see `named`; an associated type is not one).
fn swift_alias(usr: &str) -> bool {
    usr.starts_with("s:") && usr.ends_with('a') && !usr.ends_with("Qa") && usr != "s:s4Voida"
}

/// A C declaration's USR (`c:@S@CGRect`, `c:@T@NSURLResourceKey`); an Objective-C class's is `named`'s.
fn imported(usr: &str) -> bool {
    usr.starts_with("c:@")
}

/// A type imported from C or Objective-C as apus writes it, from its USR and graph kind: `__C.NSRunLoop(class)`,
/// `__C.NSDecimal(typedef struct)` (see [`imported_type`]); `None` for another, or a typedef the graph names otherwise.
fn imported_name(usr: &str, kind: &str) -> Option<String> {
    if let Some(class) = usr.strip_prefix("c:objc(cs)") {
        return Some(format!("__C.{class}(class)"));
    }
    let (c_kind, name) = usr.strip_prefix("c:@")?.split_once('@')?;
    match (c_kind, kind) {
        ("S" | "E", _) => Some(format!("__C.{name}(struct)")),
        ("SA", _) | ("T", "swift.struct") => Some(format!("__C.{name}(typedef struct)")),
        ("T", "swift.class") => Some(format!("__C.{name}(typedef class)")),
        _ => None,
    }
}

/// The module of an extension the USR of a type declared in it names, for a type extending an imported one
/// (`s:So9NSRunLoopC10FoundationE17SchedulerTimeTypeV`: Foundation).
fn imported_extension_module(usr: &str) -> Option<&str> {
    fn identifier(s: &str) -> Option<(&str, &str)> {
        let digits = s.chars().take_while(|c| c.is_ascii_digit()).count();
        let length: usize = s[..digits].parse().ok()?;
        let rest = &s[digits..];
        (rest.len() >= length).then(|| rest.split_at(length))
    }
    let (_, rest) = identifier(usr.strip_prefix("s:So")?)?;
    let rest = rest.strip_prefix(['C', 'V', 'a'])?;
    let (module, rest) = identifier(rest)?;
    rest.starts_with('E').then_some(module)
}

/// Whether the fragment at `i`, nested in the type before it, is written whole (a typealias's type or a C type's own
/// name), so the outer type and its `.` already written are dropped.
fn replaces_outer(fragments: &[Fragment], i: usize) -> bool {
    let nested = i >= 2 && fragments[i - 1].spelling == "." && fragments[i - 2].kind == "typeIdentifier";
    nested && fragments[i].precise.as_deref().is_some_and(|usr| imported(usr) || swift_alias(usr))
}

/// A type imported from C, mangled by its C name in `__C` (`So6CGRectV`): a struct or enum (`NS_ENUM`, `NS_OPTIONS`)
/// as a struct, an anonymous struct by its typedef (`So9NSDecimala`), a typedef as what the graph of `module` makes of
/// it: a type named after it (`NS_TYPED_ENUM`, a Core Foundation class) or the type it stands for. A module-qualified
/// enum USR (`c:@M@_DarwinFoundation1@E@POSIXErrorCode`) is an `@objc` enum of that Swift module, mangled there.
fn imported_type(f: &Fragment, usr: &str, params: &[&str], module: &str, depth: usize) -> Result<String, String> {
    let full = || format!("`{}` ({usr}): write the declaration in full", f.spelling);
    let (kind, name) = usr[3..].split_once('@').ok_or_else(full)?;
    match kind {
        "M" => match name.split_once("@E@") {
            Some((swift_module, enumeration)) if !enumeration.contains('@') => Ok(format!("{swift_module}.{enumeration}(enum)")),
            _ => Err(full()),
        },
        "S" | "E" => Ok(format!("__C.{name}(struct)")),
        "SA" => Ok(format!("__C.{name}(typedef struct)")),
        "T" => {
            let graph = graphs::graphs().module(module).ok_or_else(full)?;
            let symbol = graph.by_usr(usr, None).find(|s| matches!(s.kind.as_str(), "swift.struct" | "swift.class" | "swift.typealias"));
            match symbol.map(|s| (s.kind.as_str(), s)) {
                Some(("swift.struct", _)) => Ok(format!("__C.{name}(typedef struct)")),
                Some(("swift.class", _)) => Ok(format!("__C.{name}(typedef class)")),
                Some((_, alias)) if depth < 4 => expand_alias(alias, f, params, module, depth),
                _ => Err(full()),
            }
        }
        _ => Err(full()),
    }
}

/// `typealias Name = Type`: what follows ` = `, which names no generic parameter of the alias's context.
fn expand_alias(alias: &graphs::Symbol, f: &Fragment, params: &[&str], module: &str, depth: usize) -> Result<String, String> {
    let full = || format!("`{}` ({}): write the declaration in full", f.spelling, alias.usr);
    if depth >= 4 {
        return Err(full());
    }
    let start = alias.fragments.iter().position(|f| f.kind == "text" && f.spelling.contains('=')).ok_or_else(full)?;
    let target = &alias.fragments[start + 1..];
    if target.iter().any(|f| f.kind == "typeIdentifier" && f.precise.is_none()) {
        return Err(format!("`{}` ({}): a generic typealias: write the declaration in full", f.spelling, alias.usr));
    }
    let mut written = alias.fragments[start].spelling.split('=').nth(1).unwrap_or("").trim_start().to_string();
    let mut outer = written.len();
    for j in start + 1..alias.fragments.len() {
        match alias.fragments[j].kind.as_str() {
            "typeIdentifier" => {
                if replaces_outer(&alias.fragments, j) {
                    written.truncate(outer);
                }
                outer = written.len();
                written.push_str(&type_fragment(&alias.fragments, j, params, module, depth + 1)?);
            }
            _ => written.push_str(&alias.fragments[j].spelling),
        }
    }
    Ok(written)
}

/// Marks a variadic generic type (`struct Predicate<each Input>`, from its module's graph), whose arguments
/// [`packs`] writes as its one pack.
const VARIADIC: char = '\u{1}';

fn variadic_mark(f: &Fragment) -> Option<&'static str> {
    let Some(usr) = f.precise.as_deref() else { return Some("") };
    let module = graphs::usr_module(usr).and_then(|m| graphs::graphs().module(&m));
    let Some(ty) = module.and_then(|m| m.by_usr(usr, None).next()) else { return Some("") };
    match ty.fragments.iter().any(|f| f.kind == "text" && f.spelling.contains("each ")) {
        false => Some(""),
        // One parameter, the pack: anything else is not written.
        true if ty.params.len() == 1 => Some("\u{1}"),
        true => None,
    }
}

/// `Predicate\u{1}<A, B>` as `Predicate<Pack{A, B}>`.
fn packs(body: &str) -> String {
    let mut out = String::new();
    let mut closing: Vec<usize> = Vec::new(); // angle depths whose `>` closes a pack
    let mut depth = 0usize;
    let mut chars = body.chars().peekable();
    let mut previous = ' ';
    while let Some(c) = chars.next() {
        match c {
            VARIADIC if chars.peek() == Some(&'<') => {
                chars.next();
                depth += 1;
                closing.push(depth);
                out.push_str("<Pack{");
                previous = '<';
                continue;
            }
            VARIADIC => continue,
            '<' => depth += 1,
            '>' if previous != '-' => {
                if closing.last() == Some(&depth) {
                    closing.pop();
                    out.push('}');
                }
                depth = depth.saturating_sub(1);
            }
            _ => {}
        }
        out.push(c);
        previous = c;
    }
    out
}

/// A type nested in the one before it (`Inner` of `Outer.Inner`): its name and kind.
fn nested_name(f: &Fragment, outer: &Fragment, params: &[&str], module: &str) -> Result<String, String> {
    let kind = match f.precise.as_deref().and_then(|usr| usr.chars().last()) {
        Some('V') => "(struct)",
        Some('C') => "(class)",
        Some('O') => "(enum)",
        _ => return Err(format!("`{}` ({:?}): write the declaration in full", f.spelling, f.precise)),
    };
    // A type declared in an extension of an imported one (`RunLoop.SchedulerTimeType`) is marked with it.
    if outer.precise.as_deref().is_some_and(|usr| usr.starts_with("c:")) {
        let module = f.precise.as_deref().and_then(imported_extension_module);
        let module = module.ok_or_else(|| format!("`{}` ({:?}): write the declaration in full", f.spelling, f.precise))?;
        return Ok(format!("(extension in {module}) {}{kind}", f.spelling));
    }
    // One another module declares in a Swift type's extension (`String.Encoding`, Foundation's): its USR is the
    // outer's, then that module and `E` (Mangling.rst's extension context).
    let extension = (f.precise.as_deref().zip(outer.precise.as_deref()))
        .and_then(|(usr, outer)| usr.strip_prefix(outer).filter(|_| outer.starts_with("s:")))
        .and_then(|rest| {
            // A leading `0` starts an identifier with word substitutions (`05UnitsE0V`), not a module's.
            let digits = rest.chars().take_while(|c| c.is_ascii_digit()).count();
            let len: usize = rest[..digits].parse().ok().filter(|_| !rest.starts_with('0'))?;
            let module = rest.get(digits..digits + len)?;
            rest[digits + len..].starts_with('E').then_some(module)
        });
    if let Some(module) = extension {
        return Ok(format!("(extension in {module}) {}{kind}", f.spelling));
    }
    // One declared in a constrained extension of the type enclosing it (`Measurement<UnitType>.AttributedStyle`, in
    // `extension Measurement where UnitType: Dimension`): marked with its requirements, as its context is.
    let declared = f.precise.as_deref().and_then(|usr| graphs::usr_module(usr)).and_then(|m| graphs::graphs().module(&m));
    let nested = declared.zip(f.precise.as_deref()).and_then(|(m, usr)| m.by_usr(usr, None).next());
    if let Some((e, nested)) = nested.and_then(|n| n.extension.as_ref().map(|e| (e, n))) {
        // The type it is nested in, by its path (the fragment before it may be a generic argument).
        let path: Vec<&str> = nested.path[..nested.path.len() - 1].iter().map(String::as_str).collect();
        let outer = declared.and_then(|m| m.find(&path).find(|s| graphs::type_kind(&s.kind).is_some()));
        let added: Vec<_> = e.constraints.iter().filter(|c| !outer.is_some_and(|o| o.constraints.contains(c))).collect();
        if !added.is_empty() {
            let requirements: Vec<String> = added
                .into_iter()
                .map(|(kind, lhs, rhs, usr)| requirement(kind, lhs, rhs, usr.as_deref(), params, module))
                .collect::<Result<_, _>>()?;
            return Ok(format!("(extension in {} where {}) {}{kind}", e.extended_module, requirements.join(", "), f.spelling));
        }
    }
    Ok(format!("{}{kind}", f.spelling))
}

/// The kind of the standard library type at `i` when a nested type follows it (`String` in `String.Index`).
fn nested_standard_type(fragments: &[Fragment], i: usize) -> Option<&'static str> {
    let followed = fragments.get(i + 1).is_some_and(|f| f.spelling == ".") && fragments.get(i + 2).is_some_and(|f| f.kind == "typeIdentifier");
    if !followed {
        return None;
    }
    // Mangling.rst's standard substitutions of the types that declare nested types.
    match fragments[i].precise.as_deref()? {
        "s:SS" | "s:Sa" | "s:SD" | "s:Sh" | "s:Ss" => Some("(struct)"),
        "s:Sq" => Some("(enum)"),
        _ => None,
    }
}

/// The `T: AnyObject` requirements of a declaration's `where` clause (`… where Value : AnyObject, Value : Observable`).
fn any_object_requirements(fragments: &[Fragment]) -> Vec<String> {
    let Some(at) = fragments.iter().position(|f| f.kind == "keyword" && f.spelling == "where") else { return Vec::new() };
    let clause: String = fragments[at + 1..].iter().map(|f| f.spelling.as_str()).collect();
    clause
        .split(',')
        .filter_map(|item| {
            let (lhs, rhs) = item.split_once(':')?;
            (rhs.trim() == "AnyObject").then(|| format!("{}: AnyObject", lhs.trim()))
        })
        .collect()
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
    /// A member of an extension that fixes its type's parameter (`where Content == FormStyleConfiguration.Content`):
    /// the type's own requirements are written too, as they change the generic signature (`…RszrlE`, not `…RszlE`).
    #[test]
    fn fixed_type_parameters() {
        let text = "SwiftUI.Form.init(_:)";
        let expanded = super::expand(text).unwrap().unwrap();
        assert!(expanded.contains("SwiftUI.Form(struct)<Content: SwiftUI.View>"), "{expanded}");
        let symbol = apus_mangle::Declaration::parse(&expanded).unwrap().symbol().unwrap();
        assert_eq!(Some(symbol), super::expected_symbol(text), "{expanded}");
    }
    /// What the graph says outside `swiftGenerics`: `@preconcurrency` (its symbol drops `@MainActor` from function
    /// types), a layout requirement (`where Value: AnyObject`, `RlzC`), and a standard library type's nested type
    /// (`String.Index`, `SS5IndexV`).
    #[test]
    fn attributes_layouts_and_nested_standard_types() {
        let check = |text: &str, contains: &str| {
            let expanded = super::expand(text).unwrap().unwrap();
            assert!(expanded.contains(contains), "{expanded}");
            let symbol = apus_mangle::Declaration::parse(&expanded).unwrap().symbol().unwrap();
            assert_eq!(Some(symbol), super::expected_symbol(text), "{expanded}");
        };
        check("SwiftUI.Button.init(action:label:)", "@preconcurrency");
        check("Foundation.UndoManager.registerUndo(withTarget:handler:)", "TargetType: AnyObject");
        check("Foundation.URLComponents.rangeOfHost { get }", "Swift.String(struct).Index(struct)");
    }
    /// Imported Objective-C classes (by their Objective-C name), `@backDeployed` and `@MainActor` attributes.
    #[test]
    fn imported_classes_and_attributes() {
        let check = |text: &str, contains: &str| {
            let expanded = super::expand(text).unwrap().unwrap();
            assert!(expanded.contains(contains), "{expanded}");
            let symbol = apus_mangle::Declaration::parse(&expanded).unwrap().symbol().unwrap();
            assert_eq!(Some(symbol), super::expected_symbol(text), "{expanded}");
        };
        check("WeatherKit.WeatherMetadata.location { get }", "location: __C.CLLocation(class) { get }");
        check("WeatherKit.DayWeather.highTemperature { get }", "Foundation.Measurement(struct)<__C.NSUnitTemperature(class)>");
        check("StoreKit.Product.SubscriptionPeriod.weekly { get }", "static StoreKit.Product(struct).SubscriptionPeriod(struct).weekly");
        check("StoreKit.AppStore.presentMultiSeatInvitationSheet(from:originalTransactionID:)", "@MainActor static");
    }

    /// Types imported from C by their C names: a struct or enum (`NS_OPTIONS`) as `__C.Name(struct)`, one Swift names
    /// after its typedef (`NS_TYPED_ENUM`, an anonymous struct) as `typedef struct`, a plain typedef as its type.
    #[test]
    fn imported_c_types() {
        let check = |text: &str, contains: &str| {
            let expanded = super::expand(text).unwrap().unwrap();
            assert!(expanded.contains(contains), "{expanded}");
            let symbol = apus_mangle::Declaration::parse(&expanded).expect(&expanded).symbol().unwrap();
            assert_eq!(Some(symbol), super::expected_symbol(text), "{expanded}");
        };
        check(
            "Foundation.FileManager.replaceItemAt(_:withItemAt:backupItemName:options:)",
            "(extension in Foundation) __C.NSFileManager(class).replaceItemAt(_: Foundation.URL(struct), withItemAt: Foundation.URL(struct), backupItemName: String?, options: __C.NSFileManagerItemReplacementOptions(struct))",
        );
        check("Foundation.URLResourceValues.allValues { get }", "__C.NSURLResourceKey(typedef struct)");
        check("Foundation.RunLoop.SchedulerTimeType.Stride.magnitude { get }", "magnitude: Double");
        check("Foundation.Decimal.FormatStyle.attributed { get }", "__C.NSDecimal(typedef struct)");
        // An `@objc` enum of a Swift module (`c:@M@…`), mangled by its module's ABI name (`_DarwinFoundation1`: Darwin).
        check("Foundation.POSIXError.EPERM { get }", "_DarwinFoundation1.POSIXErrorCode(enum)");
        check("Foundation.MachError.success { get }", "Darwin.MachErrorCode(enum)");
        // Its value crosses as its raw integer, like a C enum's, whichever module it is written in.
        for text in [
            "static Foundation.POSIXError(struct).EPERM: _DarwinFoundation1.POSIXErrorCode(enum) { get }",
            "static Foundation.POSIXError(struct).EPERM: Darwin.POSIXErrorCode(enum) { get }",
            "static Foundation.MachError(struct).success: Darwin.MachErrorCode(enum) { get }",
        ] {
            let parsed = apus_mangle::Declaration::parse_resolving(text, crate::graphs::graphs()).unwrap();
            assert!(matches!(parsed.result_kind(), Some(apus_mangle::ResultKind::CValue(false))), "{text}");
        }
    }

    /// Default values are left out (`#isolation` aside); a typealias is written as the type it names.
    #[test]
    fn defaults_and_typealiases() {
        let check = |text: &str, contains: &str| {
            let expanded = super::expand(text).unwrap().unwrap();
            assert!(expanded.contains(contains), "{expanded}");
            let symbol = apus_mangle::Declaration::parse(&expanded).unwrap().symbol().unwrap();
            assert_eq!(Some(symbol), super::expected_symbol(text), "{expanded}");
        };
        check("StoreKit.Product.SubscriptionPeriod.dateRange(referenceDate:)", "dateRange(referenceDate: Foundation.Date(struct)) ->");
        check(
            "StoreKit.Product.SubscriptionInfo.RenewalInfo.AdvancedCommerceInfo.period { get }",
            "period: StoreKit.Product(struct).SubscriptionPeriod(struct) { get }",
        );
    }

    /// A variadic generic type's arguments are its pack: `Predicate<each Input>`.
    #[test]
    fn variadic_types() {
        let text = "SwiftData.HistoryDescriptor.predicate { get }";
        let expanded = super::expand(text).unwrap().unwrap();
        assert!(expanded.contains("predicate: Foundation.Predicate(struct)<Pack{TransactionType}>? { get }"), "{expanded}");
        let symbol = apus_mangle::Declaration::parse(&expanded).unwrap().symbol().unwrap();
        assert_eq!(Some(symbol), super::expected_symbol(text), "{expanded}");
    }

    /// A type that moved to another module (`@_originallyDefinedIn`) is mangled with the module it was defined in, as
    /// the framework exports it: `CGFloat` is CoreFoundation's, defined in CoreGraphics.
    #[test]
    fn moved_types() {
        crate::graphs::graphs();
        let expanded = super::expand("Foundation.AffineTransform.m11 { get }").unwrap().unwrap();
        let symbol = apus_mangle::Declaration::parse(&expanded).unwrap().symbol().unwrap();
        assert_eq!(symbol, "$s10Foundation15AffineTransformV3m1112CoreGraphics7CGFloatVvg", "{expanded}");
        assert!(crate::sdk::exported_symbols("Foundation").unwrap().contains(&symbol));
    }

    /// A parameter's result builder (`@ContentBuilder`) is a compile-time feature: left out, the closure stays.
    #[test]
    fn result_builder_parameters() {
        let module = crate::graphs::graphs().module("SwiftUI").unwrap();
        let path = ["AlertScene", "init(_:isPresented:actions:message:)"];
        let symbol = module.find(&path).find(|s| s.fragments.iter().any(|f| f.spelling == "LocalizedStringKey")).unwrap();
        let expanded = super::rewrite("SwiftUI", module, &path[..1], symbol, None).unwrap();
        assert!(expanded.contains("isPresented: SwiftUI.Binding(struct)<Bool>, actions: () -> Actions, message: () -> Message)"), "{expanded}");
        let mangled = apus_mangle::Declaration::parse(&expanded).unwrap().symbol().unwrap();
        assert_eq!(Some(mangled), super::usr_symbol(&symbol.usr, None), "{expanded}");
    }

    /// The graph leaves out `__shared`, which the symbol keeps (`h`): an initializer's parameter written so is
    /// borrowed, not consumed, and only those its symbol marks are written so.
    #[test]
    fn shared_parameters() {
        let text = "Foundation.URL.init(string:relativeTo:)";
        let expanded = super::expand(text).unwrap().unwrap();
        assert!(expanded.contains("init(string: __shared String, relativeTo: __shared Foundation.URL(struct)?)"), "{expanded}");
        let symbol = apus_mangle::Declaration::parse(&expanded).unwrap().symbol().unwrap();
        assert_eq!(Some(symbol), super::expected_symbol(text), "{expanded}");
        let text = "Foundation.URL.init(fileURLWithFileSystemRepresentation:isDirectory:relativeTo:)";
        let expanded = super::expand(text).unwrap().unwrap();
        assert!(expanded.contains("(fileURLWithFileSystemRepresentation: UnsafePointer<Swift.Int8(struct)>, isDirectory: Bool, relativeTo: __shared"), "{expanded}");
    }

    /// A type another module declares in a type's extension (`String.Encoding`, Foundation's) is marked with it.
    #[test]
    fn nested_types_of_other_modules() {
        let text = "Foundation.CocoaError.stringEncoding { get }";
        let expanded = super::expand(text).unwrap().unwrap();
        assert!(expanded.ends_with("stringEncoding: Swift.String(struct).(extension in Foundation) Encoding(struct)? { get }"), "{expanded}");
        let symbol = apus_mangle::Declaration::parse(&expanded).unwrap().symbol().unwrap();
        assert_eq!(Some(symbol), super::expected_symbol(text), "{expanded}");
    }

    /// A type declared in a constrained extension of the one enclosing it (`extension Measurement where UnitType:
    /// Dimension { struct AttributedStyle }`) is marked with it, in the context and in the signature alike.
    #[test]
    fn types_of_constrained_extensions() {
        let text = "Foundation.Measurement.AttributedStyle.locale(_:)";
        let expanded = super::expand(text).unwrap().unwrap();
        let style = "Foundation.Measurement(struct)<UnitType>.(extension in Foundation where UnitType: __C.NSDimension(class)) AttributedStyle(struct)";
        assert!(expanded.ends_with(&format!("AttributedStyle(struct).locale(_: Foundation.Locale(struct)) -> {style}")), "{expanded}");
        let symbol = apus_mangle::Declaration::parse(&expanded).unwrap().symbol().unwrap();
        assert_eq!(Some(symbol), super::expected_symbol(text), "{expanded}");
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

    /// A member of an imported C type outside any Swift extension: made by the importer, with no symbol to call.
    fn importer_made(declaration: &str) -> bool {
        let mut rest = declaration;
        while let Some((word, tail)) = rest.split_once(' ') {
            if !matches!(word, "static" | "mutating" | "nonmutating") && !word.starts_with('@') {
                break;
            }
            rest = tail;
        }
        rest.starts_with("__C.")
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
            let (mut total, mut callable, mut generic, mut foreign) = (0, 0, 0, 0);
            for symbol in module.symbols() {
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
                let mut written = None;
                let outcome = (|| {
                    let declaration = rewrite(name, module, &path[..path.len() - 1], symbol, accessor).map_err(|e| {
                        if std::env::var_os("APUS_COVERAGE_DETAIL").is_some() {
                            println!("REWRITE {}\n  {e}", path.join("."));
                        }
                        reason("rewrite", &e)
                    })?;
                    written = Some(declaration.clone());
                    let parsed = apus_mangle::Declaration::parse_resolving(&declaration, graphs::graphs()).map_err(|e| {
                        if std::env::var_os("APUS_COVERAGE_DETAIL").is_some() {
                            println!("PARSE {declaration}\n  {e}");
                        }
                        reason("parse", &e.to_string())
                    })?;
                    let mangled = parsed.symbol().map_err(|e| reason("mangle", &e.to_string()))?;
                    let exported_as_mangled = crate::exported_by_sdk(&parsed, &mangled);
                    if let Some(expected) = usr_symbol(&symbol.usr, accessor).filter(|expected| *expected != mangled && !exported_as_mangled) {
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
                            // An Objective-C or C declaration is left out below, not reported.
                            if std::env::var_os("APUS_COVERAGE_DETAIL").is_some() && !symbol.usr.starts_with("c:") && !importer_made(&declaration) {
                                println!("EXPORT {declaration}\n  {call}{suffix}");
                            }
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
                    // An Objective-C or C declaration without a Swift symbol (one for the Objective-C runtime), or a
                    // member the importer makes in each client (a C type's `init(rawValue:)`, memberwise `init`).
                    Err(_) if symbol.usr.starts_with("c:") || written.as_deref().is_some_and(importer_made) => {
                        total -= 1;
                        foreign += 1;
                    }
                    Err(why) => {
                        let entry = reasons.entry(why).or_insert((0, path.join(".")));
                        entry.0 += 1;
                    }
                }
            }
            println!("\n{name}: {callable}/{total} callable ({generic} of them bound generics; {foreign} Objective-C or C declarations and imported members left out)");
            let mut sorted: Vec<_> = reasons.into_iter().collect();
            sorted.sort_by(|a, b| b.1 .0.cmp(&a.1 .0));
            for (why, (count, example)) in sorted.iter().take(25) {
                println!("  {count:5}  {why}   e.g. {example}");
            }
        }
    }
}
