use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=tests/support");
    if env::var_os("CARGO_FEATURE_SWIFT").is_none() {
        return;
    }
    if env::var_os("CARGO_FEATURE_TEST_PROBE").is_some() {
        let out = PathBuf::from(env::var("OUT_DIR").unwrap());
        let arch = match env::var("CARGO_CFG_TARGET_ARCH").unwrap().as_str() {
            "aarch64" => "arm64",
            other => other,
        }
        .to_string();
        // MainActor.assumeIsolated needs macOS 14.
        let target = format!("{arch}-apple-macos14.0");
        let status = Command::new("xcrun")
            .args(["swiftc", "-O", "-parse-as-library", "-static", "-emit-library", "-swift-version", "5"])
            .args(["-target", &target, "-module-name", "Probe", "tests/support/probe.swift", "tests/support/fns.swift", "-o"])
            .arg(out.join("libasync_swift_probe.a"))
            .status()
            .expect("failed to run swiftc for the test probe");
        assert!(status.success(), "swiftc failed for the test probes");
        // A second module built for library evolution, like Apple's frameworks (tests/swift_existential.rs).
        let status = Command::new("xcrun")
            .args(["swiftc", "-O", "-parse-as-library", "-static", "-emit-library", "-enable-library-evolution", "-swift-version", "5"])
            .args(["-target", &target, "-module-name", "Res", "tests/support/resilient.swift", "-o"])
            .arg(out.join("libapus_resilient_probe.a"))
            .status()
            .expect("failed to run swiftc for the resilient test probe");
        assert!(status.success(), "swiftc failed for the resilient test probe");
        println!("cargo:rustc-link-search=native={}", out.display());
        println!("cargo:rustc-link-lib=static=async_swift_probe");
        println!("cargo:rustc-link-lib=static=apus_resilient_probe");
        let toolchain = Command::new("xcrun").args(["--find", "swiftc"]).output().unwrap();
        let swiftc = PathBuf::from(String::from_utf8(toolchain.stdout).unwrap().trim());
        let compat = swiftc.parent().unwrap().parent().unwrap().join("lib/swift/macosx");
        println!("cargo:rustc-link-search=native={}", compat.display());
    }
}
