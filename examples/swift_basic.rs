//! Rust futures as Swift Tasks: priorities, inheritance, cancellation, timers.
//! Run: cargo run --example swift_basic
use async_swift_rs::swift::{cancelled, current_task, priority, sleep, spawn, spawn_with};
use async_swift_rs::{is_main_thread, run_main, Priority, SpawnOptions};
use std::time::{Duration, Instant};

fn main() {
    let start = Instant::now();
    run_main(async move {
        println!("main task: Swift task {:?}, main thread {}", current_task().is_some(), is_main_thread());

        let utility = SpawnOptions { priority: Priority::Utility, ..SpawnOptions::default() };
        let p = spawn_with(utility, async { priority() }).await.unwrap();
        println!("utility task runs at {p:?}");

        let handles: Vec<_> = (0..1000u64).map(|i| spawn(async move { sleep(Duration::from_millis(20)).await; i })).collect();
        let mut sum = 0;
        for h in handles {
            sum += h.await.unwrap();
        }
        println!("1000 concurrent 20ms sleeps: sum {sum} at {:?}", start.elapsed());

        let long = spawn(async {
            cancelled().await;
            "noticed cancellation"
        });
        sleep(Duration::from_millis(10)).await;
        long.cancel();
        println!("cancel(): {}", long.await.unwrap());
    });
}
