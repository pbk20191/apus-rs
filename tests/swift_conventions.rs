//! Function conventions: an `@autoclosure` takes a `SwiftClosure`; a `@convention(c)` function an `extern "C" fn`;
//! a `@convention(block)` function a block2 block (feature `block2`). The C function's and block's types are checked
//! against the declared ones.
#[path = "common/mod.rs"]
mod common;
#[path = "common/probe.rs"]
mod probe;

use apus::swift::{CallError, SwiftClosure};
use common::{check, Failed, Trial};

apus::swift! {
    #[swift_linked = "Res.autoValue(_: @autoclosure () -> Int) -> Int"]
    fn auto_value(f: &SwiftClosure) -> i64;
    #[swift_linked = "Res.autoLater(_: @escaping @autoclosure () -> String) -> String"]
    fn auto_later(f: &SwiftClosure) -> String;
    #[swift_linked = "Res.callC(_: @convention(c) (Int32, Double) -> Int32, _: Int32) -> Int32"]
    fn call_c(f: extern "C" fn(i32, f64) -> i32, x: i32) -> i32;
    #[swift_linked = "Res.callC(_: @convention(c) (Int32, Double) -> Int32, _: Int32) -> Int32"]
    fn call_c_wrongly(f: extern "C" fn(i64) -> i32, x: i32) -> i32;
    #[swift_linked = "Res.callMaybeC(_: (@convention(c) (Int) -> Int)?) -> Int"]
    fn call_maybe_c(f: Option<extern "C" fn(i64) -> i64>) -> i64;
    #[swift_linked = "Res.cDoubler() -> @convention(c) (Int) -> Int"]
    fn c_doubler() -> extern "C" fn(i64) -> i64;
}

extern "C" fn scaled(x: i32, by: f64) -> i32 {
    (f64::from(x) * by * 10.0) as i32
}

extern "C" fn plus_one(x: i64) -> i64 {
    x + 1
}

extern "C" fn ignored(_: i64) -> i32 {
    0
}

fn autoclosures_are_swift_closures() -> Result<(), Failed> {
    check(auto_value(&SwiftClosure::new_sync(|| 41i64)).unwrap() == 42, "autoclosure")?;
    let later = auto_later(&SwiftClosure::new_sync(|| String::from("later"))).unwrap();
    check(later == "later!", later)
}

fn c_functions_are_function_pointers() -> Result<(), Failed> {
    check(call_c(scaled, 4).unwrap() == 20, "C function argument")?;
    check(call_maybe_c(Some(plus_one)).unwrap() == 21, "optional C function")?;
    check(call_maybe_c(None).unwrap() == -1, "nil")?;
    check(c_doubler().unwrap()(8) == 16, "C function result")?;
    // A function pointer of another type is refused before the call.
    let refused = call_c_wrongly(ignored, 4);
    check(matches!(refused, Err(CallError::ClosureMismatch(_))), format!("{refused:?}"))
}

#[cfg(feature = "block2")]
mod blocks {
    use super::*;
    use block2::{Block, RcBlock, StackBlock};
    use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
    use std::sync::Arc;

    apus::swift! {
        #[swift_linked = "Res.callBlock(_: @convention(block) (Int) -> Int) -> Int"]
        fn call_block(f: &Block<dyn Fn(isize) -> isize>) -> i64;
        #[swift_linked = "Res.callBlock(_: @convention(block) (Int) -> Int) -> Int"]
        fn call_block_wrongly(f: &Block<dyn Fn(f64) -> isize>) -> i64;
        #[swift_linked = "Res.keepBlock(_: (@convention(block) () -> Int)?)"]
        fn keep_block(f: Option<&Block<dyn Fn() -> isize>>);
        #[swift_linked = "Res.callKeptBlock() -> Int"]
        fn call_kept_block() -> i64;
        #[swift_linked = "Res.adder(_: Int) -> @convention(block) (Int) -> Int"]
        fn adder(k: i64) -> RcBlock<dyn Fn(isize) -> isize>;
        #[swift_linked = "Res.BlockHolder(struct).init(f: @escaping @convention(block) () -> Int)"]
        fn holder(f: &Block<dyn Fn() -> isize>) -> apus::swift::SwiftValue;
        #[swift_linked = "Res.BlockHolder(struct).call() -> Int"]
        fn call(self: &apus::swift::SwiftValue) -> i64;
    }

    /// Counts its drops: the block's captures go when its last reference does.
    struct Dropped(Arc<AtomicUsize>);
    impl Drop for Dropped {
        fn drop(&mut self) {
            self.0.fetch_add(1, SeqCst);
        }
    }

    pub fn blocks_are_objective_c_blocks() -> Result<(), Failed> {
        let stack = StackBlock::new(|x: isize| x * 2);
        check(call_block(&stack).unwrap() == 42, "stack block argument")?;
        let refused = call_block_wrongly(&RcBlock::new(|_: f64| 0isize));
        check(matches!(refused, Err(CallError::ClosureMismatch(_))), format!("{refused:?}"))?;
        // Swift keeps a copy past the call, and releases it.
        let drops = Arc::new(AtomicUsize::new(0));
        let guard = Dropped(drops.clone());
        keep_block(Some(&RcBlock::new(move || {
            let _ = &guard;
            7isize
        })))
        .unwrap();
        check(drops.load(SeqCst) == 0 && call_kept_block().unwrap() == 7, "kept by Swift")?;
        keep_block(None).unwrap();
        check(drops.load(SeqCst) == 1 && call_kept_block().unwrap() == -1, "released by Swift")?;
        // A block result is +1; an initializer consumes its block.
        check(adder(5).unwrap().call((10,)) == 15, "block result")?;
        let drops = Arc::new(AtomicUsize::new(0));
        let guard = Dropped(drops.clone());
        let held = holder(&RcBlock::new(move || {
            let _ = &guard;
            3isize
        }))
        .unwrap();
        check(call(&held).unwrap() == 3 && drops.load(SeqCst) == 0, "held")?;
        drop(held);
        check(drops.load(SeqCst) == 1, "released with its holder")
    }
}

fn main() {
    #[allow(unused_mut)]
    let mut trials = vec![
        Trial::test("autoclosures_are_swift_closures", autoclosures_are_swift_closures),
        Trial::test("c_functions_are_function_pointers", c_functions_are_function_pointers),
    ];
    #[cfg(feature = "block2")]
    trials.push(Trial::test("blocks_are_objective_c_blocks", blocks::blocks_are_objective_c_blocks));
    common::run(trials);
}
