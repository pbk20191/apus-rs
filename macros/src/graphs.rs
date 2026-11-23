//! Symbol graphs (`swift-symbolgraph-extract`, `swiftc -emit-symbol-graph`): the declarations a module exports, as the
//! compiler type-checked them. `swift!` takes from them the kinds a declaration leaves out and, for a declaration
//! written by name only, the whole declaration (see `short.rs`). The symbols it calls are still mangled by apus.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::ffi::c_void;
use std::sync::{Arc, Mutex, OnceLock};

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
    /// Each module loaded once: a thread asking for one another is loading waits for it.
    modules: Mutex<HashMap<String, Arc<OnceLock<Option<&'static Module>>>>>,
}

pub static GRAPHS: OnceLock<Graphs> = OnceLock::new();

/// `APUS_GRAPH_TRACE=<file>`: each module's load (start and end) and each wait for one, appended to the file with the
/// seconds since the process first traced and the thread, to see which loads a build waits for in turn.
fn trace(event: std::fmt::Arguments) {
    use std::io::Write;
    static FILE: OnceLock<Option<(std::time::Instant, Mutex<std::fs::File>)>> = OnceLock::new();
    let file = FILE.get_or_init(|| {
        let path = std::env::var_os("APUS_GRAPH_TRACE")?;
        let file = std::fs::OpenOptions::new().create(true).append(true).open(path).ok()?;
        Some((std::time::Instant::now(), Mutex::new(file)))
    });
    if let Some((start, file)) = file {
        let thread = std::thread::current().id();
        let line = format!("{:8.3} {thread:?} {event}\n", start.elapsed().as_secs_f64());
        let _ = file.lock().unwrap_or_else(|e| e.into_inner()).write_all(line.as_bytes());
    }
}

