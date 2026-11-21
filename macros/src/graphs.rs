//! Symbol graphs (`swift-symbolgraph-extract`, `swiftc -emit-symbol-graph`): the declarations a module exports, as the
//! compiler type-checked them. `swift!` takes from them the kinds a declaration leaves out and, for a declaration
//! written by name only, the whole declaration (see `short.rs`). The symbols it calls are still mangled by apus.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

#[derive(serde::Serialize, serde::Deserialize)]
pub struct Fragment {
    pub kind: String,
    pub spelling: String,
    pub precise: Option<String>,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct Symbol {
    /// `swift.class`, `swift.method`, …
    pub kind: String,
    pub usr: String,
    pub path: Vec<String>,
    pub fragments: Vec<Fragment>,
    /// `swiftGenerics`: the generic parameters (name, depth) and the requirements (kind, lhs, rhs, rhs's USR).
    pub params: Vec<(String, u64)>,
    pub constraints: Vec<(String, String, String, Option<String>)>,
    /// `swiftExtension`: the member is declared in an extension of a type of `extended_module`, which states these
    /// requirements.
    pub extension: Option<Extension>,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct Extension {
    pub extended_module: String,
    pub constraints: Vec<(String, String, String, Option<String>)>,
}

/// A module's symbols: indexed by kind, USR and path, each read whole only when asked for (an SDK module's from its
/// compact copy, see [`load_cached`]).
pub struct Module {
    entries: Vec<Entry>,
    data: Option<std::fs::File>,
}

struct Entry {
    kind: String,
    usr: String,
    path: Vec<String>,
    /// Where its symbol is in the data file.
    at: (u64, u64),
    symbol: OnceLock<Option<Symbol>>,
}

impl Module {
    fn symbol<'a>(&'a self, entry: &'a Entry) -> Option<&'a Symbol> {
        entry
            .symbol
            .get_or_init(|| {
                use std::os::unix::fs::FileExt;
                let mut bytes = vec![0; (entry.at.1 - entry.at.0) as usize];
                self.data.as_ref()?.read_exact_at(&mut bytes, entry.at.0).ok()?;
                serde_json::from_slice(&bytes).ok()
            })
            .as_ref()
    }

    pub fn find<'a>(&'a self, path: &'a [&str]) -> impl Iterator<Item = &'a Symbol> + 'a {
        let matches = move |e: &&Entry| e.path.len() == path.len() && e.path.iter().zip(path).all(|(a, b)| a == b);
        self.entries.iter().filter(matches).filter_map(|e| self.symbol(e))
    }

    /// The symbols with USR `usr` (a typealias's, a type's), of kind `kind` if given.
    pub fn by_usr<'a>(&'a self, usr: &'a str, kind: Option<&'a str>) -> impl Iterator<Item = &'a Symbol> + 'a {
        let matches = move |e: &&Entry| e.usr == usr && kind.is_none_or(|k| e.kind == k);
        self.entries.iter().filter(matches).filter_map(|e| self.symbol(e))
    }

    #[cfg(test)]
    pub fn symbols(&self) -> impl Iterator<Item = &Symbol> {
        self.entries.iter().filter_map(|e| self.symbol(e))
    }

    fn of(symbols: Vec<Symbol>) -> Module {
        let entries = symbols
            .into_iter()
            .map(|s| Entry { kind: s.kind.clone(), usr: s.usr.clone(), path: s.path.clone(), at: (0, 0), symbol: OnceLock::from(Some(s)) })
            .collect();
        Module { entries, data: None }
    }
}

pub struct Graphs {
    modules: Mutex<HashMap<String, Option<&'static Module>>>,
}

pub static GRAPHS: OnceLock<Graphs> = OnceLock::new();

/// The process's graphs, set as apus-mangle's resolver of kinds left out.
pub fn graphs() -> &'static Graphs {
    let graphs = GRAPHS.get_or_init(|| Graphs { modules: Mutex::new(HashMap::new()) });
    apus_mangle::set_kind_resolver(graphs);
    graphs
}

