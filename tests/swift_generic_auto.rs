//! `swift!` fills a generic function's type metadata, witness tables and typed-throws error slot itself when the Rust
//! signature lists only the Swift parameters.
#[path = "common/mod.rs"]
mod common;
#[path = "common/probe.rs"]
mod probe;

use apus::swift::{spawn, CallError, DirectValue, SwiftIter, SwiftObject, SwiftType, SwiftValue};
use common::{block, check, Failed, Trial};

apus::swift! {
    #[swift_linked = "Probe.echo<T>(_: T) async -> T"]
    async fn echo(x: &SwiftValue) -> SwiftValue;
    #[swift_linked = "Probe.describeIt<T: CustomStringConvertible>(_: T, times: Int) async throws -> String"]
    async fn describe_it(x: &SwiftValue, times: i64) throws -> String;
    #[swift_linked = "Probe.makeBoom(code: Int) -> Boom(struct)"]
    fn make_boom(code: i64) -> SwiftValue;
    #[swift_linked = "Probe.genericTypedSync<E: Error>(_: Bool, _: E) throws(E) -> Int"]
    fn generic_typed_sync(fail: bool, error: &SwiftValue) -> i64;

    #[swift_linked = "Res.pearTray(count: Int) -> Tray(struct)<String>"]
    fn pear_tray(count: i64) -> SwiftValue;
    #[swift_linked = "Res.Tray(struct)<T>.count: Int { get }"]
    fn count(self: &SwiftValue) -> i64;
    #[swift_linked = "Res.Tray(struct)<T>.count: Int { set }"]
    fn set_count(self: &mut SwiftValue, count: i64);
    #[swift_linked = "Res.Tray(struct)<T>.item: T { get }"]
    fn item(self: &SwiftValue) -> SwiftValue;
    #[swift_linked = "Res.Tray(struct)<T>.doubled: Int { get }"]
    fn doubled(self: &SwiftValue) -> i64;
    #[swift_linked = "mutating Res.Tray(struct)<T>.add(_: Int)"]
    fn add(self: &mut SwiftValue, n: i64);
    #[swift_linked = "Res.Tray(struct)<T>.describe<U>(_: U, _: Int) -> String"]
    fn describe(self: &SwiftValue, u: &SwiftValue, n: i64, u_type: SwiftType) -> String;
    #[swift_linked = "Res.Tray(struct)<T>.later(_: Int) async -> Int"]
    async fn later(self: &SwiftValue, n: i64) -> i64;
    #[swift_linked = "static Res.Tray(struct)<T>.size(plus: Int) -> Int"]
    fn size(n: i64, t: SwiftType) -> i64;

    #[swift_linked = "Res.Tray(struct)<T>.describe<U>(_: U, _: Int) -> String", T = "Swift.String", U = "Swift.Double"]
    fn describe_bound(self: &SwiftValue, u: &SwiftValue, n: i64) -> String;
    #[swift_linked = "Res.Tray(struct)<T>.describe<U>(_: U, _: Int) -> String", T = "Swift.Int", U = "Swift.Double"]
    fn describe_int_tray(self: &SwiftValue, u: &SwiftValue, n: i64) -> String;
    #[swift_linked = "static Res.Tray(struct)<T>.size(plus: Int) -> Int", T = "Swift.Int"]
    fn size_of_int_tray(n: i64) -> i64;
    #[swift_linked = "Res.holdInt(_: Int) -> Holder(class)<Int>"]
    fn hold_int(n: i64) -> SwiftObject;
    #[swift_linked = "Res.Holder(class)<T>.show<U>(_: U) -> String", T = "Swift.Int", U = "Swift.Bool"]
    fn show(self: &SwiftObject, u: &SwiftValue) -> String;
    #[swift_linked = "Res.Holder(class)<T>.show<U>(_: U) -> String", T = "Swift.String", U = "Swift.Bool"]
    fn show_string_holder(self: &SwiftObject, u: &SwiftValue) -> String;
    #[swift_linked = "static Res.Holder(class)<T>.kind() -> String", T = "Swift.Int"]
    fn holder_kind() -> String;
    #[swift_linked = "Res.rank<T: Hashable & Comparable>(_: T, _: T) -> Int", T = "Swift.Int"]
    fn rank(a: &SwiftValue, b: &SwiftValue) -> i64;
    #[swift_linked = "Res.joined<S: Sequence>(_: S) -> String where S.Element == String", S = "[Swift.String]"]
    fn joined(s: &SwiftValue) -> String;
    #[swift_linked = "Res.joined<S: Sequence>(_: S) -> String where S.Element == String", S = "[Swift.Int]"]
    fn joined_ints(s: &SwiftValue) -> String; // refused before the call: [Int]'s Element is not String
    #[swift_linked = "Res.described<S: Sequence>(_: S) -> String where S.Element: CustomStringConvertible", S = "[Swift.Int]"]
    fn described(s: &SwiftValue) -> String;
    #[swift_linked = "Res.renamed<T: Animal(class)>(_: T, suffix: String) -> T", T = "Probe.Minted(class)"]
    fn renamed_minted(x: &SwiftObject, suffix: &str) -> SwiftObject; // refused before the call: Minted is no Animal
    #[swift_linked = "Probe.genericTypedSync<E: Error>(_: Bool, _: E) throws(E) -> Int", E = "Probe.Boom(struct)"]
    fn typed_bound(fail: bool, error: &SwiftValue) -> i64;

    #[swift_linked = "Res.sumText<T: AdditiveArithmetic>(_: [T]) -> String", T = "Swift.Double"]
    fn sum_text(values: Vec<f64>) -> String;
    #[swift = "Res.sumOf<T: AdditiveArithmetic>(_: [T]) -> T", T = "Swift.Int"]
    fn sum_of(values: Vec<i64>) -> SwiftValue;
    #[swift = "Res.sumOf<T: AdditiveArithmetic>(_: [T]) -> T", T = "Res.Nope(struct)"]
    fn sum_of_nope(values: Vec<i64>) -> SwiftValue; // refused before the call: Nope is not AdditiveArithmetic
    #[swift_linked = "Res.labelOf<T: Named(AnyObject)>(_: T) -> String", T = "Res.Dog(class)"]
    fn label_of_dog(x: &SwiftObject) -> String;
    #[swift_linked = "Res.dog(name: String) -> Dog(class)"]
    fn dog(name: &str) -> SwiftObject;
    #[swift = "Res.doubled<T: AdditiveArithmetic>(_: T) -> T", T = "Swift.Int"]
    fn doubled_int(x: &SwiftValue) -> SwiftValue;
    #[swift_linked = "Probe.Minted(class).init(n: Int)"]
    fn minted(n: i64) -> SwiftObject;
    #[swift_linked = "Res.nopeType() -> Any.Type"]
    fn nope_type() -> SwiftType;
    #[swift_linked = "Res.dogs(_: [String]) -> [Dog(class)]"]
    fn dogs(names: Vec<String>) -> DirectValue; // an Array: returned in registers
    #[swift_linked = "Res.Animal(class).name: String { get }"]
    fn animal_name(self: &SwiftObject) -> String;
    #[swift_linked = "Res.Meter(struct).init(_: Double)"]
    fn meter(value: f64) -> SwiftValue;
    #[swift_linked = "Res.Meter(struct).value: Double { get }"]
    fn meter_value(self: &SwiftValue) -> f64;
}

