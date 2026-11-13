//! Spike: Rust futures that *are* Swift Tasks (created via swift_task_create_common, no Swift source).
use std::ffi::{c_char, c_void, CStr};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, atomic::{AtomicBool, AtomicPtr, AtomicU8, Ordering::*}};
use std::task::{Context, Poll, Wake, Waker};
use std::time::{Duration, Instant};

extern "C" {
    fn rs_task_create(rust: *const c_void, priority: u8, main: bool, copy_task_locals: bool) -> *mut c_void;
    fn rs_continuation_resume(t: *mut c_void);
    fn rs_current_task() -> *mut c_void;
    fn rs_task_is_cancelled(t: *mut c_void) -> bool;
    fn rs_task_priority(t: *mut c_void) -> u8;
    fn rs_task_escalate(t: *mut c_void, p: u8);
    fn rs_task_cancel(t: *mut c_void);
    fn swift_release(o: *mut c_void);
    fn rs_job_init(j: *mut c_void, priority: u8);
    fn rs_enqueue_after(j: *mut c_void, ns: u64);
    fn CFRunLoopRun();
    fn pthread_main_np() -> i32;
    // Swift probes (test only)
    fn probe_has_task() -> bool;
    fn probe_is_cancelled() -> bool;
    fn probe_priority() -> u8;
    fn probe_assume_main() -> bool;
    fn probe_with_local(v: *const c_char, body: extern "C" fn());
    fn probe_read_local(buf: *mut c_char, n: usize);
    fn probe_spawn_swift_task(ctx: *const c_void, cb: extern "C" fn(*const c_void, u8));
}

// ---------------- task state machine ----------------
const IDLE: u8 = 0; const RUNNING: u8 = 1; const NOTIFIED: u8 = 2; const PARKED: u8 = 3; const DONE: u8 = 4;
struct RTask {
    fut: Mutex<Option<Pin<Box<dyn Future<Output = ()> + Send>>>>,
    state: AtomicU8,
    continuation: AtomicPtr<c_void>,
    main: bool,
    cancelled: AtomicBool,
    cancel_waker: Mutex<Option<Waker>>,
}
impl Wake for RTask {
    fn wake(self: Arc<Self>) { self.wake_by_ref() }
    fn wake_by_ref(self: &Arc<Self>) {
        loop { match self.state.load(SeqCst) {
            PARKED => if self.state.compare_exchange(PARKED, IDLE, SeqCst, SeqCst).is_ok() {
                unsafe { rs_continuation_resume(self.continuation.swap(std::ptr::null_mut(), SeqCst)) }; return; },
            RUNNING => if self.state.compare_exchange(RUNNING, NOTIFIED, SeqCst, SeqCst).is_ok() { return },
            _ => return,
        } }
    }
}
unsafe fn rt<'a>(p: *mut c_void) -> &'a RTask { &*(p as *const RTask) }
#[no_mangle] extern "C" fn rs_poll(p: *mut c_void) -> i32 {
    let t = unsafe { rt(p) };
    t.state.store(RUNNING, SeqCst);
    let prev = CURRENT.replace(p as *const RTask);
    let waker = unsafe { Arc::increment_strong_count(p as *const RTask); Waker::from(Arc::from_raw(p as *const RTask)) };
    let mut slot = t.fut.lock().unwrap();
    let ready = slot.as_mut().map_or(true, |f| f.as_mut().poll(&mut Context::from_waker(&waker)).is_ready());
    if ready { *slot = None; t.state.store(DONE, SeqCst); }
    CURRENT.set(prev);
    ready as i32
}
#[no_mangle] extern "C" fn rs_park(p: *mut c_void, cont: *mut c_void) -> i32 {
    let t = unsafe { rt(p) };
    t.continuation.store(cont, SeqCst);
    if t.state.compare_exchange(RUNNING, PARKED, SeqCst, SeqCst).is_ok() { return 1 }
    t.continuation.store(std::ptr::null_mut(), SeqCst); // NOTIFIED during poll
    t.state.store(IDLE, SeqCst);
    0
}
#[no_mangle] extern "C" fn rs_finish(p: *mut c_void) { drop(unsafe { Arc::from_raw(p as *const RTask) }) }
#[no_mangle] extern "C" fn rs_is_main(p: *mut c_void) -> bool { unsafe { rt(p) }.main }
#[no_mangle] extern "C" fn rs_cancelled(p: *mut c_void) {
    let t = unsafe { rt(p) };
    t.cancelled.store(true, SeqCst);
    if let Some(w) = t.cancel_waker.lock().unwrap().take() { w.wake() }
}

