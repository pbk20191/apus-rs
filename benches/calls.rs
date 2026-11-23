//! Per-call cost of Swift calls from Rust through `swift!`, against the same calls made from Swift (a loop in the
//! callee's module: native Swift's best case). `cargo bench --features test-probe --bench calls`; results in
//! docs/superpowers/specs/2026-11-23-call-overhead.md.
use apus::swift::{spawn, SwiftValue};
use std::future::Future;
use std::hint::black_box;
use std::pin::pin;
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};
use std::time::Instant;

apus::swift! {
    #[swift_linked = "Res.benchAdd(_: Int, _: Int) -> Int"]
    fn add(a: i64, b: i64) -> i64;
    #[swift_linked = "Res.benchLength(_: String) -> Int"]
    fn length(s: &str) -> i64;
    #[swift_linked = "Res.benchPoint(_: Double) -> BenchPoint(struct)"]
    fn point(x: f64) -> SwiftValue;
    #[swift_linked = "Res.benchMax<T: Comparable>(_: T, _: T) -> T", T = "Swift.Int"]
    fn max_int(a: &SwiftValue, b: &SwiftValue) -> SwiftValue;
    #[swift_linked = "Res.benchAddAsync(_: Int, _: Int) async -> Int"]
    async fn add_async(a: i64, b: i64) -> i64;

    #[swift_linked = "Res.benchLoopAdd(_: Int) -> Int"]
    fn loop_add(n: i64) -> i64;
    #[swift_linked = "Res.benchLoopLength(_: Int) -> Int"]
    fn loop_length(n: i64) -> i64;
    #[swift_linked = "Res.benchLoopPoint(_: Int) -> Double"]
    fn loop_point(n: i64) -> f64;
    #[swift_linked = "Res.benchLoopMax(_: Int) -> Int"]
    fn loop_max(n: i64) -> i64;
    #[swift_linked = "Res.benchLoopAddAsync(_: Int) async -> Int"]
    async fn loop_add_async(n: i64) -> i64;
}

struct Unpark(std::thread::Thread);
impl Wake for Unpark {
    fn wake(self: Arc<Self>) {
        self.0.unpark()
    }
}

fn block<F: Future>(f: F) -> F::Output {
    let mut f = pin!(f);
    let waker = Waker::from(Arc::new(Unpark(std::thread::current())));
    loop {
        if let Poll::Ready(v) = f.as_mut().poll(&mut Context::from_waker(&waker)) {
            return v;
        }
        std::thread::park();
    }
}

/// The fastest of five runs of `n` iterations, in nanoseconds per iteration.
fn ns_per_call(n: usize, mut run: impl FnMut(usize)) -> f64 {
    run(n / 10); // warm up: first lookups, caches
    (0..5)
        .map(|_| {
            let start = Instant::now();
            run(n);
            start.elapsed().as_nanos() as f64 / n as f64
        })
        .fold(f64::INFINITY, f64::min)
}

fn report(name: &str, rust: f64, swift: f64) {
    println!("{name:<40} {rust:>9.1} ns   (Swift: {swift:>7.1} ns, ×{:.1})", rust / swift.max(0.01));
}

fn main() {
    const N: usize = 1_000_000;
    // `APUS_BENCH_SPIN=add|generic|async`: one case, long enough to profile (`sample <pid>`).
    if let Some(case) = std::env::var_os("APUS_BENCH_SPIN") {
        let mut s = 0i64;
        match case.to_str() {
            Some("generic") => loop {
                let (a, b) = (SwiftValue::new(s), SwiftValue::new(0i64));
                black_box(max_int(&a, &b).unwrap());
                s += 1;
            },
            Some("async") => block(spawn(async move {
                loop {
                    s = add_async(s, 1).await.unwrap();
                }
            }))
            .unwrap(),
            _ => loop {
                s = add(black_box(s), 1).unwrap();
            },
        }
    }

    let rust = ns_per_call(N, |n| {
        let mut s = 0;
        for i in 0..n {
            s = add(black_box(s), i as i64).unwrap();
        }
        black_box(s);
    });
    let swift = ns_per_call(N, |n| _ = black_box(loop_add(n as i64).unwrap()));
    report("Int, Int -> Int", rust, swift);

    let rust = ns_per_call(N, |n| {
        for _ in 0..n {
            black_box(length(black_box("hello, apus")).unwrap());
        }
    });
    let swift = ns_per_call(N, |n| _ = black_box(loop_length(n as i64).unwrap()));
    report("String -> Int", rust, swift);

    let rust = ns_per_call(N, |n| {
        for i in 0..n {
            black_box(point(i as f64).unwrap());
        }
    });
    let swift = ns_per_call(N, |n| _ = black_box(loop_point(n as i64).unwrap()));
    report("Double -> resilient struct (SwiftValue)", rust, swift);

    let rust = ns_per_call(N, |n| {
        for i in 0..n {
            let (a, b) = (SwiftValue::new(i as i64), SwiftValue::new(0i64));
            black_box(max_int(&a, &b).unwrap());
        }
    });
    let swift = ns_per_call(N, |n| _ = black_box(loop_max(n as i64).unwrap()));
    report("bound generic <T: Comparable> T, T -> T", rust, swift);

    let rust = ns_per_call(N, |n| {
        block(spawn(async move {
            let mut s = 0;
            for i in 0..n {
                s = add_async(s, i as i64).await.unwrap();
            }
            black_box(s);
        }))
        .unwrap();
    });
    let swift = ns_per_call(N, |n| _ = black_box(block(spawn(async move { loop_add_async(n as i64).await.unwrap() })).unwrap()));
    report("async Int, Int -> Int (in a Task)", rust, swift);
}
