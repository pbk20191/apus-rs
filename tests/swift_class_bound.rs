//! Class-bound generic parameters (`T: SomeClass`, `T: AnyObject`) and metatypes: Swift passes the first as one
//! reference and the second as one metadata pointer, where an unconstrained `T` goes by address.
#[path = "common/mod.rs"]
mod common;
#[path = "common/probe.rs"]
mod probe;

use apus::swift::{CallError, Opened, SwiftObject, SwiftType, SwiftValue};
use common::{check, Failed, Trial};

apus::swift! {
    #[swift_linked = "Res.dog(name: String) -> Dog(class)"]
    fn dog(name: &str) -> SwiftObject;
    #[swift_linked = "Res.renamed<T: Animal(class)>(_: T, suffix: String) -> T"]
    fn renamed(animal: &SwiftObject, suffix: &str) -> SwiftObject;
    #[swift_linked = "Res.nameOf<T: Animal(class)>(_: T) -> String"]
    fn name_of(animal: &SwiftObject) -> String;
    #[swift_linked = "Res.same<T: AnyObject>(_: T) -> T"]
    fn same(object: &SwiftObject) -> SwiftObject;
    #[swift_linked = "Res.describeType<T>(_: T.Type) -> String"]
    fn describe_type(ty: SwiftType) -> String;
    #[swift_linked = "Res.make<T: Animal(class)>(_: T.Type, name: String) -> T"]
    fn make(ty: SwiftType, name: &str) -> SwiftObject;
    #[swift_linked = "Res.dogType() -> Animal(class).Type"]
    fn dog_type() -> SwiftType;
    #[swift_linked = "Res.anyTypeName(_: Any.Type) -> String"]
    fn any_type_name(ty: SwiftType) -> String;
    #[swift_linked = "Res.zero<T: AdditiveArithmetic>(_: T.Type) -> T"]
    fn zero(ty: SwiftType) -> SwiftValue;
    #[swift_linked = "Res.meters(_: Double) -> Foundation.Measurement(struct)<__C.NSUnitLength(class)>"]
    fn meters(value: f64) -> SwiftValue;
    #[swift_linked = "Res.labelOf<T: Named(AnyObject)>(_: T) -> String"]
    fn label_of(named: &SwiftObject) -> String;
    // The same function declared without the mark, and a protocol wrongly marked: refused before the call.
    #[swift_linked = "Res.labelOf<T: Named>(_: T) -> String"]
    fn label_of_unmarked(named: &SwiftValue) -> String;
    #[swift_linked = "Res.isTagged<T: Tagged(AnyObject)>(_: T) -> Bool"]
    fn is_tagged_marked(tagged: &SwiftObject) -> bool;
    #[swift_linked = "Res.anyNamed(_: String) -> any Named"]
    fn any_named(name: &str) -> SwiftValue;
    #[swift_linked = "Res.labelOfAny(_: any Named) -> String"]
    fn label_of_any(named: &SwiftValue) -> String;
    #[swift_linked = "Res.maybeNamed(_: String) -> (any Named)?"]
    fn maybe_named(name: &str) -> SwiftValue;
    #[swift_linked = "Res.labelOfMaybe(_: (any Named)?) -> String"]
    fn label_of_maybe(named: &SwiftValue) -> String;
    #[swift_linked = "Res.Named(protocol).label: String { get }"]
    fn named_label(self: &Opened) -> String;
    #[swift_linked = "(extension in Res) Res.Named(protocol).shout() -> String"]
    fn shout(self: &Opened) -> String;
    #[swift_linked = "Res.unitSymbol(_: __C.NSUnit(class)) -> String"]
    fn unit_symbol(unit: &SwiftObject) -> String;
    // Foundation's own: a member of a generic type whose parameter is class-bound.
    #[swift = "Foundation.Measurement(struct)<UnitType: __C.NSUnit(class)>.unit: UnitType { get }"]
    fn unit(self: &SwiftValue) -> SwiftObject;
    #[swift = "Foundation.Measurement(struct)<UnitType: __C.NSUnit(class)>.value: Double { get }"]
    fn value(self: &SwiftValue) -> f64;
}

