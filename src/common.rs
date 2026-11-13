//! Types shared by both backends.

use std::any::Any;
use std::fmt;

/// Task priority. Raw values are QoS classes, which Swift's `JobPriority` also uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Priority {
    UserInteractive,
    UserInitiated,
    Default,
    Utility,
    Background,
    /// Inherit from the current task, or from the current thread's QoS.
    Unspecified,
}

impl Priority {
    pub fn raw(self) -> u8 {
        match self {
            Priority::UserInteractive => 0x21,
            Priority::UserInitiated => 0x19,
            Priority::Default => 0x15,
            Priority::Utility => 0x11,
            Priority::Background => 0x09,
            Priority::Unspecified => 0,
        }
    }

    /// Maps a raw value to the nearest class at or below it.
    pub fn from_raw(raw: u8) -> Priority {
        match raw {
            0 => Priority::Unspecified,
            0x21.. => Priority::UserInteractive,
            0x19.. => Priority::UserInitiated,
            0x15.. => Priority::Default,
            0x11.. => Priority::Utility,
            _ => Priority::Background,
        }
    }
}

/// Why a task produced no value.
pub enum JoinError {
    Aborted,
    Panicked(Box<dyn Any + Send>),
}

impl fmt::Debug for JoinError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            JoinError::Aborted => f.write_str("Aborted"),
            JoinError::Panicked(_) => f.write_str("Panicked(..)"),
        }
    }
}

impl fmt::Display for JoinError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            JoinError::Aborted => f.write_str("task was aborted"),
            JoinError::Panicked(_) => f.write_str("task panicked"),
        }
    }
}

impl std::error::Error for JoinError {}

/// How to spawn a task. `Default` matches Swift's `Task {}`: inherit priority, copy task-locals, not main.
#[derive(Clone, Copy, Debug)]
pub struct SpawnOptions {
    pub priority: Priority,
    pub main: bool,
    pub copy_task_locals: bool,
}

impl Default for SpawnOptions {
    fn default() -> Self {
        SpawnOptions { priority: Priority::Unspecified, main: false, copy_task_locals: true }
    }
}
