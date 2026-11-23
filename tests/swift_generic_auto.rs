//! `swift!` fills a generic function's type metadata, witness tables and typed-throws error slot itself when the Rust
//! signature lists only the Swift parameters.
#[path = "common/mod.rs"]
mod common;
#[path = "common/probe.rs"]
mod probe;

use apus::swift::{spawn, CallError, DirectValue, SwiftIter, SwiftObject, SwiftString, SwiftType, SwiftValue};
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
    #[swift_linked = "(extension in Res where T: Weighed) Res.Holder(class)<T>.weighed<U: CustomStringConvertible>(_: U) -> String", T = "Swift.Int", U = "Swift.Int"]
    fn weighed(self: &SwiftObject, u: &SwiftValue) -> String;
    #[swift_linked = "static (extension in Res where T: Weighed) Res.Holder(class)<T>.unit() -> Int", T = "Swift.Int"]
    fn holder_unit() -> i64;
    #[swift_linked = "Res.intTray(count: Int) -> Tray(struct)<Int>"]
    fn int_tray(count: i64) -> SwiftValue;
    #[swift_linked = "(extension in Res where T: Weighed) Res.Tray(struct)<T>.weight() -> Int", T = "Swift.Int"]
    fn tray_weight(self: &SwiftValue) -> i64;
    #[swift_linked = "(extension in Res where T: Weighed) Res.Tray(struct)<T>.weight() -> Int", T = "Swift.String"]
    fn string_tray_weight(self: &SwiftValue) -> i64; // refused before the call: String is not Weighed
    #[swift_linked = "Res.Tray(struct)<T>.heaviest() -> Int where T: Comparable", T = "Swift.Int"]
    fn heaviest(self: &SwiftValue) -> i64;
    #[swift_linked = "Res.Pair(struct)<A, B>.init(second: B) where A == Int", B = "Swift.String"]
    fn pair_of_string(second: &SwiftValue) -> SwiftValue;
    /// `unsafe fn`: the receiver is not checked; the caller says it is an Animal.
    #[swift_linked = "Res.Animal(class).name: String { get }"]
    unsafe fn animal_name_unchecked(self: &SwiftObject) -> String;
    #[swift_linked = "Res.Meter(struct).subscript(scale: Double) -> Double { get async throws }"]
    async unsafe fn meter_scaled_unchecked(self: &SwiftValue, scale: f64) throws -> f64;
    #[swift_linked = "Res.Pair(struct)<A, B>.init(second: B) where A == Int", B = "[Res.Kept(class)]"]
    fn pair_of_kept(second: &SwiftValue) -> SwiftValue;
    #[swift_linked = "Res.makeKept() -> [Kept(class)]"]
    fn make_kept() -> DirectValue;
    #[swift_linked = "Res.keptAlive() -> Int"]
    fn kept_alive() -> i64;
    #[swift_linked = "Res.benchMax<T: Comparable>(_: T, _: T) -> T", T = "Swift.Int"]
    fn max_int(a: &i64, b: &i64) -> SwiftValue;
    #[swift_linked = "Res.echoSome<T: Equatable>(_: T) -> some Equatable", T = "Swift.Int"]
    fn echo_some_int(t: &i64) -> SwiftValue;
    #[swift_linked = "Res.benchMax<T: Comparable>(_: T, _: T) -> T", T = "Swift.Double"]
    fn max_double(a: &f64, b: &f64) -> SwiftValue;
    #[swift_linked = "Res.laterMax<T: Comparable>(_: T, _: T) async -> T", T = "Swift.Int"]
    async fn later_max_int(a: &i64, b: &i64) -> SwiftValue;
    #[swift_linked = "Res.stringLength(_: String) -> Int"]
    fn string_length(s: &SwiftString) -> i64;
    #[swift_linked = "Res.stringLength(_: String) -> Int"]
    fn string_length_of_str(s: &str) -> i64;
    #[swift_linked = "Res.keepString(_: consuming String)"]
    fn keep_string(s: &SwiftString);
    #[swift_linked = "Res.keptString() -> String"]
    fn kept_string() -> SwiftString;
    #[swift_linked = "Res.stringSumLater(_: String) async -> Int"]
    async fn string_sum_later(s: &SwiftString) -> i64;
    #[swift_linked = "Res.makeOneKept() -> Kept(class)"]
    fn make_one_kept() -> SwiftObject;
    #[swift_linked = "Res.consumeKept(_: consuming Kept(class)) -> Int"]
    fn consume_kept(k: &SwiftObject) -> i64;
    #[swift_linked = "Res.Pair(struct)<A, B>.init(second: B) where A == Int", B = "Swift.Int"]
    fn pair_of_int(second: &SwiftValue) -> SwiftValue;
    #[swift_linked = "static Res.Pair(struct)<A, B>.tag(_: B) -> String where A == Int", B = "Swift.String"]
    fn pair_tag(b: &SwiftValue) -> String;
    #[swift_linked = "Res.Pair(struct)<A, B>.sum() -> Int where A == Int, B == Int"]
    fn pair_sum(self: &SwiftValue) -> i64;
    #[swift_linked = "Res.Pair(struct)<A, B>.text: String { get }"]
    fn pair_text(self: &SwiftValue) -> String;
    #[swift_linked = "Res.Flag(struct)<V>.init() where V == Bool"]
    fn flag() -> SwiftValue;
    #[swift_linked = "Res.Flag(struct)<V>.text: String { get }"]
    fn flag_text(self: &SwiftValue) -> String;
    #[swift_linked = "Res.rank<T: Hashable & Comparable>(_: T, _: T) -> Int", T = "Swift.Int"]
    fn rank(a: &SwiftValue, b: &SwiftValue) -> i64;
    #[swift_linked = "Res.joined<S: Sequence>(_: S) -> String where S.Element == String", S = "[Swift.String]"]
    fn joined(s: &SwiftValue) -> String;
    #[swift_linked = "Res.joined<S: Sequence>(_: S) -> String where S.Element == String", S = "[Swift.Int]"]
    fn joined_ints(s: &SwiftValue) -> String; // refused before the call: [Int]'s Element is not String
    #[swift_linked = "Res.described<S: Sequence>(_: S) -> String where S.Element: CustomStringConvertible", S = "[Swift.Int]"]
    fn described(s: &SwiftValue) -> String;
    #[swift_linked = "Res.flattened<S: Sequence>(_: S) -> String where S.Element: Sequence, S.Element.Element(of Sequence) == String", S = "[[Swift.String]]"]
    fn flattened(s: &SwiftValue) -> String;
    #[swift_linked = "Res.flattened<S: Sequence>(_: S) -> String where S.Element: Sequence, S.Element.Element(of Sequence) == String", S = "[[Swift.Int]]"]
    fn flattened_ints(s: &SwiftValue) -> String; // refused before the call: [[Int]]'s Element.Element is not String
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
    #[swift_linked = "Res.nameOf(_: Animal(class)) -> String"]
    fn name_of_dog(animal: &Dog) -> String;
    #[swift_linked = "Res.nameOf(_: Animal(class)) -> String"]
    fn name_of_minted(animal: &Minted) -> String;
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
    check(matches!(minted.name(), Err(CallError::InvalidArgument(_))), "a Minted is no Animal")?;
    // Another class's wrapper as an argument too.
    check(name_of_dog(&dogs[0]).unwrap() == "rex", "a Dog for an Animal")?;
    check(matches!(name_of_minted(&minted), Err(CallError::InvalidArgument(_))), "a Minted for an Animal")
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
    // Members of constrained extensions, and a member's own `where`: written with their `(extension in …)`.
    #[swift_linked = "Res.Tray.weight()", T = "Swift.Int"]
    fn weight_named(self: &SwiftValue) -> i64;
    #[swift_linked = "Res.Holder.unit()", T = "Swift.Int"]
    fn unit_named() -> i64;
    #[swift_linked = "Res.Tray.heaviest()", T = "Swift.Int"]
    fn heaviest_named(self: &SwiftValue) -> i64;
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
    check(renamed.name().unwrap() == "rex!", "Res.renamed(_:suffix:), a superclass requirement")?;
    let ints = int_tray(3).unwrap();
    check(weight_named(&ints).unwrap() == 120, "Res.Tray.weight(), extension where T: Weighed")?;
    check(unit_named().unwrap() == 1, "Res.Holder.unit(), static in a constrained extension")?;
    check(heaviest_named(&ints).unwrap() == 3, "Res.Tray.heaviest(), where T: Comparable")
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
    // A call site keeps the class it last accepted, and only that: a refused one is refused again, another one is
    // checked in full, the first still accepted.
    check(invalid(animal_name(&minted(3).unwrap()).map(|_| ())), "nor is another Minted")?;
    check(animal_name(&dog("fido").unwrap()).unwrap() == "fido", "a Dog again")?;
    check(invalid(animal_name(&minted(2).unwrap()).map(|_| ())), "a Minted again")?;
    check(meter_value(&meter(2.5).unwrap()).unwrap() == 2.5, "a Meter")?;
    check(invalid(meter_value(&make_boom(1).unwrap()).map(|_| ())), "a Boom is not a Meter")?;
    check(count(&pear_tray(3).unwrap()).unwrap() == 3, "a Tray<String>")?;
    check(invalid(count(&meter(1.0).unwrap()).map(|_| ())), "a Meter is not a Tray")?;
    // Declared `unsafe fn`: not checked, the caller having said what the receiver is.
    check(unsafe { animal_name_unchecked(&dog("rex").unwrap()) }.unwrap() == "rex", "unchecked: a Dog")?;
    let later = unsafe { meter_scaled_unchecked(&meter(1.5).unwrap(), 2.0) };
    check(matches!(block(spawn(async move { later.await })).unwrap(), Ok(v) if v == 3.0), "unchecked, async")
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
    // A conformance an extension or the member adds to the type's parameter: its witness table is passed too.
    check(weighed(&holder, &SwiftValue::new(2i64)).unwrap() == "70 2", "Holder<Int> where T: Weighed")?;
    check(holder_unit().unwrap() == 1, "static member where T: Weighed")?;
    let ints = int_tray(3).unwrap();
    check(tray_weight(&ints).unwrap() == 120, "Tray<Int> where T: Weighed")?;
    check(heaviest(&ints).unwrap() == 3, "func heaviest() where T: Comparable")?;
    check(string_tray_weight(&ints).is_err(), "String is not Weighed")?;
    // A member's `where` that fixes the type's parameters: only the others are generic arguments.
    let pear = pair_of_string(&SwiftValue::new("pear".to_string())).unwrap();
    check(pair_text(&pear).unwrap() == "7 pear", "init(second:) where A == Int, B bound")?;
    check(pair_tag(&SwiftValue::new("fig".to_string())).unwrap() == "Int String fig", "static tag(_:) where A == Int")?;
    check(pair_sum(&pair_of_int(&SwiftValue::new(5i64)).unwrap()).unwrap() == 12, "sum() where A == Int, B == Int")?;
    check(flag_text(&flag().unwrap()).unwrap() == "true", "init() where V == Bool: no generic argument")?;
    check(rank(&SwiftValue::new(1i64), &SwiftValue::new(2i64)).unwrap() == -1, "Hashable & Comparable")?;
    let words = SwiftValue::new(vec!["a".to_string(), "b".to_string()]);
    check(joined(&words).unwrap() == "a,b", "S.Element == String")?;
    let numbers = SwiftValue::new(vec![1i64, 2]);
    check(matches!(joined_ints(&numbers), Err(CallError::InvalidArgument(_))), "[Int]'s Element is not String")?;
    check(described(&numbers).unwrap() == "1+2", "S.Element: CustomStringConvertible")?;
    // A nested associated type, found a step at a time.
    let rows = SwiftValue::new(vec![vec!["a".to_string()], vec!["b".to_string(), "c".to_string()]]);
    check(flattened(&rows).unwrap() == "a,b,c", "S.Element.Element == String")?;
    let grid = SwiftValue::new(vec![vec![1i64]]);
    check(matches!(flattened_ints(&grid), Err(CallError::InvalidArgument(_))), "[[Int]]'s Element.Element is not String")?;
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

