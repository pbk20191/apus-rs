//! A Rust future running as a genuine Swift Task.
//!
//! The funclets in csrc/funclets.c call `rs_poll` from inside the Task. When the future is Pending they
//! initialize a continuation and call `rs_park`; the Rust waker resumes that continuation. States:
//! IDLE (not being polled, not parked) → RUNNING (in rs_poll) → PARKED (continuation stored) → IDLE on wake.
//! A wake during RUNNING sets NOTIFIED, and rs_park then declines so the funclet resumes itself and re-polls.

use super::join::{JoinHandle, JoinState};
use super::{rt, selfcheck};
use crate::common::guard;
use crate::SpawnOptions;
use std::cell::Cell;
use std::ffi::c_void;
use std::future::Future;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU8, Ordering::SeqCst};
use std::sync::{Arc, Mutex};
use std::task::{Context, Wake, Waker};

const IDLE: u8 = 0;
const RUNNING: u8 = 1;
const NOTIFIED: u8 = 2;
const PARKED: u8 = 3;
const DONE: u8 = 4;

type BoxFuture = Pin<Box<dyn Future<Output = ()> + Send>>;

pub(crate) struct Task {
    future: Mutex<Option<BoxFuture>>,
    state: AtomicU8,
    continuation: AtomicPtr<c_void>,
    pub(crate) main: bool,
    pub(crate) cancelled: AtomicBool,
    pub(crate) cancel_wakers: Mutex<Vec<Waker>>,
    on_finish: Mutex<Option<Box<dyn FnOnce() + Send>>>,
    /// Swift calls registered by the current poll, not started yet (call.rs).
    pub(crate) calls: Mutex<Vec<Arc<super::call::CallShared>>>,
}

thread_local! {
    /// The task being polled on this thread (for current-task APIs that need our state, not just the runtime's).
    static CURRENT: Cell<*const Task> = const { Cell::new(std::ptr::null()) };
}

/// Whether a wake arrived while the current poll was running.
pub(crate) fn notified(t: &Task) -> bool {
    t.state.load(SeqCst) == NOTIFIED
}

/// The task being polled on this thread, if any.
pub(crate) fn current() -> Option<&'static Task> {
    let p = CURRENT.get();
    (!p.is_null()).then(|| unsafe { &*p })
}

impl Wake for Task {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref()
    }

    fn wake_by_ref(self: &Arc<Self>) {
        loop {
            match self.state.load(SeqCst) {
                PARKED => {
                    if self.state.compare_exchange(PARKED, IDLE, SeqCst, SeqCst).is_ok() {
                        let continuation = self.continuation.swap(std::ptr::null_mut(), SeqCst);
                        unsafe { rt::swift_continuation_resume(continuation) };
                        return;
                    }
                }
                RUNNING => {
                    if self.state.compare_exchange(RUNNING, NOTIFIED, SeqCst, SeqCst).is_ok() {
                        return;
                    }
                }
                _ => return, // IDLE (a poll is coming), NOTIFIED, DONE
            }
        }
    }
}

unsafe fn task<'a>(rust: *mut c_void) -> &'a Task {
    &*(rust as *const Task)
}

#[no_mangle]
extern "C" fn rs_poll(rust: *mut c_void) -> i32 {
    let t = unsafe { task(rust) };
    t.state.store(RUNNING, SeqCst);
    let waker = unsafe {
        Arc::increment_strong_count(rust as *const Task);
        Waker::from(Arc::from_raw(rust as *const Task))
    };
    let previous = CURRENT.replace(t);
    let mut slot = t.future.lock().unwrap();
    // The future is `Guarded`, which already turns panics into JoinError; this catch only keeps a panic
    // in that machinery from unwinding into the funclet.
    let ready = catch_unwind(AssertUnwindSafe(|| {
        slot.as_mut().map_or(true, |f| f.as_mut().poll(&mut Context::from_waker(&waker)).is_ready())
    }))
    .unwrap_or(true);
    if ready {
        *slot = None;
        t.state.store(DONE, SeqCst);
    }
    drop(slot);
    CURRENT.set(previous);
    ready as i32
}

