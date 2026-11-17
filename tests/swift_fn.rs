//! Direct calls of ordinary Swift async functions (tests/support/fns.swift: no @_cdecl, no glue) by their
//! async function pointer, in the calling Task.
#[path = "common/mod.rs"]
mod common;
#[path = "common/probe.rs"]
mod probe;

use apus::is_main_thread;
use apus::swift::{
    current_task, sleep, spawn, spawn_on_main, timeout, AsyncFunctionPointer, CallError, SwiftAny, SwiftAsyncFn, SwiftObject,
};
use common::{block, check, Failed, Trial};
use std::collections::HashMap;
use std::ffi::c_void;
use std::future::{poll_fn, Future};
use std::pin::pin;
use std::task::Poll;
use std::time::{Duration, Instant};

extern "C" {
    #[link_name = "$s5Probe8addAsyncyS2i_SitYaFTu"] static ADD: AsyncFunctionPointer;
    #[link_name = "$s5Probe15currentTaskBitsSiyYaFTu"] static CURRENT_TASK_BITS: AsyncFunctionPointer;
    #[link_name = "$s5Probe5greet_5times7sleepMsS2S_S2itYaKFTu"] static GREET: AsyncFunctionPointer;
    #[link_name = "$s5Probe4half_4plusS2d_SitYaFTu"] static HALF: AsyncFunctionPointer;
    #[link_name = "$s5Probe7failing4codeS2i_tYaKFTu"] static FAILING: AsyncFunctionPointer;
    #[link_name = "$s5Probe5names5countSaySSGSi_tYaFTu"] static NAMES: AsyncFunctionPointer;
    #[link_name = "$s5Probe5maybeySiSgSiYaFTu"] static MAYBE: AsyncFunctionPointer;
    #[link_name = "$s5Probe9maybeNameySSSgSiYaFTu"] static MAYBE_NAME: AsyncFunctionPointer;
    #[link_name = "$s5Probe8describeySSSb_s5Int32Vs6UInt64VtYaFTu"] static DESCRIBE: AsyncFunctionPointer;
    #[link_name = "$s5Probe8isOnMainSbyYaFTu"] static IS_ON_MAIN: AsyncFunctionPointer;
    #[link_name = "$s5Probe6isEvenySbSiYaFTu"] static IS_EVEN: AsyncFunctionPointer;
    #[link_name = "$s5Probe7nothingyyYaFTu"] static NOTHING: AsyncFunctionPointer;
    #[link_name = "$s5Probe8anyValueyypSiYaFTu"] static ANY_VALUE: AsyncFunctionPointer;
    #[link_name = "$s5Probe6scoresSDySSSiGyYaFTu"] static SCORES: AsyncFunctionPointer;
    #[link_name = "$s5Probe7CounterC4bump2byS2i_tYaFTu"] static COUNTER_BUMP: AsyncFunctionPointer;
    #[link_name = "$s5Probe16makeCounterAsync5startAA0C0CSi_tYaFTu"] static MAKE_COUNTER_ASYNC: AsyncFunctionPointer;
    /// A synchronous Swift function returning a +1 object.
    #[link_name = "$s5Probe11makeCounterAA0C0CyF"] fn make_counter() -> *mut c_void;
}

fn f(pointer: &'static AsyncFunctionPointer) -> SwiftAsyncFn {
    unsafe { SwiftAsyncFn::from_pointer(pointer) }
}
macro_rules! f { ($name:ident) => { f(unsafe { &$name }) }; }

/// `Task.sleep(nanoseconds:)` in the OS's libswift_Concurrency, found at run time.
static TASK_SLEEP: SwiftAsyncFn = SwiftAsyncFn::new("$sScTss5NeverORszABRs_rlE5sleep11nanosecondsys6UInt64V_tYaKFZ");
static MISSING: SwiftAsyncFn = SwiftAsyncFn::new("$s5Probe12doesNotExistyyYaF");

fn in_task<T: Send + 'static>(fut: impl Future<Output = T> + Send + 'static) -> T {
    block(spawn(fut)).expect("task failed")
}

async fn join2<A, B>(a: impl Future<Output = A>, b: impl Future<Output = B>) -> (A, B) {
    let (mut a, mut b) = (pin!(a), pin!(b));
    let (mut ra, mut rb) = (None, None);
    poll_fn(|cx| {
        if ra.is_none() { if let Poll::Ready(v) = a.as_mut().poll(cx) { ra = Some(v) } }
        if rb.is_none() { if let Poll::Ready(v) = b.as_mut().poll(cx) { rb = Some(v) } }
        if ra.is_some() && rb.is_some() { Poll::Ready((ra.take().unwrap(), rb.take().unwrap())) } else { Poll::Pending }
    })
    .await
}

