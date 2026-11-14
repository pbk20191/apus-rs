//! Can a tokio current_thread runtime live as a guest on the main run loop?
//! The run loop owns the thread; a run loop timer gives tokio short non-blocking turns.
use objc2_core_foundation::{kCFRunLoopCommonModes, CFAbsoluteTimeGetCurrent, CFRunLoop, CFRunLoopTimer};
use std::cell::RefCell;
use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
use std::time::{Duration, Instant};

thread_local! { static RT: RefCell<Option<tokio::runtime::Runtime>> = const { RefCell::new(None) }; }
static TURNS: AtomicUsize = AtomicUsize::new(0);
static DONE: AtomicUsize = AtomicUsize::new(0);

/// One non-blocking turn: run ready tasks, fire due timers, return.
fn turn() {
    TURNS.fetch_add(1, SeqCst);
    RT.with(|rt| rt.borrow().as_ref().unwrap().block_on(tokio::task::yield_now()));
}
extern "C-unwind" fn tick(_timer: *mut CFRunLoopTimer, _info: *mut std::ffi::c_void) {
    turn();
    if DONE.load(SeqCst) == 3 { CFRunLoop::main().unwrap().stop() }
}

fn main() {
    let rt = tokio::runtime::Builder::new_current_thread().enable_time().build().unwrap();
    let start = Instant::now();
    let main_thread = std::thread::current().id();
    {
        let _guard = rt.enter();
        // 1. a spawned task with a tokio timer
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(100)).await;
            println!("1 tokio::spawn + sleep(100ms) finished at {:?}, on the main thread: {}", start.elapsed(), std::thread::current().id() == main_thread);
            DONE.fetch_add(1, SeqCst);
        });
        // 2. timeout + channel + nested spawn + select, the things rmcp's service loop uses
        tokio::spawn(async move {
            let (tx, mut rx) = tokio::sync::mpsc::channel::<u32>(4);
            tokio::spawn(async move { for i in 0..3 { tokio::time::sleep(Duration::from_millis(20)).await; tx.send(i).await.unwrap(); } });
            let mut got = vec![];
            loop {
                tokio::select! {
                    v = rx.recv() => match v { Some(v) => got.push(v), None => break },
                    _ = tokio::time::sleep(Duration::from_secs(5)) => break,
                }
            }
            let timed_out = tokio::time::timeout(Duration::from_millis(30), std::future::pending::<()>()).await.is_err();
            println!("2 mpsc + select + nested spawn got {got:?}; timeout fired: {timed_out}; at {:?}", start.elapsed());
            DONE.fetch_add(1, SeqCst);
        });
        // 3. a wake from another thread (what a Network.framework callback on a dispatch queue would do)
        let (tx, rx) = tokio::sync::oneshot::channel::<&str>();
        std::thread::spawn(move || { std::thread::sleep(Duration::from_millis(50)); tx.send("from another thread").unwrap(); });
        tokio::spawn(async move {
            println!("3 oneshot {:?} at {:?}", rx.await.unwrap(), start.elapsed());
            DONE.fetch_add(1, SeqCst);
        });
    }
    RT.with(|slot| *slot.borrow_mut() = Some(rt));
    let timer = unsafe { CFRunLoopTimer::new(None, CFAbsoluteTimeGetCurrent(), 0.005, 0, 0, Some(tick), std::ptr::null_mut()) }.unwrap();
    CFRunLoop::main().unwrap().add_timer(Some(&timer), unsafe { kCFRunLoopCommonModes });
    CFRunLoop::run();
    println!("run loop stopped after {:?}, {} turns", start.elapsed(), TURNS.load(SeqCst));
}
