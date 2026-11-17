use std::env;
use std::path::PathBuf;

/// The deployment floor: macOS 14, or 15 with the `macos15` feature.
const FLOOR: (u32, u32) = (14, 0);
const FLOOR_MACOS15: (u32, u32) = (15, 0);

fn parse(version: &str) -> Option<(u32, u32)> {
    let mut parts = version.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next().map_or(Some(0), |m| m.parse().ok())?;
    Some((major, minor))
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
    cc::Build::new()
        .file("csrc/funclets.c")
        .file("csrc/thunks.c")
        .include(&include)
        .opt_level(2)
        .warnings(true)
        .extra_warnings(true)
        .flag("-Wno-unused-parameter")
        // Boundary functions are found by name inside the binary only, never exported.
        .flag("-fvisibility=hidden")
        .flag(format!("-mmacosx-version-min={}.{}", minimum.0, minimum.1))
        .compile("apus_funclets");
    println!("cargo:rustc-link-search=native=/usr/lib/swift");
    println!("cargo:rustc-link-lib=dylib=swiftCore");
    println!("cargo:rustc-link-lib=dylib=swift_Concurrency");
    println!("cargo:include={}", include.display());
}