fn integers_in_and_out() -> Result<(), Failed> {
    let r = in_task(async { f!(ADD).call::<i64>((3i64, 4i64)).await });
    check(matches!(r, Ok(7)), format!("got {r:?}"))
}

fn runs_in_the_callers_task() -> Result<(), Failed> {
    let (me, seen) = in_task(async {
        (current_task().unwrap().as_ptr() as usize, f!(CURRENT_TASK_BITS).call::<i64>(()).await.unwrap() as usize)
    });
    check(me == seen, format!("caller 0x{me:x}, callee saw 0x{seen:x}"))
}

fn string_argument_and_result() -> Result<(), Failed> {
    let r = in_task(async { f!(GREET).call::<String>(("héllo👋", 2i64, 10i64)).throws().await });
    check(matches!(&r, Ok(s) if s == "héllo👋 héllo👋 "), format!("got {r:?}"))
}

fn mixed_scalar_arguments() -> Result<(), Failed> {
    let (half, text, even) = in_task(async {
        (
            f!(HALF).call::<f64>((3.0f64, 10i64)).await,
            f!(DESCRIBE).call::<String>((true, -7i32, u64::MAX)).await,
            f!(IS_EVEN).call::<bool>((10i64,)).await,
        )
    });
    check(matches!(half, Ok(v) if v == 11.5), format!("half {half:?}"))?;
    check(matches!(&text, Ok(s) if s == "true -7 18446744073709551615"), format!("describe {text:?}"))?;
    check(matches!(even, Ok(true)), format!("isEven {even:?}"))
}

fn void_result() -> Result<(), Failed> {
    let r = in_task(async { f!(NOTHING).call::<()>(()).await });
    check(matches!(r, Ok(())), format!("got {r:?}"))
}

fn thrown_error() -> Result<(), Failed> {
    let r = in_task(async { f!(FAILING).call::<i64>((5i64,)).throws().await });
    match r {
        Err(CallError::Swift(e)) => check(e.description().contains("code(5)"), e.description()),
        other => Err(format!("got {other:?}").into()),
    }
}

fn collections_and_optionals() -> Result<(), Failed> {
    let (names, some, none, some_name, no_name, scores) = in_task(async {
        (
            f!(NAMES).call::<Vec<String>>((3i64,)).await,
            f!(MAYBE).call::<Option<i64>>((4i64,)).await,
            f!(MAYBE).call::<Option<i64>>((-1i64,)).await,
            f!(MAYBE_NAME).call::<Option<String>>((7i64,)).await,
            f!(MAYBE_NAME).call::<Option<String>>((0i64,)).await,
            f!(SCORES).call::<HashMap<String, i64>>(()).await,
        )
    });
    check(matches!(&names, Ok(v) if v == &["name0", "name1", "name2"]), format!("names {names:?}"))?;
    check(matches!((&some, &none), (Ok(Some(40)), Ok(None))), format!("maybe {some:?} {none:?}"))?;
    check(matches!((&some_name, &no_name), (Ok(Some(s)), Ok(None)) if s == "name7"), format!("maybeName {some_name:?} {no_name:?}"))?;
    let expected: HashMap<String, i64> = [("a".to_string(), 1), ("b".into(), 2)].into();
    check(matches!(&scores, Ok(m) if m == &expected), format!("scores {scores:?}"))
}

fn indirect_any_result() -> Result<(), Failed> {
    let r = in_task(async { f!(ANY_VALUE).call::<SwiftAny>((21i64,)).await.map(|any| any.downcast::<i64>()) });
    check(matches!(r, Ok(Some(42))), format!("got {r:?}"))
}

fn methods_and_object_results() -> Result<(), Failed> {
    let (a, b, c) = in_task(async {
        let counter = unsafe { SwiftObject::from_raw(make_counter()) };
        let a = f!(COUNTER_BUMP).call::<i64>((5i64,)).on(&counter).await;
        let b = f!(COUNTER_BUMP).call::<i64>((5i64,)).on(&counter).await;
        let made = f!(MAKE_COUNTER_ASYNC).call::<SwiftObject>((100i64,)).await.unwrap();
        let c = f!(COUNTER_BUMP).call::<i64>((1i64,)).on(&made).await;
        (a, b, c)
    });
    check(matches!((&a, &b, &c), (Ok(5), Ok(10), Ok(101))), format!("got {a:?} {b:?} {c:?}"))
}

fn main_actor_is_restored() -> Result<(), Failed> {
    let r = block(spawn_on_main(async {
        let callee_on_main = f!(IS_ON_MAIN).call::<bool>(()).await.unwrap();
        (callee_on_main, is_main_thread() && unsafe { probe::probe_assume_main() })
    }));
    check(matches!(r, Ok((false, true))), format!("got {r:?}"))
}