/// libdispatch (libSystem), for loading graphs concurrently.
mod dispatch {
    use std::ffi::c_void;
    /// <sys/qos.h>: `QOS_CLASS_USER_INITIATED`.
    pub const QOS_CLASS_USER_INITIATED: isize = 0x19;
    extern "C" {
        pub fn dispatch_get_global_queue(identifier: isize, flags: usize) -> *mut c_void;
        pub fn dispatch_async_f(queue: *mut c_void, context: *mut c_void, work: extern "C" fn(*mut c_void));
    }
}

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
        let cell = Arc::clone(self.modules.lock().unwrap_or_else(|e| e.into_inner()).entry(name.to_string()).or_default());
        if let Some(loaded) = cell.get() {
            return *loaded;
        }
        let start = std::time::Instant::now();
        let loaded = *cell.get_or_init(|| Self::load_module(name));
        trace(format_args!("waited {:.3} s for {name}", start.elapsed().as_secs_f64()));
        loaded
    }

    /// Starts loading `names` (extracting each graph the cache lacks) on libdispatch's global concurrent queue:
    /// `swift-symbolgraph-extract` is a process per module, so several take about as long as the longest. Whoever
    /// asks for one meanwhile waits for its load.
    pub fn prefetch<'a>(&'static self, names: impl IntoIterator<Item = &'a str>) {
        struct Load(String, Arc<OnceLock<Option<&'static Module>>>);
        extern "C" fn run(context: *mut c_void) {
            let Load(name, cell) = *unsafe { Box::from_raw(context.cast::<Load>()) };
            cell.get_or_init(|| Graphs::load_module(&name));
        }
        for name in names {
            let cell = Arc::clone(self.modules.lock().unwrap_or_else(|e| e.into_inner()).entry(name.to_string()).or_default());
            if cell.get().is_none() {
                let context = Box::into_raw(Box::new(Load(name.to_string(), cell)));
                // <dispatch/queue.h>: dispatch_get_global_queue, dispatch_async_f.
                unsafe { dispatch::dispatch_async_f(dispatch::dispatch_get_global_queue(dispatch::QOS_CLASS_USER_INITIATED, 0), context.cast(), run) };
            }
        }
    }

    /// Reads `name`'s graph alone, never another module's: a load waits for no other, so modules that re-export
    /// each other (SwiftUI and SwiftUICore; SwiftUI even itself) cannot deadlock their loads. What looks at other
    /// modules (a re-exported type, an inherited protocol) does so once a load is done.
    fn load_module(name: &str) -> Option<&'static Module> {
        trace(format_args!("load {name}"));
        let loaded = Self::read_module(name);
        trace(format_args!("loaded {name}{}", if loaded.is_some() { "" } else { " (none)" }));
        loaded
    }

    fn read_module(name: &str) -> Option<&'static Module> {
        locate(name)
            .and_then(|(file, cached)| match cached {
                // A copy that cannot be read, its graph's file deleted: extracted again.
                true => load_cached(&file).or_else(|| {
                    let (index, data) = compact_files(&file);
                    let (_, _) = (std::fs::remove_file(index), std::fs::remove_file(data));
                    extract_into(name, file.parent()?).and_then(|file| load_cached(&file))
                }),
                false => load(&file),
            })
            .map(|m| &*Box::leak(Box::new(m)))
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

    /// From the protocol's declaration (`protocol TabContent<TabValue>`), in `module`'s graph or those of the modules
    /// it re-exports (SwiftUI's `TabContent` is SwiftUICore's).
    fn primary_associated_types(&self, module: &str, name: &str) -> Option<Vec<String>> {
        let path = [name];
        let declared = |module: &str| -> Option<Vec<String>> {
            let symbol = self.module(module)?.find(&path).find(|s| s.kind == "swift.protocol")?;
            let at = symbol.fragments.iter().position(|f| f.kind == "identifier" && f.spelling == name)?;
            let mut names = Vec::new();
            let mut fragments = symbol.fragments[at + 1..].iter();
            if !fragments.next().is_some_and(|f| f.kind == "text" && f.spelling.starts_with('<')) {
                return Some(names); // none: `some P<…>` is refused
            }
            for f in fragments {
                match f.kind.as_str() {
                    "typeIdentifier" => names.push(f.spelling.clone()),
                    "text" if f.spelling.trim_start().starts_with('>') => break,
                    "text" if f.spelling.trim() == "," => {}
                    _ => return None,
                }
            }
            Some(names)
        };
        declared(module).or_else(|| interface(module).reexports.iter().filter(|m| *m != module).find_map(|m| declared(m)))
    }
}

/// The Swift modules an SDK module re-exports (`SwiftUI`: `SwiftUICore`, which declares `View` and `Text`).
pub fn reexports(module: &str) -> &'static [String] {
    &interface(module).reexports
}

/// `module` and the modules it re-exports, and theirs (read from `.swiftinterface` files, not graphs).
pub fn with_reexports(module: &str) -> Vec<String> {
    let mut all = vec![module.to_string()];
    let mut i = 0;
    while i < all.len() {
        for other in reexports(&all[i]) {
            if !all.contains(other) {
                all.push(other.clone());
            }
        }
        i += 1;
    }
    all
}

