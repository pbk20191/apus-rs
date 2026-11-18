//! Generic functions (metadata and witness tables), values passed and returned by address, synchronous calls.
#[path = "common/mod.rs"]
mod common;

use apus::swift::{
    spawn, AsyncFunctionPointer, CallError, SwiftAsyncFn, SwiftFn, SwiftProtocol, SwiftType, SwiftValue,
};
use common::{block, check, Failed, Trial};
use std::future::Future;
use std::time::Duration;

extern "C" {
    #[link_name = "$s5Probe4echoyxxYalFTu"] static ECHO: AsyncFunctionPointer;
    #[link_name = "$s5Probe7joinAll_3sepSSx_SStYaSlRzSS7ElementRtzlFTu"] static JOIN_ALL: AsyncFunctionPointer;
    #[link_name = "$s5Probe10describeIt_5timesSSx_SitYaKs23CustomStringConvertibleRzlFTu"] static DESCRIBE_IT: AsyncFunctionPointer;
    #[link_name = "$s5Probe3BigV4make1nACSi_tYaFZTu"] static BIG_MAKE: AsyncFunctionPointer;
    #[link_name = "$s5Probe6bigSum_4plusSiAA3BigV_SitYaFTu"] static BIG_SUM: AsyncFunctionPointer;
    #[link_name = "$s5Probe6sumAllySiSaySiGYaFTu"] static SUM_ALL: AsyncFunctionPointer;
    #[link_name = "$s5Probe7syncAddySdSi_SdtF"] static SYNC_ADD: u8;
    #[link_name = "$s5Probe9syncThrowySiSbKF"] static SYNC_THROW: u8;
    #[link_name = "$s5Probe3BigV5totalSivg"] static BIG_TOTAL: u8;
    #[link_name = "$s5Probe3BigV6scaled2byACSi_tF"] static BIG_SCALED: u8;
    #[link_name = "$s5Probe9byteCountySiSSF"] static BYTE_COUNT: u8;
}
macro_rules! f { ($name:ident) => { unsafe { SwiftAsyncFn::from_pointer(&$name) } }; }
macro_rules! s { ($name:ident) => { unsafe { SwiftFn::from_address(&$name as *const u8 as *const std::ffi::c_void) } }; }

static COLLECTION: SwiftProtocol = SwiftProtocol::new("$sSlMp");
static CUSTOM_STRING_CONVERTIBLE: SwiftProtocol = SwiftProtocol::new("$ss23CustomStringConvertibleMp");
static HASHABLE: SwiftProtocol = SwiftProtocol::new("$sSHMp");

fn big_type() -> SwiftType {
    SwiftType::from_mangled_name("5Probe3BigV").expect("Probe.Big")
}

fn in_task<T: Send + 'static>(fut: impl Future<Output = T> + Send + 'static) -> T {
    block(spawn(fut)).expect("task failed")
}

fn types_and_conformances() -> Result<(), Failed> {
    let strings = SwiftType::of::<Vec<String>>();
    check(SwiftType::from_mangled_name("SaySSG") == Some(strings), "SaySSG is not [String]")?;
    check(strings.name() == "Swift.Array<Swift.String>", strings.name())?;
    check(big_type().name() == "Probe.Big" && big_type().size() == 48, format!("{} {}", big_type().name(), big_type().size()))?;
    check(SwiftType::from_mangled_name("5Probe7NoSuchV").is_none(), "unknown type resolved")?;
    check(strings.conformance(&COLLECTION).is_some(), "[String] should be a Collection")?;
    check(strings.conformance(&HASHABLE).is_some(), "[String] should be Hashable")?;
    check(big_type().conformance(&HASHABLE).is_none(), "Big should not be Hashable")?;
    // An imported C type named by a typedef: its name in a declaration's result is one the runtime resolves.
    let decimal = SwiftType::from_mangled_name(apus::result_type!("M.f() -> __C.NSDecimal(typedef struct)").unwrap());
    check(decimal.is_some_and(|ty| ty.size() == 20), "NSDecimal")?;
    let color = SwiftType::from_mangled_name(apus::result_type!("M.f() -> __C.CGColorRef(typedef class)").unwrap());
    check(color.is_some_and(|ty| ty.name() == "__C.CGColorRef"), format!("{:?}", color.map(|ty| ty.name())))
}