/// A generic parameter bound to `Int`, `Double` or `Bool` takes a Rust `&i64`, `&f64` or `&bool`: its address for a
/// call made right away, a copy's for one that may run later.
fn stack_values_for_bound_generics() -> Result<(), Failed> {
    check(max_int(&3, &7).unwrap().get::<i64>() == Some(7), "Int from the stack")?;
    check(max_double(&2.5, &-1.0).unwrap().get::<f64>() == Some(2.5), "Double from the stack")?;
    // An opaque result of the bound type: its type found with the generic arguments the call passed.
    check(echo_some_int(&41).unwrap().get::<i64>() == Some(41), "some Equatable, an Int")?;
    #[inline(never)]
    fn made_in_a_frame_since_gone() -> apus::swift::SwiftCall<SwiftValue> {
        let (a, b) = (11i64, 5i64);
        later_max_int(&a, &b)
    }
    #[inline(never)]
    fn scribble() {
        let mut stack = [0x55u8; 4096];
        std::hint::black_box(&mut stack);
    }
    let later = made_in_a_frame_since_gone();
    scribble();
    let got = block(spawn(async move { later.await })).unwrap();
    check(matches!(&got, Ok(v) if v.get::<i64>() == Some(11)), format!("an async call keeps copies: {got:?}"))
}

