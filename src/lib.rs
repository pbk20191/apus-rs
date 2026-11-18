//! Run Rust futures as Swift Concurrency Tasks, or on libdispatch.
//!
//! The main queue must be drained by `CFRunLoopRun` or `NSApplication.run`, never by `dispatch_main`:
//! `dispatch_main` drains it on a worker thread, which breaks AppKit and MainActor code.

mod common;
#[cfg(feature = "dispatch")]
pub mod dispatch;
mod runloop;
#[cfg(feature = "swift")]
pub mod swift;

#[cfg(feature = "swift")]
pub use apus_macros::{mangle, result_type};
#[cfg(feature = "swift")]
#[doc(hidden)]
pub use apus_macros::{__call_symbol, __consumes_self, __opaque_descriptor, __swift_route};
pub use common::{JoinError, Priority, SpawnOptions};
pub use runloop::{is_main_thread, run_main};