fn values_round_trip() -> Result<(), Failed> {
    let n = SwiftValue::new(42i64);
    check(n.get::<i64>() == Some(42) && n.get::<String>().is_none() && n.ty() == SwiftType::of::<i64>(), "Int value")?;
    let s = SwiftValue::new("with\0nul héllo👋");
    check(s.get::<String>().as_deref() == Some("with\0nul héllo👋"), format!("{:?}", s.get::<String>()))?;
    let v = SwiftValue::new(vec![vec!["a".to_string()], vec![], vec!["b".into(), "c".into()]]);
    let copy = v.clone();
    drop(v);
    check(copy.get::<Vec<Vec<String>>>() == Some(vec![vec!["a".to_string()], vec![], vec!["b".into(), "c".into()]]), copy.description())?;
    check(copy.description() == r#"[["a"], [], ["b", "c"]]"#, copy.description())?;
    let empty = SwiftValue::new(Vec::<f64>::new());
    check(empty.get::<Vec<f64>>() == Some(vec![]), "empty array")
}

fn generic_identity() -> Result<(), Failed> {
    let (int, string, array) = in_task(async {
        let n = SwiftValue::new(42i64);
        let int = f!(ECHO).call_returning(n.ty(), (&n, n.ty())).await.map(|v| v.get::<i64>());
        let s = SwiftValue::new("generic héllo");
        let string = f!(ECHO).call_returning(s.ty(), (&s, s.ty())).await.map(|v| v.get::<String>());
        let a = SwiftValue::new(vec![1i64, 2, 3]);
        let array = f!(ECHO).call_returning(a.ty(), (&a, a.ty())).await.map(|v| v.get::<Vec<i64>>());
        (int, string, array)
    });
    check(matches!(int, Ok(Some(42))), format!("int {int:?}"))?;
    check(matches!(&string, Ok(Some(s)) if s == "generic héllo"), format!("string {string:?}"))?;
    check(matches!(&array, Ok(Some(v)) if v == &[1, 2, 3]), format!("array {array:?}"))
}

fn constrained_generics_take_witness_tables() -> Result<(), Failed> {
    let (joined, described, sum) = in_task(async {
        let names = SwiftValue::new(vec!["a".to_string(), "b".into(), "c".into()]);
        let collection = names.ty().conformance(&COLLECTION).unwrap();
        let joined = f!(JOIN_ALL).call::<String>((&names, "-", names.ty(), collection)).await;
        let seven = SwiftValue::new(7i64);
        let printable = seven.ty().conformance(&CUSTOM_STRING_CONVERTIBLE).unwrap();
        let described = f!(DESCRIBE_IT).call::<String>((&seven, 3i64, seven.ty(), printable)).throws().await;
        // A concrete `[Int]` parameter is passed directly, as one reference.
        let sum = f!(SUM_ALL).call::<i64>((vec![1i64, 2, 3, 4],)).await;
        (joined, described, sum)
    });
    check(matches!(&joined, Ok(s) if s == "a-b-c"), format!("joined {joined:?}"))?;
    check(matches!(&described, Ok(s) if s == "777"), format!("described {described:?}"))?;
    check(matches!(sum, Ok(10)), format!("sum {sum:?}"))
}

fn large_values_by_address() -> Result<(), Failed> {
    let (text, sum, total, scaled_total, scaled_text) = in_task(async {
        let big = f!(BIG_MAKE).call_returning(big_type(), (1i64,)).await.unwrap();
        let sum = f!(BIG_SUM).call::<i64>((&big, 100i64)).await;
        let total = s!(BIG_TOTAL).call::<i64>(()).on_value(&big).invoke();
        let scaled = s!(BIG_SCALED).call_returning(big_type(), (2i64,)).on_value(&big).invoke().unwrap();
        let scaled_total = s!(BIG_TOTAL).call::<i64>(()).on_value(&scaled).invoke();
        (big.description(), sum, total, scaled_total, scaled.description())
    });
    check(text == r#"Big(a: 1, b: 2, c: 3, d: 4, name: "big1")"#, text)?;
    check(matches!((&sum, &total, &scaled_total), (Ok(110), Ok(10), Ok(20))), format!("{sum:?} {total:?} {scaled_total:?}"))?;
    check(scaled_text.contains("big1x2"), scaled_text)
}

fn synchronous_calls() -> Result<(), Failed> {
    let add = s!(SYNC_ADD).call::<f64>((2i64, 0.5f64)).invoke();
    let ok = s!(SYNC_THROW).call::<i64>((false,)).throws().invoke();
    let err = s!(SYNC_THROW).call::<i64>((true,)).throws().invoke();
    let bytes = s!(BYTE_COUNT).call::<i64>(("héllo\0",)).invoke();
    check(matches!(add, Ok(v) if v == 2.5), format!("add {add:?}"))?;
    check(matches!(ok, Ok(1)), format!("ok {ok:?}"))?;
    check(matches!(&err, Err(CallError::Swift(e)) if e.description().contains("code(9)")), format!("err {err:?}"))?;
    check(matches!(bytes, Ok(7)), format!("bytes {bytes:?}"))
}

/// StoreKit 2 for real: `static func Product.products<I: Collection>(for: I) async throws -> [Product]`
/// where `I.Element == String`. Outside an app bundle it throws; either way the call must come back.
fn storekit_products_reaches_the_framework() -> Result<(), Failed> {
    static STOREKIT: SwiftAsyncFn = SwiftAsyncFn::new("$s8StoreKit7ProductV8products3forSayACGx_tYaKSlRzSS7ElementRtzlFZ");
    extern "C" { fn dlopen(path: *const std::ffi::c_char, mode: i32) -> *mut std::ffi::c_void; }
    unsafe { dlopen(c"/System/Library/Frameworks/StoreKit.framework/StoreKit".as_ptr(), 1) };
    check(STOREKIT.is_available(), "Product.products(for:) not found")?;
    let product = SwiftType::from_mangled_name("8StoreKit7ProductV").ok_or("StoreKit.Product type not found")?;
    check(product.name() == "StoreKit.Product" && product.size() > 0, product.name())?;
    let h = spawn(async {
        let ids = SwiftValue::new(vec!["com.example.pro".to_string()]);
        let collection = ids.ty().conformance(&COLLECTION).unwrap();
        // `[Product]` comes back as one reference; elements are opaque here, so only count them.
        match STOREKIT.call_direct(SwiftType::from_mangled_name("Say8StoreKit7ProductVG").unwrap(), (&ids, ids.ty(), collection)).throws().await {
            Ok(products) => format!("ok: {}", products.description()),
            Err(CallError::Swift(e)) => format!("threw: {}", e.description()),
            Err(other) => format!("failed: {other:?}"),
        }
    });
    let r = common::block_with_timeout(h, Duration::from_secs(15));
    check(matches!(&r, Some(Ok(s)) if s.starts_with("ok:") || s.starts_with("threw:")), format!("got {r:?}"))
}

/// Arguments a call keeps alive are released even when the call is never run or fails before running.
fn unrun_calls_release_their_arguments() -> Result<(), Failed> {
    use apus::swift::SwiftObject;
    extern "C" { fn probe_make_flagged_object(flag: *mut bool) -> *mut std::ffi::c_void; }
    static MISSING: SwiftFn = SwiftFn::new("$s5Probe12doesNotExistyyF");
    let (mut never_run, mut unavailable, mut run) = (false, false, false);
    {
        let object = unsafe { SwiftObject::from_raw(probe_make_flagged_object(&mut never_run)) };
        drop(s!(BYTE_COUNT).call::<i64>((&object,)));
    }
    {
        let object = unsafe { SwiftObject::from_raw(probe_make_flagged_object(&mut unavailable)) };
        let r = MISSING.call::<()>((&object,)).invoke();
        check(matches!(r, Err(CallError::Unavailable(_))), format!("got {r:?}"))?;
    }
    {
        let object = unsafe { SwiftObject::from_raw(probe_make_flagged_object(&mut run)) };
        let _ = s!(SYNC_ADD).call::<f64>((2i64, 0.5f64, &object)).invoke();
    }
    check(never_run && unavailable && run, format!("released: never run {never_run}, unavailable {unavailable}, run {run}"))
}

// Declared in Swift syntax; symbols and result types are computed at compile time.
apus::swift! {
    /// `Task.sleep(nanoseconds:)` lives in a constrained extension of `Task`.
    #[swift = "static (extension in _Concurrency where Success == Never, Failure == Never) \
               _Concurrency.Task<Success, Failure>.sleep(nanoseconds: UInt64) async throws"]
    async fn task_sleep(nanoseconds: u64) throws;

    /// StoreKit 2: `static var updates: Transaction.Transactions { get }`, an AsyncSequence.
    #[swift = "static StoreKit.Transaction(struct).updates: Transaction(struct).Transactions(struct) { get }"]
    fn transaction_updates() -> SwiftValue;
}

fn extension_members_by_declaration() -> Result<(), Failed> {
    let (r, took) = in_task(async {
        let start = std::time::Instant::now();
        (task_sleep(40_000_000).await, start.elapsed())
    });
    check(matches!(r, Ok(())) && took >= Duration::from_millis(35), format!("{r:?} after {took:?}"))
}

/// The real `Transaction.updates`: obtained through a declared static getter, turned into a `SwiftStream`.
/// Nothing is awaited from it (there are no transactions in a test process).
fn storekit_transaction_updates_is_a_stream() -> Result<(), Failed> {
    use apus::swift::SwiftStream;
    extern "C" { fn dlopen(path: *const std::ffi::c_char, mode: i32) -> *mut std::ffi::c_void; }
    unsafe { dlopen(c"/System/Library/Frameworks/StoreKit.framework/StoreKit".as_ptr(), 1) };
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let outcome = transaction_updates().and_then(|updates| {
            let sequence = updates.ty().name();
            SwiftStream::new(updates).map(|stream| (sequence, stream.element_type().name()))
        });
        let _ = tx.send(outcome);
    });
    let outcome = rx.recv_timeout(Duration::from_secs(15)).map_err(|_| "StoreKit did not answer")?;
    match outcome {
        Ok((sequence, element)) => check(
            sequence == "StoreKit.Transaction.Transactions" && element == "StoreKit.VerificationResult<StoreKit.Transaction>",
            format!("{sequence} of {element}"),
        ),
        Err(error) => Err(format!("failed: {error:?}").into()),
    }
}

fn main() {
    common::run(vec![
        Trial::test("types_and_conformances", types_and_conformances),
        Trial::test("values_round_trip", values_round_trip),
        Trial::test("generic_identity", generic_identity),
        Trial::test("constrained_generics_take_witness_tables", constrained_generics_take_witness_tables),
        Trial::test("large_values_by_address", large_values_by_address),
        Trial::test("synchronous_calls", synchronous_calls),
        Trial::test("unrun_calls_release_their_arguments", unrun_calls_release_their_arguments),
        Trial::test("storekit_products_reaches_the_framework", storekit_products_reaches_the_framework),
        Trial::test("extension_members_by_declaration", extension_members_by_declaration),
        Trial::test("storekit_transaction_updates_is_a_stream", storekit_transaction_updates_is_a_stream),
    ])
}
