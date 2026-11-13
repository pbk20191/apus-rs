# async-swift-rs Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A Rust crate that runs Rust futures as genuine Swift Concurrency Tasks via the Swift runtime ABI (no Swift source), plus a libdispatch backend with the same API.

**Architecture:** One crate `async-swift-rs`, two features. `dispatch` schedules `async-task` runnables with `dispatch_async_f`. `swift` (default) creates tasks with `swift_task_create_common`; a C shim (clang `swiftcall`/`swiftasynccall`) implements the async entry/step/continuation funclets and calls back into Rust over the C ABI to poll. Swift async closures are awaited in the same Task (or a child Task when concurrent).

**Tech Stack:** Rust 2021, `async-task` 4, `cc` (build), clang Swift CC attributes, OS Swift runtime (`/usr/lib/swift`), CoreFoundation, `libtest-mimic` (dev), swiftc (test probe only).

**Spec:** `docs/superpowers/specs/2026-11-13-async-swift-rs-design.md` (Korean). Read it before any task. Spike code it references: `docs/superpowers/specs/spikes/`.

## Global Constraints

- macOS only; arm64 and x86_64. No arm64e.
- No Swift source in the crate. Swift only in `tests/support/probe.swift`, compiled only when feature `test-probe` is on.
- No hand-written assembly. Swift calling conventions only through clang `swiftcall`, `swiftasynccall`, `swift_context`, `swift_async_context`.
- Every transcribed ABI struct, constant, or flag carries a comment naming its source file in `swiftlang/swift` @ `swift-6.4.2-RELEASE`.
- No GPL code copied or consulted (`eugenehp/rswift` is off-limits). Code taken from `dgrijalva/async-dispatch` (Apache-2.0) gets an attribution comment.
- No bindgen. Declare the few dispatch/CF/Swift-runtime functions by hand.
- Priority values: `UserInteractive=0x21, UserInitiated=0x19, Default=0x15, Utility=0x11, Background=0x09, Unspecified=0`.
- Main queue must be drained by `CFRunLoopRun`/`NSApplication.run`, never `dispatch_main` (it drains on a non-main thread).
- Panics never cross an FFI boundary.
- Commits: authorized. Docs are committed on `main`; implementation goes on branch `feat/runtime`, one commit per task step that says so. Never push.
- Code placement (spec §3.1): sync runtime functions are called **from Rust** via `extern "C"` (ABI-identical, verified by `tests/abi_equivalence.rs`); Swift-CC `bool` returns are read as `u8 & 1`. C (`csrc/funclets.c`) holds only what the runtime calls back: entry, step, resume point, after-call, child entry/after, after-waitAll, cancel handler, Job invoke.
- M0 decisions in `docs/superpowers/specs/m0-report.md` are binding: `()` metadata via `swift_getTupleTypeMetadata(0,0,NULL,NULL,NULL)` (never `$sytN`); MainActor hop-back with `swift_task_switch` after a same-Task call; children via discarding TaskGroup + `swift_taskGroup_waitAll`; `JoinHandle` resolves at Swift Task completion (`rs_finish`).

## Review Focus

1. A future that panics inside a Swift-backend task: the process must not abort; `JoinHandle` yields `Err(JoinError::Panicked)`. (test: Task 4 `panic_becomes_join_error`)
2. Dropping a `JoinHandle` before the task finishes: the task still runs to completion and the Swift task object is released exactly once. (test: Task 4 `dropped_handle_detaches`)
3. A waker cloned out of a task and woken after the task completed: no-op, no resume of a stale continuation. (test: Task 6 `late_wake_after_done_is_noop`)
4. `run_main` called off the main thread, and `block_on` on the main thread waiting for a main-queue task: clear panic message instead of deadlock. (tests: Task 3 `run_main_off_main_panics`, `block_on_main_waiting_on_main_panics`)
5. A `SwiftAsync` dropped without being awaited: its closure context is released (no leak). (test: Task 8 `dropped_swift_async_releases_context`)

---

## File Structure

