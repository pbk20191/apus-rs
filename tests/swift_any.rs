//! SwiftAny downcasts, type names and drops; SwiftError descriptions.
use async_swift_rs::swift::{SwiftAny, SwiftError};
use std::ffi::c_void;

extern "C" {
    fn probe_any_int(out: *mut c_void);
    fn probe_any_string(out: *mut c_void);
    fn probe_any_optional_nil(out: *mut c_void);
    fn probe_any_optional_some(out: *mut c_void);
    fn probe_any_void(out: *mut c_void);
    fn probe_any_object(out: *mut c_void, flag: *mut bool);
    fn probe_make_error(out: *mut c_void);
}

fn any(make: unsafe extern "C" fn(*mut c_void)) -> SwiftAny {
    let mut buf = [0usize; 4];
    unsafe { make(buf.as_mut_ptr().cast()); SwiftAny::from_raw_initialized(buf) }
}

fn main() {
    let mut failures = Vec::new();
    let mut t = |name: &str, ok: bool, detail: String| {
        if ok { println!("{name} ... ok") } else { failures.push(format!("{name}: {detail}")) }
    };

    let v = any(probe_any_int);
    t("downcast_int", v.downcast::<i64>() == Some(42) && v.downcast::<String>().is_none(),
      format!("{:?} / {:?}", v.downcast::<i64>(), v.downcast::<String>()));
    t("type_name_reports_swift_type", v.type_name() == "Swift.Int", v.type_name());

    let v = any(probe_any_string);
    t("downcast_string", v.downcast::<String>().as_deref() == Some("héllo👋"), format!("{:?}", v.downcast::<String>()));

    let v = any(probe_any_optional_nil);
    t("downcast_optional_nil", v.downcast::<Option<i64>>() == Some(None), format!("{:?}", v.downcast::<Option<i64>>()));
    let v = any(probe_any_optional_some);
    t("downcast_optional_some", v.downcast::<Option<i64>>() == Some(Some(7)) && v.downcast::<i64>() == Some(7),
      format!("{:?}", v.downcast::<Option<i64>>()));

    let v = any(probe_any_void);
    t("downcast_void", v.downcast::<()>() == Some(()), format!("{:?}", v.downcast::<()>()));

    let mut deinit = false;
    let mut buf = [0usize; 4];
    unsafe { probe_any_object(buf.as_mut_ptr().cast(), &mut deinit) };
    let v = unsafe { SwiftAny::from_raw_initialized(buf) };
    let before = deinit;
    drop(v);
    t("any_drop_releases", !before && deinit, format!("before {before}, after {deinit}"));

    let mut word = std::ptr::null_mut::<c_void>();
    unsafe { probe_make_error((&mut word as *mut *mut c_void).cast()) };
    let e = unsafe { SwiftError::from_raw(word) };
    let d = e.description();
    t("error_description", d.contains("io(code: 5)") && e.type_name().ends_with("ProbeError"), format!("{d} / {}", e.type_name()));

    if failures.is_empty() { return }
    eprintln!("{}", failures.join("\n"));
    std::process::exit(1);
}
