#!/usr/bin/env python3
"""How much of the SDK's Swift API `swift!` can call: for each module, every public Swift function, method,
initializer, property getter, subscript getter, operator and enum case in its symbol graph is written in apus's
declaration syntax, mangled by apus-mangle (mangle/examples/mangle_lines.rs) and compared with the symbol the graph
names. A declaration also needs its symbol exported by the framework (an @inlinable or @_alwaysEmitIntoClient one has
none), checked against the SDK's .tbd files.

    scripts/sdk-coverage.py [Module …]        (default: a set of Swift-first Apple frameworks)

Symbol graphs are cached in $APUS_SG_CACHE (default: target/sdk-coverage). Writes target/sdk-coverage/report.md and,
per module, <Module>.details.json (every declaration with its outcome).
"""
import collections
import glob
import json
import os
import re
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
CACHE = os.environ.get("APUS_SG_CACHE", os.path.join(ROOT, "target", "sdk-coverage"))
TARGET = "arm64-apple-macos14.0"
DEFAULT_MODULES = ["StoreKit", "SwiftData", "MusicKit", "WeatherKit", "AppIntents", "Observation", "CoreLocation",
                   "MapKit", "SwiftUI", "Foundation"]

CALLABLE = {"swift.func", "swift.method", "swift.type.method", "swift.init", "swift.property", "swift.type.property",
            "swift.var", "swift.subscript", "swift.type.subscript", "swift.func.op", "swift.enum.case"}
TYPE_KINDS = {"swift.struct": "struct", "swift.class": "class", "swift.enum": "enum", "swift.protocol": "protocol",
              "swift.actor": "actor"}
USR_KIND = {"V": "struct", "C": "class", "O": "enum", "P": "protocol"}
KEPT_MODIFIERS = {"static", "class", "mutating", "consuming", "borrowing", "__consuming", "nonisolated", "prefix",
                  "postfix"}
KEPT_TYPE_ATTRIBUTES = {"@escaping", "@Sendable", "@MainActor"}


class Untranslatable(Exception):
    pass


# The graph being converted: its typealiases resolve to their underlying types, as Swift mangles them.
GRAPH = None


def sdk():
    return subprocess.run(["xcrun", "--show-sdk-path"], capture_output=True, text=True, check=True).stdout.strip()


def extract(module):
    out = os.path.join(CACHE, module)
    if not glob.glob(os.path.join(out, "*.symbols.json")):
        os.makedirs(out, exist_ok=True)
        subprocess.run(["xcrun", "swift", "symbolgraph-extract", "-module-name", module, "-target", TARGET, "-sdk", sdk(),
                        "-output-dir", out, "-minimum-access-level", "public"], check=True, capture_output=True)
    # The module's own graph and its extensions of other modules' types; not cross-import overlays (`_A_B@…`).
    return [f for f in glob.glob(os.path.join(out, "*.symbols.json")) if not os.path.basename(f).startswith("_")]


def exported(module):
    symbols = set()
    root = sdk()
    for pattern in (f"System/Library/Frameworks/{module}.framework/**/*.tbd", f"usr/lib/swift/libswift{module}.tbd",
                    f"usr/lib/swift/libswift_{module}.tbd", f"usr/lib/swift/lib{module}.tbd"):
        for path in glob.glob(os.path.join(root, pattern), recursive=True):
            with open(path, errors="replace") as f:
                symbols.update(re.findall(r"\$s[A-Za-z0-9_$]+", f.read()))
    return symbols


# C typedefs of other types, which Swift mangles as the type they name (a sample of the SDK's common ones).
C_TYPEDEFS = {
    **{name: "Double" for name in ("NSTimeInterval", "CLLocationDistance", "CLLocationDegrees", "CLLocationSpeed",
                                   "CLLocationDirection", "CLLocationAccuracy", "CLLocationDirectionAccuracy",
                                   "CLLocationSpeedAccuracy", "CFTimeInterval", "CFAbsoluteTime")},
    "FourCharCode": "UInt32", "OSType": "UInt32", "unichar": "UInt16", "UTF32Char": "UInt32", "OSStatus": "Int32",
    "NSRect": "__C.CGRect(struct)", "NSPoint": "__C.CGPoint(struct)", "NSSize": "__C.CGSize(struct)",
}


