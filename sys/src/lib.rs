//! Swift runtime ABI declarations for apus, generated from `include/apus/abi.h` (sys/tests/fresh.rs). Internal: no
//! stability guarantees, apus pins an exact version.
#![allow(non_upper_case_globals, non_camel_case_types, non_snake_case)]

#[cfg(not(target_vendor = "apple"))]
compile_error!("apus-sys supports Apple platforms only");

include!("bindings.rs");
#[cfg(feature = "macos15")]
include!("bindings_macos15.rs");

mod boundary;
mod symbols;
pub use boundary::BOUNDARY_PREFIX;
pub use symbols::{OPTIONAL_SYMBOLS, REQUIRED_SYMBOLS};

/// macOS 15 declarations, looked up at run time unless the `macos15` feature makes them required.
pub mod optional {
    #[allow(unused_imports)]
    use super::*;
    include!("optional.rs");
}
