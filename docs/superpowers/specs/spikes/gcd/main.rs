//! libdispatch-backed async executor, std only.
use std::ffi::c_void;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, atomic::{AtomicU8, Ordering::*}};
use std::task::{Context, Poll, Wake, Waker};
use std::time::{Duration, Instant};

#[allow(non_camel_case_types)] type dispatch_queue_t = *mut c_void;
extern "C" {
    fn dispatch_get_global_queue(qos: isize, flags: usize) -> dispatch_queue_t;
    fn dispatch_async_f(q: dispatch_queue_t, ctx: *mut c_void, f: extern "C" fn(*mut c_void));
    fn dispatch_after_f(when: u64, q: dispatch_queue_t, ctx: *mut c_void, f: extern "C" fn(*mut c_void));
    fn dispatch_time(base: u64, delta_ns: i64) -> u64;
    fn pthread_mach_thread_np(t: usize) -> u32;
    fn pthread_self() -> usize;
    fn qos_class_self() -> u32;
}
const QOS_USER_INITIATED: isize = 0x19; const QOS_UTILITY: isize = 0x11;

// ---- task: future + scheduling state; the Arc<Task> *is* the waker ----
const IDLE: u8 = 0; const SCHEDULED: u8 = 1; const RUNNING: u8 = 2; const NOTIFIED: u8 = 3; const DONE: u8 = 4;
struct Task { fut: Mutex<Option<Pin<Box<dyn Future<Output = ()> + Send>>>>, state: AtomicU8, q: usize }
unsafe impl Sync for Task {}

extern "C" fn run(ctx: *mut c_void) {
    let task = unsafe { Arc::from_raw(ctx as *const Task) };
    loop {
        task.state.store(RUNNING, SeqCst);
        let waker = Waker::from(task.clone());
        let mut slot = task.fut.lock().unwrap();
        let done = slot.as_mut().map_or(true, |f| f.as_mut().poll(&mut Context::from_waker(&waker)).is_ready());
        if done { *slot = None; task.state.store(DONE, SeqCst); return; }
        drop(slot);
        // woken while running → poll again right here instead of re-enqueueing
        if task.state.compare_exchange(RUNNING, IDLE, SeqCst, SeqCst).is_ok() { return; }
    }
}
impl Wake for Task {
    fn wake(self: Arc<Self>) {
        loop {
            match self.state.load(SeqCst) {
                IDLE => if self.state.compare_exchange(IDLE, SCHEDULED, SeqCst, SeqCst).is_ok() {
                    unsafe { dispatch_async_f(self.q as _, Arc::into_raw(self) as _, run) }; return; },
                RUNNING => if self.state.compare_exchange(RUNNING, NOTIFIED, SeqCst, SeqCst).is_ok() { return; },
                _ => return, // SCHEDULED / NOTIFIED / DONE
            }
        }
    }
}

// ---- spawn + JoinHandle (oneshot via Mutex<Option<T>> + waker) ----
struct Join<T> { inner: Arc<Mutex<(Option<T>, Option<Waker>)>> }
impl<T> Future for Join<T> {
    type Output = T;
    fn poll(self: Pin<&mut Self>, cx: &mut Context) -> Poll<T> {
        let mut g = self.inner.lock().unwrap();
        match g.0.take() { Some(v) => Poll::Ready(v), None => { g.1 = Some(cx.waker().clone()); Poll::Pending } }
    }
}
fn spawn_qos<T: Send + 'static>(qos: isize, f: impl Future<Output = T> + Send + 'static) -> Join<T> {
    let inner = Arc::new(Mutex::new((None::<T>, None::<Waker>)));
    let out = inner.clone();
    let fut = async move { let v = f.await; let mut g = out.lock().unwrap(); g.0 = Some(v); if let Some(w) = g.1.take() { w.wake() } };
    let q = unsafe { dispatch_get_global_queue(qos, 0) } as usize;
    let task = Arc::new(Task { fut: Mutex::new(Some(Box::pin(fut))), state: AtomicU8::new(IDLE), q });
    task.wake();
    Join { inner }
}
fn spawn<T: Send + 'static>(f: impl Future<Output = T> + Send + 'static) -> Join<T> { spawn_qos(QOS_USER_INITIATED, f) }

// ---- timer: dispatch_after_f wakes the waker ----
struct Sleep { at: Instant, armed: bool }
fn sleep(d: Duration) -> Sleep { Sleep { at: Instant::now() + d, armed: false } }
extern "C" fn fire(ctx: *mut c_void) { unsafe { Box::from_raw(ctx as *mut Waker) }.wake() }
impl Future for Sleep {
    type Output = ();
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<()> {
        let left = self.at.saturating_duration_since(Instant::now());
        if left.is_zero() { return Poll::Ready(()) }
        if !self.armed {
            self.armed = true;
            let w = Box::into_raw(Box::new(cx.waker().clone())) as *mut c_void;
            unsafe { dispatch_after_f(dispatch_time(0, left.as_nanos() as i64), dispatch_get_global_queue(0x15, 0), w, fire) }
        }
        Poll::Pending
    }
}

// ---- block_on for main thread (park/unpark) ----
struct Unpark(std::thread::Thread);
impl Wake for Unpark { fn wake(self: Arc<Self>) { self.0.unpark() } }
fn block_on<T>(f: impl Future<Output = T>) -> T {
    let mut f = std::pin::pin!(f);
    let w = Waker::from(Arc::new(Unpark(std::thread::current())));
    loop { if let Poll::Ready(v) = f.as_mut().poll(&mut Context::from_waker(&w)) { return v } std::thread::park() }
}

fn tid() -> u32 { unsafe { pthread_mach_thread_np(pthread_self()) } }
fn qos_name() -> &'static str { match unsafe { qos_class_self() } { 0x19 => "user-initiated", 0x11 => "utility", 0x15 => "default", _ => "?" } }

fn main() {
    let t0 = Instant::now();
    block_on(async {
        // 10k tasks each sleeping 50 ms concurrently → should take ~50 ms, not 500 s
        let hs: Vec<_> = (0..10_000u64).map(|i| spawn(async move { sleep(Duration::from_millis(50)).await; i })).collect();
        let mut sum: u64 = 0; for h in hs { sum += h.await; }
        println!("10k sleeps: sum={sum} in {:?}", t0.elapsed());

        let a = spawn_qos(QOS_USER_INITIATED, async { sleep(Duration::from_millis(1)).await; (tid(), qos_name()) }).await;
        let b = spawn_qos(QOS_UTILITY, async { sleep(Duration::from_millis(1)).await; (tid(), qos_name()) }).await;
        println!("QoS after await: {:?} / {:?}", a, b);
        let mut threads: Vec<u32> = Vec::new();
        for h in (0..64).map(|_| spawn(async { let x = tid(); sleep(Duration::from_millis(5)).await; (x, tid()) })).collect::<Vec<_>>() {
            let (x, y) = h.await; threads.push(x); threads.push(y);
        }
        threads.sort(); threads.dedup();
        println!("64 tasks ran on {} distinct GCD worker threads", threads.len());
    });
}
