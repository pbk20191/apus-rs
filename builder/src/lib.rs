//! Compiles a program's own Swift sources, the Swift a Rust program still needs (a SwiftData `@Model`, say), into a
//! static library and links it. Call it from `build.rs`:
//!
//! ```no_run
//! apus_build::SwiftModule::new("Todos", "swift").build();
//! ```
//!
//! The module is built like Apple's frameworks (library evolution), so `swift!` calls its classes' members the same
//! way, for the macOS version the program is built for (`MACOSX_DEPLOYMENT_TARGET`, which apus needs anyway).

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

/// A Swift module compiled from source files into `lib<name>.a`.
pub struct SwiftModule {
    name: String,
    sources: PathBuf,
    library_evolution: bool,
    flags: Vec<String>,
}

impl SwiftModule {
    /// `sources`: a directory (every `*.swift` in it) or one file, relative to the package.
    pub fn new(name: &str, sources: impl AsRef<Path>) -> SwiftModule {
        SwiftModule { name: name.to_string(), sources: sources.as_ref().to_path_buf(), library_evolution: true, flags: Vec::new() }
    }

    /// Whether to build for library evolution, as Apple's frameworks are (the default).
    pub fn library_evolution(mut self, on: bool) -> SwiftModule {
        self.library_evolution = on;
        self
    }

    /// One more `swiftc` flag.
    pub fn flag(mut self, flag: &str) -> SwiftModule {
        self.flags.push(flag.to_string());
        self
    }

    /// Compiles the module and tells Cargo to link it. Panics with the reason on any failure (in a build script, the
    /// build stops with that message).
    pub fn build(self) {
        println!("cargo:rerun-if-changed={}", self.sources.display());
        println!("cargo:rerun-if-env-changed=MACOSX_DEPLOYMENT_TARGET");
        if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
            panic!("apus-build: only macOS targets are supported");
        }
        let Ok(deployment) = env::var("MACOSX_DEPLOYMENT_TARGET") else {
            panic!(
                "apus-build: set MACOSX_DEPLOYMENT_TARGET (for example in .cargo/config.toml: [env] MACOSX_DEPLOYMENT_TARGET = \"14.0\")"
            );
        };
        let arch = match env::var("CARGO_CFG_TARGET_ARCH").expect("CARGO_CFG_TARGET_ARCH").as_str() {
            "aarch64" => "arm64".to_string(),
            other => other.to_string(),
        };
        let sources = self.source_files();
        let out = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
        let library = self.name.to_lowercase();
        let optimize = if env::var("PROFILE").as_deref() == Ok("release") { "-O" } else { "-Onone" };
        let mut swiftc = Command::new("xcrun");
        swiftc.args(["swiftc", optimize, "-parse-as-library", "-static", "-emit-library", "-module-name", &self.name]);
        swiftc.args(["-target", &format!("{arch}-apple-macos{deployment}")]);
        if self.library_evolution {
            swiftc.arg("-enable-library-evolution");
        }
        swiftc.args(&self.flags).args(&sources).arg("-o").arg(out.join(format!("lib{library}.a")));
        let status = swiftc.status().unwrap_or_else(|e| panic!("apus-build: cannot run `xcrun swiftc`: {e}"));
        assert!(status.success(), "apus-build: swiftc failed for module {}", self.name);
        println!("cargo:rustc-link-search=native={}", out.display());
        println!("cargo:rustc-link-lib=static={library}");
        // The toolchain's compatibility libraries, which a static Swift library may reference.
        let found = Command::new("xcrun").args(["--find", "swiftc"]).output().expect("xcrun --find swiftc");
        let swiftc = PathBuf::from(String::from_utf8(found.stdout).expect("a path").trim());
        let toolchain = swiftc.parent().and_then(Path::parent).expect("swiftc in <toolchain>/usr/bin");
        println!("cargo:rustc-link-search=native={}", toolchain.join("lib/swift/macosx").display());
    }

    /// The source files, sorted: the directory's `*.swift`, or the one file.
    fn source_files(&self) -> Vec<PathBuf> {
        if !self.sources.is_dir() {
            return vec![self.sources.clone()];
        }
        let entries = std::fs::read_dir(&self.sources).unwrap_or_else(|e| panic!("apus-build: {}: {e}", self.sources.display()));
        let mut files: Vec<PathBuf> =
            entries.filter_map(Result::ok).map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "swift")).collect();
        files.sort();
        assert!(!files.is_empty(), "apus-build: no .swift files in {}", self.sources.display());
        files
    }
}
