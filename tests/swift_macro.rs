//! `swift!`: Swift functions declared as Rust signatures.
#[path = "common/mod.rs"]
mod common;

use async_swift_rs::swift::{spawn, CallError};
use common::{block, check, Failed, Trial};
use std::time::{Duration, Instant};

async_swift_rs::swift! {
    /// `func addAsync(_ a: Int, _ b: Int) async -> Int`
    #[linked = "$s5Probe8addAsyncyS2i_SitYaF"]
    pub async fn add_async(a: i64, b: i64) -> i64;

    #[linked = "$s5Probe5greet_5times7sleepMsS2S_S2itYaKF"]
    async fn greet(name: &str, times: i64, sleep_ms: i64) throws -> String;

    #[linked = "$s5Probe7failing4codeS2i_tYaKF"]
    async fn failing(code: i64) throws -> i64;

    #[linked = "$s5Probe7nothingyyYaF"]
    async fn nothing();

    /// Found at run time in the OS; a missing symbol would be `CallError::Unavailable`.
    #[symbol = "$sScTss5NeverORszABRs_rlE5sleep11nanosecondsys6UInt64V_tYaKFZ"]
    async fn task_sleep(nanoseconds: u64) throws;

    #[symbol = "$s5Probe12doesNotExistyyYaF"]
    async fn missing();

    #[linked = "$s5Probe7syncAddySdSi_SdtF"]
    fn sync_add(a: i64, b: f64) -> f64;

    #[linked = "$s5Probe9syncThrowySiSbKF"]
    fn sync_throw(fail: bool) throws -> i64;

    #[linked = "$s5Probe9byteCountySiSSF"]
    fn byte_count(text: &str) -> i64;

    #[symbol = "$s5Probe12doesNotExistyyF"]
    fn missing_sync();
}

// The same, with the symbol computed from the Swift declaration at compile time.
mod declared {
    async_swift_rs::swift! {
        #[swift_linked = "Probe.addAsync(_: Int, _: Int) async -> Int"]
        pub async fn add_async(a: i64, b: i64) -> i64;

        #[swift_linked = "Probe.greet(_: String, times: Int, sleepMs: Int) async throws -> String"]
        pub async fn greet(name: &str, times: i64, sleep_ms: i64) throws -> String;

        #[swift_linked = "Probe.names(count: Int) async -> [String]"]
        pub async fn names(count: i64) -> Vec<String>;

        #[swift_linked = "Probe.scores() async -> [String: Int]"]
        pub async fn scores() -> std::collections::HashMap<String, i64>;

        #[swift_linked = "Probe.syncThrow(_: Bool) throws -> Int"]
        pub fn sync_throw(fail: bool) throws -> i64;

        #[swift_linked = "Probe.byteCount(_: String) -> Int"]
        pub fn byte_count(text: &str) -> i64;

        /// Looked up at run time: absent from this process.
        #[swift = "Probe.neverDefined(_: Int) async -> Int"]
        pub async fn never_defined(x: i64) -> i64;
    }
}

fn declarations_compute_the_symbol() -> Result<(), Failed> {
    check(async_swift_rs::mangle!("Probe.addAsync(_: Int, _: Int) async -> Int") == "$s5Probe8addAsyncyS2i_SitYaF", "mangle! literal")?;
    let (sum, text, names, scores, absent) = block(spawn(async {
        (
            declared::add_async(20, 22).await,
            declared::greet("Lee", 1, 1).await,
            declared::names(2).await,
            declared::scores().await,
            declared::never_defined(1).await,
        )
    }))
    .unwrap();
    check(matches!(sum, Ok(42)), format!("sum {sum:?}"))?;
    check(matches!(&text, Ok(s) if s == "Lee "), format!("text {text:?}"))?;
    check(matches!(&names, Ok(v) if v == &["name0", "name1"]), format!("names {names:?}"))?;
    check(matches!(&scores, Ok(m) if m.len() == 2 && m["b"] == 2), format!("scores {scores:?}"))?;
    check(matches!(absent, Err(CallError::Unavailable(_))), format!("absent {absent:?}"))?;
    let (ok, err, bytes) = (declared::sync_throw(false), declared::sync_throw(true), declared::byte_count("héllo"));
    check(matches!(ok, Ok(1)) && matches!(err, Err(CallError::Swift(_))) && matches!(bytes, Ok(6)), format!("sync {ok:?} {err:?} {bytes:?}"))
}

fn async_functions() -> Result<(), Failed> {
    let (sum, text, error, unit, slept, absent) = block(spawn(async {
        let start = Instant::now();
        let slept = task_sleep(30_000_000).await.map(|()| start.elapsed());
        (
            add_async(3, 4).await,
            greet("Kim", 2, 5).await,
            failing(5).await,
            nothing().await,
            slept,
            missing().await,
        )
    }))
    .unwrap();
    check(matches!(sum, Ok(7)), format!("sum {sum:?}"))?;
    check(matches!(&text, Ok(s) if s == "Kim Kim "), format!("text {text:?}"))?;
    check(matches!(&error, Err(CallError::Swift(e)) if e.description().contains("code(5)")), format!("error {error:?}"))?;
    check(matches!(unit, Ok(())), format!("unit {unit:?}"))?;
    check(matches!(slept, Ok(d) if d >= Duration::from_millis(25)), format!("slept {slept:?}"))?;
    check(matches!(absent, Err(CallError::Unavailable(_))), format!("absent {absent:?}"))
}

fn calls_are_builders() -> Result<(), Failed> {
    // The generated function returns the call itself, so `.child()` and `timeout` apply.
    let timed_out = block(spawn(async {
        async_swift_rs::swift::timeout(Duration::from_millis(30), greet("x", 1, 10_000)).await.is_none()
    }))
    .unwrap();
    check(timed_out, "timeout did not fire")
}

fn sync_functions() -> Result<(), Failed> {
    let (add, ok, err, bytes, absent) = (sync_add(2, 0.5), sync_throw(false), sync_throw(true), byte_count("héllo"), missing_sync());
    check(matches!(add, Ok(v) if v == 2.5), format!("add {add:?}"))?;
    check(matches!(ok, Ok(1)), format!("ok {ok:?}"))?;
    check(matches!(&err, Err(CallError::Swift(e)) if e.description().contains("code(9)")), format!("err {err:?}"))?;
    check(matches!(bytes, Ok(6)), format!("bytes {bytes:?}"))?;
    check(matches!(absent, Err(CallError::Unavailable(_))), format!("absent {absent:?}"))
}

fn main() {
    common::run(vec![
        Trial::test("async_functions", async_functions),
        Trial::test("calls_are_builders", calls_are_builders),
        Trial::test("sync_functions", sync_functions),
        Trial::test("declarations_compute_the_symbol", declarations_compute_the_symbol),
    ])
}
