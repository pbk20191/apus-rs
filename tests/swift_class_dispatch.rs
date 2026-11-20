//! A class's non-final members dispatch to the override, as Swift calls them: through the dispatch thunk where the
//! module has one (library evolution), else through the class's vtable (`swift_lookUpClassMethod`).
#[path = "common/mod.rs"]
mod common;
#[path = "common/probe.rs"]
mod probe;

use apus::swift::{spawn, CallError, SwiftObject, SwiftType};
use common::{block, check, Failed, Trial};

apus::swift! {
    #[swift_linked = "Probe.makeLoud() -> Speaker(class)"]
    fn make_loud() -> SwiftObject;
    #[swift_linked = "Probe.loudType() -> Speaker(class).Type"]
    fn loud_type() -> SwiftType;
    #[swift_linked = "Probe.Minted(class).init(n: Int)"]
    fn minted(n: i64) -> SwiftObject;
    #[swift = "Probe.Speaker(class).who() -> Int"]
    fn who(self: &SwiftObject) -> i64;
    #[swift_linked = "Probe.Speaker(class).who() -> Int"]
    fn who_linked(self: &SwiftObject) -> i64;
    #[swift = "Probe.Speaker(class).later() async -> Int"]
    async fn later(self: &SwiftObject) -> i64;
    #[swift = "static Probe.Speaker(class).kind() -> Int"]
    fn kind(self: &SwiftType) -> i64;
    #[swift = "Probe.Speaker(class).fixed() -> Int"]
    fn fixed(self: &SwiftObject) -> i64;
}

fn overrides_are_called() -> Result<(), Failed> {
    let loud = make_loud().unwrap();
    check(who(&loud).unwrap() == 10, "#[swift] who() on a Loud")?;
    check(who_linked(&loud).unwrap() == 10, "#[swift_linked] who() on a Loud")?;
    let later = block(spawn(async move { later(&loud).await })).unwrap();
    check(matches!(later, Ok(10)), format!("async later() on a Loud: {later:?}"))?;
    check(kind(&loud_type().unwrap()).unwrap() == 10, "class func kind() on Loud.self")?;
    check(fixed(&make_loud().unwrap()).unwrap() == 5, "a final member: its own symbol")
}

fn another_class_is_refused() -> Result<(), Failed> {
    let minted = minted(1).unwrap();
    check(matches!(who(&minted), Err(CallError::InvalidArgument(_))), "a Minted is no Speaker")
}

fn main() {
    common::run(vec![
        Trial::test("overrides_are_called", overrides_are_called),
        Trial::test("another_class_is_refused", another_class_is_refused),
    ]);
}
