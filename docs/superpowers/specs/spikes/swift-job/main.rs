//! Rust futures as Swift Concurrency Jobs: global pool, MainActor, and awaiting Swift async APIs.
use std::ffi::c_void;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, atomic::{AtomicU8, Ordering::*}};
use std::task::{Context, Poll, Wake, Waker};
use std::time::{Duration, Instant};

#[repr(C, align(16))] struct SwiftJob([u8; 64]);
#[repr(C)] struct RJob { job: SwiftJob, kind: Kind }
enum Kind { Poll(Arc<Task>), Wake(Waker) }
extern "C" {
    fn rs_job_init(j: *mut SwiftJob, priority: u8);
    fn rs_enqueue(j: *mut SwiftJob);
    fn rs_enqueue_main(j: *mut SwiftJob);
    fn rs_enqueue_after(j: *mut SwiftJob, ns: u64);
    fn dispatch_main() -> !;
    fn CFRunLoopRun();
    fn pthread_main_np() -> i32;
    fn qos_class_self() -> u32;
    fn swift_sleep_async(ms: i64, ctx: *const c_void, done: extern "C" fn(*const c_void, i32, u8)) -> *mut c_void;
    fn swift_handle_cancel(h: *mut c_void);
    fn swift_handle_release(h: *mut c_void);
    fn swift_assume_main_actor() -> bool;
}
fn new_job(prio: u8, kind: Kind) -> *mut SwiftJob {
    let j = Box::into_raw(Box::new(RJob { job: SwiftJob([0; 64]), kind }));
    unsafe { rs_job_init(&mut (*j).job, prio) }; j.cast()
}
#[no_mangle] extern "C" fn rs_job_run(j: *mut c_void) {
    match unsafe { Box::from_raw(j as *mut RJob) }.kind { Kind::Poll(t) => run(t), Kind::Wake(w) => w.wake() }
}

// ---- executor: a task remembers where it lives (global pool at a priority, or MainActor) ----
#[derive(Clone, Copy)] enum Target { Global(u8), Main }
const IDLE: u8 = 0; const SCHEDULED: u8 = 1; const RUNNING: u8 = 2; const NOTIFIED: u8 = 3; const DONE: u8 = 4;
struct Task { fut: Mutex<Option<Pin<Box<dyn Future<Output = ()> + Send>>>>, state: AtomicU8, target: Target }
fn run(task: Arc<Task>) {
    loop {
        task.state.store(RUNNING, SeqCst);
        let waker = Waker::from(task.clone());
        let mut slot = task.fut.lock().unwrap();
        if slot.as_mut().map_or(true, |f| f.as_mut().poll(&mut Context::from_waker(&waker)).is_ready()) {
            *slot = None; task.state.store(DONE, SeqCst); return;
        }
        drop(slot);
        if task.state.compare_exchange(RUNNING, IDLE, SeqCst, SeqCst).is_ok() { return; }
    }
}
impl Wake for Task {
    fn wake(self: Arc<Self>) {
        loop { match self.state.load(SeqCst) {
            IDLE => if self.state.compare_exchange(IDLE, SCHEDULED, SeqCst, SeqCst).is_ok() {
                match self.target {
                    Target::Global(p) => unsafe { rs_enqueue(new_job(p, Kind::Poll(self))) },
                    Target::Main => unsafe { rs_enqueue_main(new_job(0x19, Kind::Poll(self))) },
                }
                return; },
            RUNNING => if self.state.compare_exchange(RUNNING, NOTIFIED, SeqCst, SeqCst).is_ok() { return; },
            _ => return,
        } }
    }
}
struct Join<T>(Arc<Mutex<(Option<T>, Option<Waker>)>>);
impl<T> Future for Join<T> {
    type Output = T;
    fn poll(self: Pin<&mut Self>, cx: &mut Context) -> Poll<T> {
        let mut g = self.0.lock().unwrap();
        match g.0.take() { Some(v) => Poll::Ready(v), None => { g.1 = Some(cx.waker().clone()); Poll::Pending } }
    }
}
fn spawn_on<T: Send + 'static>(target: Target, f: impl Future<Output = T> + Send + 'static) -> Join<T> {
    let inner = Arc::new(Mutex::new((None::<T>, None::<Waker>))); let out = inner.clone();
    let fut = async move { let v = f.await; let mut g = out.lock().unwrap(); g.0 = Some(v); if let Some(w) = g.1.take() { w.wake() } };
    Arc::new(Task { fut: Mutex::new(Some(Box::pin(fut))), state: AtomicU8::new(IDLE), target }).wake();
    Join(inner)
}
const USER_INITIATED: u8 = 0x19; const UTILITY: u8 = 0x11;

