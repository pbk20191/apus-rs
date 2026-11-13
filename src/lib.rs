//! Run Rust futures as Swift Concurrency Tasks, or on libdispatch.
//!
//! The main queue must be drained by `CFRunLoopRun` or `NSApplication.run`, never by `dispatch_main`:
//! `dispatch_main` drains it on a worker thread, which breaks AppKit and MainActor code.

mod common;
#[cfg(feature = "dispatch")]
pub mod dispatch;

pub use common::{JoinError, Priority, SpawnOptions};