// Typed wrappers: receivers, arguments and results checked against the declarations at compile time.
apus::swift! {
    class Animal = "Res.Animal(class)";
    class Dog = "Res.Dog(class)";
    struct Meter = "Res.Meter(struct)";

    impl Animal {
        #[swift_linked = "Res.Animal(class).name: String { get }"]
        fn name(&self) -> String;
    }
    impl Meter {
        #[swift_linked = "Res.Meter(struct).init(_: Double)"]
        fn new(value: f64) -> Meter;
        #[swift_linked = "Res.Meter(struct).value: Double { get }"]
        fn value(&self) -> f64;
    }
    class Minted = "Probe.Minted(class)";
    impl Dog {
        #[swift_linked = "Res.Animal(class).name: String { get }"]
        fn name(&self) -> String; // a superclass's member: checked by cast when called
    }
    impl Minted {
        #[swift_linked = "Res.Animal(class).name: String { get }"]
        fn name(&self) -> String; // compiles (both classes), refused when called
    }
    #[swift_linked = "Res.dogs(_: [String]) -> [Dog(class)]"]
    fn dog_list(names: Vec<String>) -> Vec<Dog>;
    #[swift_linked = "Res.labelOf<T: Named(AnyObject)>(_: T) -> String", T = "Res.Dog(class)"]
    fn label_of(dog: &Dog) -> String;
}