fn cancellation_reaches_the_callee() -> Result<(), Failed> {
    let h = spawn(async {
        match f!(GREET).call::<String>(("x", 1i64, 10_000i64)).throws().await {
            Err(CallError::Swift(e)) => e.description(),
            other => format!("unexpected {other:?}"),
        }
    });
    std::thread::sleep(Duration::from_millis(20));
    h.cancel();
    let r = common::block_with_timeout(h, Duration::from_secs(2));
    check(matches!(&r, Some(Ok(s)) if s.contains("CancellationError")), format!("got {r:?}"))
}

fn os_function_found_at_run_time() -> Result<(), Failed> {
    let (available, r, took) = in_task(async {
        let start = Instant::now();
        let r = TASK_SLEEP.call::<()>((50_000_000u64,)).throws().await;
        (TASK_SLEEP.is_available(), r, start.elapsed())
    });
    check(available && matches!(r, Ok(())), format!("available {available}, got {r:?}"))?;
    check(took >= Duration::from_millis(45) && took < Duration::from_millis(500), format!("took {took:?}"))
}

fn missing_symbol_is_an_error() -> Result<(), Failed> {
    let r = in_task(async { MISSING.call::<()>(()).await });
    check(!MISSING.is_available() && matches!(r, Err(CallError::Unavailable(_))), format!("got {r:?}"))
}

fn argument_edge_cases() -> Result<(), Failed> {
    let (nul, many) = in_task(async {
        (
            // A NUL byte is a legal character in both languages.
            f!(GREET).call::<String>(("a\0b", 1i64, 0i64)).throws().await,
            f!(GREET).call::<String>(("a", "b", "c", "d", "e")).throws().await, // 10 integer registers
        )
    });
    check(matches!(&nul, Ok(s) if s == "a\0b "), format!("nul: {nul:?}"))?;
    check(matches!(many, Err(CallError::InvalidArgument(_))), format!("many: {many:?}"))
}

fn outside_a_task() -> Result<(), Failed> {
    let mut call = pin!(f!(ADD).call::<i64>((1i64, 2i64)));
    let r = call.as_mut().poll(&mut std::task::Context::from_waker(std::task::Waker::noop()));
    check(matches!(r, Poll::Ready(Err(CallError::NoCurrentTask))), format!("got {r:?}"))
}

fn concurrent_calls_become_children() -> Result<(), Failed> {
    let (a, b, took, timed_out) = in_task(async {
        let start = Instant::now();
        let (a, b) = join2(
            f!(GREET).call::<String>(("a", 1i64, 50i64)).throws(),
            f!(GREET).call::<String>(("b", 1i64, 50i64)).throws(),
        )
        .await;
        let took = start.elapsed();
        let timed_out = timeout(Duration::from_millis(30), f!(GREET).call::<String>(("c", 1i64, 10_000i64)).throws()).await.is_none();
        (a, b, took, timed_out)
    });
    check(matches!((&a, &b), (Ok(x), Ok(y)) if x == "a " && y == "b "), format!("{a:?} {b:?}"))?;
    check(took < Duration::from_millis(90), format!("join took {took:?}"))?;
    check(timed_out, "timeout did not fire")
}

fn many_sequential_calls() -> Result<(), Failed> {
    let total = in_task(async {
        let mut total = 0;
        for i in 0..2000i64 {
            total = f!(ADD).call::<i64>((total, i)).await.unwrap();
            if i % 500 == 0 { sleep(Duration::from_millis(1)).await }
        }
        total
    });
    check(total == 1_999_000, format!("total {total}"))
}

fn main() {
    common::run(vec![
        Trial::test("integers_in_and_out", integers_in_and_out),
        Trial::test("runs_in_the_callers_task", runs_in_the_callers_task),
        Trial::test("string_argument_and_result", string_argument_and_result),
        Trial::test("mixed_scalar_arguments", mixed_scalar_arguments),
        Trial::test("void_result", void_result),
        Trial::test("thrown_error", thrown_error),
        Trial::test("collections_and_optionals", collections_and_optionals),
        Trial::test("indirect_any_result", indirect_any_result),
        Trial::test("methods_and_object_results", methods_and_object_results),
        Trial::test("main_actor_is_restored", main_actor_is_restored),
        Trial::test("cancellation_reaches_the_callee", cancellation_reaches_the_callee),
        Trial::test("os_function_found_at_run_time", os_function_found_at_run_time),
        Trial::test("missing_symbol_is_an_error", missing_symbol_is_an_error),
        Trial::test("argument_edge_cases", argument_edge_cases),
        Trial::test("outside_a_task", outside_a_task),
        Trial::test("concurrent_calls_become_children", concurrent_calls_become_children),
        Trial::test("many_sequential_calls", many_sequential_calls),
    ])
}
