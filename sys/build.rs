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
    let target = env::var("MACOSX_DEPLOYMENT_TARGET").ok();
    let wanted = target.as_deref().and_then(parse);
    if let (Some(value), Some(wanted)) = (target.as_deref(), wanted) {
        if floor == FLOOR_MACOS15 && wanted < FLOOR_MACOS15 {
            panic!("apus-sys: the macos15 feature needs MACOSX_DEPLOYMENT_TARGET >= 15.0, but it is {value}");
        }
    }
    let minimum = wanted.filter(|w| *w > floor).unwrap_or(floor);

    let include = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap()).join("include");
    cc::Build::new()
        .file("csrc/funclets.c")
        .include(&include)
        .opt_level(2)
        .warnings(true)
        .extra_warnings(true)
        .flag("-Wno-unused-parameter")
        .flag(format!("-mmacosx-version-min={}.{}", minimum.0, minimum.1))
        .compile("apus_funclets");
    println!("cargo:rustc-link-search=native=/usr/lib/swift");
    println!("cargo:rustc-link-lib=dylib=swiftCore");
    println!("cargo:rustc-link-lib=dylib=swift_Concurrency");
    // A link argument reaches only this crate's own targets (its tests); apus sets its own.
    println!("cargo:rustc-link-arg=-Wl,-rpath,/usr/lib/swift");
    println!("cargo:include={}", include.display());
}
