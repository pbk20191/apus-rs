//! Swift runtime ABI declarations for apus. Internal: no stability guarantees, apus pins an exact version.
#[cfg(not(target_vendor = "apple"))]
compile_error!("apus-sys supports Apple platforms only");
