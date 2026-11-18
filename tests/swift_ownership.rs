//! Ownership modifiers: `consuming` parameters and `self` are handed over (+1), `borrowing` initializer parameters
//! are not. A `Ticket` records its deinit: a reference released once too often, or never, shows.
#[path = "common/mod.rs"]
mod common;
#[path = "common/probe.rs"]
mod probe;

use apus::swift::{spawn, CallError, SwiftObject, SwiftType, SwiftValue};
use common::{block, check, Failed, Trial};

apus::swift! {
    #[swift_linked = "Res.Ticket(class).init(id: Int)"]
    fn ticket(self: &SwiftType, id: i64) -> SwiftObject;
    #[swift_linked = "Res.ticketGone(_: Int) -> Bool"]
    fn gone(id: i64) -> bool;
    #[swift_linked = "Res.keep(_: consuming Ticket(class))"]
    fn keep(t: &SwiftObject);
    #[swift_linked = "Res.releaseKept()"]
    fn release_kept();
    #[swift_linked = "Res.Stub(struct).init(borrowing: borrowing Ticket(class))"]
    fn stub(t: &SwiftObject) -> SwiftValue;
    #[swift_linked = "Res.Pass(struct).init(_: Ticket(class))"]
    fn pass(t: &SwiftObject) -> SwiftValue;
    #[swift_linked = "consuming Res.Pass(struct).redeem() -> Int"]
    fn redeem(self: &SwiftValue) -> i64;
    // Not in the binary: the calls are refused (Unavailable) before Swift runs them.
    #[swift = "Res.missing(_: consuming Ticket(class))"]
    fn missing(t: &SwiftObject);
    #[swift = "Res.missingLater(_: consuming Ticket(class)) async"]
    async fn missing_later(t: &SwiftObject);
}

fn new_ticket(id: i64) -> SwiftObject {
    ticket(&SwiftType::from_mangled_name("3Res6TicketC").unwrap(), id).unwrap()
}

fn consuming_parameters_are_handed_over() -> Result<(), Failed> {
    let t = new_ticket(2001);
    keep(&t).unwrap();
    drop(t);
    check(!gone(2001).unwrap(), "kept by Swift after Rust let go")?;
    release_kept().unwrap();
    check(gone(2001).unwrap(), "gone once Swift lets go")
}

fn borrowing_initializer_parameters_are_not() -> Result<(), Failed> {
    let t = new_ticket(2002);
    let s = stub(&t).unwrap();
    drop(t);
    check(!gone(2002).unwrap(), "the stub holds its own copy")?;
    drop(s);
    check(gone(2002).unwrap(), "and nothing else holds it")
}

fn consuming_self_takes_a_copy() -> Result<(), Failed> {
    let t = new_ticket(2003);
    let p = pass(&t).unwrap();
    check(redeem(&p).unwrap() == 2003 && redeem(&p).unwrap() == 2003, "redeemed twice from copies")?;
    drop(t);
    check(!gone(2003).unwrap(), "the pass still holds it")?;
    drop(p);
    check(gone(2003).unwrap(), "until the pass goes")
}

fn a_refused_call_keeps_what_it_would_have_handed_over() -> Result<(), Failed> {
    let t = new_ticket(2004);
    check(matches!(missing(&t), Err(CallError::Unavailable(_))), "sync: unavailable")?;
    let later = missing_later(&t);
    check(matches!(block(spawn(later)).unwrap(), Err(CallError::Unavailable(_))), "async: unavailable")?;
    drop(t);
    check(gone(2004).unwrap(), "nothing kept a reference")
}

fn main() {
    common::run(vec![
        Trial::test("consuming_parameters_are_handed_over", consuming_parameters_are_handed_over),
        Trial::test("borrowing_initializer_parameters_are_not", borrowing_initializer_parameters_are_not),
        Trial::test("consuming_self_takes_a_copy", consuming_self_takes_a_copy),
        Trial::test("a_refused_call_keeps_what_it_would_have_handed_over", a_refused_call_keeps_what_it_would_have_handed_over),
    ]);
}