fn typed_wrappers() -> Result<(), Failed> {
    check(Meter::new(2.5).unwrap().value().unwrap() == 2.5, "Meter(2.5).value")?;
    let dogs = dog_list(vec!["rex".into(), "fido".into()]).unwrap();
    check(dogs.len() == 2 && label_of(&dogs[1]).unwrap() == "named fido", "[Dog] as Vec<Dog>")?;
    // From a type-erased value: checked when wrapped. A Dog is an Animal; a Minted is not, nor a Boom a Meter.
    let animal = Animal::try_from(dogs[0].clone().into_inner()).unwrap();
    check(animal.name().unwrap() == "rex", "a Dog as an Animal")?;
    check(matches!(Animal::try_from(minted(1).unwrap()), Err(CallError::InvalidArgument(_))), "a Minted is no Animal")?;
    check(matches!(Meter::try_from(make_boom(1).unwrap()), Err(CallError::InvalidArgument(_))), "a Boom is no Meter")?;
    // Another class's wrapper as the receiver: checked by cast.
    check(dogs[0].name().unwrap() == "rex", "Dog.name, an Animal's")?;
    let minted = Minted::try_from(minted(2).unwrap()).unwrap();
    check(matches!(minted.name(), Err(CallError::InvalidArgument(_))), "a Minted is no Animal")
}

// Kinds left out: the module's symbol graph gives them (`Named` is class-bound, so `T` is a reference).
apus::swift! {
    #[swift_linked = "Res.Meter.value: Double { get }"]
    fn meter_value_bare(self: &SwiftValue) -> f64;
    #[swift_linked = "Res.labelOf<T: Named>(_: T) -> String", T = "Res.Dog"]
    fn label_of_bare(dog: &Dog) -> String;
}

// Declarations by name: the rest comes from the module's symbol graph.
apus::swift! {
    #[swift_linked = "Res.Meter.init(_:)"]
    fn meter_named(value: f64) -> SwiftValue;
    #[swift_linked = "Res.Meter.value { get }"]
    fn meter_value_named(self: &SwiftValue) -> f64;
    #[swift_linked = "Res.Tray.add(_:)", T = "Swift.String"]
    fn add_named(self: &mut SwiftValue, n: i64);
    #[swift_linked = "Res.Tray.size(plus:)", T = "Swift.String"]
    fn size_named(n: i64) -> i64;
    #[swift_linked = "Res.sumOf(_:)", T = "Swift.Int"]
    fn sum_of_named(values: Vec<i64>) -> SwiftValue;
    #[swift_linked = "Res.labelOf(_:)", T = "Res.Dog"]
    fn label_of_named(dog: &Dog) -> String;
    #[swift_linked = "Res.renamed(_:suffix:)", T = "Res.Dog"]
    fn renamed_named(dog: &SwiftObject, suffix: &str) -> SwiftObject;
}

fn declarations_by_name() -> Result<(), Failed> {
    let meter = meter_named(2.0).unwrap();
    check(meter_value_named(&meter).unwrap() == 2.0, "Res.Meter.init(_:), .value")?;
    let mut tray = pear_tray(1).unwrap();
    add_named(&mut tray, 2).unwrap();
    check(count(&tray).unwrap() == 3, "Res.Tray.add(_:), mutating")?;
    check(size_named(4).unwrap() == size(4, SwiftType::of::<String>()).unwrap(), "static Res.Tray.size(plus:)")?;
    check(sum_of_named(vec![2, 3]).unwrap().get::<i64>() == Some(5), "Res.sumOf(_:) bound to Int")?;
    let dogs = dog_list(vec!["rex".into()]).unwrap();
    check(label_of_named(&dogs[0]).unwrap() == "named rex", "Res.labelOf(_:)")?;
    let renamed = Dog::try_from(renamed_named(&dogs[0], "!").unwrap()).unwrap();
    check(renamed.name().unwrap() == "rex!", "Res.renamed(_:suffix:), a superclass requirement")
}