/// Whether `name` is a module a graph can be had for: an SDK Swift module, or one in `APUS_SYMBOL_GRAPHS` (a file
/// check, no graph read).
pub fn is_module(name: &str) -> bool {
    if matches!(name, "Swift" | "_Concurrency" | "__C") || name.is_empty() {
        return false;
    }
    let graphs = std::env::var("APUS_SYMBOL_GRAPHS").unwrap_or_default();
    if graphs.split(':').filter(|d| !d.is_empty()).any(|d| Path::new(d).join(format!("{name}.symbols.json")).exists()) {
        return true;
    }
    let Some(sdk) = crate::sdk::sdk() else { return false };
    sdk.join(format!("System/Library/Frameworks/{name}.framework/Modules/{name}.swiftmodule")).exists()
        || sdk.join(format!("usr/lib/swift/{name}.swiftmodule")).exists()
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
/// USRs and paths (`.apus3.idx`), and the symbols (`.apus3.dat`), each read when looked up. The graph is tens or
/// hundreds of megabytes (SwiftUI's: 500), which every crate naming the module would otherwise read whole.
fn load_cached(file: &Path) -> Option<Module> {
    let (index, data) = compact_files(file);
    if let Some(module) = load_compact(&index, &data) {
        return Some(module);
    }
    let module = load(file)?;
    // The copy written, the graph's file and copies in older formats go (SwiftUI's graph: 500 MB).
    if save_compact(&module, &index, &data).is_ok() {
        let _ = std::fs::remove_file(file);
        for old in ["apus", "apus2"] {
            let _ = std::fs::remove_file(file.with_extension(format!("{old}.idx")));
            let _ = std::fs::remove_file(file.with_extension(format!("{old}.dat")));
        }
    }
    Some(module)
}

/// The compact copy's format, part of its files' names: a new one leaves an older copy unread.
const COMPACT: &str = "apus3";

/// A graph's compact copy: its index and its data.
fn compact_files(file: &Path) -> (PathBuf, PathBuf) {
    (file.with_extension(format!("{COMPACT}.idx")), file.with_extension(format!("{COMPACT}.dat")))
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
    // Extracted before: the graph's file, or (once read) its compact copy, the file then deleted.
    if file.exists() || compact_files(&file).0.exists() {
        return Some(file);
    }
    if dir.join(format!("{module}.missing")).exists() {
        return None;
    }
    extract_into(module, &dir)
}

/// Extracts `module`'s graph from the SDK into `dir` (`{module}.symbols.json`), or remembers there that the SDK has
/// no such module.
fn extract_into(module: &str, dir: &Path) -> Option<PathBuf> {
    let sdk = crate::sdk::sdk()?;
    let version = crate::sdk::sdk_version()?;
    let file = dir.join(format!("{module}.symbols.json"));
    let missing = dir.join(format!("{module}.missing"));
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
    //! Strings borrowed from the file's bytes where they need no unescaping: only the symbols kept are copied.
    use serde::Deserialize;
    use std::borrow::Cow;

    #[derive(Deserialize)]
    pub struct Graph<'a> {
        #[serde(borrow)]
        pub symbols: Vec<Symbol<'a>>,
    }

    #[derive(Deserialize)]
    pub struct Symbol<'a> {
        #[serde(borrow)]
        pub kind: Named<'a>,
        #[serde(borrow)]
        pub identifier: Identifier<'a>,
        #[serde(rename = "pathComponents", default, borrow)]
        pub path: Vec<Cow<'a, str>>,
        #[serde(rename = "declarationFragments", default, borrow)]
        pub fragments: Vec<Fragment<'a>>,
        #[serde(rename = "swiftGenerics", default, borrow)]
        pub generics: Option<Generics<'a>>,
        #[serde(rename = "swiftExtension", default, borrow)]
        pub extension: Option<Extension<'a>>,
    }

    #[derive(Deserialize)]
    pub struct Named<'a> {
        #[serde(borrow)]
        pub identifier: Cow<'a, str>,
    }

    #[derive(Deserialize)]
    pub struct Identifier<'a> {
        #[serde(borrow)]
        pub precise: Cow<'a, str>,
    }

    #[derive(Deserialize)]
    pub struct Fragment<'a> {
        #[serde(borrow)]
        pub kind: Cow<'a, str>,
        #[serde(borrow)]
        pub spelling: Cow<'a, str>,
        #[serde(rename = "preciseIdentifier", default, borrow)]
        pub precise: Option<Cow<'a, str>>,
    }

    #[derive(Deserialize, Default)]
    pub struct Generics<'a> {
        #[serde(default, borrow)]
        pub parameters: Vec<Parameter<'a>>,
        #[serde(default, borrow)]
        pub constraints: Vec<Constraint<'a>>,
    }

    #[derive(Deserialize)]
    pub struct Parameter<'a> {
        #[serde(borrow)]
        pub name: Cow<'a, str>,
        pub depth: u64,
    }

    #[derive(Deserialize)]
    pub struct Constraint<'a> {
        #[serde(borrow)]
        pub kind: Cow<'a, str>,
        #[serde(borrow)]
        pub lhs: Cow<'a, str>,
        #[serde(borrow)]
        pub rhs: Cow<'a, str>,
        #[serde(rename = "rhsPrecise", default, borrow)]
        pub rhs_precise: Option<Cow<'a, str>>,
    }

    #[derive(Deserialize)]
    pub struct Extension<'a> {
        #[serde(rename = "extendedModule", default, borrow)]
        pub extended_module: Cow<'a, str>,
        #[serde(default, borrow)]
        pub constraints: Vec<Constraint<'a>>,
    }
}