def usr_module_kind(usr):
    """(module, kind) of a type's USR; kind None when it is not a nominal type apus can name."""
    if usr.startswith("c:objc(cs)"):
        return "__C", "class"
    if usr.startswith("c:objc(pl)"):
        return "__C", "protocol"
    if usr.startswith(("c:@E@", "c:@S@", "c:@EA@", "c:@SA@")):
        return "__C", "struct"
    if not usr.startswith("s:"):
        return None, None
    body = usr[2:]
    if body.startswith("So"):
        module = "__C"
    elif body.startswith("Sc"):
        module = "_Concurrency"
    elif body[:1] in ("S", "s"):
        module = "Swift"
    else:
        m = re.match(r"(\d+)", body)
        module = body[len(m.group(1)):len(m.group(1)) + int(m.group(1))] if m else None
    if usr.endswith("Qa"):
        return module, "associatedtype"
    return module, USR_KIND.get(usr[-1], "typealias" if usr.endswith("a") else None)


def objc_name(usr):
    """The C/ObjC name in a Clang USR (`c:objc(cs)NSString`, `c:@E@NSComparisonResult`): what Swift mangles."""
    m = re.match(r"c:(?:objc\((?:cs|pl)\)|@(?:E|S|EA|SA)@)([A-Za-z0-9_]+)$", usr)
    return m.group(1) if m else None


def protocol_name(spelling, usr):
    module, _ = usr_module_kind(usr) if usr else (None, None)
    return spelling if module is None or "." in spelling else f"{module}.{spelling}"


def render_chain(chain):
    first = chain[0]
    usr = first.get("preciseIdentifier")
    names = [c["spelling"] for c in chain]
    if usr is None:  # a generic parameter (or Self), possibly with an associated type after it
        return ".".join(names)
    # Standard types spelled by substitution (`s:SS`, `s:Si`, `s:ScS`): the parser knows them by name.
    if len(chain) == 1 and re.fullmatch(r"s:S[a-zA-Z]|s:Sc[a-zA-Z]", usr):
        return first["spelling"]
    if usr.startswith("c:@SA@") or usr.startswith("c:@T@"):
        name = usr.split("@")[-1]
        if len(chain) != 1:
            raise Untranslatable("C typedef")
        if name in C_TYPEDEFS:  # a typedef of another type is mangled as that type
            return C_TYPEDEFS[name]
        if usr.startswith("c:@T@") and name == first["spelling"] + "Ref":
            return f"__C.{name}(typedef class)"  # a Core Foundation class
        # An anonymous struct named by its typedef, or a `swift_newtype` (NS_TYPED_ENUM) struct.
        return f"__C.{name}(typedef struct)"
    if usr.startswith("c:"):
        name = objc_name(usr)
        module, kind = usr_module_kind(usr)
        if len(chain) != 1 or name is None or kind is None:
            raise Untranslatable("imported type unknown")
        return f"__C.{name}" if kind == "protocol" else f"__C.{name}({kind})"
    if len(chain) >= 1 and chain[-1].get("preciseIdentifier", "").endswith("a") and GRAPH is not None:
        alias = GRAPH.by_usr.get(chain[-1]["preciseIdentifier"])
        if alias is not None and alias["kind"]["identifier"] == "swift.typealias" and not alias.get("swiftGenerics"):
            frags = alias["declarationFragments"]
            eq = next((i for i, f in enumerate(frags) if f["kind"] == "text" and "=" in f["spelling"]), None)
            if eq is not None:
                rest = [dict(frags[eq], spelling=frags[eq]["spelling"].split("=", 1)[1])] + frags[eq + 1:]
                return render(rest)
        raise Untranslatable("typealias from another module" if alias is None else "generic typealias")
    parts = []
    module = None
    for c in chain:
        m, kind = usr_module_kind(c.get("preciseIdentifier", ""))
        module = module or m
        if kind in (None, "typealias", "associatedtype"):
            raise Untranslatable(f"type {kind or 'unknown'}" if kind else "type unknown")
        parts.append((c["spelling"], kind))
    if parts[-1][1] == "protocol":
        return protocol_name(".".join(names), usr)
    return f"{module}." + ".".join(f"{name}({kind})" for name, kind in parts)


