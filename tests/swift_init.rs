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
    #[swift_linked = "Probe.Minted(class).init(n: Int)"]
    fn minted_implicit(n: i64) -> SwiftObject;
    #[swift = "static Probe.Minted(class).twice(_: Int) -> Minted(class)"]
    fn twice_implicit(n: i64) -> SwiftObject;
    #[swift_linked = "Probe.Minted(class).value: Int { get }"]
    fn value(self: &SwiftObject) -> i64;
    #[swift_linked = "Probe.mintedGone(_: Int) -> Bool"]
    fn minted_gone(n: i64) -> bool;
    #[swift_linked = "Probe.Holder(class).init(s: String, o: Minted(class))"]
    fn holder(self: &SwiftType, s: &str, o: &SwiftObject) -> SwiftObject;
    #[swift_linked = "Probe.Holder(class).text: String { get }"]
    fn text(self: &SwiftObject) -> String;
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

/// Without `self`, the declared class's own metadata is `self`.
fn classes_pass_themselves_implicitly() -> Result<(), Failed> {
    check(value(&minted_implicit(7).unwrap()).unwrap() == 7, "Minted(n: 7)")?;
    check(value(&twice_implicit(4).unwrap()).unwrap() == 8, "Minted.twice(4)")
}

fn initializer_arguments_are_consumed() -> Result<(), Failed> {
    let minted_class = SwiftType::from_mangled_name("5Probe6MintedC").unwrap();
    let holder_class = SwiftType::from_mangled_name("5Probe6HolderC").unwrap();
    let kept = minted(&minted_class, 1001).unwrap();
    let long = "a string too long to be stored inline in the String itself";
    for _ in 0..3 {
        let held = holder(&holder_class, long, &kept).unwrap();
        check(text(&held).unwrap() == long, "the held String")?;
    }
    // Each holder released its own references: had they been borrowed, `kept` would be gone by now.
    check(!minted_gone(1001).unwrap(), "the kept object is alive")?;
    check(value(&kept).unwrap() == 1001, "and usable")?;
    drop(kept);
    check(minted_gone(1001).unwrap(), "until its last reference goes")
}

fn main() {
    common::run(vec![
        Trial::test("value_types_pass_no_self", value_types_pass_no_self),
        Trial::test("classes_pass_themselves_implicitly", classes_pass_themselves_implicitly),
        Trial::test("classes_pass_themselves", classes_pass_themselves),
        Trial::test("initializer_arguments_are_consumed", initializer_arguments_are_consumed),
    ]);
}
