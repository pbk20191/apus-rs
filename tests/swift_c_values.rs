//! Values of C types (`CGRect`, `NSRange`, `CMTime`, an option set, an `NS_TYPED_ENUM` string) cross as C lays them
//! out: Rust passes its own `#[repr(C)]` type (an objc2 crate's, here declared locally) and gets one back in a `CValue`.
#[path = "common/mod.rs"]
mod common;
#[path = "common/probe.rs"]
mod probe;

use apus::swift::{spawn, CValue};
use common::{block, check, Failed, Trial};
use objc2::encode::{Encode, Encoding};
use objc2::runtime::AnyObject;

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
struct CGPoint {
    x: f64,
    y: f64,
}
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
struct CGSize {
    width: f64,
    height: f64,
}
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
struct CGRect {
    origin: CGPoint,
    size: CGSize,
}
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
struct NSRange {
    location: usize,
    length: usize,
}
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
struct CMTime {
    value: i64,
    timescale: i32,
    flags: u32,
    epoch: i64,
}
unsafe impl Encode for CGPoint {
    const ENCODING: Encoding = Encoding::Struct("CGPoint", &[f64::ENCODING, f64::ENCODING]);
}
unsafe impl Encode for CGSize {
    const ENCODING: Encoding = Encoding::Struct("CGSize", &[f64::ENCODING, f64::ENCODING]);
}
unsafe impl Encode for CGRect {
    const ENCODING: Encoding = Encoding::Struct("CGRect", &[CGPoint::ENCODING, CGSize::ENCODING]);
}
unsafe impl Encode for NSRange {
    const ENCODING: Encoding = Encoding::Struct("_NSRange", &[usize::ENCODING, usize::ENCODING]);
}
unsafe impl Encode for CMTime {
    const ENCODING: Encoding = Encoding::Struct("?", &[i64::ENCODING, i32::ENCODING, u32::ENCODING, i64::ENCODING]);
}

apus::swift! {
    #[swift_linked = "Probe.cRect(_: __C.CGRect(struct)) -> __C.CGRect(struct)"]
    fn c_rect(r: CGRect) -> CValue<CGRect>;
    #[swift_linked = "Probe.cRange(_: __C._NSRange(struct)) -> __C._NSRange(struct)"]
    fn c_range(r: NSRange) -> CValue<NSRange>;
    #[swift_linked = "Probe.cTime(_: __C.CMTime(typedef struct)) -> __C.CMTime(typedef struct)"]
    fn c_time(t: CMTime) -> CValue<CMTime>;
    #[swift_linked = "Probe.cUnit(_: __C.NSCalendarUnit(struct)) -> __C.NSCalendarUnit(struct)"]
    fn c_unit(u: u64) -> CValue<u64>;
    #[swift_linked = "Probe.cResult(_: __C.NSComparisonResult(struct)) -> __C.NSComparisonResult(struct)"]
    fn c_result(r: isize) -> CValue<isize>;
    #[swift_linked = "Probe.cKey(_: __C.NSURLResourceKey(typedef struct)) -> Int"]
    fn c_key(k: &AnyObject) -> i64;
    #[swift_linked = "Probe.cRectAsync(_: __C.CGRect(struct)) async -> __C.CGRect(struct)"]
    async fn c_rect_async(r: CGRect) -> CValue<CGRect>;
    #[swift_linked = "Probe.echoObject(_: __C.NSObject(class)) -> __C.NSObject(class)"]
    fn echo(o: &objc2::rc::Retained<AnyObject>) -> objc2::rc::Retained<AnyObject>;
}

fn structs_cross_in_registers() -> Result<(), Failed> {
    let rect = CGRect { origin: CGPoint { x: 1.5, y: 2.5 }, size: CGSize { width: 3.0, height: 4.0 } };
    let moved = c_rect(rect).unwrap().0;
    check(moved == CGRect { origin: CGPoint { x: 2.5, y: 4.5 }, ..rect }, format!("CGRect: {moved:?}"))?;
    let range = c_range(NSRange { location: 3, length: 4 }).unwrap().0;
    check(range == NSRange { location: 4, length: 8 }, format!("NSRange: {range:?}"))?;
    // Two 32-bit fields share one integer register.
    let time = c_time(CMTime { value: 21, timescale: 600, flags: 1, epoch: 0 }).unwrap().0;
    check(time.value == 42 && time.timescale == 601 && time.flags == 1, format!("CMTime: {time:?}"))?;
    let inset = block(spawn(async move { c_rect_async(rect).await })).unwrap().unwrap().0;
    check(inset.size == CGSize { width: 1.0, height: 2.0 }, format!("async CGRect: {inset:?}"))
}

fn enums_and_typed_strings() -> Result<(), Failed> {
    check(c_unit(8).unwrap().0 == 8 | 16, "NSCalendar.Unit: .month ∪ .day")?;
    check(c_result(-1).unwrap().0 == 1, "ComparisonResult: ascending -> descending")?;
    let string: objc2::rc::Retained<AnyObject> = unsafe { objc2::msg_send![objc2::class!(NSString), stringWithUTF8String: c"NSURLNameKey".as_ptr()] };
    check(c_key(&string).unwrap() == 12, "URLResourceKey: its string")?;
    check(echo(&string).is_ok(), "the string is still alive")
}

fn main() {
    common::run(vec![
        Trial::test("structs_cross_in_registers", structs_cross_in_registers),
        Trial::test("enums_and_typed_strings", enums_and_typed_strings),
    ]);
}
