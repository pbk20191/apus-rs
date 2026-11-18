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

    #[swift_linked = "Res.pearTray(count: Int) -> Tray(struct)<String>"]
    fn pear_tray(count: i64) -> SwiftValue;
    #[swift_linked = "Res.Tray(struct)<T>.count: Int { get }"]
    fn count(self: &SwiftValue) -> i64;
    #[swift_linked = "Res.Tray(struct)<T>.count: Int { set }"]
    fn set_count(self: &mut SwiftValue, count: i64);
    #[swift_linked = "Res.Tray(struct)<T>.item: T { get }"]
    fn item(self: &SwiftValue) -> SwiftValue;
    #[swift_linked = "Res.Tray(struct)<T>.doubled: Int { get }"]
    fn doubled(self: &SwiftValue) -> i64;
    #[swift_linked = "mutating Res.Tray(struct)<T>.add(_: Int)"]
    fn add(self: &mut SwiftValue, n: i64);
    #[swift_linked = "Res.Tray(struct)<T>.describe<U>(_: U, _: Int) -> String"]
    fn describe(self: &SwiftValue, u: &SwiftValue, n: i64, u_type: SwiftType) -> String;
    #[swift_linked = "Res.Tray(struct)<T>.later(_: Int) async -> Int"]
    async fn later(self: &SwiftValue, n: i64) -> i64;
    #[swift_linked = "static Res.Tray(struct)<T>.size(plus: Int) -> Int"]
    fn size(n: i64, t: SwiftType) -> i64;
}

/// A member of a generic struct gets `Self`'s metadata from its receiver, after its arguments and before its own
/// generic arguments; a static member takes the type's generic arguments explicitly.
fn members_of_generic_structs() -> Result<(), Failed> {
    let mut tray = pear_tray(3).unwrap();
    check(count(&tray).unwrap() == 3 && doubled(&tray).unwrap() == 6, "getters")?;
    // A result of the type's own parameter T: its type is read from the receiver's metadata.
    check(item(&tray).unwrap().get::<String>().as_deref() == Some("pear"), "T result")?;
    set_count(&mut tray, 5).unwrap();
    add(&mut tray, 2).unwrap();
    check(count(&tray).unwrap() == 7, "setter and mutating method")?;
    let described = describe(&tray, &SwiftValue::new(1.5f64), 1, SwiftType::of::<f64>()).unwrap();
    check(described == "String Double 8", described)?;
    let later = block(spawn(async move { later(&tray, 10).await })).unwrap();
    check(matches!(later, Ok(17)), format!("async {later:?}"))?;
    check(size(1, SwiftType::of::<i64>()).unwrap() == 9, "static member")
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
        Trial::test("members_of_generic_structs", members_of_generic_structs),
    ]);
}