// ---------------- spawn / JoinHandle ----------------
struct JoinHandle<T> { out: Arc<Mutex<(Option<T>, Option<Waker>)>>, task: *mut c_void }
unsafe impl<T: Send> Send for JoinHandle<T> {}
impl<T> JoinHandle<T> {
    fn cancel(&self) { unsafe { rs_task_cancel(self.task) } }            // == Swift's task.cancel()
    fn escalate(&self, p: u8) { unsafe { rs_task_escalate(self.task, p) } }
}
impl<T> Drop for JoinHandle<T> { fn drop(&mut self) { unsafe { swift_release(self.task) } } }
impl<T> Future for JoinHandle<T> {
    type Output = T;
    fn poll(self: Pin<&mut Self>, cx: &mut Context) -> Poll<T> {
        let mut g = self.out.lock().unwrap();
        match g.0.take() { Some(v) => Poll::Ready(v), None => { g.1 = Some(cx.waker().clone()); Poll::Pending } }
    }
}
struct Opts { priority: u8, main: bool, copy_locals: bool }
fn spawn_with<T: Send + 'static>(o: Opts, f: impl Future<Output = T> + Send + 'static) -> JoinHandle<T> {
    let out = Arc::new(Mutex::new((None::<T>, None::<Waker>))); let o2 = out.clone();
    let fut = async move { let v = f.await; let mut g = o2.lock().unwrap(); g.0 = Some(v); if let Some(w) = g.1.take() { w.wake() } };
    let t = Arc::new(RTask { fut: Mutex::new(Some(Box::pin(fut))), state: AtomicU8::new(IDLE), continuation: AtomicPtr::default(),
                             main: o.main, cancelled: AtomicBool::new(false), cancel_waker: Mutex::new(None) });
    let task = unsafe { rs_task_create(Arc::into_raw(t).cast(), o.priority, o.main, o.copy_locals) };
    JoinHandle { out, task }
}
const UNSPECIFIED: u8 = 0; const USER_INITIATED: u8 = 0x19; const UTILITY: u8 = 0x11;
fn spawn<T: Send + 'static>(p: u8, f: impl Future<Output = T> + Send + 'static) -> JoinHandle<T> {
    spawn_with(Opts { priority: p, main: false, copy_locals: true }, f)
}

// ---------------- current-task helpers ----------------
fn current() -> *mut c_void { unsafe { rs_current_task() } }
fn current_priority() -> u8 { unsafe { rs_task_priority(current()) } }
thread_local! { static CURRENT: std::cell::Cell<*const RTask> = const { std::cell::Cell::new(std::ptr::null()) }; }
/// Resolves when the Swift task we run in is cancelled; woken by the runtime's cancellation handler.
fn cancelled() -> impl Future<Output = ()> {
    std::future::poll_fn(|cx| {
        let t = unsafe { &*CURRENT.get() };
        if t.cancelled.load(SeqCst) { return Poll::Ready(()) }
        *t.cancel_waker.lock().unwrap() = Some(cx.waker().clone());
        if t.cancelled.load(SeqCst) { Poll::Ready(()) } else { Poll::Pending }
    })
}

// ---------------- timer: delayed Swift Job that wakes a waker ----------------
#[repr(C, align(16))] struct WakeJob { job: [u8; 64], waker: Waker }
#[no_mangle] extern "C" fn rs_job_run(j: *mut c_void) { unsafe { Box::from_raw(j as *mut WakeJob) }.waker.wake() }
fn sleep(d: Duration) -> impl Future<Output = ()> {
    let at = Instant::now() + d; let mut armed = false;
    std::future::poll_fn(move |cx| {
        let left = at.saturating_duration_since(Instant::now());
        if left.is_zero() { return Poll::Ready(()) }
        if !armed { armed = true;
            let j = Box::into_raw(Box::new(WakeJob { job: [0; 64], waker: cx.waker().clone() }));
            unsafe { rs_job_init(j.cast(), 0x15); rs_enqueue_after(j.cast(), left.as_nanos() as u64) } }
        Poll::Pending
    })
}
async fn race<T>(a: impl Future<Output = T>, b: impl Future<Output = T>) -> T {
    let (mut a, mut b) = (std::pin::pin!(a), std::pin::pin!(b));
    std::future::poll_fn(|cx| { if let Poll::Ready(v) = a.as_mut().poll(cx) { return Poll::Ready(v) } b.as_mut().poll(cx) }).await
}
fn read_local() -> String { let mut b = [0 as c_char; 64]; unsafe { probe_read_local(b.as_mut_ptr(), 64); CStr::from_ptr(b.as_ptr()) }.to_string_lossy().into() }