```
Cargo.toml                 features: default=["swift"], swift, dispatch, test-probe
build.rs                   cc-compile csrc/*.c (swift); link swiftCore, swift_Concurrency (+rpath /usr/lib/swift), CoreFoundation;
                           swiftc tests/support/probe.swift -> libprobe.a when CARGO_FEATURE_TEST_PROBE
csrc/swift_abi.h           transcribed ABI: AsyncContext, ContinuationAsyncContext, RsTaskContext, ClosureBox, Job header
csrc/funclets.c            runtime-invoked code only: entry, step, resume_point, after_call, finish, after_wait_all,
                           child_entry/child_after, cancel handler, Job invoke/run_job, job metadata
src/swift/rt.rs            Rust extern "C" declarations of the Swift runtime sync functions (+ flag/option constants with sources)
src/lib.rs                 feature gates, re-exports
src/common.rs              Priority, JoinError, SpawnOptions
src/runloop.rs             run_main, is_main_thread (CF)
src/dispatch/{mod,ffi,queue,block_on}.rs
src/swift/{mod,rt,task,join,current,timer,selfcheck,call,any}.rs
tests/common/mod.rs        main-thread harness on libtest-mimic
tests/support/probe.swift  test-only Swift probes
tests/dispatch.rs  tests/swift_task.rs  tests/swift_call.rs   (all harness = false)
tests/abi_equivalence.rs   clang swiftcall-vs-C lowering check for every function in src/swift/rt.rs
examples/{dispatch_basic,swift_basic}.rs
README.md
```

---

### Task 1: M0 gate — Swift async closure in the same Task (spike, throwaway)

Prove G3 before building on it. Work in `docs/superpowers/specs/spikes/m0/`, starting from a copy of `spikes/swift-task/`. Not product code.

**Files:**
- Create: `docs/superpowers/specs/spikes/m0/{shim.c,main.rs,probe.swift,build.sh}`
- Create: `docs/superpowers/specs/m0-report.md`

**Interfaces:**
- Produces (in `m0-report.md`, consumed by Tasks 4, 8, 9): the closure value representation passed Swift→Rust; the user-side Swift recipe (≤20 lines, public Swift only, `unsafeBitCast` allowed, no `@_silgen_name`); how the async function pointer is resolved to code + context size; result `Any` and error delivery to the resume funclet; `IsChildTask` creation and the parent-wait rule; whether future tasks with `$sytN` work; Instruments observation.

- [ ] **Step 1:** Read `stdlib/public/Concurrency/Task.cpp`, `include/swift/ABI/Task.h`, `include/swift/Runtime/Concurrency.h` at `swift-6.4.2-RELEASE`, plus IRGen's async function pointer and async return lowering (`lib/IRGen/GenCall.cpp`, `GenFunc.cpp`). Note in the report how a thick `() async throws -> Any` closure's function word and context word are laid out and how callee context size is found.
- [ ] **Step 2:** Convert the spike's task creation to a future task: result type `$sytN`, entry shaped `AsyncGenericClosureEntryPoint(indirectResult, ctx, closure)`, context ≥ `FutureAsyncContext`. Rerun the 6 scenarios. Expected: all 6 lines as in spec §9, `20/20 clean exits`.
- [ ] **Step 3:** Write the recipe in `probe.swift` handing Rust a `() async throws -> Any`. Probe closures: `{ withUnsafeCurrentTask { $0!.hashValue } }` (task identity), `{ try await Task.sleep(nanoseconds: 50_000_000); return "slept" }`, `{ throw POSIXError(.EIO) }`, `{ () }`.
- [ ] **Step 4:** In `shim.c` implement the call: `swift_task_alloc` the callee context, set Parent/ResumeParent, tail-call; resume funclet receives the result/error, `swift_task_dealloc`s, wakes Rust. Verify: identity closure returns the same task as `swift_task_getCurrent()` in the caller; `"slept"` downcast; EIO error surfaces; cancelling the outer task mid-sleep makes the closure throw `CancellationError`.
- [ ] **Step 5:** Run two calls concurrently as child tasks (`IsChildTask`, bit 8). Verify both overlap in time (2×50ms sleeps finish in <90ms), child sees parent's task-local and priority, and parent cannot complete before children (await them before `ResumeParent`).
- [ ] **Step 6:** Run under Instruments "Swift Tasks" template (`xctrace record --template 'Swift Concurrency' --launch ./m0`). Record whether Rust tasks appear.
- [ ] **Step 7:** Write `m0-report.md` with the decisions above. Verdict: PASS only if steps 2–5 pass and the recipe meets the spec §7.6 bar.
- [ ] **Step 8: STOP.** Report the verdict to the user. On FAIL, do not continue (spec §8). On PASS, continue only after the user acknowledges.