fn kinds_from_symbol_graphs() -> Result<(), Failed> {
    check(meter_value_bare(&meter(1.5).unwrap()).unwrap() == 1.5, "Res.Meter.value")?;
    let dogs = dog_list(vec!["rex".into()]).unwrap();
    check(label_of_bare(&dogs[0]).unwrap() == "named rex", "Res.labelOf<T: Named> bound to Res.Dog")
}

/// A receiver of another type than the declaration's is refused before Swift runs: a class's object (a subclass's
/// passes), a struct's value, a generic struct's value of another struct.
fn receivers_are_checked() -> Result<(), Failed> {
    let invalid = |r: Result<_, CallError>| matches!(r, Err(CallError::InvalidArgument(_)));
    check(animal_name(&dog("rex").unwrap()).unwrap() == "rex", "a Dog is an Animal")?;
    check(invalid(animal_name(&minted(1).unwrap()).map(|_| ())), "a Minted is not an Animal")?;
    check(meter_value(&meter(2.5).unwrap()).unwrap() == 2.5, "a Meter")?;
    check(invalid(meter_value(&make_boom(1).unwrap()).map(|_| ())), "a Boom is not a Meter")?;
    check(count(&pear_tray(3).unwrap()).unwrap() == 3, "a Tray<String>")?;
    check(invalid(count(&meter(1.0).unwrap()).map(|_| ())), "a Meter is not a Tray")
}

/// Any Sequence, through its conformance: an Array of objects, element by element, then the end.
fn sequences_iterate() -> Result<(), Failed> {
    let pack = SwiftIter::new(dogs(vec!["rex".into(), "fido".into()]).unwrap().0).unwrap();
    let labels: Vec<String> = pack.map(|dog| label_of_dog(&dog.into_object().unwrap()).unwrap()).collect();
    check(labels == ["named rex", "named fido"], format!("{labels:?}"))?;
    let not_a_sequence = SwiftIter::new(make_boom(1).unwrap());
    check(matches!(not_a_sequence, Err(CallError::InvalidArgument(_))), "a Boom is not a Sequence")
}

/// Generic parameters bound to concrete types: their metadata and witness tables come from the types named.
fn concrete_bindings() -> Result<(), Failed> {
    check(sum_text(vec![1.5, 2.0]).unwrap() == "3.5", "sumText<Double>")?;
    let sum = sum_of(vec![1, 2, 3]).unwrap();
    check(sum.ty() == SwiftType::of::<i64>(), "sumOf<Int> is an Int")?;
    check(sum.description() == "6", sum.description())?;
    check(label_of_dog(&dog("Rex").unwrap()).unwrap() == "named Rex", "labelOf<Dog>")?;
    check(doubled_int(&SwiftValue::new(21i64)).unwrap().description() == "42", "doubled<Int>")?;
    // Arguments must hold the bound types: Dog's witness table on another object, or Int's on a String, is refused.
    check(matches!(doubled_int(&SwiftValue::new("21".to_string())), Err(CallError::InvalidArgument(_))), "a String for Int")?;
    check(matches!(label_of_dog(&minted(1).unwrap()), Err(CallError::InvalidArgument(_))), "another class for Dog")?;
    let _ = nope_type(); // keeps Nope in the archive
    check(matches!(sum_of_nope(Vec::new()), Err(CallError::NotConforming { .. })), "Nope is not AdditiveArithmetic")
}

