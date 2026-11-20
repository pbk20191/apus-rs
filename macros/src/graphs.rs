//! Symbol graphs (`swift-symbolgraph-extract`, `swiftc -emit-symbol-graph`): the declarations a module exports, as the
//! compiler type-checked them. `swift!` takes from them the kinds a declaration leaves out and, for a declaration
//! written by name only, the whole declaration (see `short.rs`). The symbols it calls are still mangled by apus.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

pub struct Fragment {
    pub kind: String,
    pub spelling: String,
    pub precise: Option<String>,
}

pub struct Symbol {
    /// `swift.class`, `swift.method`, …
    pub kind: String,
    pub usr: String,
    pub path: Vec<String>,
    pub fragments: Vec<Fragment>,
    /// `swiftGenerics`: the generic parameters (name, depth) and the requirements (kind, lhs, rhs, rhs's USR).
    pub params: Vec<(String, u64)>,
    pub constraints: Vec<(String, String, String, Option<String>)>,
}

pub struct Module {
    pub symbols: Vec<Symbol>,
}

impl Module {
    pub fn find<'a>(&'a self, path: &'a [&str]) -> impl Iterator<Item = &'a Symbol> + 'a {
        self.symbols.iter().filter(move |s| s.path.len() == path.len() && s.path.iter().zip(path).all(|(a, b)| a == b))
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
        let loaded = locate(name).and_then(|file| load(&file)).map(|m| &*Box::leak(Box::new(m)));
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

fn locate(module: &str) -> Option<PathBuf> {
    for dir in std::env::var("APUS_SYMBOL_GRAPHS").unwrap_or_default().split(':').filter(|d| !d.is_empty()) {
        let file = Path::new(dir).join(format!("{module}.symbols.json"));
        if file.exists() {
            return Some(file);
        }
    }
    if module == "Swift" || module == "_Concurrency" {
        return None;
    }
    extract(module)
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

fn load(file: &Path) -> Option<Module> {
    let text = std::fs::read_to_string(file).ok()?;
    let json: serde_json::Value = serde_json::from_str(&text).ok()?;
    let string = |v: &serde_json::Value| v.as_str().map(str::to_string);
    let symbols = json["symbols"]
        .as_array()?
        .iter()
        .filter_map(|s| {
            let generics = &s["swiftGenerics"];
            Some(Symbol {
                kind: string(&s["kind"]["identifier"])?,
                usr: string(&s["identifier"]["precise"])?,
                path: s["pathComponents"].as_array()?.iter().filter_map(string).collect(),
                fragments: s["declarationFragments"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|f| Some(Fragment { kind: string(&f["kind"])?, spelling: string(&f["spelling"])?, precise: string(&f["preciseIdentifier"]) }))
                    .collect(),
                params: generics["parameters"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|p| Some((string(&p["name"])?, p["depth"].as_u64()?)))
                    .collect(),
                constraints: generics["constraints"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|c| Some((string(&c["kind"])?, string(&c["lhs"])?, string(&c["rhs"])?, string(&c["rhsPrecise"]))))
                    .collect(),
            })
        })
        .collect();
    Some(Module { symbols })
}
