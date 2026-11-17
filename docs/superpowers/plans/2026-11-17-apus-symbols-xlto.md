# apus boundary symbols and xLTO Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give apus's C↔Rust boundary symbols a collision-proof namespace, keep C ones out of the export list, and add an `xlto` feature that compiles the C side for cross-language LTO.

**Architecture:** Link names come from the header (`APUS_BOUNDARY(name)` asm labels with `APUS_TAG`); bindgen carries them to Rust callers, a generated `boundary_name!` macro gives Rust-defined callbacks the same names. `xlto` makes apus-sys emit LLVM bitcode with rustc's LLVM and CPU; the final link is the user's (script and README provided).

**Tech Stack:** Rust, cc, bindgen (existing), Homebrew LLVM 23 (only for `xlto` and `scripts/check-xlto.sh`).

**Spec:** `docs/superpowers/specs/2026-11-17-apus-symbols-xlto-design.md`

## Global Constraints

- Link name of every boundary symbol: `apus_<APUS_TAG>_<C name>`; `APUS_TAG` = sys version with `.`→`_`, `_`, FNV-1a 32-bit (lower 32 bits of the 64-bit FNV-1a used in fresh) of the version string as 8 lowercase hex digits.
- C compiled with `-fvisibility=hidden`.
- No function-pointer tables, no `ld -r`: boundary calls stay direct.
- `xlto` refuses a C compiler whose LLVM major differs from rustc's, naming both and `CC_<target>`.
- Default CPU when RUSTFLAGS set none: `apple-m1` (aarch64, `-mcpu`), `penryn` (x86_64, `-march`).
- Fewer tests: new `tests/symbols.rs` only; `scripts/check-xlto.sh` is not part of `cargo test`.
- Commits on feat/runtime with the Co-Authored-By trailer; no push.

## Review Focus

1. A Rust callback renamed in the header but not in Rust: the link fails (undefined `_apus_<tag>_…`), not a silent fallback — Task 1 build proves it by construction.
2. Bumping `sys` version without updating `APUS_TAG`: `fresh` fails with the expected tag — Task 1 adds the check.
3. `xlto` with Apple clang 21 against rustc LLVM 23: clear build error — Task 2 verifies.
4. `xlto` build still runs and `step` has no call to the poll callback — Task 2 check script.
5. Default (non-xlto) build: exports contain no C-defined boundary symbol — Task 1 smoke test.

---

### Task 1: boundary namespace and hidden C symbols

**Files:** `sys/include/apus/abi.h`, `sys/build.rs`, `sys/tests/fresh.rs`, `sys/src/lib.rs`, generated `sys/src/boundary.rs` (+ regenerated bindings), `src/swift/{task,call,timer}.rs` (callbacks), Create `tests/symbols.rs`, `Cargo.toml` ([[test]] symbols, harness false, required-features swift).

**Interfaces:** Produces `#define APUS_TAG "0_1_0_xxxxxxxx"`, `APUS_BOUNDARY(name)`; `apus_sys::boundary_name!(rs_poll)` → `"apus_<tag>_rs_poll"` for every Rust-implemented callback; `apus_sys::BOUNDARY_PREFIX: &str`.

- [ ] Step 1: write `tests/symbols.rs` (reads `xcrun dyld_info -exports <current_exe>`; asserts no export ends with a C-defined boundary name (`rs_entry_fn`, `rs_context_size`, `rs_job_metadata`, `rs_run_job_fn`, `rs_string_from_utf8`, `rs_call_sync`, `apus_swift_task_isCancelled`) and every export containing `rs_` or `apus_` starts with `_` + `BOUNDARY_PREFIX`). Run → FAIL (unprefixed `_rs_poll` exported).
- [ ] Step 2: header: `APUS_TAG`, `APUS_BOUNDARY(n) __asm__("_apus_" APUS_TAG "_" #n)` on every funclet export, thunk and Rust callback declaration; build.rs `-fvisibility=hidden`; fresh: check tag against `CARGO_PKG_VERSION`, generate `boundary.rs` (`BOUNDARY_PREFIX` and `macro_rules! boundary_name`, one arm per Rust-implemented callback, read from the header's Rust-callback section); lib.rs `#[macro_use]`-exports it; Rust callbacks switch to `#[export_name = apus_sys::boundary_name!(…)]`. Bless.
- [ ] Step 3: verify: `cargo test --workspace --features dispatch,test-probe,stream` all pass incl. `symbols`; `MACOSX_DEPLOYMENT_TARGET=15.0 cargo test --workspace --all-features` pass; `nm` of `swift_basic` shows no `_rs_poll`, only `_apus_<tag>_rs_poll`; fresh with a wrong tag fails naming the expected one.
- [ ] Step 4: commit "Boundary symbols in their own namespace; C ones never exported".

### Task 2: `xlto` feature, link script, check script

**Files:** `sys/build.rs`, `sys/Cargo.toml` + root `Cargo.toml` (feature), Create `scripts/xlto-link`, `scripts/check-xlto.sh`, `README.md` section.

- [ ] Step 1: `scripts/check-xlto.sh` (builds `swift_basic` release for aarch64 with `--features xlto`, `CC_aarch64_apple_darwin=$APUS_LLVM/bin/clang`, `RUSTFLAGS=-Clinker-plugin-lto -Clinker=scripts/xlto-link`, `CARGO_PROFILE_RELEASE_LTO=fat`, separate target dir; runs it; fails if exports contain `apus_`/`rs_` or `_step` calls `…rs_poll`; skips with a message if `$APUS_LLVM/bin/clang` is missing). Run → FAIL (feature missing).
- [ ] Step 2: features; build.rs xlto branch (version check, `-flto=full`, CPU flag, `llvm-ar`); `scripts/xlto-link` (filters `-plugin-opt=*` and `-Wl,-plugin-opt=*`, execs `$APUS_LLVM/bin/clang -fuse-ld=/usr/bin/ld -Wl,-lto_library,$APUS_LLVM/lib/libLTO.dylib`).
- [ ] Step 3: verify: check script passes; `cargo build --features xlto` with Apple clang stops with the version message; default tests unchanged.
- [ ] Step 4: README "Cross-language LTO" section with the `.cargo/config.toml` example; commit "xlto feature: C as LLVM bitcode for cross-language LTO".
