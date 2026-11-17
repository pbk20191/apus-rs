//! `any P` values: opened (inline or boxed in the existential container) and their requirements called through the
//! protocol's dispatch thunks, as for Apple's frameworks (tests/support/resilient.swift is built for library evolution).
#[path = "common/mod.rs"]
mod common;
#[path = "common/probe.rs"]
mod probe;

use apus::swift::{spawn, DirectValue, Opened, SwiftValue};
use common::{block, check, Failed, Trial};

apus::swift! {
    #[swift_linked = "Res.makeSquare(side: Double) -> any Shape"]
    fn make_square(side: f64) -> SwiftValue;
    #[swift_linked = "Res.makePolygon() -> any Shape"]
    fn make_polygon() -> SwiftValue;
    #[swift_linked = "Res.Shape(protocol).area() -> Double"]
    fn area(self: &Opened) -> f64;
    #[swift_linked = "Res.Shape(protocol).label() -> String"]
    fn label(self: &Opened) -> String;
    #[swift_linked = "Res.Shape(protocol).grown(by: Double) async -> Double"]
    async fn grown(self: &Opened, factor: f64) -> f64;
    #[swift_linked = "Res.Shape(protocol).stretch() -> Double"]
    fn stretch(self: &Opened) -> f64;
    #[swift_linked = "Probe.makeAnyError() -> any Error"]
    fn make_any_error() -> DirectValue;
}

fn opened_values() -> Result<(), Failed> {
    let square = make_square(3.0).unwrap();
    let polygon = make_polygon().unwrap();
    let (s, p) = (square.open().expect("any Shape opens"), polygon.open().expect("boxed any Shape opens"));
    check(s.ty().name() == "Res.Square" && p.ty().name() == "Res.Polygon", format!("{} {}", s.ty().name(), p.ty().name()))?;
    check(area(&s).unwrap() == 9.0 && label(&s).unwrap() == "square", "inline value")?;
    check(area(&p).unwrap() == 15.0 && label(&p).unwrap() == "polygon", "boxed value")?;
    let grown = block(spawn(async move { (grown(&s, 2.0).await.unwrap(), grown(&p, 2.0).await.unwrap()) })).unwrap();
    check(grown == (36.0, 30.0), format!("async requirement {grown:?}"))?;
    check(SwiftValue::new(1i64).open().is_none(), "an Int is not an existential")?;
    // `any Error` is a one-word box, not an opaque container.
    let error = make_any_error().unwrap().0;
    check(error.open().is_none(), format!("{} opened", error.ty().name()))
}

/// A call keeps its opened receiver alive: here the temporary `Opened` (and its box) is gone before the call runs.
fn temporary_receiver() -> Result<(), Failed> {
    let result = block(spawn(async {
        let call = grown(&make_polygon().unwrap().open().unwrap(), 2.0);
        let garbage: Vec<Vec<u8>> = (0..256).map(|_| vec![0xff; 56]).collect();
        let result = call.await;
        drop(garbage);
        result
    }))
    .unwrap();
    check(matches!(result, Ok(r) if r == 30.0), format!("{result:?}"))
}

/// A receiver is passed by value, as `&SwiftValue` receivers are: a `mutating` requirement changes a copy, never the
/// value it was opened from (a boxed value shares its box with that value).
fn mutating_requirements_change_a_copy() -> Result<(), Failed> {
    let original = make_polygon().unwrap();
    let opened = original.open().unwrap();
    check(matches!(stretch(&opened), Ok(25.0)), "stretch")?;
    let after = original.open().map(|o| area(&o));
    check(matches!(after, Some(Ok(15.0))), format!("the original changed: {after:?}"))?;
    check(matches!(area(&opened), Ok(15.0)), "the opened value changed")
}

fn main() {
    common::run(vec![
        Trial::test("opened_values", opened_values),
        Trial::test("temporary_receiver", temporary_receiver),
        Trial::test("mutating_requirements_change_a_copy", mutating_requirements_change_a_copy),
    ]);
}