impl Graphs {
    /// Module `name`'s graph: from a directory in `APUS_SYMBOL_GRAPHS` (a program's own modules, which apus-build
    /// emits), else extracted from the SDK once and cached. `None` for the standard library (apus knows its names) and
    /// for a module neither has.
    pub fn module(&self, name: &str) -> Option<&'static Module> {
        if let Some(known) = self.modules.lock().unwrap_or_else(|e| e.into_inner()).get(name) {
            return *known;
        }
        let loaded = locate(name).and_then(|(file, cached)| if cached { load_cached(&file) } else { load(&file) }).map(|m| &*Box::leak(Box::new(m)));
        self.modules.lock().unwrap_or_else(|e| e.into_inner()).insert(name.to_string(), loaded);
        loaded
    }
}

impl apus_mangle::KindResolver for Graphs {
    fn kind(&self, module: &str, path: &[&str]) -> Option<apus_mangle::TypeKind> {
        self.module(module)?.find(path).find_map(|s| type_kind(&s.kind))
    }

    fn class_bound(&self, module: &str, protocol: &str) -> Option<bool> {
        self.class_bound_at(module, protocol, 0)
    }

    /// Looked up in `module`'s graph; only a type it does not declare, in those of the modules it re-exports
    /// (`Darwin.POSIXErrorCode`: `_DarwinFoundation1`'s).
    fn objc_enum(&self, module: &str, name: &str) -> bool {
        let path = [name];
        let declared = |module: &str| -> Option<bool> {
            let mut enums = self.module(module)?.find(&path).filter(|s| s.kind == "swift.enum").peekable();
            enums.peek()?;
            Some(enums.any(|s| s.usr.starts_with("c:")))
        };
        declared(module).unwrap_or_else(|| interface(module).reexports.iter().any(|m| m != module && declared(m) == Some(true)))
    }

    fn original_module(&self, module: &str, name: &str) -> Option<String> {
        let moved = interface(module);
        moved.types.get(name).cloned().or_else(|| moved.abi_name.clone())
    }
}

/// What an SDK module's `.swiftinterface` says, read once.
fn interface(module: &str) -> &'static Moved {
    static MOVED: OnceLock<Mutex<HashMap<String, &'static Moved>>> = OnceLock::new();
    let mut modules = MOVED.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner());
    modules.entry(module.to_string()).or_insert_with(|| Box::leak(Box::new(moved_types(module))))
}

/// What an SDK module's `.swiftinterface` says about the module its types are mangled in, and what it re-exports.
#[derive(Default)]
struct Moved {
    /// The top-level types that moved there from another module (`@_originallyDefinedIn(module: "M", macOS …)`;
    /// symbol graphs leave the attribute out), by name, with the module they were defined in.
    types: HashMap<String, String>,
    /// The name its symbols are mangled with when it is not the module's (`-module-abi-name Darwin` in the flags of
    /// `_DarwinFoundation1`, which declares `POSIXErrorCode`).
    abi_name: Option<String>,
    /// The Swift modules it re-exports (`@_exported import _DarwinFoundation1`).
    reexports: Vec<String>,
}

fn moved_types(module: &str) -> Moved {
    let mut moved = Moved::default();
    if matches!(module, "Swift" | "_Concurrency" | "__C") {
        return moved;
    }
    let Some(sdk) = crate::sdk::sdk() else { return moved };
    let arch = if cfg!(target_arch = "aarch64") { ["arm64e", "arm64"] } else { ["x86_64", "x86_64"] };
    let dirs = [
        sdk.join(format!("System/Library/Frameworks/{module}.framework/Modules/{module}.swiftmodule")),
        sdk.join(format!("usr/lib/swift/{module}.swiftmodule")),
    ];
    let interface = dirs.iter().flat_map(|dir| arch.map(|a| dir.join(format!("{a}-apple-macos.swiftinterface")))).find(|f| f.exists());
    let Some(text) = interface.and_then(|f| std::fs::read_to_string(f).ok()) else { return moved };
    if let Some(flags) = text.lines().find_map(|line| line.strip_prefix("// swift-module-flags:")) {
        let mut words = flags.split_whitespace();
        moved.abi_name = words.by_ref().find(|w| *w == "-module-abi-name").and(words.next()).filter(|name| *name != module).map(str::to_string);
    }
    let mut original: Option<&str> = None;
    for line in text.lines() {
        if let Some(name) = line.strip_prefix("@_exported import ").and_then(|rest| rest.split_whitespace().next()) {
            if !name.contains('.') {
                moved.reexports.push(name.to_string());
            }
            continue;
        }
        if let Some(rest) = line.strip_prefix("@_originallyDefinedIn(module: \"") {
            if rest.contains("macOS") {
                original = rest.split('"').next();
            }
            continue;
        }
        let Some(from) = original else { continue };
        // The declaration the attributes are on (`@frozen public struct CGFloat …`, `public protocol View …`); other
        // attributes and `#if` lines may come between.
        let words: Vec<&str> = line.split(|c: char| c.is_whitespace() || c == ':' || c == '<' || c == '{').filter(|w| !w.is_empty()).collect();
        let introducer = words.iter().position(|w| matches!(*w, "struct" | "class" | "enum" | "protocol" | "actor" | "typealias"));
        match introducer.and_then(|at| words.get(at + 1)) {
            Some(name) => {
                moved.types.insert(name.trim_matches('`').to_string(), from.to_string());
                original = None;
            }
            None if line.starts_with('@') || line.starts_with('#') => {}
            None => original = None,
        }
    }
    moved
}