---

### Task 2: Crate scaffold, common types, test harness, dispatch `spawn`

**Files:**
- Create: `Cargo.toml`, `build.rs` (dispatch part only for now), `src/lib.rs`, `src/common.rs`, `src/dispatch/{mod.rs,ffi.rs}`, `tests/common/mod.rs`, `tests/dispatch.rs`, `.gitignore`

**Interfaces:**
- Produces:
  - `pub enum Priority { UserInteractive, UserInitiated, Default, Utility, Background, Unspecified }` with `fn raw(self) -> u8` (values from Global Constraints) and `fn from_raw(u8) -> Priority` (unknown → nearest lower).
  - `pub enum JoinError { Aborted, Panicked(Box<dyn Any + Send>) }` implementing `Error + Display`.
  - `pub struct SpawnOptions { pub priority: Priority, pub main: bool, pub copy_task_locals: bool }` (`Default`: `Unspecified, false, true`).
  - `dispatch::spawn<F: Future + Send + 'static>(f: F) -> dispatch::JoinHandle<F::Output>` where `F::Output: Send`; `dispatch::spawn_with(opts: SpawnOptions, f) -> JoinHandle<_>`; `JoinHandle<T>: Future<Output = Result<T, JoinError>>` with `abort(&self)`; drop = detach.
  - `tests/common::run(tests: Vec<Trial>)`: runs each trial body on a background thread while the main thread spins `CFRunLoopRun`; exits with libtest-mimic's status.

- [ ] **Step 1: Failing tests** in `tests/dispatch.rs`:
  - `spawn_returns_value`: `assert_eq!(block(spawn(async { 7 })), Ok(7))` (local `block` = park/unpark helper in the test).
  - `ten_thousand_sleeps_overlap`: 10_000 tasks each `sleep(50ms)` then return `i`; sum == 49_995_000; elapsed < 500ms.
  - `priority_maps_to_qos`: task spawned with `Priority::Utility` observes `qos_class_self() == 0x11`; `UserInitiated` → `0x19`.
  - `abort_yields_aborted`: task sleeping 10s, `abort()` after 10ms → `Err(JoinError::Aborted)` within 100ms.
  - `panic_becomes_join_error`: `spawn(async { panic!("boom") })` → `Err(JoinError::Panicked(_))`, process alive.
  - `dropped_handle_detaches`: drop handle, task still sets an `AtomicBool` within 100ms.
- [ ] **Step 2:** `cargo test --no-default-features --features dispatch --test dispatch` → FAIL (unresolved items).
- [ ] **Step 3:** Implement. `async_task::spawn(future, schedule)` with `schedule = |r| dispatch_async_f(queue, r.into_raw(), trampoline)`; `trampoline` does `Runnable::from_raw(..).run()`. Queue = `dispatch_get_global_queue(priority.raw() as isize, 0)` (`Unspecified` → `Default`). Panic catching via `catch_unwind` around the user future (`AssertUnwindSafe`). `sleep` = `dispatch_after_f` waking a boxed `Waker`. Attribution comment to async-dispatch where its structure is followed.
- [ ] **Step 4:** Same command → PASS (6 tests).
- [ ] **Step 5:** Commit `feat(dispatch): spawn, JoinHandle, sleep, priorities`.

---

### Task 3: Dispatch main queue, `run_main`, serial queues, `block_on`, inheritance

**Files:**
- Create: `src/runloop.rs`, `src/dispatch/{queue.rs,block_on.rs}`
- Modify: `src/dispatch/mod.rs`, `src/lib.rs`, `tests/dispatch.rs`

**Interfaces:**
- Consumes: Task 2 types.
- Produces:
  - `pub fn run_main<F: Future + 'static>(f: F) -> F::Output` (crate root; panics `"run_main must be called on the main thread"` off main). Spawns `f` locally on the main queue, runs `CFRunLoopRun`, `CFRunLoopStop` on completion.
  - `pub fn is_main_thread() -> bool` (`pthread_main_np`).
  - `dispatch::spawn_on_main(f) -> JoinHandle<_>`, `dispatch::spawn_local_on_main<F: Future + 'static>(f) -> JoinHandle<F::Output>` (panics off main), `dispatch::spawn_detached(f)`.
  - `dispatch::SerialQueue::new(label: &str, priority: Priority) -> SerialQueue`, `SerialQueue::spawn(&self, f) -> JoinHandle<_>`.
  - `dispatch::block_on<F: Future>(f: F) -> F::Output`.

