//! Swift `AsyncSequence`s consumed from Rust, element by element, in the calling Task.
#[path = "common/mod.rs"]
mod common;
#[path = "common/probe.rs"]
mod probe;

use apus::is_main_thread;
use apus::swift::{sleep, spawn, spawn_on_main, within, CallError, SwiftFn, SwiftStream, SwiftType, SwiftValue};
use common::{block, check, Failed, Trial};
use std::future::Future;
use std::time::{Duration, Instant};

extern "C" {
    #[link_name = apus::mangle!("Probe.countdown(from: Int) -> AsyncStream<Int>")]
    static COUNTDOWN: u8;
    #[link_name = apus::mangle!("Probe.failingStream(after: Int) -> AsyncThrowingStream<String, any Error>")]
    static FAILING_STREAM: u8;
    #[link_name = apus::mangle!("Probe.failingTypedStream(after: Int) -> TypedFailing(struct)")]
    static FAILING_TYPED_STREAM: u8;
    #[link_name = apus::mangle!("Probe.ticks(_: Int) -> Ticks(struct)")]
    static TICKS: u8;
    #[link_name = apus::mangle!("Probe.endless(tag: Int) -> AsyncStream<Int>")]
    static ENDLESS: u8;
    fn probe_endless_terminated(tag: isize) -> bool;
}
macro_rules! s { ($name:ident) => { unsafe { SwiftFn::from_address(&$name as *const u8 as *const std::ffi::c_void) } }; }

fn ty(mangled: &str) -> SwiftType {
    SwiftType::from_mangled_name(mangled).expect(mangled)
}
fn countdown(from: i64) -> SwiftValue {
    s!(COUNTDOWN).call_returning(ty("ScSySiG"), (from,)).invoke().unwrap()
}
fn failing_stream(after: i64) -> SwiftValue {
    s!(FAILING_STREAM).call_returning(ty("ScsySSs5Error_pG"), (after,)).invoke().unwrap()
}
fn failing_typed_stream(after: i64) -> SwiftValue {
    // One word: returned in a register.
    s!(FAILING_TYPED_STREAM).call_direct(ty("5Probe12TypedFailingV"), (after,)).invoke().unwrap()
}
fn ticks(n: i64) -> SwiftValue {
    s!(TICKS).call_direct(ty("5Probe5TicksV"), (n,)).invoke().unwrap() // one word: returned in a register
}
fn endless(tag: i64) -> SwiftValue {
    s!(ENDLESS).call_returning(ty("ScSySiG"), (tag,)).invoke().unwrap()
}

fn in_task<T: Send + 'static>(fut: impl Future<Output = T> + Send + 'static) -> T {
    block(spawn(fut)).expect("task failed")
}

fn stream_yields_elements() -> Result<(), Failed> {
    let (items, element) = in_task(async {
        let mut stream = SwiftStream::new(countdown(3)).unwrap();
        // macOS 15 and later (the test host): elements come from next(isolation:).
        assert!(stream.uses_isolated_next(), "next(isolation:) not used");
        let element = stream.element_type().name();
        let mut items = Vec::new();
        while let Some(item) = stream.next().await {
            items.push(item.unwrap().get::<i64>().unwrap());
        }
        (items, element)
    });
    check(items == [3, 2, 1] && element == "Swift.Int", format!("{items:?} of {element}"))
}

fn throwing_stream_ends_with_its_error() -> Result<(), Failed> {
    let (a, b, error, after) = in_task(async {
        let mut stream = SwiftStream::new(failing_stream(2)).unwrap();
        let a = stream.next_value::<String>().await;
        let b = stream.next_value::<String>().await;
        let error = match stream.next().await {
            Some(Err(CallError::Swift(e))) => e.description(),
            other => format!("unexpected {other:?}"),
        };
        (a, b, error, stream.next().await.is_none())
    });
    check(matches!((&a, &b), (Some(Ok(x)), Some(Ok(y))) if x == "item0" && y == "item1"), format!("{a:?} {b:?}"))?;
    check(error.contains("code(7)") && after, format!("error {error}, finished afterwards {after}"))
}

/// A concrete `Failure` (next(isolation:) writes a DemoError into the error slot) comes back boxed as `any Error`,
/// as `next()` gives it.
fn typed_failure_is_an_error_box() -> Result<(), Failed> {
    let (a, error, after) = in_task(async {
        let mut stream = SwiftStream::new(failing_typed_stream(1)).unwrap();
        let a = stream.next_value::<String>().await;
        let error = match stream.next().await {
            Some(Err(CallError::Swift(e))) => e.description(),
            other => format!("unexpected {other:?}"),
        };
        (a, error, stream.next().await.is_none())
    });
    check(matches!(&a, Some(Ok(x)) if x == "item0"), format!("{a:?}"))?;
    check(error.contains("code(7)") && after, format!("error {error}, finished afterwards {after}"))
}

