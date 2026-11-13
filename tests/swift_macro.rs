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

// Methods and properties: `self` is the receiver; `SwiftValue` results take their Swift type from the declaration.
mod members {
    use async_swift_rs::swift::{DirectValue, SwiftObject, SwiftValue};
    async_swift_rs::swift! {
        #[swift_linked = "static Probe.Big(struct).make(n: Int) async -> Big(struct)"]
        pub async fn big_make(n: i64) -> SwiftValue;

        #[swift_linked = "Probe.Big(struct).total: Int { get }"]
        pub fn total(self: &SwiftValue) -> i64;

        #[swift_linked = "Probe.Big(struct).name: String { get }"]
        pub fn name(self: &SwiftValue) -> String;

        #[swift_linked = "Probe.Big(struct).scaled(by: Int) -> Big(struct)"]
        pub fn scaled(self: &SwiftValue, by: i64) -> SwiftValue;

        #[swift_linked = "Probe.bigSum(_: Big(struct), plus: Int) async -> Int"]
        pub async fn big_sum(big: &SwiftValue, plus: i64) -> i64;

        #[swift_linked = "Probe.makeCounter() -> Counter(class)"]
        pub fn make_counter() -> SwiftObject;

        #[swift_linked = "Probe.Counter(class).bump(by: Int) async -> Int"]
        pub async fn bump(self: &SwiftObject, by: i64) -> i64;

        /// `[String]` kept as an opaque Swift value (it is returned in a register).
        #[swift_linked = "Probe.names(count: Int) async -> [String]"]
        pub async fn names_opaque(count: i64) -> DirectValue;

        /// Compiles against the real framework declaration; there is no Product to call it on here.
        #[swift = "StoreKit.Product(struct).displayName: String { get }"]
        pub fn display_name(self: &SwiftValue) -> String;

        /// A symbol alone does not say which Swift type comes back.
        #[linked = "$s5Probe3BigV4make1nACSi_tYaFZ"]
        pub async fn big_make_by_symbol(n: i64) -> SwiftValue;
    }
}

fn members_and_opaque_results() -> Result<(), Failed> {
    use members::*;
    let (text, total, name, scaled_total, scaled_name, sum, bumps, names, by_symbol) = block(spawn(async {
        let big = big_make(1).await.unwrap();
        let scaled = scaled(&big, 3).unwrap();
        let counter = make_counter().unwrap();
        let bumps = (bump(&counter, 5).await.unwrap(), bump(&counter, 5).await.unwrap());
        let names = names_opaque(2).await.unwrap();
        (
            big.description(),
            total(&big),
            name(&big),
            total(&scaled),
            members::name(&scaled),
            big_sum(&big, 100).await,
            bumps,
            (names.0.ty().name(), names.0.get::<Vec<String>>()),
            big_make_by_symbol(1).await.map(|v| v.description()),
        )
    }))
    .unwrap();
    check(text.contains("big1"), text)?;
    check(matches!(total, Ok(10)) && matches!(&name, Ok(n) if n == "big1"), format!("{total:?} {name:?}"))?;
    check(matches!(scaled_total, Ok(30)) && matches!(&scaled_name, Ok(n) if n == "big1x3"), format!("{scaled_total:?} {scaled_name:?}"))?;
    check(matches!(sum, Ok(110)) && bumps == (5, 10), format!("{sum:?} {bumps:?}"))?;
    check(names.0 == "Swift.Array<Swift.String>" && names.1.as_deref() == Some(&["name0".to_string(), "name1".into()][..]), format!("{names:?}"))?;
    check(matches!(by_symbol, Err(CallError::InvalidArgument(_))), format!("{by_symbol:?}"))?;
    let _ = display_name; // compiles; needs a real Product to call
    check(async_swift_rs::result_type!("Probe.f() -> [Big(struct)?]") == Some("Say5Probe3BigVSgG"), "result_type!")?;
    check(async_swift_rs::result_type!("Probe.f<T>(_: T) -> T").is_none(), "a generic result has no fixed type")
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
        Trial::test("members_and_opaque_results", members_and_opaque_results),
    ])
}