- [ ] **Step 1: Failing tests** (all run through `tests/common::run`):
  - `main_task_runs_on_main_thread`: `spawn_on_main(async { is_main_thread() })` → `Ok(true)`, also after a `sleep(5ms)`.
  - `spawn_inside_main_stays_main`: inside a main task, `spawn(async { is_main_thread() })` → `true`; `spawn_detached(..)` → `false`.
  - `priority_inherited`: inside a `Utility` task, `spawn` child observes `qos_class_self() == 0x11`.
  - `serial_queue_never_overlaps`: 100 tasks on one `SerialQueue` each increment an in-flight counter, `sleep(1ms)`, decrement; max in-flight == 1.
  - `block_on_off_main`: `block_on(async { sleep(5ms).await; 3 })` from a background thread → 3.
  - `run_main_off_main_panics`: `catch_unwind(|| run_main(async {}))` on a background thread → message contains `"run_main must be called on the main thread"`.
  - `block_on_main_waiting_on_main_panics`: separate `harness=false` test binary `tests/dispatch_main_deadlock.rs` whose `main()` calls `block_on(spawn_on_main(async {}))` and asserts the panic message contains `"use run_main"`.
- [ ] **Step 2:** `cargo test --no-default-features --features dispatch` → FAIL.
- [ ] **Step 3:** Implement. Main queue = `&_dispatch_main_q`. Inheritance: a thread-local "current dispatch target" set by the trampoline around `run()`; `spawn` reads it (main → main queue; otherwise inherit QoS via `Unspecified` → `qos_class_self()`). `block_on` panics with `"block_on on the main thread cannot wait for main-queue work; use run_main"` when called on main.
- [ ] **Step 4:** Same command → PASS.
- [ ] **Step 5:** Commit `feat(dispatch): main queue, run_main, serial queues, block_on`.

---

### Task 4: Swift backend core — runtime bindings, funclets, task creation, poll/park/resume, self-check

**Files:**
- Create: `csrc/swift_abi.h`, `csrc/funclets.c`, `src/swift/{mod.rs,rt.rs,task.rs,join.rs,selfcheck.rs}`, `tests/support/probe.swift`, `tests/swift_task.rs`, `tests/abi_equivalence.rs`
- Modify: `build.rs`, `Cargo.toml`, `src/lib.rs`

**Interfaces:**
- Consumes: `m0-report.md`; Task 2 `Priority`, `JoinError`, `SpawnOptions`; Task 3 `run_main`.
- Produces:
  - `src/swift/rt.rs`: `extern "C"` decls for `swift_task_create_common(usize, *mut c_void, *const c_void, *const c_void, *mut c_void, usize) -> AsyncTaskAndContext` (`#[repr(C)] struct { task, initial_context }`), `swift_task_getMainExecutor() -> SerialExecutorRef` (`#[repr(C)] { identity: *const c_void, implementation: usize }`), `swift_continuation_init(*mut c_void, usize) -> *mut c_void`, `swift_continuation_resume`, `swift_task_cancel`, `swift_task_getCurrent`, `swift_task_isCancelled -> u8`, `swift_task_currentPriority -> usize`, `swift_task_escalate`, `swift_task_alloc`, `swift_task_dealloc`, `swift_taskGroup_initializeWithFlags`, `swift_taskGroup_destroy`, `swift_getTupleTypeMetadata -> MetadataResponse`, `swift_job_run`, `swift_task_enqueueGlobalWithDelay`, `swift_release`, `swift_retain`; constants `CREATE_*`, `CONT_HAS_EXECUTOR_OVERRIDE`, `OPTION_KIND_*`, `GROUP_DISCARD_RESULTS`, each with its source file comment. `fn empty_tuple_metadata() -> *const c_void` (cached).
  - C → Rust (`#[no_mangle] extern "C"` in `task.rs`): `rs_poll(*mut c_void) -> i32`, `rs_park(rust, continuation) -> i32`, `rs_finish(rust)`, `rs_cancelled(rust)`, `rs_take_call(rust, *mut *mut c_void, *mut *mut c_void, *mut *const c_void) -> i32`, `rs_take_child(rust, *mut *const c_void) -> i32`, `rs_call_closure(rec, *mut *mut c_void, *mut *mut c_void)`, `rs_set_child(rec, *mut c_void)`, `rs_call_done(rec, *mut c_void)` (the call ones return 0 / no-op until Task 8).
  - Rust → C: only `rs_entry_fn() -> *const c_void` (address of the `AsyncGenericClosureEntryPoint` funclet), `rs_child_entry_fn() -> *const c_void`, `rs_context_size() -> usize`, `rs_child_context_size() -> usize`. `entry` asks Rust `rs_is_main(rust) -> bool` for the executor, as in the spike.
  - Rust: `swift::spawn`, `swift::spawn_with(SpawnOptions, f)`, `swift::spawn_on_main`, `swift::spawn_detached`; `swift::JoinHandle<T>: Future<Output = Result<T, JoinError>>` resolving in `rs_finish`; crate-root re-exports when `swift` is on.
  - `swift::selfcheck::ensure()` (via `Once`) before the first spawn; panics `"async-swift-rs: Swift runtime ABI self-check failed: <detail>"`.
  - `tests/support/probe.swift`: copy of `spikes/m0/probe.swift` (C names `probe_*` as there, plus the `AsyncSwiftClosure` recipe).

