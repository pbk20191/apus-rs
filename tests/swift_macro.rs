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
    ])
}
