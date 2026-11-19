//! Builds the Swift model (swift/Todos.swift) into a static library: the Swift a SwiftData program still needs.
use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=swift");
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    let arch = match env::var("CARGO_CFG_TARGET_ARCH").unwrap().as_str() {
        "aarch64" => "arm64",
        other => other,
    }
    .to_string();
    // SwiftData needs macOS 14. Built for library evolution, like Apple's frameworks, so its classes' members are
    // called the same way.
    let status = Command::new("xcrun")
        .args(["swiftc", "-O", "-parse-as-library", "-static", "-emit-library", "-enable-library-evolution"])
        .args(["-target", &format!("{arch}-apple-macos14.0"), "-module-name", "Todos", "swift/Todos.swift", "-o"])
        .arg(out.join("libtodos.a"))
        .status()
        .expect("failed to run swiftc");
    assert!(status.success(), "swiftc failed for swift/Todos.swift");
    println!("cargo:rustc-link-search=native={}", out.display());
    println!("cargo:rustc-link-lib=static=todos");
    // The toolchain's compatibility libraries, which a static Swift library may reference.
    let swiftc = Command::new("xcrun").args(["--find", "swiftc"]).output().unwrap();
    let swiftc = PathBuf::from(String::from_utf8(swiftc.stdout).unwrap().trim());
    println!("cargo:rustc-link-search=native={}", swiftc.parent().unwrap().parent().unwrap().join("lib/swift/macosx").display());
}
