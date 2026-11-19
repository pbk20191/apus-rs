//! Errors thrown by Swift: their dynamic type, a copy of the value, and NSError's domain and code.
#[path = "common/mod.rs"]
mod common;
#[path = "common/probe.rs"]
mod probe;

use apus::swift::{CallError, SwiftError, SwiftProtocol, SwiftType, SwiftValue, WitnessTable};
use common::{check, Failed, Trial};

apus::swift! {
    #[swift_linked = "Probe.throwStruct(code: Int) throws -> Int"]
    fn throw_struct(code: i64) throws -> i64;
    #[swift_linked = "Probe.throwNSError(code: Int) throws -> Int"]
    fn throw_ns_error(code: i64) throws -> i64;
    #[swift_linked = "Probe.syncThrow(_: Bool) throws -> Int"]
    fn sync_throw(fail: bool) throws -> i64;
    #[swift_linked = "Probe.structErrorCode<E: Error>(_: E) -> Int"]
    fn struct_error_code(error: &SwiftValue, ty: SwiftType, error_witness: WitnessTable) -> i64;
    #[swift_linked = "Probe.makeError(code: Int) -> any Error"]
    fn make_error(code: i64) -> SwiftError;
    #[swift_linked = "Probe.anyErrorCode(_: any Error) -> Int"]
    fn any_error_code(error: &SwiftError) -> i64;
    #[swift_linked = "Probe.maybeError(code: Int) -> (any Error)?"]
    fn maybe_error(code: i64) -> Option<SwiftError>;
    #[swift_linked = "Probe.maybeErrorCode(_: (any Error)?) -> Int"]
    fn maybe_error_code(error: Option<&SwiftError>) -> i64;
}

static ERROR: SwiftProtocol = SwiftProtocol::new("$ss5ErrorMp");

fn thrown(result: Result<i64, CallError>) -> SwiftError {
    match result {
        Err(CallError::Swift(error)) => error,
        other => panic!("expected a Swift error, got {other:?}"),
    }
}

fn downcast_to_the_thrown_type() -> Result<(), Failed> {
    let error = thrown(throw_struct(5));
    let ty = SwiftType::from_mangled_name("5Probe11StructErrorV").expect("StructError");
    check(error.ty() == ty, format!("dynamic type {:?}", error.ty()))?;
    let value = error.downcast(ty).expect("a StructError");
    let witness = ty.conformance(&ERROR).expect("StructError: Error");
    check(struct_error_code(&value, ty, witness).unwrap() == 5, "the copied value")?;
    check(error.downcast(SwiftType::of::<String>()).is_none(), "not a String")?;
    check(error.downcast_value::<i64>().is_none(), "not an Int")?;
    check(error.description().contains("code: 5"), "the error is still usable after a downcast")
}

fn domain_and_code() -> Result<(), Failed> {
    let ns = thrown(throw_ns_error(42));
    check(ns.domain() == "probe.domain" && ns.code() == 42, format!("NSError {} {}", ns.domain(), ns.code()))?;
    // An NSError's value lives in the scratch slot swift_getErrorValue fills: still valid while it is described.
    check(ns.description().contains("probe.domain"), ns.description())?;
    let swift = thrown(throw_struct(5));
    check(swift.domain() == "Probe.StructError" && swift.code() == 1, format!("struct {} {}", swift.domain(), swift.code()))?;
    let enumeration = thrown(sync_throw(true));
    check(enumeration.domain() == "Probe.DemoError" && enumeration.code() == 0, format!("enum {} {}", enumeration.domain(), enumeration.code()))
}

/// `any Error` is one box reference, passed and returned in a register.
fn any_error_in_registers() -> Result<(), Failed> {
    let error = make_error(7).unwrap();
    check(error.description().contains("made"), error.description())?;
    let code = any_error_code(&error);
    check(matches!(code, Ok(7)), format!("{code:?}"))?;
    // A thrown error passed back.
    let thrown = thrown(throw_struct(5));
    check(matches!(any_error_code(&thrown), Ok(5)), "a thrown error")?;
    // Optional: nil is a null box.
    let some = maybe_error(3).unwrap();
    check(some.is_some() && maybe_error(0).unwrap().is_none(), "(any Error)? results")?;
    let codes = (maybe_error_code(some.as_ref()), maybe_error_code(None));
    check(matches!(codes, (Ok(3), Ok(0))), format!("{codes:?}"))
}

fn main() {
    common::run(vec![
        Trial::test("downcast_to_the_thrown_type", downcast_to_the_thrown_type),
        Trial::test("domain_and_code", domain_and_code),
        Trial::test("any_error_in_registers", any_error_in_registers),
    ]);
}
