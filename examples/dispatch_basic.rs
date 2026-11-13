//! The libdispatch backend: QoS queues, the main queue, serial queues.
//! Run: cargo run --example dispatch_basic --no-default-features --features dispatch
use async_swift_rs::dispatch::{sleep, spawn, spawn_with, SerialQueue};
use async_swift_rs::{is_main_thread, run_main, Priority, SpawnOptions};
use std::time::Duration;

fn main() {
    run_main(async {
        println!("main-queue task on the main thread: {}", is_main_thread());

        let background = SpawnOptions { priority: Priority::Background, ..SpawnOptions::default() };
        let stays_main = spawn_with(background, async { is_main_thread() }).await.unwrap();
        println!("spawn_with(Background) from main stays on main (inherits like Swift Task {{}}): {stays_main}");

        let queue = SerialQueue::new("example.serial", Priority::Utility);
        let order: Vec<_> = (0..5).map(|i| queue.spawn(async move { sleep(Duration::from_millis(5 - i)).await; i })).collect();
        let mut done = Vec::new();
        for h in order {
            done.push(h.await.unwrap());
        }
        println!("serial queue results in spawn order: {done:?}");
        println!("plain spawn: {}", spawn(async { 6 * 7 }).await.unwrap());
    });
}
