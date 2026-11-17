//! Guest mode with no thread of tokio's at all: the LocalSet is polled on the main run loop (wakes are
//! event-driven), and a run loop timer turns tokio's timer wheel, armed only while work is in flight.
use objc2_core_foundation::{kCFRunLoopCommonModes, CFAbsoluteTimeGetCurrent, CFRunLoop, CFRunLoopTimer};
use std::cell::RefCell;
use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
use std::time::{Duration, Instant};

thread_local! {
    static RT: RefCell<Option<tokio::runtime::Runtime>> = const { RefCell::new(None) };
    static TICKER: RefCell<Option<objc2_core_foundation::CFRetained<CFRunLoopTimer>>> = const { RefCell::new(None) };
}
static TURNS: AtomicUsize = AtomicUsize::new(0);
const FAR: f64 = 1e10;

/// Fires due tokio timers and runs anything `tokio::spawn` queued; never blocks.
extern "C-unwind" fn turn(_timer: *mut CFRunLoopTimer, _info: *mut std::ffi::c_void) {
    TURNS.fetch_add(1, SeqCst);
    RT.with(|rt| rt.borrow().as_ref().unwrap().block_on(tokio::task::yield_now()));
}
/// The embedder says when timers may be pending (a request is in flight); otherwise the wheel is left alone.
fn ticking(on: bool) {
    TICKER.with(|t| t.borrow().as_ref().unwrap().set_next_fire_date(if on { CFAbsoluteTimeGetCurrent() } else { FAR }));
}

fn main() {
    let rt = tokio::runtime::Builder::new_current_thread().enable_time().build().unwrap();
    let handle = rt.handle().clone();
    RT.with(|slot| *slot.borrow_mut() = Some(rt));
    let timer = unsafe { CFRunLoopTimer::new(None, FAR, 0.05, 0, 0, Some(turn), std::ptr::null_mut()) }.unwrap();
    unsafe { timer.set_tolerance(0.02) };
    CFRunLoop::main().unwrap().add_timer(Some(&timer), unsafe { kCFRunLoopCommonModes });
    TICKER.with(|t| *t.borrow_mut() = Some(timer));

    let threads_before = thread_count();
    let start = Instant::now();
    let main_thread = std::thread::current().id();
    let on_main = move || std::thread::current().id() == main_thread;
    let _guard = handle.enter();
    let local = tokio::task::LocalSet::new();
    apus::run_main(async move {
        local.run_until(async move {
            ticking(true);
            let a = tokio::task::spawn_local(async move {
                tokio::time::sleep(Duration::from_millis(100)).await;
                println!("1 spawn_local + sleep(100ms) at {:?}, on main: {}", start.elapsed(), on_main());
            });
            let b = tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(10)).await;
                println!("2 tokio::spawn + sleep(10ms) at {:?}, on main: {}", start.elapsed(), on_main());
            });
            let c = tokio::task::spawn_local(async move {
                let (tx, rx) = tokio::sync::oneshot::channel::<u32>();
                std::thread::spawn(move || { std::thread::sleep(Duration::from_millis(30)); tx.send(7).unwrap(); });
                println!("3 wake from another thread {:?} at {:?} (no tick needed)", rx.await.unwrap(), start.elapsed());
            });
            let _ = tokio::join!(a, b, c);
            ticking(false);
            let turns = TURNS.load(SeqCst);
            // Idle: wait 500 ms on something that is not a tokio timer.
            let (tx, rx) = tokio::sync::oneshot::channel::<()>();
            std::thread::spawn(move || { std::thread::sleep(Duration::from_millis(500)); tx.send(()).unwrap(); });
            rx.await.unwrap();
            println!("4 turns: {turns} while working, {} during 500 ms idle", TURNS.load(SeqCst) - turns);
        }).await
    });
    println!("done at {:?}; threads now {} (before {}; 2 std helper threads came and went)", start.elapsed(), thread_count(), threads_before);
}

fn thread_count() -> u32 {
    extern "C" { fn task_threads(task: u32, list: *mut *mut u32, count: *mut u32) -> i32; static mach_task_self_: u32; }
    let (mut list, mut count) = (std::ptr::null_mut(), 0u32);
    unsafe { task_threads(mach_task_self_, &mut list, &mut count) };
    count
}
