//! `some P` results: returned by address with the actual type the runtime gives from the declaration's opaque type
//! descriptor, then usable as a `P` (`conforming`).
#[path = "common/mod.rs"]
mod common;
#[path = "common/probe.rs"]
mod probe;

use apus::swift::{spawn, Opened, SwiftProtocol, SwiftValue};
use common::{block, check, Failed, Trial};

apus::swift! {
    #[swift_linked = "Res.makeSomeShape(side: Double) -> some Shape"]
    fn make_some_shape(side: f64) -> SwiftValue;
    #[swift_linked = "Res.makeSomeShapeLater(side: Double) async -> some Shape"]
    async fn make_some_shape_later(side: f64) -> SwiftValue;
    #[swift_linked = "Res.ShapeMaker(struct).init()"]
    fn shape_maker() -> SwiftValue;
    #[swift_linked = "Res.ShapeMaker(struct).polygon: some Shape { get }"]
    fn polygon(self: &SwiftValue) -> SwiftValue;
    #[swift_linked = "(extension in Res) Res.Shape(protocol).enlarged() -> some Shape"]
    fn enlarged(self: &Opened) -> SwiftValue;
    #[swift_linked = "Res.Shape(protocol).area() -> Double"]
    fn area(self: &Opened) -> f64;
}

static SHAPE: SwiftProtocol = SwiftProtocol::new("$s3Res5ShapeMp");

fn results_have_their_actual_type() -> Result<(), Failed> {
    let square = make_some_shape(3.0).unwrap();
    check(square.ty().name() == "Res.Square", square.ty().name())?;
    let square = square.conforming(&SHAPE).expect("a Shape");
    check(area(&square).unwrap() == 9.0, "area of the square")?;
    let polygon = polygon(&shape_maker().unwrap()).unwrap();
    check(polygon.ty().name() == "Res.Polygon", polygon.ty().name())?;
    // A protocol extension method: the opaque type's generic arguments are Self and its conformance.
    let bigger = enlarged(&square).unwrap().conforming(&SHAPE).expect("a Shape");
    check(area(&bigger).unwrap() == 81.0, "enlarged: a square of side 9")?;
    let later = block(spawn(async { make_some_shape_later(2.0).await })).unwrap().unwrap();
    check(area(&later.conforming(&SHAPE).unwrap()).unwrap() == 4.0, "async")
}

fn main() {
    common::run(vec![Trial::test("results_have_their_actual_type", results_have_their_actual_type)]);
}