fn load(file: &Path) -> Option<Module> {
    let bytes = std::fs::read(file).ok()?;
    let graph: raw::Graph = serde_json::from_slice(&bytes).ok()?;
    let owned = |c: std::borrow::Cow<str>| c.into_owned();
    let constraints = |list: Vec<raw::Constraint>| {
        list.into_iter().map(|c| (owned(c.kind), owned(c.lhs), owned(c.rhs), c.rhs_precise.map(owned))).collect()
    };
    // A protocol extension's member shown again on each conforming type (`…::SYNTHESIZED::…`, 95% of SwiftUI's
    // graph): called through the protocol, never by such a copy, whose USR is no symbol.
    let symbols = graph
        .symbols
        .into_iter()
        .filter(|s| !s.identifier.precise.contains("::SYNTHESIZED::"))
        .map(|s| {
            let generics = s.generics.unwrap_or_default();
            Symbol {
                kind: owned(s.kind.identifier),
                usr: owned(s.identifier.precise),
                path: s.path.into_iter().map(owned).collect(),
                fragments: s
                    .fragments
                    .into_iter()
                    .map(|f| Fragment { kind: owned(f.kind), spelling: owned(f.spelling), precise: f.precise.map(owned) })
                    .collect(),
                params: generics.parameters.into_iter().map(|p| (owned(p.name), p.depth)).collect(),
                constraints: constraints(generics.constraints),
                extension: s.extension.map(|e| Extension { extended_module: owned(e.extended_module), constraints: constraints(e.constraints) }),
            }
        })
        .collect();
    Some(Module::of(symbols))
}

#[cfg(test)]
mod tests {
    /// Threads asking for a module at once share one load (and one extraction): the same graph for each.
    #[test]
    fn one_load_per_module() {
        let threads: Vec<_> = (0..4).map(|_| std::thread::spawn(|| super::graphs().module("WeatherKit").map(|m| m as *const _ as usize))).collect();
        let loaded: Vec<_> = threads.into_iter().map(|t| t.join().unwrap()).collect();
        assert!(loaded[0].is_some() && loaded.iter().all(|m| *m == loaded[0]), "{loaded:?}");
    }

    /// Once its compact copy is written, a cached graph's file and copies in older formats are deleted (SwiftUI's
    /// graph is 500 MB); the copy alone is read after.
    #[test]
    fn cache_keeps_the_compact_copy_only() {
        let dir = std::env::temp_dir().join(format!("apus-cache-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = super::extract_into("SwiftData", &dir).expect("the SDK's SwiftData graph");
        let stale = file.with_extension("apus2.idx");
        std::fs::write(&stale, "[]").unwrap();
        let first = super::load_cached(&file).unwrap().symbols().count();
        assert!(!file.exists() && !stale.exists(), "the graph's file and an older copy are deleted");
        assert!(file.with_extension(format!("{}.idx", super::COMPACT)).exists());
        assert_eq!(super::load_cached(&file).unwrap().symbols().count(), first, "read again from the copy");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A graph read from its compact copy holds what the graph itself gives.
    #[test]
    fn compact_copy() {
        let dir = std::env::temp_dir().join(format!("apus-compact-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = super::extract_into("SwiftData", &dir).expect("the SDK's SwiftData graph");
        let full = super::load(&file).unwrap();
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
