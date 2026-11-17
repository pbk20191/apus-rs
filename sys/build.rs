use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The deployment floor: macOS 14, or 15 with the `macos15` feature.
const FLOOR: (u32, u32) = (14, 0);
const FLOOR_MACOS15: (u32, u32) = (15, 0);

fn parse(version: &str) -> Option<(u32, u32)> {
    let mut parts = version.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next().map_or(Some(0), |m| m.parse().ok())?;
    Some((major, minor))
}

/// The first number after `marker` in `text`: `clang version 23.1.5` -> 23.
fn major_after(text: &str, marker: &str) -> Option<u32> {
    let rest = &text[text.find(marker)? + marker.len()..];
    rest.trim_start().split(|c: char| !c.is_ascii_digit()).next()?.parse().ok()
}

fn output(program: &Path, arg: &str) -> String {
    Command::new(program).arg(arg).output().map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default()
}

/// The values of rustc codegen option `name` in RUSTFLAGS, in order (`-Cname=v`, `-C name=v`, `--codegen name=v`,
/// `--codegen=name=v`); an option without `=` gives an empty value. rustc uses the last one.
fn codegen_options(name: &str) -> Vec<String> {
    let flags = env::var("CARGO_ENCODED_RUSTFLAGS").unwrap_or_default();
    let flags: Vec<&str> = flags.split('\x1f').filter(|f| !f.is_empty()).collect();
    let mut values = Vec::new();
    let mut i = 0;
    while i < flags.len() {
        let option = match flags[i] {
            "-C" | "--codegen" => {
                i += 1;
                flags.get(i).copied()
            }
            flag => flag.strip_prefix("--codegen=").or_else(|| flag.strip_prefix("-C").filter(|rest| !rest.is_empty())),
        };
        if let Some((key, value)) = option.map(|o| o.split_once('=').unwrap_or((o, ""))) {
            if key == name {
                values.push(value.to_string());
            }
        }
        i += 1;
    }
    values
}

/// Cross-language LTO: the C side becomes LLVM bitcode that rustc's LLVM reads at link time, so it must come from a clang
/// with the same LLVM major version, compiled for the same CPU (otherwise LLVM refuses to inline Rust into C), and the
/// final link must run rustc's linker-plugin LTO.
fn configure_xlto(build: &mut cc::Build) {
    let guide = "README: Cross-language LTO";
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        panic!("apus-sys: the xlto feature supports macOS targets only ({guide})");
    }
    let plugin = codegen_options("linker-plugin-lto");
    if plugin.last().is_none_or(|value| matches!(value.as_str(), "no" | "n" | "off" | "false")) {
        panic!(
            "apus-sys: the xlto feature needs RUSTFLAGS -Clinker-plugin-lto and the scripts/xlto-link linker; without them Apple's ld reads the bitcode with its own LTO library and fails ({guide})"
        );
    }
    let compiler = build.get_compiler();
    let clang = compiler.path().to_path_buf();
    let target = env::var("TARGET").unwrap_or_default().replace('-', "_");
    let version = output(&clang, "--version");
    if version.contains("Apple clang") {
        panic!(
            "apus-sys: the xlto feature needs an LLVM clang, not Apple clang (whose version numbers are Apple's, not LLVM's); set CC_{target} to one with rustc's LLVM version ({guide})"
        );
    }
    let rustc = PathBuf::from(env::var("RUSTC").unwrap_or_else(|_| "rustc".into()));
    let theirs = major_after(&version, "clang version");
    let ours = major_after(&output(&rustc, "-vV"), "LLVM version:");
    if theirs.is_none() || theirs != ours {
        panic!(
            "apus-sys: the xlto feature needs a clang with rustc's LLVM {}, but {} is LLVM {}; set CC_{target} to such a clang, for example /opt/homebrew/opt/llvm/bin/clang ({guide})",
            ours.map_or("?".into(), |v| v.to_string()),
            clang.display(),
            theirs.map_or("?".into(), |v| v.to_string()),
        );
    }
    let arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    // rustc's defaults for aarch64-apple-darwin and x86_64-apple-darwin.
    let default = if arch == "aarch64" { "apple-m1" } else { "penryn" };
    let cpu = codegen_options("target-cpu").pop().unwrap_or_else(|| default.into());
    build.flag("-flto=full").flag(if arch == "aarch64" { format!("-mcpu={cpu}") } else { format!("-march={cpu}") });
    let archiver = clang.with_file_name("llvm-ar");
    assert!(archiver.exists(), "apus-sys: the xlto feature needs llvm-ar next to {}", clang.display());
    build.archiver(archiver);
}

fn main() {
    println!("cargo:rerun-if-changed=include");
    println!("cargo:rerun-if-changed=csrc");
    println!("cargo:rerun-if-env-changed=MACOSX_DEPLOYMENT_TARGET");
    if env::var("CARGO_CFG_TARGET_VENDOR").as_deref() != Ok("apple") {
        return; // lib.rs stops the build with a clear message
    }
    let floor = if env::var_os("CARGO_FEATURE_MACOS15").is_some() { FLOOR_MACOS15 } else { FLOOR };
    let minimum = if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        // The floor must reach the final binary, not just these C objects: rustc links for macOS 11 unless told
        // otherwise, and below macOS 12 the SDK points the binary at an @rpath copy of the Swift runtime that a
        // dependency cannot add an rpath for, so it would fail to launch.
        let target = env::var("MACOSX_DEPLOYMENT_TARGET").ok();
        match target.as_deref().map(|value| (value, parse(value))) {
            Some((_, Some(wanted))) if wanted >= floor => wanted,
            Some((value, _)) => panic!(
                "apus-sys: MACOSX_DEPLOYMENT_TARGET is {value}, below the floor {}.{}{}",
                floor.0,
                floor.1,
                if floor == FLOOR_MACOS15 { " of the macos15 feature" } else { "" }
            ),
            None => panic!(
                "apus-sys: set MACOSX_DEPLOYMENT_TARGET to {}.{} or later (for example in .cargo/config.toml: [env] MACOSX_DEPLOYMENT_TARGET = \"{}.{}\")",
                floor.0, floor.1, floor.0, floor.1
            ),
        }
    } else {
        floor
    };

    let include = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap()).join("include");
    println!("cargo:rerun-if-env-changed=CARGO_ENCODED_RUSTFLAGS");
    let mut build = cc::Build::new();
    if env::var_os("CARGO_FEATURE_XLTO").is_some() {
        configure_xlto(&mut build);
    }
    build
        .file("csrc/funclets.c")
        .file("csrc/thunks.c")
        .include(&include)
        .opt_level(2)
        .warnings(true)
        .extra_warnings(true)
        .flag("-Wno-unused-parameter")
        // Boundary functions are found by name inside the binary only, never exported.
        .flag("-fvisibility=hidden")
        // The header renames the boundary identifiers only for apus's own C (not for C that includes it elsewhere).
        .define("APUS_SYS_BUILD", None)
        .flag(format!("-mmacosx-version-min={}.{}", minimum.0, minimum.1))
        .compile("apus_funclets");
    println!("cargo:rustc-link-search=native=/usr/lib/swift");
    println!("cargo:rustc-link-lib=dylib=swiftCore");
    println!("cargo:rustc-link-lib=dylib=swift_Concurrency");
    println!("cargo:include={}", include.display());
}
