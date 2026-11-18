//! Initializers (`init`) and static members: a struct or enum passes no `self`, a class passes itself as `self`.
#[path = "common/mod.rs"]
mod common;
#[path = "common/probe.rs"]
mod probe;

use apus::swift::{SwiftEnumCase, SwiftObject, SwiftType, SwiftValue};
use common::{check, Failed, Trial};

apus::swift! {
    #[swift_linked = "Res.Square(struct).init(side: Double)"]
    fn square(side: f64) -> SwiftValue;
    #[swift_linked = "Res.Grade(enum).init(rawValue: Int) -> Grade(enum)?"]
    fn grade(raw: i64) -> SwiftValue;
    #[swift_linked = "Res.Grade(enum).rawValue: Int { get }"]
    fn raw_value(self: &SwiftValue) -> i64;
    #[swift_linked = "Probe.Minted(class).init(n: Int)"]
    fn minted(self: &SwiftType, n: i64) -> SwiftObject;
    #[swift_linked = "static Probe.Minted(class).twice(_: Int) -> Minted(class)"]
    fn twice(self: &SwiftType, n: i64) -> SwiftObject;
    #[swift_linked = "Probe.Minted(class).value: Int { get }"]
    fn value(self: &SwiftObject) -> i64;
}

static HIGH: SwiftEnumCase = SwiftEnumCase::new(apus::mangle!("case Res.Grade(enum).high"));

fn value_types_pass_no_self() -> Result<(), Failed> {
    let square = square(3.0).unwrap();
    check(square.description() == "Square(side: 3.0)", square.description())?;
    // `Grade?`: Optional's case `some` is tag 0, `none` tag 1.
    let found = grade(5).unwrap();
    check(found.enum_tag() == Some(0), "Grade(rawValue: 5) is some")?;
    let high = unsafe { found.enum_payload(SwiftType::from_mangled_name("3Res5GradeO").unwrap()) }.unwrap();
    check(high.enum_tag() == HIGH.tag(), "and holds .high")?;
    check(raw_value(&high).unwrap() == 5, "whose rawValue is 5")?;
    check(grade(7).unwrap().enum_tag() == Some(1), "Grade(rawValue: 7) is nil")
}

fn classes_pass_themselves() -> Result<(), Failed> {
    let class = SwiftType::from_mangled_name("5Probe6MintedC").unwrap();
    let made = minted(&class, 3).unwrap();
    check(value(&made).unwrap() == 3, "Minted(n: 3)")?;
    check(value(&twice(&class, 4).unwrap()).unwrap() == 8, "Minted.twice(4)")
}

fn main() {
    common::run(vec![
        Trial::test("value_types_pass_no_self", value_types_pass_no_self),
        Trial::test("classes_pass_themselves", classes_pass_themselves),
    ]);
}