#[no_mangle]
extern "C" fn rs_park(rust: *mut c_void, continuation: *mut c_void) -> i32 {
    let t = unsafe { task(rust) };
    t.continuation.store(continuation, SeqCst);
    if t.state.compare_exchange(RUNNING, PARKED, SeqCst, SeqCst).is_ok() {
        return 1;
    }
    t.continuation.store(std::ptr::null_mut(), SeqCst); // NOTIFIED during poll
    t.state.store(IDLE, SeqCst);
    0
}

#[no_mangle]
extern "C" fn rs_finish(rust: *mut c_void) {
    let t = unsafe { Arc::from_raw(rust as *const Task) };
    let on_finish = t.on_finish.lock().unwrap().take();
    if let Some(f) = on_finish {
        f();
    }
}

#[no_mangle]
extern "C" fn rs_is_main(rust: *mut c_void) -> bool {
    unsafe { task(rust) }.main
}

#[no_mangle]
extern "C" fn rs_cancelled(rust: *mut c_void) {
    let t = unsafe { task(rust) };
    t.cancelled.store(true, SeqCst);
    let wakers = std::mem::take(&mut *t.cancel_wakers.lock().unwrap());
    wakers.into_iter().for_each(Waker::wake);
}

/// Creates the Swift Task. `on_finish` runs when the Task completes (after any children).
pub(crate) fn create(options: SpawnOptions, future: BoxFuture, on_finish: Box<dyn FnOnce() + Send>) -> *mut c_void {
    let task = Arc::new(Task {
        future: Mutex::new(Some(future)),
        state: AtomicU8::new(IDLE),
        continuation: AtomicPtr::default(),
        main: options.main,
        cancelled: AtomicBool::new(false),
        cancel_wakers: Mutex::new(Vec::new()),
        on_finish: Mutex::new(Some(on_finish)),
        calls: Mutex::new(Vec::new()),
    });
    let mut flags = options.priority.raw() as usize | rt::CREATE_ENQUEUE_JOB | rt::CREATE_IS_TASK_FUNCTION_CONSUMED;
    if options.copy_task_locals {
        flags |= rt::CREATE_COPY_TASK_LOCALS;
    }
    unsafe {
        let mut main_option = rt::InitialSerialExecutorOption {
            flags: rt::OPTION_KIND_INITIAL_SERIAL_EXECUTOR,
            parent: std::ptr::null_mut(),
            executor: rt::swift_task_getMainExecutor(),
        };
        let options_ptr = if options.main { (&mut main_option as *mut rt::InitialSerialExecutorOption).cast() } else { std::ptr::null_mut() };
        rt::swift_task_create_common(
            flags,
            options_ptr,
            rt::empty_tuple_metadata(),
            rt::rs_entry_fn(),
            Arc::into_raw(task) as *mut c_void,
            rt::rs_context_size(),
        )
        .task
    }
}

/// `inherit`: like Swift's `Task {}`, a task spawned from a MainActor task stays on the MainActor
/// (priority and task-locals are inherited by the runtime itself). `Task.detached` passes false.
pub(crate) fn spawn_with<F>(mut options: SpawnOptions, inherit: bool, future: F) -> JoinHandle<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    selfcheck::ensure();
    if inherit && current().is_some_and(|t| t.main) {
        options.main = true;
    }
    let (guarded, abort) = guard(future);
    let state = JoinState::new();
    let staged = Arc::new(Mutex::new(None));
    let staged_in = staged.clone();
    let future: BoxFuture = Box::pin(async move {
        let result = guarded.await;
        *staged_in.lock().unwrap() = Some(result);
    });
    let finished = state.clone();
    let on_finish = Box::new(move || {
        if let Some(result) = staged.lock().unwrap().take() {
            finished.complete(result);
        }
    });
    let task = create(options, future, on_finish);
    JoinHandle { state, task, abort }
}
