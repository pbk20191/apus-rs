//! Class-bound generic parameters (`T: SomeClass`, `T: AnyObject`) and metatypes: Swift passes the first as one
//! reference and the second as one metadata pointer, where an unconstrained `T` goes by address.
#[path = "common/mod.rs"]
mod common;
#[path = "common/probe.rs"]
mod probe;

use apus::swift::{SwiftObject, SwiftType, SwiftValue};
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
        Trial::test("members_of_class_bound_generic_types", members_of_class_bound_generic_types),
    ]);
}