/// Every generic parameter bound, a generic type's too: its members, several protocols, associated types, a
/// superclass and a typed error, each requirement checked before the call.
fn specialized_generics() -> Result<(), Failed> {
    let tray = pear_tray(3).unwrap();
    let described_tray = describe_bound(&tray, &SwiftValue::new(1.5f64), 1).unwrap();
    check(described_tray == "String Double 4", described_tray)?;
    check(matches!(describe_int_tray(&tray, &SwiftValue::new(1.5f64), 1), Err(CallError::InvalidArgument(_))), "a Tray<String> for Tray<Int>")?;
    check(size_of_int_tray(1).unwrap() == 9, "static member of Tray<Int>")?;
    let holder = hold_int(7).unwrap();
    check(show(&holder, &SwiftValue::new(true)).unwrap() == "7 Bool", "Holder<Int>.show<Bool>")?;
    check(matches!(show_string_holder(&holder, &SwiftValue::new(true)), Err(CallError::InvalidArgument(_))), "a Holder<Int> for Holder<String>")?;
    check(holder_kind().unwrap() == "Holder<Int>", "static member of Holder<Int>, its class as self")?;
    check(rank(&SwiftValue::new(1i64), &SwiftValue::new(2i64)).unwrap() == -1, "Hashable & Comparable")?;
    let words = SwiftValue::new(vec!["a".to_string(), "b".to_string()]);
    check(joined(&words).unwrap() == "a,b", "S.Element == String")?;
    let numbers = SwiftValue::new(vec![1i64, 2]);
    check(matches!(joined_ints(&numbers), Err(CallError::InvalidArgument(_))), "[Int]'s Element is not String")?;
    check(described(&numbers).unwrap() == "1+2", "S.Element: CustomStringConvertible")?;
    check(matches!(renamed_minted(&minted(1).unwrap(), "!"), Err(CallError::InvalidArgument(_))), "Minted is no Animal")?;
    let boom = make_boom(5).unwrap();
    check(typed_bound(false, &boom).unwrap() == 42, "throws(Boom), not thrown")?;
    check(matches!(typed_bound(true, &boom), Err(CallError::Typed(_))), "throws(Boom), thrown")
}

/// A member of a generic struct gets `Self`'s metadata from its receiver, after its arguments and before its own
/// generic arguments; a static member takes the type's generic arguments explicitly.
fn members_of_generic_structs() -> Result<(), Failed> {
    let mut tray = pear_tray(3).unwrap();
    check(count(&tray).unwrap() == 3 && doubled(&tray).unwrap() == 6, "getters")?;
    // A result of the type's own parameter T: its type is read from the receiver's metadata.
    check(item(&tray).unwrap().get::<String>().as_deref() == Some("pear"), "T result")?;
    set_count(&mut tray, 5).unwrap();
    add(&mut tray, 2).unwrap();
    check(count(&tray).unwrap() == 7, "setter and mutating method")?;
    let described = describe(&tray, &SwiftValue::new(1.5f64), 1, SwiftType::of::<f64>()).unwrap();
    check(described == "String Double 8", described)?;
    let later = block(spawn(async move { later(&tray, 10).await })).unwrap();
    check(matches!(later, Ok(17)), format!("async {later:?}"))?;
    check(size(1, SwiftType::of::<i64>()).unwrap() == 9, "static member")
}

fn automatic_generic_arguments() -> Result<(), Failed> {
    let (echoed, described) = block(spawn(async {
        let n = SwiftValue::new(42i64);
        let echoed = echo(&n).await.map(|v| v.get::<i64>());
        let described = describe_it(&SwiftValue::new("hi"), 2).await;
        (echoed, described)
    }))
    .unwrap();
    check(matches!(echoed, Ok(Some(42))), format!("echo {echoed:?}"))?;
    check(matches!(&described, Ok(s) if s.contains("hi")), format!("describe {described:?}"))?;
    let boom = make_boom(3).unwrap();
    check(matches!(generic_typed_sync(false, &boom), Ok(42)), "typed throws: return")?;
    check(matches!(generic_typed_sync(true, &boom), Err(CallError::Typed(v)) if v.ty() == boom.ty()), "typed throws: the slot comes from E")
}

fn a_value_that_does_not_conform() -> Result<(), Failed> {
    let boom = make_boom(3).unwrap(); // Boom is not CustomStringConvertible
    let result = block(spawn(async move { describe_it(&boom, 1).await })).unwrap();
    check(matches!(&result, Err(CallError::NotConforming { .. })), format!("{result:?}"))?;
    let _ = SwiftType::of::<i64>();
    Ok(())
}

fn main() {
    common::run(vec![
        Trial::test("automatic_generic_arguments", automatic_generic_arguments),
        Trial::test("a_value_that_does_not_conform", a_value_that_does_not_conform),
        Trial::test("members_of_generic_structs", members_of_generic_structs),
        Trial::test("concrete_bindings", concrete_bindings),
        Trial::test("specialized_generics", specialized_generics),
        Trial::test("sequences_iterate", sequences_iterate),
        Trial::test("receivers_are_checked", receivers_are_checked),
        Trial::test("typed_wrappers", typed_wrappers),
        Trial::test("kinds_from_symbol_graphs", kinds_from_symbol_graphs),
        Trial::test("declarations_by_name", declarations_by_name),
    ]);
}