def render(frags):
    out = []
    i = 0
    while i < len(frags):
        f = frags[i]
        kind, spelling = f["kind"], f["spelling"]
        if kind == "typeIdentifier":
            chain, j = [f], i + 1
            while j + 1 < len(frags) and frags[j]["kind"] == "text" and frags[j]["spelling"] == "." \
                    and frags[j + 1]["kind"] == "typeIdentifier":
                chain.append(frags[j + 1])
                j += 2
            out.append(render_chain(chain))
            i = j
            continue
        if kind == "attribute" and spelling.strip() == "@" and i + 1 < len(frags) and frags[i + 1]["kind"] == "typeIdentifier":
            i += 2  # a result builder (`@ViewBuilder`): not part of the symbol
            continue
        if kind == "attribute":
            if spelling.strip() not in KEPT_TYPE_ATTRIBUTES:
                raise Untranslatable(f"attribute {spelling.strip()}")
            out.append(spelling.strip() + " ")
        elif kind == "keyword":
            if spelling in ("sending", "each", "repeat"):
                raise Untranslatable(f"keyword {spelling}")
            out.append(spelling + " ")
        else:
            out.append(spelling)
        i += 1
    return re.sub(r"\s+", " ", "".join(out)).strip()


def signature(frags):
    """The declaration text from its name on, in apus syntax: `label: Type` parameters (an internal name alone is
    `_`), no default values."""
    out, depth, skipping, last_external = [], 0, False, False
    for f in frags:
        kind, spelling = f["kind"], f["spelling"]
        if kind == "text":
            text = ""
            for ch in spelling:
                if skipping:
                    if ch in "([":
                        depth += 1
                    elif ch in ")]":
                        depth -= 1
                    if (ch == "," and depth == 1) or depth == 0:
                        skipping = False
                        text += ch
                    continue
                if ch == "(":
                    depth += 1
                elif ch == ")":
                    depth -= 1
                text += ch
                if depth == 1 and text.endswith(" ="):
                    text = text[:-2]
                    skipping = True
            out.append(dict(f, spelling=text))
            if text.strip():
                last_external = False
            continue
        if skipping:
            continue
        if depth == 1 and kind == "externalParam":
            out.append({"kind": "text", "spelling": f" {spelling}"})
            last_external = True
            continue
        if depth == 1 and kind == "internalParam":
            if not last_external:
                out.append({"kind": "text", "spelling": " _"})
            last_external = False
            continue
        out.append(f)
    text = ""
    for f in out:
        if f["kind"] == "text" and ">." in f["spelling"]:
            raise Untranslatable("member type of a generic type")
    return render(out)


def split_params(frags, start):
    """Labels of the parameters in the parenthesis opening at or after `start`, and the index after it."""
    depth, labels, current = 0, [], None
    i = start
    while i < len(frags):
        f = frags[i]
        if f["kind"] == "text":
            for ch in f["spelling"]:
                if ch == "(":
                    depth += 1
                elif ch == ")":
                    depth -= 1
                    if depth == 0:
                        return labels, i + 1
        elif depth == 1 and f["kind"] == "externalParam":
            labels.append(f["spelling"])
            current = "external"
        elif depth == 1 and f["kind"] == "internalParam":
            if current != "external":
                labels.append("_")  # a lone (internal) name: no argument label
            current = "internal"
        if depth == 1 and f["kind"] == "text" and "," in f["spelling"]:
            current = None
        i += 1
    raise Untranslatable("unbalanced parameters")


def param_type(param):
    frags = param.get("declarationFragments", [])
    for i, f in enumerate(frags):
        if f["kind"] == "text" and f["spelling"].startswith(":"):
            rest = [dict(f, spelling=f["spelling"][1:])] + frags[i + 1:]
            return render(rest)
    raise Untranslatable("parameter without a type")


