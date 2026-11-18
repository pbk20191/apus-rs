//! Setters, subscripts, `mutating` methods and `inout` parameters: a struct's `self` and an `inout` argument are the
//! Rust value itself (`&mut SwiftValue`), and a new value is consumed.
#[path = "common/mod.rs"]
mod common;
#[path = "common/probe.rs"]
mod probe;

use apus::swift::{SwiftObject, SwiftType, SwiftValue};
use common::{check, Failed, Trial};

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

fn main() {
    common::run(vec![
        Trial::test("struct_setters_change_the_value", struct_setters_change_the_value),
        Trial::test("class_setters_and_inout_arguments", class_setters_and_inout_arguments),
    ]);
}