- [ ] **Step 1: Failing tests** in `tests/swift_task.rs` (`required-features = ["test-probe"]`, harness=false, via `tests/common::run`):
  - `task_is_swift_task`: inside `spawn`, `probe_has_task()` true and `swift::current_task().is_some()` (temporary internal accessor until Task 6 publishes it).
  - `utility_priority_seen_by_swift`: `spawn_with(Utility)` → `probe_priority() == 0x11`; Swift `Task {}` spawned inside reports `0x11`.
  - `spawn_returns_value`, `panic_becomes_join_error`, `dropped_handle_detaches` (same assertions as Task 2, Swift backend).
  - `wake_during_poll_repolls`: future that wakes its own waker then returns Pending once, then Ready → completes (1s timeout).
  - `tests/abi_equivalence.rs` `runtime_calls_lower_identically`: for each function declared in `src/swift/rt.rs`, generate a C caller with `swiftcall` and with the C convention, compile both with `clang -O2 -arch {arm64,x86_64}`, disassemble, normalize branch targets, assert equal (bool-returning ones: assert only the documented `and #1` difference). Function list kept in one table shared with `rt.rs` (e.g. `src/swift/rt_sigs.txt` included by both).
- [ ] **Step 2:** `cargo test --features test-probe --test swift_task --test abi_equivalence` → FAIL.
- [ ] **Step 3:** Implement. `funclets.c` from `spikes/m0/shim.c`, minus every runtime call that Rust now makes (task creation moves to Rust: `spawn` builds flags/options and calls `swift_task_create_common` with `rs_entry_fn()` and `rs_context_size()`). Keep the state machine from the spike (or `async-task` if it fits, spec §7.2). `catch_unwind` inside `rs_poll`. `build.rs`: `cc` `-O2` for `csrc/funclets.c`; link `swiftCore`, `swift_Concurrency`, `CoreFoundation`, rpath `/usr/lib/swift`; probe via `swiftc -parse-as-library -static -emit-library -swift-version 5` when `CARGO_FEATURE_TEST_PROBE`.
- [ ] **Step 4:** Verify tail calls: `objdump -d` of the built `funclets.o` shows completion, `swift_continuation_await`, `swift_task_switch`, `swift_taskGroup_waitAll` and the closure call as `b`/`br`, not `bl`. Record in a comment in `funclets.c`.
- [ ] **Step 5:** Same test command → PASS.
- [ ] **Step 6:** Commit `feat(swift): tasks are Swift Tasks (create, poll, park, resume, self-check)`.

---

### Task 5: Swift backend — MainActor tasks, timers, inheritance

**Files:**
- Create: `src/swift/timer.rs`
- Modify: `csrc/funclets.c`
- Modify: `src/swift/{mod.rs,task.rs}`, `tests/swift_task.rs`

