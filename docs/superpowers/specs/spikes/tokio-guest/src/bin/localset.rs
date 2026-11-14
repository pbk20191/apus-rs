//! Guest mode without polling: a LocalSet polled on the main run loop (async-swift-rs `run_main`), while the
//! runtime's driver (the timer wheel) sleeps on a helper thread that runs no tasks of ours.
use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
use std::time::{Duration, Instant};

static POLLS: AtomicUsize = AtomicUsize::new(0);
/// Counts how often the main run loop polls the LocalSet.
struct Counted<F>(F);
impl<F: std::future::Future + Unpin> std::future::Future for Counted<F> {
    type Output = F::Output;
    fn poll(mut self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> std::task::Poll<F::Output> {
        POLLS.fetch_add(1, SeqCst);
        std::pin::Pin::new(&mut self.0).poll(cx)
    }
}

fn main() {
    let rt = tokio::runtime::Builder::new_current_thread().enable_time().build().unwrap();
    let handle = rt.handle().clone();
    // The driver thread: parks inside tokio with the exact next-timer deadline, and runs whatever `tokio::spawn` gets.
    std::thread::Builder::new().name("tokio-driver".into()).spawn(move || rt.block_on(std::future::pending::<()>())).unwrap();

    let start = Instant::now();
    let main_thread = std::thread::current().id();
    let on_main = move || std::thread::current().id() == main_thread;
    let _guard = handle.enter();
    let local = tokio::task::LocalSet::new();
    let work = async move {
        // 1. spawn_local + tokio timer (what rmcp's `local` feature does for its service loop)
        let a = tokio::task::spawn_local(async move {
            tokio::time::sleep(Duration::from_millis(100)).await;
            println!("1 spawn_local + sleep(100ms) at {:?}, on main: {}", start.elapsed(), on_main());
        });
        // 2. select + mpsc + timeout
        let b = tokio::task::spawn_local(async move {
            let (tx, mut rx) = tokio::sync::mpsc::channel::<u32>(4);
            tokio::task::spawn_local(async move { for i in 0..3 { tokio::time::sleep(Duration::from_millis(20)).await; tx.send(i).await.unwrap(); } });
            let mut got = vec![];
            loop {
                tokio::select! {
                    v = rx.recv() => match v { Some(v) => got.push(v), None => break },
                    _ = tokio::time::sleep(Duration::from_secs(5)) => break,
                }
            }
            let timed_out = tokio::time::timeout(Duration::from_millis(30), std::future::pending::<()>()).await.is_err();
            println!("2 mpsc + select got {got:?}; timeout fired: {timed_out}; at {:?}, on main: {}", start.elapsed(), on_main());
        });
        // 3. plain tokio::spawn (rmcp still has a few): where does it run?
        let c = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(10)).await;
            println!("3 tokio::spawn ran on thread {:?}, on main: {}", std::thread::current().name(), on_main());
        });
        // 4. a long idle wait: the main thread must not be polled meanwhile
        let before = POLLS.load(SeqCst);
        let _ = tokio::join!(a, b, c);
        let mid = POLLS.load(SeqCst);
        tokio::time::sleep(Duration::from_millis(500)).await;
        println!("4 polls of the LocalSet: {} while working, {} during a 500 ms idle sleep", mid - before, POLLS.load(SeqCst) - mid);
    };
    async_swift_rs::run_main(Counted(Box::pin(async move { local.run_until(work).await })));
    println!("done at {:?}", start.elapsed());
}