fn custom_sequence_with_stateful_iterator() -> Result<(), Failed> {
    let items = in_task(async {
        let mut stream = SwiftStream::new(ticks(3)).unwrap();
        let mut items = Vec::new();
        while let Some(item) = stream.next_value::<String>().await {
            items.push(item.unwrap());
        }
        items
    });
    check(items == ["tick1 of 3, after 0 earlier", "tick2 of 3, after 1 earlier", "tick3 of 3, after 2 earlier"], format!("{items:?}"))
}

fn not_an_async_sequence() -> Result<(), Failed> {
    let r = SwiftStream::new(SwiftValue::new(1i64));
    check(matches!(r, Err(CallError::InvalidArgument(_))), format!("{:?}", r.err()))?;
    let wrong_type = in_task(async {
        let mut stream = SwiftStream::new(countdown(1)).unwrap();
        stream.next_value::<String>().await
    });
    check(matches!(wrong_type, Some(Err(CallError::InvalidArgument(_)))), format!("{wrong_type:?}"))
}

fn dropping_the_stream_ends_the_sequence() -> Result<(), Failed> {
    let taken = in_task(async {
        let mut stream = SwiftStream::new(endless(1)).unwrap();
        let mut taken = Vec::new();
        for _ in 0..3 {
            taken.push(stream.next_value::<i64>().await.unwrap().unwrap());
        }
        taken
    });
    check(taken == [0, 1, 2], format!("{taken:?}"))?;
    let deadline = Instant::now() + Duration::from_secs(1);
    while !unsafe { probe_endless_terminated(1) } && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    check(unsafe { probe_endless_terminated(1) }, "the Swift stream was not terminated")
}

fn child_polling_allows_timeouts() -> Result<(), Failed> {
    let (first, timed_out, took) = in_task(async {
        // The countdown's producer sleeps 2 ms between items: a 50 ms budget is plenty for the first.
        let mut stream = SwiftStream::new(countdown(1)).unwrap().child();
        let first = within(Duration::from_millis(500), stream.next()).await.flatten().map(|r| r.map(|v| v.get::<i64>()));
        // An exhausted-but-open endless stream never ends; a timeout must win over the pending element.
        let mut slow = SwiftStream::new(endless(2)).unwrap().child();
        let _ = slow.next().await;
        let start = Instant::now();
        let mut ticks = 0;
        while within(Duration::from_micros(200), slow.next()).await.is_some() {
            ticks += 1;
            if ticks > 10_000 { break }
        }
        (first, ticks <= 10_000, start.elapsed())
    });
    check(matches!(first, Some(Ok(Some(1)))), format!("first {first:?}"))?;
    check(timed_out && took < Duration::from_secs(2), format!("timed out {timed_out} after {took:?}"))
}

fn main_actor_is_kept_between_elements() -> Result<(), Failed> {
    let r = block(spawn_on_main(async {
        let mut stream = SwiftStream::new(countdown(2)).unwrap();
        let mut on_main = true;
        while let Some(item) = stream.next().await {
            item.unwrap();
            on_main &= is_main_thread() && unsafe { probe::probe_assume_main() };
        }
        on_main
    }));
    check(matches!(r, Ok(true)), format!("got {r:?}"))
}

fn outside_a_task() -> Result<(), Failed> {
    let mut stream = SwiftStream::new(countdown(1)).unwrap();
    let mut next = std::pin::pin!(stream.next());
    let r = next.as_mut().poll(&mut std::task::Context::from_waker(std::task::Waker::noop()));
    check(matches!(r, std::task::Poll::Ready(Some(Err(CallError::NoCurrentTask)))), format!("got {r:?}"))
}

#[cfg(feature = "stream")]
fn implements_futures_stream() -> Result<(), Failed> {
    use futures_core::Stream;
    let items = in_task(async {
        let mut stream = SwiftStream::new(countdown(3)).unwrap();
        let mut items = Vec::new();
        while let Some(item) = std::future::poll_fn(|cx| std::pin::Pin::new(&mut stream).poll_next(cx)).await {
            items.push(item.unwrap().get::<i64>().unwrap());
        }
        items
    });
    check(items == [3, 2, 1], format!("{items:?}"))
}

fn main() {
    let _ = sleep; // re-exported API used by the timeout helper
    #[allow(unused_mut)]
    let mut trials = vec![
        Trial::test("stream_yields_elements", stream_yields_elements),
        Trial::test("throwing_stream_ends_with_its_error", throwing_stream_ends_with_its_error),
        Trial::test("typed_failure_is_an_error_box", typed_failure_is_an_error_box),
        Trial::test("custom_sequence_with_stateful_iterator", custom_sequence_with_stateful_iterator),
        Trial::test("not_an_async_sequence", not_an_async_sequence),
        Trial::test("dropping_the_stream_ends_the_sequence", dropping_the_stream_ends_the_sequence),
        Trial::test("child_polling_allows_timeouts", child_polling_allows_timeouts),
        Trial::test("main_actor_is_kept_between_elements", main_actor_is_kept_between_elements),
        Trial::test("outside_a_task", outside_a_task),
    ];
    #[cfg(feature = "stream")]
    trials.push(Trial::test("implements_futures_stream", implements_futures_stream));
    common::run(trials)
}