class Graph:
    def __init__(self, module, files):
        self.module = module
        self.symbols, self.relations = [], []
        for path in files:
            with open(path) as f:
                data = json.load(f)
            self.symbols += data["symbols"]
            self.relations += data.get("relationships", [])
        self.by_usr = {s["identifier"]["precise"]: s for s in self.symbols}
        self.types_by_path = {tuple(s["pathComponents"]): s for s in self.symbols
                              if s["kind"]["identifier"] in TYPE_KINDS}
        self.parent = {r["source"]: r["target"] for r in self.relations
                       if r["kind"] in ("memberOf", "requirementOf", "optionalRequirementOf")}

    def context(self, sym):
        """(module, [(name, kind, generic clause)], generic params, own constraints of the generic enclosing type)."""
        path = sym["pathComponents"][:-1]
        if not path:
            return self.module, [], [], []
        target = self.parent.get(sym["identifier"]["precise"], "")
        if target.startswith("c:"):
            # An imported type: Swift mangles it flat, under its C/ObjC name.
            name = objc_name(target)
            _, kind = usr_module_kind(target)
            if name is None or kind is None:
                raise Untranslatable("imported context unknown")
            return "__C", [(name, kind, "")], [], []
        parts, params, constraints = [], [], []
        for n in range(1, len(path) + 1):
            t = self.types_by_path.get(tuple(path[:n]))
            clause = ""
            if t is not None:
                kind = TYPE_KINDS[t["kind"]["identifier"]]
                generics = t.get("swiftGenerics", {})
                own = [p["name"] for p in generics.get("parameters", []) if p.get("depth", 0) == 0 and p["name"] not in params]
                if own and kind != "protocol":
                    if params:
                        raise Untranslatable("generic type nested in a generic type")
                    params, constraints = own, generics.get("constraints", [])
                    bound = collections.defaultdict(list)
                    for c in constraints:
                        if c["kind"] == "conformance" and c["lhs"] in own:
                            bound[c["lhs"]].append(protocol_name(c["rhs"], c.get("rhsPrecise")))
                        elif sym["kind"]["identifier"] == "swift.enum.case":
                            # A case's symbol carries the enum's whole generic signature.
                            raise Untranslatable("enum constraint beyond a conformance")
                    clause = "<" + ", ".join(f"{p}: {' & '.join(bound[p])}" if p in bound else p for p in own) + ">"
            elif n == len(path) and self.parent.get(sym["identifier"]["precise"]):
                _, kind = usr_module_kind(self.parent[sym["identifier"]["precise"]])
                if kind not in ("struct", "class", "enum", "protocol"):
                    raise Untranslatable("context kind unknown")
            else:
                raise Untranslatable("context in another module")
            parts.append((path[n - 1], kind, clause))
        target = self.parent.get(sym["identifier"]["precise"])
        module = usr_module_kind(target)[0] if target and target.startswith("s:") else None
        if parts[-1][1] == "protocol":
            params = ["Self"]
        return module or self.module, parts, params, constraints


def constraint_text(graph, c):
    if c["kind"] == "conformance":
        return f"{c['lhs']}: {protocol_name(c['rhs'], c.get('rhsPrecise'))}"
    if c["kind"] == "sameType":
        rhs = c["rhs"]
        if "<" in rhs:
            raise Untranslatable("same-type requirement on a generic type")
        if c.get("rhsPrecise"):
            module, kind = usr_module_kind(c["rhsPrecise"])
            if module not in ("Swift", "_Concurrency"):
                if kind not in ("struct", "class", "enum"):
                    raise Untranslatable("same-type requirement on an unnamed type")
                names = rhs.split(".")
                kinds = [TYPE_KINDS.get(graph.types_by_path.get(tuple(names[:n + 1]), {}).get("kind", {}).get("identifier"))
                         for n in range(len(names))]
                kinds[-1] = kind
                if None in kinds:
                    raise Untranslatable("same-type requirement on a nested type of another module")
                rhs = f"{module}." + ".".join(f"{name}({k})" for name, k in zip(names, kinds))
        return f"{c['lhs']} == {rhs}"
    raise Untranslatable(f"{c['kind']} requirement")


