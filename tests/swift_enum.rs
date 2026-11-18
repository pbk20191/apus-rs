//! Swift enum values by case tag: which case a value holds, a copy of its payload, and new values of a case.
#[path = "common/mod.rs"]
mod common;
#[path = "common/probe.rs"]
mod probe;

use apus::swift::{EnumCases, SwiftEnumCase, SwiftType, SwiftValue};
use common::{check, Failed, Trial};

apus::swift! {
    #[swift_linked = "Res.figure(_: Int) -> Figure(enum)"]
    fn figure(i: i64) -> SwiftValue;
    #[swift_linked = "Probe.lightMetadata() -> UnsafeRawPointer"]
    fn light_metadata() -> u64;
}

static NONE: SwiftEnumCase = SwiftEnumCase::new(apus::mangle!("case Res.Figure(enum).none"));
static CIRCLE: SwiftEnumCase = SwiftEnumCase::new(apus::mangle!("case Res.Figure(enum).circle(Double)"));
static LABEL: SwiftEnumCase = SwiftEnumCase::new(apus::mangle!("case Res.Figure(enum).label(String)"));
static GROUP: SwiftEnumCase = SwiftEnumCase::new(apus::mangle!("case Res.Figure(enum).group([Figure(enum)])"));
static EMPTY: SwiftEnumCase = SwiftEnumCase::new(apus::mangle!("case Res.Figure(enum).empty"));

fn figure_type() -> SwiftType {
    SwiftType::from_mangled_name("3Res6FigureO").unwrap()
}

fn tags_come_from_the_case_symbols() -> Result<(), Failed> {
    check(figure_type().enum_cases() == Some(EnumCases { with_payload: 3, without_payload: 2 }), "3 payload cases, 2 empty")?;
    // Payload cases first in declaration order, then the empty ones.
    let tags: Vec<_> = [&NONE, &CIRCLE, &LABEL, &GROUP, &EMPTY].iter().map(|c| c.tag()).collect();
    check(tags == [Some(3), Some(0), Some(1), Some(2), Some(4)], &format!("tag symbols: {tags:?}"))?;
    for (i, case) in [&NONE, &CIRCLE, &LABEL, &GROUP, &EMPTY].into_iter().enumerate() {
        check(figure(i as i64).unwrap().enum_tag() == case.tag(), &format!("figure({i}) holds its case"))?;
    }
    check(SwiftEnumCase::new("$s3Res6FigureO7missingyA2CmFWC").tag().is_none(), "a missing case has no tag")
}

fn payloads_are_copied_out() -> Result<(), Failed> {
    let circle = figure(1).unwrap();
    let radius = unsafe { circle.enum_payload(SwiftType::of::<f64>()) }.unwrap();
    check(radius.get::<f64>() == Some(2.5), "circle's Double")?;
    let label = unsafe { figure(2).unwrap().enum_payload(SwiftType::of::<String>()) }.unwrap();
    check(label.get::<String>().as_deref() == Some("hi"), "label's String")?;
    let group = figure(3).unwrap();
    let members = unsafe { group.indirect_enum_payload(SwiftType::from_mangled_name("Say3Res6FigureOG").unwrap()) }.unwrap();
    check(members.description() == "[Res.Figure.circle(1.0), Res.Figure.label(\"inner\")]", &members.description())?;
    // The values the payloads came from are untouched.
    check(circle.description() == "circle(2.5)", &circle.description())?;
    check(group.description().starts_with("group("), &group.description())?;
    check(unsafe { figure(0).unwrap().enum_payload(SwiftType::of::<f64>()) }.is_none(), "an empty case has no payload")
}

fn values_are_built_from_a_case() -> Result<(), Failed> {
    let empty = SwiftValue::enum_case(figure_type(), EMPTY.tag().unwrap()).unwrap();
    check(empty.description() == "empty", &empty.description())?;
    let label = unsafe { SwiftValue::enum_case_with(figure_type(), LABEL.tag().unwrap(), SwiftValue::new("made".to_string())) };
    let label = label.ok().unwrap();
    check(label.description() == "label(\"made\")", &label.description())?;
    // A module without library evolution: tags by declaration order (amber 0, red 1, green 2).
    let light = unsafe { SwiftType::from_metadata(light_metadata().unwrap() as *const std::ffi::c_void) };
    let amber = unsafe { SwiftValue::enum_case_with(light, 0, SwiftValue::new(5i64)) }.ok().unwrap();
    check(amber.description() == "amber(5)", &amber.description())?;
    check(SwiftValue::enum_case(light, 2).unwrap().description() == "green", "green")?;
    check(SwiftValue::enum_case(light, 0).is_none(), "amber needs a payload")?;
    check(SwiftValue::enum_case(light, 3).is_none(), "no fourth case")?;
    check(unsafe { SwiftValue::enum_case_with(light, 1, SwiftValue::new(1i64)) }.is_err(), "red takes no payload")
}

fn other_types_are_refused() -> Result<(), Failed> {
    check(SwiftType::of::<i64>().enum_cases().is_none(), "Int is not an enum")?;
    check(SwiftValue::new(3i64).enum_tag().is_none(), "nor is a value of it")?;
    // An NS_ENUM holds its raw value, not a tag.
    let imported = SwiftType::from_mangled_name("So18NSComparisonResultV").unwrap();
    check(imported.enum_cases().is_none(), "an imported C enum is refused")?;
    check(SwiftType::from_mangled_name("SiSg").unwrap().enum_cases() == Some(EnumCases { with_payload: 1, without_payload: 1 }), "Optional")
}

fn main() {
    common::run(vec![
        Trial::test("tags_come_from_the_case_symbols", tags_come_from_the_case_symbols),
        Trial::test("payloads_are_copied_out", payloads_are_copied_out),
        Trial::test("values_are_built_from_a_case", values_are_built_from_a_case),
        Trial::test("other_types_are_refused", other_types_are_refused),
    ]);
}
