//! `async_main`: the process's main is a Rust future run as Swift's async main (`_runAsyncMain`): on the main actor,
//! with Swift calls in the same Task, and the process exits when the future is done (code 1 on `Err`).
#[path = "common/probe.rs"]
mod probe;

use apus::is_main_thread;
use apus::swift::{async_main, Isolation};

apus::swift! {
    #[swift_linked = "Probe.addAsync(_: Int, _: Int) async -> Int"]
    async fn add_async(a: i64, b: i64) -> i64;
    #[swift_linked = "Probe.withIsolation(_: Int, isolation: isolated (any Actor)? = #isolation) async -> Int"]
    async fn with_isolation(x: i64, isolation: Isolation) -> i64;
}

fn main() {
    async_main(async {
        if !is_main_thread() {
            return Err("not on the main thread");
        }
        if !matches!(add_async(2, 3).await, Ok(5)) {
            return Err("a Swift call in the main task");
        }
        if !matches!(with_isolation(5, Isolation::current()).await, Ok(1005)) {
            return Err("the main task is not on the main actor");
        }
        // Suspends: the main task resumes from the drained main queue.
        apus::swift::sleep(std::time::Duration::from_millis(20)).await;
        if !is_main_thread() {
            return Err("not back on the main thread after a suspension");
        }
        println!("async_main ... ok");
        Ok(())
    })
}
