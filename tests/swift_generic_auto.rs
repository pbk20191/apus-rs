//! `swift!` fills a generic function's type metadata, witness tables and typed-throws error slot itself when the Rust
//! signature lists only the Swift parameters.
#[path = "common/mod.rs"]
mod common;
#[path = "common/probe.rs"]
mod probe;

use apus::swift::{spawn, CallError, SwiftType, SwiftValue};
use common::{block, check, Failed, Trial};

apus::swift! {
    #[swift_linked = "Probe.echo<T>(_: T) async -> T"]
    async fn echo(x: &SwiftValue) -> SwiftValue;
    #[swift_linked = "Probe.describeIt<T: CustomStringConvertible>(_: T, times: Int) async throws -> String"]
    async fn describe_it(x: &SwiftValue, times: i64) throws -> String;
    #[swift_linked = "Probe.makeBoom(code: Int) -> Boom(struct)"]
    fn make_boom(code: i64) -> SwiftValue;
    #[swift_linked = "Probe.genericTypedSync<E: Error>(_: Bool, _: E) throws(E) -> Int"]
    fn generic_typed_sync(fail: bool, error: &SwiftValue) -> i64;
}

fn automatic_generic_arguments() -> Result<(), Failed> {
    let (echoed, described) = block(spawn(async {
        let n = SwiftValue::new(42i64);
        let echoed = echo(&n).await.map(|v| v.get::<i64>());
        let described = describe_it(&SwiftValue::new("hi"), 2).await;
        (echoed, described)
    }))
    .unwrap();
    check(matches!(echoed, Ok(Some(42))), format!("echo {echoed:?}"))?;
    check(matches!(&described, Ok(s) if s.contains("hi")), format!("describe {described:?}"))?;
    let boom = make_boom(3).unwrap();
    check(matches!(generic_typed_sync(false, &boom), Ok(42)), "typed throws: return")?;
    check(matches!(generic_typed_sync(true, &boom), Err(CallError::Typed(v)) if v.ty() == boom.ty()), "typed throws: the slot comes from E")
}

fn a_value_that_does_not_conform() -> Result<(), Failed> {
    let boom = make_boom(3).unwrap(); // Boom is not CustomStringConvertible
    let result = block(spawn(async move { describe_it(&boom, 1).await })).unwrap();
    check(matches!(&result, Err(CallError::NotConforming { .. })), format!("{result:?}"))?;
    let _ = SwiftType::of::<i64>();
    Ok(())
}

fn main() {
    common::run(vec![
        Trial::test("automatic_generic_arguments", automatic_generic_arguments),
        Trial::test("a_value_that_does_not_conform", a_value_that_does_not_conform),
    ]);
}
