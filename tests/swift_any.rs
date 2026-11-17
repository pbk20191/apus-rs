//! SwiftAny downcasts, type names and drops; SwiftError descriptions.
use apus::swift::{SwiftAny, SwiftError};
use std::ffi::c_void;

extern "C" {
    fn probe_any_int(out: *mut c_void);
    fn probe_any_string(out: *mut c_void);
    fn probe_any_optional_nil(out: *mut c_void);
    fn probe_any_optional_some(out: *mut c_void);
    fn probe_any_void(out: *mut c_void);
    fn probe_any_object(out: *mut c_void, flag: *mut bool);
    fn probe_make_error(out: *mut c_void);
    fn probe_any_int_array(out: *mut c_void);
    fn probe_any_string_array(out: *mut c_void);
    fn probe_any_empty_array(out: *mut c_void);
    fn probe_any_nested_array(out: *mut c_void);
    fn probe_any_optional_array(out: *mut c_void);
    fn probe_any_bool_double_array(out: *mut c_void, bools: bool);
    fn probe_any_dict(out: *mut c_void);
    fn probe_any_big_dict(out: *mut c_void);
    fn probe_any_empty_dict(out: *mut c_void);
    fn probe_any_dict_of_arrays(out: *mut c_void);
}

fn any_with(make: impl FnOnce(*mut c_void)) -> SwiftAny {
    let mut buf = [0usize; 4];
    make(buf.as_mut_ptr().cast());
    unsafe { SwiftAny::from_raw_initialized(buf) }
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

    use std::collections::HashMap;
    let v = any(probe_any_int_array);
    t("downcast_int_array", v.downcast::<Vec<i64>>() == Some(vec![1, 2, 3]) && v.downcast::<Vec<String>>().is_none() && v.downcast::<i64>().is_none(),
      format!("{:?} / {:?}", v.downcast::<Vec<i64>>(), v.downcast::<Vec<String>>()));
    let v = any(probe_any_string_array);
    let expected = vec!["a".to_string(), "héllo👋".into(), "".into(), "long".repeat(20)];
    t("downcast_string_array", v.downcast::<Vec<String>>().as_ref() == Some(&expected), format!("{:?}", v.downcast::<Vec<String>>()));
    let v = any(probe_any_empty_array);
    t("downcast_empty_array", v.downcast::<Vec<i64>>() == Some(vec![]), format!("{:?}", v.downcast::<Vec<i64>>()));
    let v = any(probe_any_nested_array);
    t("downcast_nested_array", v.downcast::<Vec<Vec<i64>>>() == Some(vec![vec![1], vec![], vec![2, 3]]), format!("{:?}", v.downcast::<Vec<Vec<i64>>>()));
    let v = any(probe_any_optional_array);
    t("downcast_optional_array", v.downcast::<Vec<Option<i64>>>() == Some(vec![Some(1), None, Some(3)]), format!("{:?}", v.downcast::<Vec<Option<i64>>>()));
    let v = any_with(|out| unsafe { probe_any_bool_double_array(out, true) });
    t("downcast_bool_array", v.downcast::<Vec<bool>>() == Some(vec![true, false, true]), format!("{:?}", v.downcast::<Vec<bool>>()));
    let v = any_with(|out| unsafe { probe_any_bool_double_array(out, false) });
    t("downcast_double_array", v.downcast::<Vec<f64>>() == Some(vec![1.5, -2.0]), format!("{:?}", v.downcast::<Vec<f64>>()));

    let v = any(probe_any_dict);
    let expected: HashMap<String, i64> = [("one".to_string(), 1), ("two".into(), 2), ("three".into(), 3)].into();
    t("downcast_dict", v.downcast::<HashMap<String, i64>>().as_ref() == Some(&expected) && v.downcast::<HashMap<i64, i64>>().is_none(),
      format!("{:?}", v.downcast::<HashMap<String, i64>>()));
    let v = any(probe_any_big_dict);
    let expected: HashMap<i64, String> = (0..200).map(|i| (i, format!("v{i}"))).collect();
    t("downcast_big_dict", v.downcast::<HashMap<i64, String>>().as_ref() == Some(&expected), format!("{:?}", v.downcast::<HashMap<i64, String>>().map(|m| m.len())));
    let v = any(probe_any_empty_dict);
    t("downcast_empty_dict", v.downcast::<HashMap<String, i64>>() == Some(HashMap::new()), format!("{:?}", v.downcast::<HashMap<String, i64>>()));
    let v = any(probe_any_dict_of_arrays);
    let expected: HashMap<String, Vec<i64>> = [("a".to_string(), vec![1, 2]), ("b".into(), vec![])].into();
    t("downcast_dict_of_arrays", v.downcast::<HashMap<String, Vec<i64>>>().as_ref() == Some(&expected), format!("{:?}", v.downcast::<HashMap<String, Vec<i64>>>()));

    if failures.is_empty() { return }
    eprintln!("{}", failures.join("\n"));
    std::process::exit(1);
}