**Interfaces:**
- Consumes: Task 4.
- Produces: `swift::sleep(Duration) -> impl Future<Output = ()>`; MainActor inheritance in `swift::spawn` (spawned inside a main task → main); `spawn_detached` → `Priority::Default`, no task-local copy, not main. `run_main` (Task 3) switches to `swift::spawn_on_main` when the `swift` feature is on, and to the dispatch main queue otherwise.

- [ ] **Step 1: Failing tests:**
  - `run_main_runs_swift_main_task`: separate `harness=false` binary `tests/swift_run_main.rs` whose `main()` asserts `run_main(async { sleep(5ms).await; probe_assume_main() }) == true`.
  - `main_task_is_main_actor`: `spawn_on_main` → `probe_has_task() && is_main_thread() && probe_assume_main()`, and again after `sleep(10ms)`.
  - `spawn_in_main_inherits_main`: inside a main task, `spawn(async { probe_assume_main() })` → true.
  - `detached_inherits_nothing`: inside a `Utility` task, `spawn_detached` child sees `probe_priority() == 0x15`.
  - `sleeps_overlap`: 10_000 × `sleep(50ms)` → elapsed < 500ms.
- [ ] **Step 2:** Run → FAIL.
- [ ] **Step 3:** Add the Job pieces to `csrc/funclets.c` from `spikes/swift-job/shim.c` (metadata kind `3|0x400|0x100`, `dispatchSwiftObjectType = 1`, JobKind 1, invoke → `swift_job_run` with generic executor; `run_job` → `rs_job_run`). Rust initializes the 64-byte job header (`rs_job_metadata()` from C gives the metadata address) and calls `swift_task_enqueueGlobalWithDelay` itself.
- [ ] **Step 4:** Run → PASS.
- [ ] **Step 5:** Commit `feat(swift): MainActor tasks, sleep, inheritance rules`.

---

### Task 6: Swift backend — cancellation, abort, escalation, current-task API, task-locals, stress

**Files:**
- Create: `src/swift/current.rs`
- Modify: `csrc/task.c`, `src/swift/{task.rs,join.rs,mod.rs}`, `tests/swift_task.rs`

**Interfaces:**
- Consumes: Tasks 4–5.
- Produces: `JoinHandle::cancel(&self)`, `JoinHandle::abort(&self)`, `JoinHandle::escalate(&self, Priority)`; `swift::current_task() -> Option<TaskRef>` (`TaskRef` = non-owning `*mut c_void` newtype); `swift::is_cancelled() -> bool`; `swift::cancelled() -> impl Future<Output = ()>`; `swift::priority() -> Priority`. C: `swift_task_addCancellationHandler` at entry (handler `SWIFT_CC(swift) void(SWIFT_CONTEXT void*)` — stays in C because of the `swift_context` argument), removed in `finish`.

- [ ] **Step 1: Failing tests:**
  - `cancel_wakes_cancelled_future`: task races `cancelled()` vs `sleep(10s)`; `cancel()` after 30ms → finishes < 100ms; `probe_is_cancelled()` true; returns `Ok`.
  - `abort_drops_future`: task holds a guard whose `Drop` sets a flag, sleeps 10s; `abort()` → `Err(JoinError::Aborted)` < 100ms, flag set.
  - `escalation_visible_after_resume`: `Utility` task sleeps 40ms; `escalate(UserInitiated)` at 10ms; after sleep `probe_priority() == 0x19` and `priority() == UserInitiated`.
  - `task_locals_copied`: inside `probe_with_local("hello", cb)`, `cb` spawns one task with default options and one with `copy_task_locals: false`; `probe_read_local` → `"hello"` and `"<none>"`.
  - `late_wake_after_done_is_noop`: task stores its waker in a global and returns; after `JoinHandle` resolves, `waker.wake()` → no crash; run 1000×.
  - `stress_spawn_wake_cancel`: 20_000 tasks, random mix of sleep(0–2ms), self-wake, cancel; all join; repeat binary 20× clean (script step).
- [ ] **Step 2:** Run → FAIL.
- [ ] **Step 3:** Implement. Cancellation handler → Rust sets flag, wakes `cancelled()` waiters on that task (track current task in a thread-local set during poll, as in the spike fix). `abort` = `swift_task_cancel` + set an abort flag; next `rs_poll` drops the future and completes with `Aborted`.
- [ ] **Step 4:** Run → PASS; then `for i in $(seq 20); do cargo test --features test-probe --test swift_task -q || exit 1; done`.
- [ ] **Step 5:** Commit `feat(swift): cancellation, abort, escalation, current-task API`.

