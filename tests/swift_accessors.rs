//! Setters, subscripts, `mutating` methods and `inout` parameters: a struct's `self` and an `inout` argument are the
//! Rust value itself (`&mut SwiftValue`), and a new value is consumed.
#[path = "common/mod.rs"]
mod common;
#[path = "common/probe.rs"]
mod probe;

use apus::swift::{spawn, CallError, SwiftObject, SwiftType, SwiftValue};
use common::{block, check, Failed, Trial};

apus::swift! {
    #[swift_linked = "Res.Bag(struct).init()"]
    fn bag() -> SwiftValue;
    #[swift_linked = "Res.Bag(struct).name: String { get }"]
    fn name(self: &SwiftValue) -> String;
    #[swift_linked = "Res.Bag(struct).name: String { set }"]
    fn set_name(self: &mut SwiftValue, name: &str);
    #[swift_linked = "Res.Bag(struct).count: Int { get }"]
    fn count(self: &SwiftValue) -> i64;
    #[swift_linked = "Res.Bag(struct).subscript(_: Int) -> Int { get }"]
    fn item(self: &SwiftValue, i: i64) -> i64;
    #[swift_linked = "Res.Bag(struct).subscript(_: Int) -> Int { set }"]
    fn set_item(self: &mut SwiftValue, value: i64, i: i64);
    #[swift_linked = "mutating Res.Bag(struct).add(_: Int)"]
    fn add(self: &mut SwiftValue, n: i64);
    #[swift_linked = "static Res.Bag(struct).total: Int { get }"]
    fn total() -> i64;
    #[swift_linked = "static Res.Bag(struct).total: Int { set }"]
    fn set_total(total: i64);
    #[swift_linked = "Res.swapInts(_: inout Int, _: inout Int)"]
    fn swap_ints(a: &mut SwiftValue, b: &mut SwiftValue);
    #[swift_linked = "Res.Shelf(class).init()"]
    fn shelf(self: &SwiftType) -> SwiftObject;
    #[swift_linked = "Res.Shelf(class).label: String { get }"]
    fn label(self: &SwiftObject) -> String;
    #[swift_linked = "Res.Shelf(class).label: String { set }"]
    fn set_label(self: &SwiftObject, label: &str);
    #[swift_linked = "Res.Shelf(class).subscript(_: String) -> String { get }"]
    fn shelf_item(self: &SwiftObject, key: &str) -> String;
    #[swift_linked = "Res.Shelf(class).subscript(_: String) -> String { set }"]
    fn set_shelf_item(self: &SwiftObject, value: &str, key: &str);
    #[swift_linked = "Res.Meter(struct).init(_: Double)"]
    fn meter(value: f64) -> SwiftValue;
    #[swift_linked = "Res.Meter(struct).value: Double { get }"]
    fn meter_value(self: &SwiftValue) -> f64;
    #[swift_linked = "Res.Meter(struct).slowDouble: Double { get async }"]
    async fn slow_double(self: &SwiftValue) -> f64;
    #[swift_linked = "Res.Meter(struct).checked: Double { get throws }"]
    fn checked(self: &SwiftValue) throws -> f64;
    #[swift_linked = "Res.Meter(struct).subscript(scale: Double) -> Double { get async throws }"]
    async fn scaled(self: &SwiftValue, s: f64) throws -> f64;
    #[swift_linked = "static Res.Meter(struct).+(_: Meter(struct), _: Meter(struct)) -> Meter(struct)"]
    fn plus(a: &SwiftValue, b: &SwiftValue) -> SwiftValue;
    #[swift_linked = "static prefix Res.Meter(struct).-(_: Meter(struct)) -> Meter(struct)"]
    fn negate(a: &SwiftValue) -> SwiftValue;
    #[swift_linked = "Res.~~(_: Meter(struct), _: Meter(struct)) -> Bool"]
    fn about(a: &SwiftValue, b: &SwiftValue) -> bool;
}

const LONG: &str = "a string too long to be stored inline in the String itself";

fn struct_setters_change_the_value() -> Result<(), Failed> {
    let mut bag = bag().unwrap();
    for round in 0..3 {
        let name = format!("{LONG} {round}");
        set_name(&mut bag, &name).unwrap();
        check(self::name(&bag).unwrap() == name, "the name just set")?;
    }
    add(&mut bag, 3).unwrap();
    check(count(&bag).unwrap() == 3, "add(3)")?;
    set_item(&mut bag, 10, 2).unwrap();
    check(count(&bag).unwrap() == 5 && item(&bag, 2).unwrap() == 10, "bag[2] = 10")?;
    set_total(7).unwrap();
    check(total().unwrap() == 7, "static total")
}

fn class_setters_and_inout_arguments() -> Result<(), Failed> {
    let shelf = shelf(&SwiftType::from_mangled_name("3Res5ShelfC").unwrap()).unwrap();
    for round in 0..3 {
        let text = format!("{LONG} {round}");
        set_label(&shelf, &text).unwrap();
        set_shelf_item(&shelf, &text, "k").unwrap();
        check(label(&shelf).unwrap() == text && shelf_item(&shelf, "k").unwrap() == text, "label and shelf[k]")?;
    }
    let (mut a, mut b) = (SwiftValue::new(1i64), SwiftValue::new(2i64));
    swap_ints(&mut a, &mut b).unwrap();
    check(a.get::<i64>() == Some(2) && b.get::<i64>() == Some(1), "swapped in place")
}

fn effectful_getters_and_operators() -> Result<(), Failed> {
    let (one, two) = (meter(1.5).unwrap(), meter(2.0).unwrap());
    let sum = plus(&one, &two).unwrap();
    check(meter_value(&sum).unwrap() == 3.5, "1.5 + 2.0")?;
    let negative = negate(&sum).unwrap();
    check(meter_value(&negative).unwrap() == -3.5, "-(3.5)")?;
    check(about(&sum, &meter(3.5004).unwrap()).unwrap(), "3.5 ~~ 3.5004")?;
    check(checked(&sum).unwrap() == 3.5, "checked")?;
    check(matches!(checked(&negative), Err(CallError::Swift(_))), "a negative meter throws")?;
    let (slow, scaled_ok, scaled_err) = block(spawn(async move {
        (slow_double(&sum).await.unwrap(), scaled(&sum, 2.0).await.unwrap(), scaled(&sum, 0.0).await.is_err())
    }))
    .unwrap();
    check(slow == 7.0 && scaled_ok == 7.0 && scaled_err, "get async, subscript get async throws")
}

fn main() {
    common::run(vec![
        Trial::test("struct_setters_change_the_value", struct_setters_change_the_value),
        Trial::test("class_setters_and_inout_arguments", class_setters_and_inout_arguments),
        Trial::test("effectful_getters_and_operators", effectful_getters_and_operators),
    ]);
}
