# apus-sys

The Swift runtime ABI as apus uses it: one C header (`include/apus/abi.h`), the C funclets compiled against it, and
the Rust declarations generated from it. Internal to [apus](..): no stability guarantees, apus pins an exact version.

Apple platforms only. Deployment floor: macOS 14; the `macos15` feature raises it to macOS 15.