impl Graphs {
    fn class_bound_at(&self, module: &str, protocol: &str, depth: usize) -> Option<bool> {
        if module == "_Concurrency" {
            return Some(matches!(protocol, "Actor" | "AnyActor" | "DistributedActor"));
        }
        if depth > 8 {
            return None;
        }
        let path = [protocol];
        let symbol = self.module(module)?.find(&path).find(|s| s.kind == "swift.protocol")?;
        // `protocol P : AnyObject, Q where …`: the inheritance clause, then each inherited protocol's.
        let clause = symbol.fragments.iter().take_while(|f| !(f.kind == "keyword" && f.spelling == "where"));
        let mut inherited = Vec::new();
        for fragment in clause {
            if fragment.kind == "text" && fragment.spelling.contains("AnyObject") {
                return Some(true);
            }
            if let Some(usr) = fragment.precise.as_deref().filter(|u| u.ends_with('P')) {
                if let Some(module) = usr_module(usr) {
                    inherited.push((module, fragment.spelling.clone()));
                }
            }
        }
        Some(inherited.iter().any(|(m, p)| m != "Swift" && self.class_bound_at(m, p, depth + 1) == Some(true)))
    }
}

pub fn type_kind(kind: &str) -> Option<apus_mangle::TypeKind> {
    use apus_mangle::TypeKind::*;
    Some(match kind {
        "swift.class" => Class,
        "swift.actor" => Actor,
        "swift.struct" => Struct,
        "swift.enum" => Enum,
        "swift.protocol" => Protocol,
        _ => return None,
    })
}

/// The module a Swift USR (`s:9SwiftData15FetchDescriptorV`) names first: a standard library substitution
/// (`s:Si`, `s:s12IdentifiableP`) is `Swift`, `Sc…` `_Concurrency`, an imported type (`So…`, `c:…`) `__C`.
pub fn usr_module(usr: &str) -> Option<String> {
    if usr.starts_with("c:") {
        return Some("__C".into());
    }
    let mangled = usr.strip_prefix("s:")?;
    let digits = mangled.chars().take_while(|c| c.is_ascii_digit()).count();
    if digits > 0 {
        let len: usize = mangled[..digits].parse().ok()?;
        return mangled.get(digits..digits + len).map(str::to_string);
    }
    Some(match mangled.as_bytes() {
        [b'S', b'o' | b'C', ..] => "__C",
        [b'S', b'c', ..] => "_Concurrency",
        [b'S' | b's', ..] => "Swift",
        _ => return None,
    }
    .to_string())
}

/// The module's graph file, and whether it is the SDK's (extracted into the cache, so its compact copy is kept).
fn locate(module: &str) -> Option<(PathBuf, bool)> {
    for dir in std::env::var("APUS_SYMBOL_GRAPHS").unwrap_or_default().split(':').filter(|d| !d.is_empty()) {
        let file = Path::new(dir).join(format!("{module}.symbols.json"));
        if file.exists() {
            return Some((file, false));
        }
    }
    if module == "Swift" || module == "_Concurrency" {
        return None;
    }
    extract(module).map(|file| (file, true))
}