static LOCAL_CHILDREN: Mutex<Vec<JoinHandle<String>>> = Mutex::new(Vec::new());
extern "C" fn spawn_two_children() {
    LOCAL_CHILDREN.lock().unwrap().push(spawn_with(Opts { priority: UNSPECIFIED, main: false, copy_locals: true }, async { read_local() }));
    LOCAL_CHILDREN.lock().unwrap().push(spawn_with(Opts { priority: UNSPECIFIED, main: false, copy_locals: false }, async { read_local() }));
}
extern "C" fn on_swift_child(ctx: *const c_void, prio: u8) {
    let slot = unsafe { &*(ctx as *const Mutex<(Option<u8>, Option<Waker>)>) };
    let mut g = slot.lock().unwrap(); g.0 = Some(prio); if let Some(w) = g.1.take() { w.wake() }
}

fn main() {
    let t0 = Instant::now();
    let _root = spawn_with(Opts { priority: USER_INITIATED, main: true, copy_locals: false }, async move {
        println!("1) MainActor Rust task: Swift sees a Task={} | main thread={} | MainActor.assumeIsolated={}",
                 unsafe { probe_has_task() }, unsafe { pthread_main_np() } == 1, unsafe { probe_assume_main() });
        sleep(Duration::from_millis(10)).await;
        println!("2) after await: Task={} | main thread={} | MainActor.assumeIsolated={}",
                 unsafe { probe_has_task() }, unsafe { pthread_main_np() } == 1, unsafe { probe_assume_main() });

        let (rp, sp, child) = spawn(UTILITY, async {
            let slot = Arc::new(Mutex::new((None::<u8>, None::<Waker>)));
            unsafe { probe_spawn_swift_task(Arc::as_ptr(&slot).cast(), on_swift_child) };
            let child = std::future::poll_fn(|cx| { let mut g = slot.lock().unwrap();
                match g.0 { Some(p) => Poll::Ready(p), None => { g.1 = Some(cx.waker().clone()); Poll::Pending } } }).await;
            (current_priority(), unsafe { probe_priority() }, child)
        }).await;
        println!("3) utility Rust task: runtime priority=0x{rp:x}, Swift Task.currentPriority=0x{sp:x}, Swift `Task {{}}` spawned inside inherits 0x{child:x}");

        let h = spawn(USER_INITIATED, async {
            let t = Instant::now();
            race(async { cancelled().await; "cancel observed" }, async { sleep(Duration::from_secs(10)).await; "slept 10s" }).await;
            (unsafe { probe_is_cancelled() }, t.elapsed())
        });
        sleep(Duration::from_millis(30)).await;
        h.cancel();
        let (swift_cancelled, after) = h.await;
        println!("4) swift_task_cancel(task) after 30ms → Rust future woke after {after:?}, Swift Task.isCancelled={swift_cancelled}");

        let h = spawn(UTILITY, async {
            let before = unsafe { probe_priority() };
            sleep(Duration::from_millis(40)).await;
            (before, unsafe { probe_priority() }, current_priority())
        });
        sleep(Duration::from_millis(10)).await;
        h.escalate(USER_INITIATED);
        let (before, after, rt_after) = h.await;
        println!("5) escalate(utility→user-initiated) while suspended: Task.currentPriority before=0x{before:x} after=0x{after:x} (runtime 0x{rt_after:x})");

        unsafe { probe_with_local(c"hello from Swift @TaskLocal".as_ptr(), spawn_two_children) };
        let hs: Vec<_> = LOCAL_CHILDREN.lock().unwrap().drain(..).collect();
        let mut vals = Vec::new(); for h in hs { vals.push(h.await) }
        println!("6) Rust tasks spawned inside Swift `$name.withValue(..)`: copyTaskLocals=true → {:?}, false → {:?}", vals[0], vals[1]);

        println!("done in {:?}", t0.elapsed());
        std::process::exit(0);
    });
    unsafe { CFRunLoopRun() }
}
