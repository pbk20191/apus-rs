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

    #[swift_linked = "Probe.trackedDeinits() -> Int"]
    fn tracked_deinits() -> i64;

    #[swift_linked = "Probe.trackedValue(_: Tracked(class)) -> Int"]
    fn tracked_value(object: &SwiftObject) -> i64;
}

/// Clones and drops of an ObjC-backed instance keep it intact and free it exactly once.
fn objc_backed_reference_counting() -> Result<(), Failed> {
    let before = tracked_deinits().unwrap();
    let object = make_tracked(21).unwrap();
    let copies: Vec<SwiftObject> = (0..3).map(|_| object.clone()).collect();
    check(tracked_value(&object).unwrap() == 21, "the instance is intact after clones")?;
    drop(copies);
    check(tracked_deinits().unwrap() == before, "still alive while one reference remains")?;
    check(tracked_value(&object).unwrap() == 21, "the instance is intact after drops")?;
    drop(object);
    check(tracked_deinits().unwrap() == before + 1, "freed exactly once")
}

fn main() {
    common::run(vec![Trial::test("objc_backed_reference_counting", objc_backed_reference_counting)]);
}