def declaration(graph, sym):
    kind = sym["kind"]["identifier"]
    frags = sym["declarationFragments"]
    intro = next((i for i, f in enumerate(frags) if f["kind"] == "keyword"
                  and f["spelling"] in ("func", "var", "let", "init", "subscript", "case")), None)
    if intro is None:
        raise Untranslatable("no declaration keyword")
    modifiers = []
    for f in frags[:intro]:
        s = f["spelling"].strip()
        if f["kind"] == "keyword" and s in KEPT_MODIFIERS:
            modifiers.append("static" if s == "class" else s)
        elif f["kind"] == "attribute" and s in ("@MainActor", "@preconcurrency"):
            modifiers.append(s)
    module, parts, ctx_params, ctx_constraints = graph.context(sym)
    head = " ".join(modifiers)
    ext = sym.get("swiftExtension")
    extension = ""
    if ext is not None:
        extra = [c for c in ext.get("constraints", []) if c not in ctx_constraints]
        protocol = bool(parts) and parts[-1][1] == "protocol"
        if extra or protocol or (ext.get("extendedModule") and ext["extendedModule"] != graph.module):
            where = (" where " + ", ".join(constraint_text(graph, c) for c in extra)) if extra else ""
            extension = f"(extension in {graph.module}{where}) "
    ctx = "".join(f".{name}({k}){clause}" for name, k, clause in parts)
    generics = sym.get("swiftGenerics", {})
    own_params = [p["name"] for p in generics.get("parameters", []) if p["name"] not in ctx_params]
    known = ctx_constraints + (ext.get("constraints", []) if ext else [])
    own_constraints = [c for c in generics.get("constraints", []) if c not in known]
    gen = f"<{', '.join(own_params)}>" if own_params else ""
    where = (" where " + ", ".join(constraint_text(graph, c) for c in own_constraints)) if own_constraints else ""
    prefix = f"{head} {extension}{module}{ctx}".strip()
    effects = [f["spelling"] for f in frags[intro:] if f["kind"] == "keyword" and f["spelling"] in ("async", "throws", "rethrows")]
    effect_text = " ".join("throws" if e == "rethrows" else e for e in effects)
    if re.search(r"throws\s*\(", "".join(f["spelling"] for f in frags)):
        raise Untranslatable("typed throws")

    if kind == "swift.enum.case":
        name = frags[intro + 2]["spelling"] if intro + 2 < len(frags) else sym["names"]["title"]
        rest = frags[intro + 3:]
        payload = render(rest) if rest else ""
        return f"case {prefix}.{name}{payload}"
    if kind in ("swift.property", "swift.type.property", "swift.var"):
        name = sym["pathComponents"][-1]
        text = "".join(f["spelling"] for f in frags)
        colon = next(i for i, f in enumerate(frags) if f["kind"] == "text" and f["spelling"].startswith(":"))
        brace = next((i for i, f in enumerate(frags) if i > colon and "{" in f["spelling"]), len(frags))
        if brace == colon:
            type_frags = [dict(frags[colon], spelling=frags[colon]["spelling"][1:].split("{")[0])]
        else:
            type_frags = [dict(frags[colon], spelling=frags[colon]["spelling"][1:])] + frags[colon + 1:brace]
            if brace < len(frags):
                type_frags.append(dict(frags[brace], spelling=frags[brace]["spelling"].split("{")[0]))
        ty = render(type_frags)
        accessor = "get" + "".join(f" {e}" for e in ("async", "throws") if re.search(rf"\{{[^}}]*\b{e}\b", text))
        return f"{prefix}.{name}: {ty} {{ {accessor} }}{where}".replace(f"{prefix}.", f"{prefix}.", 1)
    # Functions, initializers, subscripts: the declaration's own text from its name on.
    start = intro if kind in ("swift.init", "swift.subscript", "swift.type.subscript") else \
        next(i for i, f in enumerate(frags) if i > intro and f["kind"] == "identifier")
    text = signature(frags[start:]).replace("rethrows", "throws")
    text = re.sub(r"^(init|subscript)\s+([<?!(])", r"\1\2", text)
    text = re.sub(r"\bthrows\s+-> Never\b", "throws -> Never", text)
    if kind == "swift.init":
        failable = re.match(r"init\s*[?!]", text) is not None
        text = re.sub(r"^init\s*[?!]", "init", text)
        if failable:
            if ctx_params and ctx_params != ["Self"]:
                raise Untranslatable("failable initializer of a generic type")
            result = f" -> {module}{ctx}?"
            text = text.replace(" where ", result + " where ", 1) if " where " in text else text + result
    return f"{prefix}.{text}"