---

### Task 7: `SwiftAny` and `SwiftError`

**Files:**
- Create: `src/swift/any.rs`
- Modify: `tests/support/probe.swift`, `tests/swift_call.rs` (create), `src/swift/mod.rs`

**Interfaces:**
- Produces:
  - `pub struct SwiftAny` (owns a 32-byte `Any` existential; `Drop` destroys via the value witness table). `unsafe fn from_raw_initialized(buf: [usize; 4]) -> SwiftAny`; `fn downcast<T: FromSwiftAny>(&self) -> Option<T>`; `FromSwiftAny` for `()`, `bool`, `i32`, `i64`, `u64`, `f64`, `String`, `Option<T: FromSwiftAny>`; `fn type_name(&self) -> String` (`swift_getTypeName`).
  - `pub struct SwiftError` (owns `any Error` box; `Drop` → `swift_errorRelease`); `fn description(&self) -> String` via `swift_dynamicCast` to `NSError`/`CFError` + `CFErrorCopyDescription`; `pub enum CallError { Swift(SwiftError), NoCurrentTask }`.
  - Probe additions: `probe_any_int(out)`, `probe_any_string(out)`, `probe_any_optional_nil(out)`, `probe_any_void(out)`, `probe_make_error(out: UnsafeMutableRawPointer)` writing `POSIXError(.EIO)` as `any Error`.
