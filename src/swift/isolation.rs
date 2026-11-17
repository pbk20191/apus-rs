//! The isolation a Swift `isolated (any Actor)?` parameter receives (`#isolation`).
use super::func::{ArgLowering, SwiftArg};

/// The isolation of wherever the call actually runs: the main actor when it runs in a main task (`spawn_on_main`,
/// `run_main`), nil otherwise, including in a child Task (`.child()`, `join!`), like Swift's `async let`. It is decided
/// when the call runs, so it never claims an isolation the callee is not on, and it can name no other actor.
#[derive(Clone, Copy, Debug, Default)]
pub struct Isolation(());

impl Isolation {
    /// The caller's isolation, as `#isolation` gives it in Swift.
    pub fn current() -> Isolation {
        Isolation(())
    }
}

/// Two integer registers, filled when the call runs.
impl SwiftArg for Isolation {
    fn lower(&self, into: &mut ArgLowering) {
        into.isolation();
    }
}
