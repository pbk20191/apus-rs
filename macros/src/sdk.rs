//! The symbols the SDK's modules export, read from their text stubs (`.tbd`), so that `swift!` can refuse at compile
//! time a declaration whose symbol a framework does not export.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

/// The SDK this build compiles against: `SDKROOT`, else `xcrun --sdk macosx --show-sdk-path`; `None` if neither.
pub fn sdk() -> Option<&'static Path> {
    static SDK: OnceLock<Option<PathBuf>> = OnceLock::new();
    SDK.get_or_init(|| {
        if let Some(root) = std::env::var_os("SDKROOT").filter(|r| !r.is_empty()) {
            return Some(PathBuf::from(root));
        }
        let output = std::process::Command::new("xcrun").args(["--sdk", "macosx", "--show-sdk-path"]).output().ok()?;
        let path = String::from_utf8(output.stdout).ok()?;
        output.status.success().then(|| PathBuf::from(path.trim()))
    })
    .as_deref()
}

/// The SDK's version (`27.3`): `xcrun --sdk macosx --show-sdk-version`.
pub fn sdk_version() -> Option<&'static str> {
    static VERSION: OnceLock<Option<String>> = OnceLock::new();
    VERSION
        .get_or_init(|| {
            let output = std::process::Command::new("xcrun").args(["--sdk", "macosx", "--show-sdk-version"]).output().ok()?;
            output.status.success().then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
        })
        .as_deref()
}

/// The symbols module `module` exports in the SDK (its framework's and its Swift overlay's stubs, with the
/// libraries they re-export); `None` when the SDK has no such module (a program's own) or cannot be read.
pub fn exported_symbols(module: &str) -> Option<&'static HashSet<String>> {
    static MODULES: OnceLock<Mutex<HashMap<String, Option<&'static HashSet<String>>>>> = OnceLock::new();
    let mut modules = MODULES.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner());
    *modules.entry(module.to_string()).or_insert_with(|| {
        let sdk = sdk()?;
        let library = if module == "Swift" { "Core" } else { module };
        let stubs = [
            sdk.join(format!("System/Library/Frameworks/{module}.framework/{module}.tbd")),
            sdk.join(format!("usr/lib/swift/libswift{library}.tbd")),
        ];
        let mut symbols = HashSet::new();
        let mut seen = HashSet::new();
        let mut pending: Vec<PathBuf> = stubs.into_iter().filter(|stub| stub.exists()).collect();
        if pending.is_empty() {
            return None;
        }
        while let Some(stub) = pending.pop() {
            if !seen.insert(stub.clone()) {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&stub) else { continue };
            read_stub(&text, &mut symbols, |install_name| {
                let reexported = sdk.join(format!("{}.tbd", install_name.trim_start_matches('/').trim_end_matches(".dylib")));
                if reexported.exists() {
                    pending.push(reexported);
                }
            });
        }
        Some(&*Box::leak(Box::new(symbols)))
    })
}

/// The Swift symbols (`_$s…`, without the leading underscore) a stub lists, and the install names of the libraries
/// it re-exports.
fn read_stub(text: &str, symbols: &mut HashSet<String>, mut reexport: impl FnMut(&str)) {
    let symbol_char = |c: char| c.is_ascii_alphanumeric() || c == '_' || c == '$' || c == '.';
    let mut rest = text;
    while let Some(at) = rest.find("_$s") {
        let symbol = &rest[at + 1..];
        let end = symbol.find(|c: char| !symbol_char(c)).unwrap_or(symbol.len());
        symbols.insert(symbol[..end].to_string());
        rest = &symbol[end..];
    }
    // `reexported-libraries:` … `libraries: [ '/path', … ]` (YAML text stubs, v4).
    let mut in_reexports = false;
    for line in text.lines() {
        if !line.starts_with(' ') {
            in_reexports = line.starts_with("reexported-libraries:");
            continue;
        }
        if in_reexports {
            for name in line.split('\'').skip(1).step_by(2).filter(|n| n.starts_with('/')) {
                reexport(name);
            }
        }
    }
}