/// An SDK module's graph from its compact copy beside it, written the first time: an index of its symbols' kinds,
/// USRs and paths (`.apus2.idx`), and the symbols (`.apus2.dat`), each read when looked up. The graph is tens or
/// hundreds of megabytes (SwiftUI's: 500), which every crate naming the module would otherwise read whole.
fn load_cached(file: &Path) -> Option<Module> {
    let (index, data) = (file.with_extension("apus2.idx"), file.with_extension("apus2.dat"));
    if let Some(module) = load_compact(&index, &data) {
        return Some(module);
    }
    let module = load(file)?;
    let _ = save_compact(&module, &index, &data);
    Some(module)
}

type IndexEntry = (String, String, Vec<String>, u64, u64);

fn load_compact(index: &Path, data: &Path) -> Option<Module> {
    let entries: Vec<IndexEntry> = serde_json::from_slice(&std::fs::read(index).ok()?).ok()?;
    let data = std::fs::File::open(data).ok()?;
    // The index's data file, not another's: it ends where the index's last symbol does.
    if entries.last().map(|e| e.4) != Some(data.metadata().ok()?.len()) {
        return None;
    }
    let entries = entries.into_iter().map(|(kind, usr, path, start, end)| Entry { kind, usr, path, at: (start, end), symbol: OnceLock::new() }).collect();
    Some(Module { entries, data: Some(data) })
}

/// Writes the data, then the index, each whole or not at all (another build may be reading them); both are the same
/// whichever build writes them.
fn save_compact(module: &Module, index: &Path, data: &Path) -> std::io::Result<()> {
    let mut bytes = Vec::new();
    let mut entries: Vec<IndexEntry> = Vec::new();
    for entry in &module.entries {
        let Some(symbol) = module.symbol(entry) else { continue };
        let start = bytes.len() as u64;
        serde_json::to_writer(&mut bytes, symbol)?;
        entries.push((entry.kind.clone(), entry.usr.clone(), entry.path.clone(), start, bytes.len() as u64));
    }
    let write = |file: &Path, bytes: &[u8]| {
        // Its own name per file, process and thread: builds and threads may write at once.
        let mut work = file.as_os_str().to_owned();
        work.push(format!(".{}.{:?}.tmp", std::process::id(), std::thread::current().id()).replace(['(', ')'], ""));
        let work = PathBuf::from(work);
        std::fs::write(&work, bytes)?;
        std::fs::rename(&work, file)
    };
    write(data, &bytes)?;
    write(index, &serde_json::to_vec(&entries)?)
}

/// Extracts `module`'s graph from the SDK into a cache shared by the builds against this SDK (about four seconds the
/// first time). A module the SDK does not have is remembered too.
fn extract(module: &str) -> Option<PathBuf> {
    use std::hash::{Hash, Hasher};
    let sdk = crate::sdk::sdk()?;
    let version = crate::sdk::sdk_version()?;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    (sdk, version).hash(&mut hasher);
    let dir = std::env::temp_dir().join("apus-symbolgraphs").join(format!("{:016x}", hasher.finish()));
    let file = dir.join(format!("{module}.symbols.json"));
    let missing = dir.join(format!("{module}.missing"));
    if file.exists() {
        return Some(file);
    }
    if missing.exists() {
        return None;
    }
    let work = dir.join(format!("{module}.{}.tmp", std::process::id()));
    std::fs::create_dir_all(&work).ok()?;
    let arch = if cfg!(target_arch = "aarch64") { "arm64" } else { "x86_64" };
    let status = std::process::Command::new("xcrun")
        .args(["swift-symbolgraph-extract", "-module-name", module, "-target", &format!("{arch}-apple-macosx{version}")])
        .arg("-sdk")
        .arg(sdk)
        .arg("-output-dir")
        .arg(&work)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    let produced = work.join(format!("{module}.symbols.json"));
    let result = if status.is_ok_and(|s| s.success()) && produced.exists() {
        std::fs::rename(&produced, &file).ok().map(|_| file)
    } else {
        let _ = std::fs::write(&missing, "");
        None
    };
    let _ = std::fs::remove_dir_all(&work);
    result
}

/// The parts of a graph apus reads; the rest (documentation, locations, relationships, availability) is skipped.
mod raw {
    use serde::Deserialize;

    #[derive(Deserialize)]
    pub struct Graph {
        pub symbols: Vec<Symbol>,
    }

