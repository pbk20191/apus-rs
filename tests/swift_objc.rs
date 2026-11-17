//! Swift class instances backed by Objective-C (NSObject subclasses): reference counting, and with the `objc2`
//! feature, objc2 objects as arguments, results and receivers.
#[path = "common/mod.rs"]
mod common;
#[path = "common/probe.rs"]
mod probe;

use apus::swift::SwiftObject;
use common::{check, Failed, Trial};

apus::swift! {
    #[swift_linked = "Probe.makeTracked(_: Int) -> Tracked(class)"]
    fn make_tracked(value: i64) -> SwiftObject;

    #[swift_linked = "Probe.trackedDeinits(_: Int) -> Int"]
    fn tracked_deinits(value: i64) -> i64;

    #[swift_linked = "Probe.trackedValue(_: Tracked(class)) -> Int"]
    fn tracked_value(object: &SwiftObject) -> i64;
}

/// Clones and drops of an ObjC-backed instance keep it intact and free it exactly once.
fn objc_backed_reference_counting() -> Result<(), Failed> {
    const VALUE: i64 = 21;
    let before = tracked_deinits(VALUE).unwrap();
    let object = make_tracked(VALUE).unwrap();
    let copies: Vec<SwiftObject> = (0..3).map(|_| object.clone()).collect();
    check(tracked_value(&object).unwrap() == VALUE, "the instance is intact after clones")?;
    drop(copies);
    check(tracked_deinits(VALUE).unwrap() == before, "still alive while one reference remains")?;
    check(tracked_value(&object).unwrap() == VALUE, "the instance is intact after drops")?;
    drop(object);
    check(tracked_deinits(VALUE).unwrap() == before + 1, "freed exactly once")
}

/// objc2 objects as Swift arguments, results and receivers (`__C.NSObject(class)` mangles as the imported class), and
/// conversions with SwiftObject.
#[cfg(feature = "objc2")]
mod objc {
    use super::*;
    use objc2::rc::Retained;
    use objc2::runtime::NSObject;

    apus::swift! {
        #[swift_linked = "Probe.makeTracked(_: Int) -> Tracked(class)"]
        fn make(value: i64) -> Retained<NSObject>;

        #[swift_linked = "Probe.className(_: __C.NSObject(class)) -> String"]
        fn class_name(object: &Retained<NSObject>) -> String;

        #[swift_linked = "Probe.echoObject(_: __C.NSObject(class)) -> __C.NSObject(class)"]
        fn echo(object: &Retained<NSObject>) -> Retained<NSObject>;

        #[swift_linked = "Probe.Tracked(class).doubled() -> Int"]
        fn doubled(self: &Retained<NSObject>) -> i64;

        #[swift_linked = "Probe.makeCounter() -> Counter(class)"]
        fn make_counter() -> SwiftObject;
    }

    pub fn objects_cross_the_boundary() -> Result<(), Failed> {
        const VALUE: i64 = 5;
        let before = tracked_deinits(VALUE).unwrap();
        let object = make(VALUE).unwrap();
        check(class_name(&object).unwrap() == "Tracked", "the object is the Swift instance")?;
        check(Retained::as_ptr(&echo(&object).unwrap()) == Retained::as_ptr(&object), "echo returns the same object")?;
        check(doubled(&object).unwrap() == 10, "an objc2 object as self")?;

        let swift = SwiftObject::from(object.clone());
        check(tracked_value(&swift).unwrap() == 5, "Retained -> SwiftObject")?;
        let back = swift.downcast::<NSObject>();
        check(back.as_ref().is_ok_and(|b| Retained::as_ptr(b) == Retained::as_ptr(&object)), "SwiftObject -> Retained<NSObject>")?;
        check(make_counter().unwrap().downcast::<NSObject>().is_err(), "a native Swift class is not an NSObject")?;
        drop((back, object));
        check(tracked_deinits(VALUE).unwrap() == before + 1, "freed exactly once")
    }
}

fn main() {
    #[allow(unused_mut)]
    let mut trials = vec![Trial::test("objc_backed_reference_counting", objc_backed_reference_counting)];
    #[cfg(feature = "objc2")]
    trials.push(Trial::test("objc2_objects_cross_the_boundary", objc::objects_cross_the_boundary));
    common::run(trials);
}