/// A `SwiftString` is made once and passed as it is: borrowed by a call made right away, retained for one that
/// consumes it or may run later.
fn swift_strings() -> Result<(), Failed> {
    let long = "a Swift string long enough to need storage of its own";
    let s = SwiftString::new(long);
    check(string_length(&s).unwrap() == long.len() as i64, "borrowed")?;
    // A `&str` is made a Swift string for each call and released after it: once, or a call would read freed storage.
    let text = long.repeat(4);
    check((0..1000).all(|_| string_length_of_str(&text).unwrap() == text.len() as i64), "a &str, again and again")?;
    keep_string(&s).unwrap();
    drop(s);
    check(kept_string().unwrap().to_rust() == long, "a consumed one is Swift's own reference")?;
    let s = SwiftString::new(long);
    let later = string_sum_later(&s);
    drop(s);
    // Storage freed too early would be reused by these.
    let noise: Vec<SwiftString> = (0..64).map(|i| SwiftString::new(&format!("{i:>width$}", width = 20 + i))).collect();
    let n = block(spawn(async move { later.await })).unwrap();
    drop(noise);
    let sum: i64 = long.bytes().map(i64::from).sum();
    check(matches!(n, Ok(got) if got == sum), format!("an async call keeps its own: {n:?}"))
}

/// A sync call made right away passes a borrowed `&SwiftValue` by its address, but one the callee consumes (an
/// initializer's) as a copy: the caller's value lives on after the callee and its result are gone.
fn consumed_arguments_are_copies() -> Result<(), Failed> {
    let kept = make_kept().unwrap().0;
    check(kept_alive().unwrap() == 1, "one Kept made")?;
    drop(pair_of_kept(&kept).unwrap());
    check(kept_alive().unwrap() == 1, "the caller's array still holds its Kept")?;
    drop(kept);
    check(kept_alive().unwrap() == 0, "gone with the caller's array")?;
    // A class instance too, though a borrowed one is passed as it is: the callee takes its own reference.
    let one = make_one_kept().unwrap();
    consume_kept(&one).unwrap();
    consume_kept(&one).unwrap();
    check(kept_alive().unwrap() == 1, "the caller's Kept outlives two consuming calls")?;
    drop(one);
    check(kept_alive().unwrap() == 0, "gone with the caller's reference")
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
        Trial::test("consumed_arguments_are_copies", consumed_arguments_are_copies),
        Trial::test("swift_strings", swift_strings),
        Trial::test("stack_values_for_bound_generics", stack_values_for_bound_generics),
    ]);
}
