//! Static protocol requirements and protocol initializers, called on a type and its conformance (`OpenedType`), and
//! `Self` results, returned by address with the receiver's type.
#[path = "common/mod.rs"]
mod common;
#[path = "common/probe.rs"]
mod probe;

use apus::swift::{spawn, Opened, OpenedType, SwiftProtocol, SwiftType, SwiftValue};
use common::{block, check, Failed, Trial};

apus::swift! {
    #[swift_linked = "Res.thingMetadata() -> Any.Type"]
    fn thing_metadata() -> SwiftType;
    #[swift_linked = "static Res.Maker(protocol).make(_: Int) -> Self"]
    fn make(self: &OpenedType, n: i64) -> SwiftValue;
    #[swift_linked = "static Res.Maker(protocol).label: String { get }"]
    fn label(self: &OpenedType) -> String;
    #[swift_linked = "Res.Maker(protocol).init(n: Int)"]
    fn new(self: &OpenedType, n: i64) -> SwiftValue;
    #[swift_linked = "static Res.Maker(protocol).count() async -> Int"]
    async fn count(self: &OpenedType) -> i64;
    #[swift_linked = "Res.Maker(protocol).n: Int { get }"]
    fn n(self: &Opened) -> i64;
    #[swift_linked = "Res.Maker(protocol).doubled() -> Self"]
    fn doubled(self: &Opened) -> SwiftValue;
}

static MAKER: SwiftProtocol = SwiftProtocol::new("$s3Res5MakerMp");

fn thing() -> OpenedType {
    thing_metadata().unwrap().conforming(&MAKER).expect("Thing is a Maker")
}

fn statics_are_called_on_the_type() -> Result<(), Failed> {
    let made = make(&thing(), 5).unwrap();
    check(made.ty().name() == "Res.Thing", made.ty().name())?;
    check(n(&made.conforming(&MAKER).unwrap()).unwrap() == 5, "make")?;
    check(label(&thing()).unwrap() == "thing", "label")?;
    let later = block(spawn(async { count(&thing()).await })).unwrap().unwrap();
    check(later == 7, "async static")
}

fn initializers_and_self_results() -> Result<(), Failed> {
    let built = new(&thing(), 9).unwrap().conforming(&MAKER).unwrap();
    check(n(&built).unwrap() == 9, "init")?;
    check(n(&doubled(&built).unwrap().conforming(&MAKER).unwrap()).unwrap() == 18, "an instance requirement's Self")?;
    check(SwiftType::of::<i64>().conforming(&MAKER).is_none(), "Int is no Maker")
}

fn main() {
    common::run(vec![
        Trial::test("statics_are_called_on_the_type", statics_are_called_on_the_type),
        Trial::test("initializers_and_self_results", initializers_and_self_results),
    ]);
}
