//! `any P` values: opened (inline or boxed in the existential container) and their requirements called through the
//! protocol's dispatch thunks, as for Apple's frameworks (tests/support/resilient.swift is built for library evolution).
#[path = "common/mod.rs"]
mod common;
#[path = "common/probe.rs"]
mod probe;

use apus::swift::{spawn, Opened, SwiftValue};
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
    check(SwiftValue::new(1i64).open().is_none(), "an Int is not an existential")
}

fn main() {
    common::run(vec![Trial::test("opened_values", opened_values)]);
}