- [ ] **Step 1: Failing tests** (`tests/swift_call.rs`): `downcast_int` (42 → `Some(42i64)`, `None` as `String`), `downcast_string` (`"héllo👋"`), `downcast_optional_nil` (`Some(None::<i64>)`), `downcast_void`, `type_name_reports_swift_type` (`"Swift.Int"`), `error_description_mentions_eio` (contains `"Input/output error"` or `"EIO"`), `any_drop_releases` (Swift class instance in `Any`; its `deinit` flips a probe flag after drop).
- [ ] **Step 2:** `cargo test --features test-probe --test swift_call` → FAIL.
- [ ] **Step 3:** Implement with `swift_dynamicCast(dest, src, srcType, targetType, flags)` (no take-on-success, conditional). Target metadata symbols: `$sSiN` (Int), `$sSSN` (String), `$sSbN` (Bool), `$sSdN` (Double), `$sytN` (`()`), `$ss6UInt64VN`, `$ss5Int32VN`; `Optional<T>` through the metadata accessor `$sSqMa` (request 0, argument = T's metadata). `String` is read by casting to `NSString` (toll-free `CFString`) and copying with `CFStringGetCString` (UTF-8), since Swift `String` internals are not read directly. Comment each symbol's source.
- [ ] **Step 4:** Run → PASS.
- [ ] **Step 5:** Commit `feat(swift): SwiftAny downcasts and SwiftError descriptions`.

---

### Task 8: `SwiftAsync` — await a Swift async closure in the same Task

**Files:**
- Create: `src/swift/call.rs`
- Modify: `csrc/funclets.c` (call path from `spikes/m0/shim.c`), `src/swift/task.rs`, `tests/swift_call.rs`, `README.md` (recipe section)

**Interfaces:**
- Consumes: `m0-report.md` (representation + recipe), Tasks 4–7.
- Produces: `pub struct SwiftAsync` with `unsafe fn from_raw(closure_box: *mut c_void) -> SwiftAsync` (an owned `AsyncSwiftClosure`, +1; layout per `m0-report.md`), `fn child(self) -> SwiftAsync`, `impl Future for SwiftAsync { Output = Result<SwiftAny, CallError> }`; `Drop` releases the box and, if running as a child, cancels it. `swift::timeout(d: Duration, call: SwiftAsync) -> impl Future<Output = Option<Result<SwiftAny, CallError>>>` (uses `.child()`). `step()` outcomes: Ready / same-Task call / park, per spec §7.6.
- Probe additions (using the recipe from Task 1): `probe_closure_identity`, `probe_closure_sleep_50ms`, `probe_closure_throw_eio`, `probe_closure_void`, `probe_closure_deinit_flag` (captures an object whose `deinit` flips a flag).

- [ ] **Step 1: Failing tests:**
  - `same_task_identity`: identity closure's task == `current_task()` of the awaiting Rust task.
  - `result_and_error`: sleep closure → `"slept"`; EIO closure → `Err(CallError::Swift(e))` with EIO description; void closure → `downcast::<()>() == Some(())`.
  - `cancel_mid_call`: `cancel()` the outer task 10ms into the sleep closure → closure throws; description contains `"CancellationError"`.
  - `no_current_task`: await from `dispatch::block_on` → `Err(CallError::NoCurrentTask)`.
  - `dropped_swift_async_releases_context`: construct `probe_closure_deinit_flag`, drop without await → flag set.
  - `sequential_calls_keep_task`: three awaits in a row, all same task identity.
- [ ] **Step 2:** Run → FAIL.
- [ ] **Step 3:** Implement per `m0-report.md`. A pending `SwiftAsync` registers itself as "the call" on the current RTask during poll; when poll returns Pending with exactly one registered call and no notification, `step()` performs the call in-Task; the resume funclet stores the result into the call slot and re-enters `step()`.
- [ ] **Step 4:** Run → PASS. Add the recipe to README.
- [ ] **Step 5:** Commit `feat(swift): await Swift async closures in the same Task`.

---

### Task 9: Concurrent Swift calls as child Tasks

**Files:**
- Modify: `csrc/call.c`, `src/swift/{call.rs,task.rs}`, `tests/swift_call.rs`

**Interfaces:**
- Consumes: Task 8, `m0-report.md` (child-task flags and parent-wait rule).
- Produces: when more than one call is registered in a poll, a wake happened during the poll, or the call is `.child()`, calls run as child Tasks in a discarding TaskGroup stored in the task context (flags and options per `m0-report.md`); child completion wakes the parent; the parent's completion runs `swift_taskGroup_waitAll` first, so `JoinHandle` resolves after the children.

- [ ] **Step 1: Failing tests:**
  - `join_runs_concurrently`: `join(a, b)` of two 50ms sleep closures → both Ok, elapsed < 90ms.
  - `select_timeout_cancels_child`: `select(sleep_closure_10s.child(), sleep(30ms))` → timeout branch wins < 100ms; the abandoned child finished with `CancellationError` (recorded by the call record).
  - `timeout_helper`: `swift::timeout(30ms, sleep_closure_10s)` → `None` < 100ms.
  - `select_without_child_runs_to_completion`: `select(sleep_closure_200ms, sleep(30ms))` → the Swift call wins, elapsed ≥ 190ms (documents the rule).
  - `child_inherits_locals_and_priority`: inside `probe_with_local("ctx", ..)` + `Utility`, a joined closure reads `"ctx"` and `0x11`.
  - `parent_waits_for_children`: parent future returns while a child is still sleeping 30ms → `JoinHandle` resolves only after the child finished (child sets flag before the join resolves).
- [ ] **Step 2:** Run → FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run → PASS; rerun the 20× stress loop from Task 6.
- [ ] **Step 5:** Commit `feat(swift): concurrent Swift calls run as child Tasks`.

---

### Task 10: README, examples, x86_64 check

**Files:**
- Create: `examples/{dispatch_basic.rs,swift_basic.rs}`, finish `README.md`

- [ ] **Step 1:** README sections: what it is; feature flags; API overview; runloop rule (`CFRunLoopRun`/`NSApplication.run`, never `dispatch_main`) and `run_main`; Swift closure recipe; inheritance table (`spawn` vs `spawn_detached`); ABI sources (`swift-6.4.2-RELEASE` files) and risks from spec §8; tested platforms.
- [ ] **Step 2:** `cargo run --example dispatch_basic --no-default-features --features dispatch` and `cargo run --example swift_basic` → both print their results and exit 0.
- [ ] **Step 3:** `rustup target add x86_64-apple-darwin` (ask the user first: downloads a toolchain component) then `cargo test --target x86_64-apple-darwin --features test-probe --test swift_task` under Rosetta → PASS for `task_is_swift_task`, `utility_priority_seen_by_swift`, `main_task_is_main_actor`, `cancel_wakes_cancelled_future`, `escalation_visible_after_resume`, `task_locals_copied`.
- [ ] **Step 4:** Commit `docs: README, examples`.
