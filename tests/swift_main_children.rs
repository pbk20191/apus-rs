//! A MainActor task whose children outlive its future must still finish on the main thread, and `run_main`
//! must return even when that completion arrives through the TaskGroup's wait instead of the main queue.
#[path = "common/probe.rs"]
mod probe;

use apus::swift::{sleep, spawn_on_main, SwiftAsync};
use apus::{is_main_thread, run_main};
use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};
use std::time::Duration;

static ITERATION: AtomicUsize = AtomicUsize::new(0);
static DROPPED_ON: AtomicU8 = AtomicU8::new(0); // 0 = not yet, 1 = main thread, 2 = elsewhere

struct RecordDropThread;
impl Drop for RecordDropThread {
    fn drop(&mut self) {
        DROPPED_ON.store(if is_main_thread() { 1 } else { 2 }, Ordering::SeqCst);
    }
}

async fn race<T>(a: impl std::future::Future<Output = T>, b: impl std::future::Future<Output = T>) -> T {
    let (mut a, mut b) = (std::pin::pin!(a), std::pin::pin!(b));
    std::future::poll_fn(|cx| match a.as_mut().poll(cx) {
        std::task::Poll::Ready(v) => std::task::Poll::Ready(v),
        std::task::Poll::Pending => b.as_mut().poll(cx),
    })
    .await
}

fn main() {
    std::thread::spawn(|| {
        let mut last = usize::MAX;
        loop {
            std::thread::sleep(Duration::from_secs(5));
            let now = ITERATION.load(Ordering::SeqCst);
            if now == last {
                eprintln!("run_main_with_child_calls_never_hangs ... FAILED: hung at iteration {now}");
                std::process::exit(2);
            }
            last = now;
        }
    });
    for i in 0..3000 {
        ITERATION.store(i, Ordering::SeqCst);
        let ok = run_main(async { unsafe { SwiftAsync::from_raw(probe::probe_closure_void()) }.child().await.is_ok() });
        assert!(ok);
    }
    println!("run_main_with_child_calls_never_hangs ... ok");

    run_main(async {
        // The future returns at ~5ms; the uncancellable child keeps the Task alive until ~30ms. The handle is
        // dropped first, so the output is dropped when the Task finishes.
        drop(spawn_on_main(async {
            race(async { unsafe { SwiftAsync::from_raw(probe::probe_closure_block_ms(30)) }.child().await.ok().map(|_| RecordDropThread) },
                 async { sleep(Duration::from_millis(5)).await; Some(RecordDropThread) }).await
        }));
        sleep(Duration::from_millis(300)).await;
    });
    let where_ = DROPPED_ON.load(Ordering::SeqCst);
    assert_eq!(where_, 1, "main task output dropped {} ", if where_ == 2 { "off the main thread" } else { "never" });
    println!("main_task_with_children_finishes_on_main ... ok");
}
