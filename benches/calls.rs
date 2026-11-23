//! Per-call cost of Swift calls from Rust through `swift!`, against the same calls made from Swift (a loop in the
//! callee's module: native Swift's best case). `cargo bench --features test-probe --bench calls`; results in
//! docs/superpowers/specs/2026-11-23-call-overhead.md.
use apus::swift::{spawn, SwiftObject, SwiftString, SwiftValue};
use std::future::Future;
use std::hint::black_box;
use std::pin::pin;
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};
use std::time::Instant;

apus::swift! {
    class BenchBox = "Res.BenchBox(class)";
    impl BenchBox {
        #[swift_linked = "Res.BenchBox(class).n: Int { get }"]
        fn n(&self) -> i64;
    }
}

apus::swift! {
    #[swift_linked = "Res.benchAdd(_: Int, _: Int) -> Int"]
    fn add(a: i64, b: i64) -> i64;
    #[swift_linked = "Res.benchLength(_: String) -> Int"]
    fn length(s: &str) -> i64;
    #[swift_linked = "Res.benchLength(_: String) -> Int"]
    fn length_of(s: &SwiftString) -> i64;
    #[swift_linked = "Res.BenchPoint(struct).y: Double { get }"]
    fn point_y(self: &SwiftValue) -> f64;
    #[swift_linked = "Res.benchPoint(_: Double) -> BenchPoint(struct)"]
    fn point(x: f64) -> SwiftValue;
    #[swift_linked = "Res.benchMax<T: Comparable>(_: T, _: T) -> T", T = "Swift.Int"]
    fn max_int(a: &SwiftValue, b: &SwiftValue) -> SwiftValue;
    #[swift_linked = "Res.benchMakeBox(_: Int) -> BenchBox(class)"]
    fn make_box(n: i64) -> SwiftObject;
    #[swift_linked = "Res.benchBoxed(_: BenchBox(class)) -> Int"]
    fn boxed(b: &SwiftObject) -> i64;
    #[swift_linked = "Res.BenchBox(class).n: Int { get }"]
    fn box_n(self: &SwiftObject) -> i64;
    #[swift_linked = "Res.benchLoopBoxed(_: Int) -> Int"]
    fn loop_boxed(n: i64) -> i64;
    #[swift_linked = "Res.benchMax<T: Comparable>(_: T, _: T) -> T", T = "Swift.Int"]
    fn max_stack(a: &i64, b: &i64) -> SwiftValue;
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

extern "C" {
    #[link_name = "$s3Res8benchAddyS2i_SitF"]
    static BENCH_ADD: u8;
}

/// The floor of apus's sync path: its register trampoline called with registers filled by hand, nothing else.
fn trampoline_floor(n: usize) {
    let mut regs = apus_sys::CallRegs { gpr: [0; 8], fpr: [0.0; 8], self_: std::ptr::null_mut(), isolation: -1 };
    let mut ret: apus_sys::RetRegs = unsafe { std::mem::zeroed() };
    let function = (&raw const BENCH_ADD).cast::<std::ffi::c_void>();
    let mut s = 0u64;
    for i in 0..n {
        regs.gpr[0] = black_box(s);
        regs.gpr[1] = i as u64;
        unsafe { apus_sys::rs_call_sync(function, &regs, std::ptr::null_mut(), &mut ret, 0) };
        s = ret.gpr[0];
    }
    black_box(s);
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
    let floor = ns_per_call(N, trampoline_floor);
    report("  (apus trampoline alone)", floor, swift);

    let rust = ns_per_call(N, |n| {
        for _ in 0..n {
            black_box(length(black_box("hello, apus")).unwrap());
        }
    });
    let swift = ns_per_call(N, |n| _ = black_box(loop_length(n as i64).unwrap()));
    report("String -> Int", rust, swift);

    let text = SwiftString::new("hello, apus");
    let rust = ns_per_call(N, |n| {
        for _ in 0..n {
            black_box(length_of(black_box(&text)).unwrap());
        }
    });
    report("  (made once: &SwiftString)", rust, swift);

    let object = make_box(1).unwrap();
    let rust = ns_per_call(N, |n| {
        for _ in 0..n {
            black_box(boxed(black_box(&object)).unwrap());
        }
    });
    let swift = ns_per_call(N, |n| _ = black_box(loop_boxed(n as i64).unwrap()));
    report("class instance -> Int", rust, swift);

    let rust = ns_per_call(N, |n| {
        for _ in 0..n {
            black_box(box_n(black_box(&object)).unwrap());
        }
    });
    report("  (a property of it: self)", rust, swift);

    let wrapped = BenchBox::try_from(make_box(1).unwrap()).unwrap();
    let rust = ns_per_call(N, |n| {
        for _ in 0..n {
            black_box(black_box(&wrapped).n().unwrap());
        }
    });
    report("  (the same, as a typed wrapper)", rust, swift);

    let rust = ns_per_call(N, |n| {
        for i in 0..n {
            black_box(point(i as f64).unwrap());
        }
    });
    let swift = ns_per_call(N, |n| _ = black_box(loop_point(n as i64).unwrap()));
    report("Double -> resilient struct (SwiftValue)", rust, swift);

    let value = point(1.0).unwrap();
    let rust = ns_per_call(N, |n| {
        for _ in 0..n {
            black_box(point_y(black_box(&value)).unwrap());
        }
    });
    report("  (a property of one: self)", rust, swift);

    let rust = ns_per_call(N, |n| {
        for i in 0..n {
            let (a, b) = (SwiftValue::new(i as i64), SwiftValue::new(0i64));
            black_box(max_int(&a, &b).unwrap());
        }
    });
    let swift = ns_per_call(N, |n| _ = black_box(loop_max(n as i64).unwrap()));
    report("bound generic <T: Comparable> T, T -> T", rust, swift);

    let rust = ns_per_call(N, |n| {
        for i in 0..n {
            black_box(max_stack(&(i as i64), &0).unwrap());
        }
    });
    report("  (arguments from the stack: &i64)", rust, swift);

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