    #[derive(Deserialize)]
    pub struct Symbol {
        pub kind: Named,
        pub identifier: Identifier,
        #[serde(rename = "pathComponents", default)]
        pub path: Vec<String>,
        #[serde(rename = "declarationFragments", default)]
        pub fragments: Vec<Fragment>,
        #[serde(rename = "swiftGenerics", default)]
        pub generics: Option<Generics>,
        #[serde(rename = "swiftExtension", default)]
        pub extension: Option<Extension>,
    }

    #[derive(Deserialize)]
    pub struct Named {
        pub identifier: String,
    }

    #[derive(Deserialize)]
    pub struct Identifier {
        pub precise: String,
    }

    #[derive(Deserialize)]
    pub struct Fragment {
        pub kind: String,
        pub spelling: String,
        #[serde(rename = "preciseIdentifier", default)]
        pub precise: Option<String>,
    }

    #[derive(Deserialize, Default)]
    pub struct Generics {
        #[serde(default)]
        pub parameters: Vec<Parameter>,
        #[serde(default)]
        pub constraints: Vec<Constraint>,
    }

    #[derive(Deserialize)]
    pub struct Parameter {
        pub name: String,
        pub depth: u64,
    }

    #[derive(Deserialize)]
    pub struct Constraint {
        pub kind: String,
        pub lhs: String,
        pub rhs: String,
        #[serde(rename = "rhsPrecise", default)]
        pub rhs_precise: Option<String>,
    }

    #[derive(Deserialize)]
    pub struct Extension {
        #[serde(rename = "extendedModule", default)]
        pub extended_module: String,
        #[serde(default)]
        pub constraints: Vec<Constraint>,
    }
}

fn load(file: &Path) -> Option<Module> {
    let bytes = std::fs::read(file).ok()?;
    let graph: raw::Graph = serde_json::from_slice(&bytes).ok()?;
    let constraints = |list: Vec<raw::Constraint>| list.into_iter().map(|c| (c.kind, c.lhs, c.rhs, c.rhs_precise)).collect();
    let symbols = graph
        .symbols
        .into_iter()
        .map(|s| {
            let generics = s.generics.unwrap_or_default();
            Symbol {
                kind: s.kind.identifier,
                usr: s.identifier.precise,
                path: s.path,
                fragments: s.fragments.into_iter().map(|f| Fragment { kind: f.kind, spelling: f.spelling, precise: f.precise }).collect(),
                params: generics.parameters.into_iter().map(|p| (p.name, p.depth)).collect(),
                constraints: constraints(generics.constraints),
                extension: s.extension.map(|e| Extension { extended_module: e.extended_module, constraints: constraints(e.constraints) }),
            }
        })
        .collect();
    Some(Module::of(symbols))
}

#[cfg(test)]
mod tests {
    /// A graph read from its compact copy holds what the graph itself gives.
    #[test]
    fn compact_copy() {
        let file = super::extract("SwiftData").expect("the SDK's SwiftData graph");
        let full = super::load(&file).unwrap();
        let dir = std::env::temp_dir().join(format!("apus-compact-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (index, data) = (dir.join("SwiftData.idx"), dir.join("SwiftData.dat"));
        super::save_compact(&full, &index, &data).unwrap();
        let read = super::load_compact(&index, &data).unwrap();
        assert_eq!(read.symbols().count(), full.symbols().count());
        let _ = std::fs::remove_dir_all(&dir);
        let find = |m: &super::Module| {
            m.find(&["ModelContext", "fetch(_:)"]).map(|s| (s.usr.clone(), s.fragments.len(), s.constraints.clone())).collect::<Vec<_>>()
        };
        assert_eq!(find(&read), find(&full));
        assert!(!find(&full).is_empty());
    }

    #[test]
    #[ignore]
    fn load_time() {
        for m in ["Foundation", "SwiftUI"] {
            let file = super::extract(m).unwrap();
            let t = std::time::Instant::now();
            let module = super::load_cached(&file).unwrap();
            let first = t.elapsed();
            let t = std::time::Instant::now();
            let module2 = super::load_cached(&file).unwrap();
            let found = module2.find(&["View", "body"]).count() + module.find(&["Date", "now"]).count();
            println!("{m}: first {first:?} cached {:?} ({found})", t.elapsed());
        }
    }
}
