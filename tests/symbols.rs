//! The C↔Rust boundary symbols stay in apus's own namespace: C-defined ones are never exported, and every exported
//! one carries the `apus_<tag>_` prefix, so no other library's name can clash with them.
use std::process::Command;

/// Functions the C side defines and Rust calls.
const C_DEFINED: [&str; 8] = [
    "rs_entry_fn",
    "rs_context_size",
    "rs_job_metadata",
    "rs_run_job_fn",
    "rs_string_from_utf8",
    "rs_call_sync",
    "apus_swift_task_isCancelled",
    "apus_fill_isolation",
];

fn main() {
    // Make sure the boundary is linked in: the self-check runs on the first spawn.
    let value = apus::run_main(async { apus::swift::spawn(async { 7 }).await.unwrap() });
    assert_eq!(value, 7);
    let exe = std::env::current_exe().unwrap();
    let out = Command::new("xcrun").args(["dyld_info", "-exports"]).arg(&exe).output().unwrap();
    assert!(out.status.success(), "dyld_info failed: {}", String::from_utf8_lossy(&out.stderr));
    let exports = String::from_utf8(out.stdout).unwrap();
    let prefix = format!("_{}", apus::__sys::BOUNDARY_PREFIX);
    // Unstripped, the Rust-defined callbacks are exported: under the namespace. Seeing one proves the list was read.
    assert!(exports.contains(&format!("{prefix}rs_poll")), "no {prefix}rs_poll in the export list:\n{exports}");
    let mut failures = Vec::new();
    for symbol in exports.lines().filter_map(|l| l.split_whitespace().nth(1)) {
        if C_DEFINED.iter().any(|c| symbol.ends_with(c)) {
            failures.push(format!("C-defined boundary symbol exported: {symbol}"));
        } else if (symbol.starts_with("_rs_") || symbol.starts_with("_apus_")) && !symbol.starts_with(&prefix) {
            failures.push(format!("boundary symbol outside {prefix}: {symbol}"));
        }
    }
    if failures.is_empty() {
        println!("boundary_symbols_are_namespaced ... ok");
    } else {
        eprintln!("{}", failures.join("\n"));
        std::process::exit(1);
    }
}