fn members_of_class_bound_generic_types() -> Result<(), Failed> {
    let length = meters(3.5).unwrap();
    check(value(&length).unwrap() == 3.5, "value")?;
    check(unit_symbol(&unit(&length).unwrap()).unwrap() == "m", "unit")
}

fn class_bound_values_are_references() -> Result<(), Failed> {
    let rex = dog("Rex").unwrap();
    // T is the object's own (dynamic) type.
    check(name_of(&rex).unwrap() == "Dog Rex", name_of(&rex).unwrap())?;
    let renamed = renamed(&rex, "!").unwrap();
    check(name_of(&renamed).unwrap() == "Dog Rex!", "a new Dog, returned +1")?;
    check(same(&rex).unwrap().as_ptr() == rex.as_ptr(), "the same reference back")
}

/// A protocol declared `: AnyObject` makes its parameter a reference; only its descriptor says so, so the declaration
/// marks it and the call checks the mark.
fn class_bound_protocols_are_checked() -> Result<(), Failed> {
    let rex = dog("Rex").unwrap();
    check(label_of(&rex).unwrap() == "named Rex", "marked")?;
    let boxed = SwiftValue::new(7i64);
    let unmarked = label_of_unmarked(&boxed);
    check(matches!(unmarked, Err(CallError::InvalidArgument(_))), format!("unmarked: {unmarked:?}"))?;
    let mismarked = is_tagged_marked(&rex);
    check(matches!(mismarked, Err(CallError::InvalidArgument(_))), format!("mismarked: {mismarked:?}"))
}

/// `any Named` (`Named: AnyObject`) is a class existential: its reference and witness table, in registers.
fn class_existentials_are_passed_in_registers() -> Result<(), Failed> {
    let named = any_named("Rex").unwrap();
    let label = label_of_any(&named);
    check(matches!(&label, Ok(l) if l == "named Rex"), format!("{label:?}"))?;
    // Optional: the same words, nil all zero (an empty name gives nil).
    let labels = ["Fido", ""].map(|name| label_of_maybe(&maybe_named(name).unwrap()));
    check(matches!(&labels, [Ok(a), Ok(b)] if a == "named Fido" && b == "nobody"), format!("{labels:?}"))
}

/// An opened class existential: the dynamic class, and `self` passed as the reference.
fn class_existentials_open() -> Result<(), Failed> {
    let opened = any_named("Rex").unwrap().open().expect("any Named opens");
    check(opened.ty().name() == "Res.Dog", opened.ty().name())?;
    let (label, shouted) = (named_label(&opened), shout(&opened));
    check(matches!((&label, &shouted), (Ok(l), Ok(s)) if l == "named Rex" && s == "named Rex!"), format!("{label:?} {shouted:?}"))?;
    check(maybe_named("Fido").unwrap().open().is_none(), "an optional is not an existential")
}

fn metatypes_are_metadata() -> Result<(), Failed> {
    check(describe_type(SwiftType::of::<i64>()).unwrap() == "Int", "T.Type")?;
    check(any_type_name(SwiftType::of::<String>()).unwrap() == "String", "Any.Type")?;
    let dog_type = dog_type().unwrap();
    check(dog_type.name() == "Res.Dog", dog_type.name())?;
    // `T.Type` gives T itself, for a class-bound result and for one returned by address.
    check(name_of(&make(dog_type, "Fido").unwrap()).unwrap() == "Dog Fido", "make")?;
    check(zero(SwiftType::of::<f64>()).unwrap().description() == "0.0", "zero")
}

fn main() {
    common::run(vec![
        Trial::test("class_bound_values_are_references", class_bound_values_are_references),
        Trial::test("metatypes_are_metadata", metatypes_are_metadata),
        Trial::test("class_bound_protocols_are_checked", class_bound_protocols_are_checked),
        Trial::test("class_existentials_are_passed_in_registers", class_existentials_are_passed_in_registers),
        Trial::test("class_existentials_open", class_existentials_open),
        Trial::test("members_of_class_bound_generic_types", members_of_class_bound_generic_types),
    ]);
}