def expected_symbol(sym):
    usr = sym["identifier"]["precise"]
    symbol = "$s" + usr[2:]
    kind = sym["kind"]["identifier"]
    if kind in ("swift.property", "swift.type.property", "swift.var"):
        symbol = re.sub(r"vp(Z?)$", r"vg\1", symbol)
    elif kind in ("swift.subscript", "swift.type.subscript"):
        symbol = re.sub(r"ip(Z?)$", r"ig\1", symbol)
    elif kind == "swift.init":
        symbol = re.sub(r"fc$", "fC", symbol)
    elif kind == "swift.enum.case":
        symbol += "WC"
    return symbol


def mangle_all(lines):
    if not lines:
        return []
    run = subprocess.run(["cargo", "run", "-q", "-p", "apus-mangle", "--example", "mangle_lines"], cwd=ROOT,
                         input="\n".join(lines) + "\n", capture_output=True, text=True, check=True)
    return run.stdout.splitlines()


def reason_key(text):
    text = re.sub(r"`[^`]*`", "`…`", text)
    return re.sub(r" at .*$", "", text)[:90]


def measure(module):
    global GRAPH
    graph = GRAPH = Graph(module, extract(module))
    exports = exported(module)
    rows, lines = [], []
    for sym in graph.symbols:
        usr = sym["identifier"]["precise"]
        if sym["kind"]["identifier"] not in CALLABLE or not usr.startswith("s:") or "::SYNTHESIZED::" in usr:
            continue
        expected = expected_symbol(sym)
        row = {"usr": usr, "kind": sym["kind"]["identifier"], "expected": expected,
               "exported": expected in exports or expected + "Tj" in exports}
        try:
            row["declaration"] = declaration(graph, sym)
            lines.append(row["declaration"])
        except Untranslatable as e:
            row["outcome"], row["reason"] = "untranslated", str(e)
        except (StopIteration, IndexError, KeyError) as e:
            row["outcome"], row["reason"] = "untranslated", f"converter: {type(e).__name__}"
        rows.append(row)
    results = iter(mangle_all(lines))
    for row in rows:
        if "declaration" not in row:
            continue
        status, _, value = next(results).partition(" ")
        if status == "ERR":
            row["outcome"], row["reason"] = "refused", value
        elif value == row["expected"]:
            row["outcome"] = "callable"
        else:
            row["outcome"], row["reason"], row["mangled"] = "mismatch", "mangled differently", value
    with open(os.path.join(CACHE, f"{module}.details.json"), "w") as f:
        json.dump(rows, f, indent=1)
    return rows


def main():
    modules = sys.argv[1:] or DEFAULT_MODULES
    os.makedirs(CACHE, exist_ok=True)
    report = ["# SDK coverage of `swift!`", "",
              f"Target {TARGET}; SDK {os.path.basename(sdk())}. Public Swift functions, methods, initializers, getters,",
              "subscripts, operators and enum cases. *Callable*: exported and mangled exactly; *not exported*: no symbol",
              "to call (inlinable); the rest are apus's gaps.", "",
              "| Module | Declarations | Exported | Callable | Callable of exported |", "|---|---|---|---|---|"]
    reasons = collections.Counter()
    for module in modules:
        try:
            rows = measure(module)
        except subprocess.CalledProcessError as e:
            report.append(f"| {module} | (symbol graph failed: {e.returncode}) | | | |")
            continue
        total = len(rows)
        exported_rows = [r for r in rows if r["exported"]]
        callable_rows = [r for r in exported_rows if r.get("outcome") == "callable"]
        share = f"{100 * len(callable_rows) / len(exported_rows):.0f}%" if exported_rows else "-"
        report.append(f"| {module} | {total} | {len(exported_rows)} | {len(callable_rows)} | {share} |")
        for r in exported_rows:
            if r.get("outcome") != "callable":
                reasons[(r["outcome"], reason_key(r.get("reason", "")))] += 1
        print(f"{module}: {len(callable_rows)}/{len(exported_rows)} exported callable ({total} declarations)", file=sys.stderr)
    report += ["", "## Why the rest of the exported declarations are not callable", "",
               "| Outcome | Reason | Count |", "|---|---|---|"]
    for (outcome, reason), count in reasons.most_common(40):
        report.append(f"| {outcome} | {reason} | {count} |")
    path = os.path.join(CACHE, "report.md")
    with open(path, "w") as f:
        f.write("\n".join(report) + "\n")
    print(path)


if __name__ == "__main__":
    main()