// ---- timer on swift_task_enqueueGlobalWithDelay ----
struct Sleep { at: Instant, armed: bool }
fn sleep(d: Duration) -> Sleep { Sleep { at: Instant::now() + d, armed: false } }
impl Future for Sleep {
    type Output = ();
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<()> {
        let left = self.at.saturating_duration_since(Instant::now());
        if left.is_zero() { return Poll::Ready(()) }
        if !self.armed { self.armed = true;
            unsafe { rs_enqueue_after(new_job(0x15, Kind::Wake(cx.waker().clone())), left.as_nanos() as u64) } }
        Poll::Pending
    }
}
/// Resolves to Some(v) or None after `d`; dropping `f` on timeout is how cancellation propagates.
async fn timeout<T>(d: Duration, f: impl Future<Output = T>) -> Option<T> {
    let mut f = std::pin::pin!(f); let mut s = std::pin::pin!(sleep(d));
    std::future::poll_fn(|cx| {
        if let Poll::Ready(v) = f.as_mut().poll(cx) { return Poll::Ready(Some(v)) }
        if s.as_mut().poll(cx).is_ready() { return Poll::Ready(None) }
        Poll::Pending
    }).await
}

// ---- awaiting a Swift async API: Swift Task + callback → waker; drop → Task.cancel() ----
#[derive(Debug)] struct SwiftResult { cancelled: bool, swift_priority: u8 }
struct Shared { result: Mutex<(Option<SwiftResult>, Option<Waker>)>, started: Instant, label: &'static str }
extern "C" fn on_done(ctx: *const c_void, cancelled: i32, prio: u8) {
    let sh = unsafe { Arc::from_raw(ctx as *const Shared) };
    println!("    [swift side] {} finished after {:?}: cancelled={}", sh.label, sh.started.elapsed(), cancelled != 0);
    let mut g = sh.result.lock().unwrap();
    g.0 = Some(SwiftResult { cancelled: cancelled != 0, swift_priority: prio });
    if let Some(w) = g.1.take() { w.wake() }
}
struct SwiftSleep { shared: Arc<Shared>, handle: *mut c_void, finished: bool }
unsafe impl Send for SwiftSleep {}
fn swift_sleep(label: &'static str, ms: i64) -> SwiftSleep {
    let shared = Arc::new(Shared { result: Mutex::new((None, None)), started: Instant::now(), label });
    let handle = unsafe { swift_sleep_async(ms, Arc::into_raw(shared.clone()).cast(), on_done) };
    SwiftSleep { shared, handle, finished: false }
}
impl Future for SwiftSleep {
    type Output = SwiftResult;
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<SwiftResult> {
        let mut g = self.shared.result.lock().unwrap();
        match g.0.take() {
            Some(r) => { drop(g); self.finished = true; Poll::Ready(r) }
            None => { g.1 = Some(cx.waker().clone()); Poll::Pending }
        }
    }
}
impl Drop for SwiftSleep {
    fn drop(&mut self) {
        unsafe { if !self.finished { swift_handle_cancel(self.handle) } swift_handle_release(self.handle) }
    }
}

fn main() {
    let t0 = Instant::now();
    spawn_on(Target::Main, async move {
        println!("1) Rust task on MainActor: main thread={}, MainActor.assumeIsolated ok={}",
                 unsafe { pthread_main_np() } == 1, unsafe { swift_assume_main_actor() });

        let r = swift_sleep("sleep 20ms (from main)", 20).await;
        println!("2) after awaiting Swift on main → back on main thread={}, assumeIsolated(MainActor) passed, on main thread={}  {:?}",
                 unsafe { pthread_main_np() } == 1, unsafe { swift_assume_main_actor() }, r);

        for (name, p) in [("user-initiated", USER_INITIATED), ("utility", UTILITY)] {
            let (qos, r) = spawn_on(Target::Global(p), async { (unsafe { qos_class_self() }, swift_sleep("sleep 5ms", 5).await) }).await;
            println!("3) Rust {name} task (thread qos 0x{qos:x}) → Swift Task.currentPriority = 0x{:x}", r.swift_priority);
        }

        let t = Instant::now();
        let r = spawn_on(Target::Global(USER_INITIATED), timeout(Duration::from_millis(50), swift_sleep("sleep 10s", 10_000))).await;
        println!("4) Rust timeout(50ms) around Swift sleep(10s): result={r:?}, Rust gave up after {:?}", t.elapsed());
        sleep(Duration::from_millis(30)).await; // let the Swift side report its cancellation
        println!("done in {:?}", t0.elapsed());
        std::process::exit(0);
    });
    if std::env::args().any(|a| a == "--dispatch-main") { unsafe { dispatch_main() } }
    unsafe { CFRunLoopRun() } // like NSApp.run: drains the main queue on the real main thread
}
