//! M3 spike: what Rust can do with Swift classes, structs and protocols at the ABI level.
//! Usage: m3-spike <plain|evolution> <class|struct|protocol>
use apus::swift::SwiftType;
use std::ffi::{c_char, c_void, CStr, CString};

#[repr(C)]
#[derive(Default)]
struct CallRegs { gpr: [u64; 8], fpr: [f64; 8], this: usize }
#[repr(C)]
#[derive(Default, Debug, Clone, Copy)]
struct RetRegs { gpr: [u64; 4], fpr: [f64; 4], error: usize }
#[repr(C)]
struct FieldMeta { name: *const c_char, free: Option<unsafe extern "C" fn(*const c_char)>, is_strong: bool, is_var: bool }

extern "C" {
    fn rs_call_sync(function: *const c_void, regs: *const CallRegs, indirect: *mut c_void, ret: *mut RetRegs, kind: i32);
    fn rs_string_from_utf8(start: *const u8, count: isize, out: *mut [u64; 2]);
    fn dlopen(path: *const c_char, mode: i32) -> *mut c_void;
    fn dlsym(handle: *mut c_void, name: *const c_char) -> *mut c_void;
    fn swift_reflectionMirror_recursiveCount(ty: *const c_void) -> isize;
    fn swift_reflectionMirror_recursiveChildMetadata(ty: *const c_void, index: isize, field: *mut FieldMeta) -> *const c_void;
    fn swift_reflectionMirror_recursiveChildOffset(ty: *const c_void, index: isize) -> isize;
    fn swift_lookUpClassMethod(metadata: *const c_void, method: *const c_void, class_descriptor: *const c_void) -> *const c_void;
    fn swift_getObjectType(object: usize) -> *const c_void;
    fn swift_dynamicCastClass(object: usize, target: *const c_void) -> usize;
    fn swift_release(object: usize);
    fn swift_bridgeObjectRelease(object: u64);
    #[link_name = "$sSS11utf8CStrings15ContiguousArrayVys4Int8VGvg"]
    fn string_utf8(w0: u64, w1: u64) -> *mut u8;
    fn m3_title_witness() -> *const c_void;
    fn m3_greet_witness() -> *const c_void;
}
const INTS: i32 = 0;
const FLOATS: i32 = 1;
const INDIRECT: i32 = 2;

fn sym(name: &str) -> usize {
    let c = CString::new(name).unwrap();
    unsafe { dlsym(-2isize as *mut c_void, c.as_ptr()) as usize }
}
fn need(name: &str) -> usize {
    let s = sym(name);
    assert!(s != 0, "missing symbol {name}");
    s
}
fn call(function: usize, ints: &[u64], floats: &[f64], this: usize, indirect: *mut c_void, kind: i32) -> RetRegs {
    let (mut regs, mut ret) = (CallRegs::default(), RetRegs::default());
    regs.gpr[..ints.len()].copy_from_slice(ints);
    regs.fpr[..floats.len()].copy_from_slice(floats);
    regs.this = this;
    unsafe { rs_call_sync(function as *const c_void, &regs, indirect, &mut ret, kind) };
    ret
}
/// An owned Swift String.
fn mk(s: &str) -> [u64; 2] {
    let mut out = [0u64; 2];
    unsafe { rs_string_from_utf8(s.as_ptr(), s.len() as isize, &mut out) };
    out
}
/// Reads a Swift String without consuming it.
fn rd(w0: u64, w1: u64) -> String {
    unsafe {
        let array = string_utf8(w0, w1);
        let count = *(array.add(16) as *const isize) as usize;
        let s = String::from_utf8_lossy(std::slice::from_raw_parts(array.add(32), count.saturating_sub(1))).into_owned();
        swift_release(array as usize);
        s
    }
}
fn rd_ret(r: RetRegs) -> String {
    let s = rd(r.gpr[0], r.gpr[1]);
    unsafe { swift_bridgeObjectRelease(r.gpr[1]) };
    s
}
fn ty(mangled: &str) -> SwiftType {
    SwiftType::from_mangled_name(mangled).unwrap_or_else(|| panic!("no type {mangled}"))
}
fn type_name(metadata: *const c_void) -> String {
    unsafe { SwiftType::from_metadata(metadata) }.name()
}
fn fields(metadata: *const c_void) -> Vec<(String, String, isize, bool)> {
    unsafe {
        (0..swift_reflectionMirror_recursiveCount(metadata))
            .map(|i| {
                let mut meta = FieldMeta { name: std::ptr::null(), free: None, is_strong: false, is_var: false };
                let field_type = swift_reflectionMirror_recursiveChildMetadata(metadata, i, &mut meta);
                let name = CStr::from_ptr(meta.name).to_string_lossy().into_owned();
                if let Some(free) = meta.free { free(meta.name) }
                (name, type_name(field_type), swift_reflectionMirror_recursiveChildOffset(metadata, i), meta.is_var)
            })
            .collect()
    }
}
fn show_fields(metadata: *const c_void) -> String {
    fields(metadata).iter().map(|(n, t, o, var)| format!("{}{n}: {t} @{o}", if *var { "var " } else { "let " })).collect::<Vec<_>>().join(", ")
}

fn class(evolution: bool) {
    let (animal_ty, dog_ty) = (ty("5Spike6AnimalC").as_ptr(), ty("5Spike3DogC").as_ptr());
    // C1: `Animal(name:)` — allocating init: arguments, then the class's metatype as `self`.
    let name = mk("Generic");
    let animal = call(need("$s5Spike6AnimalC4nameACSS_tcfC"), &name, &[], animal_ty as usize, std::ptr::null_mut(), INTS).gpr[0] as usize;
    println!("C1 Animal(name:) from Rust → instance of {}", type_name(unsafe { swift_getObjectType(animal) }));
    let rex = mk("Rex");
    let dog = call(need("$s5Spike7makeDogyAA6AnimalCSSF"), &rex, &[], 0, std::ptr::null_mut(), INTS).gpr[0] as usize;
    unsafe { swift_bridgeObjectRelease(rex[1]) };
    println!("C5 makeDog() returns an Animal whose dynamic type is {}; cast to Dog: {}; Animal instance cast to Dog: {}",
             type_name(unsafe { swift_getObjectType(dog) }),
             unsafe { swift_dynamicCastClass(dog, dog_ty) } != 0, unsafe { swift_dynamicCastClass(animal, dog_ty) } != 0);

    // C2: virtual dispatch of `speak()` on a Dog seen as Animal.
    if !evolution {
        let direct = rd_ret(call(need("$s5Spike6AnimalC5speakSSyF"), &[], &[], dog, std::ptr::null_mut(), INTS));
        println!("C2a Animal.speak's own symbol on a Dog (no dispatch)       = {direct:?}   ← ignores the override");
    }
    let implementation = unsafe {
        swift_lookUpClassMethod(*(dog as *const *const c_void), need("$s5Spike6AnimalC5speakSSyFTq") as *const c_void, need("$s5Spike6AnimalCMn") as *const c_void)
    };
    let looked_up = rd_ret(call(implementation as usize, &[], &[], dog, std::ptr::null_mut(), INTS));
    println!("C2b swift_lookUpClassMethod(isa, speak's method descriptor)    = {looked_up:?}");
    if evolution {
        let thunk = rd_ret(call(need("$s5Spike6AnimalC5speakSSyFTj"), &[], &[], dog, std::ptr::null_mut(), INTS));
        println!("C2c dispatch thunk (resilient modules export it)           = {thunk:?}");
        let describe = rd_ret(call(need("$s5Spike6AnimalC8describeSSyFTj"), &[], &[], dog, std::ptr::null_mut(), INTS));
        println!("C2d describe() through its thunk                            = {describe:?}");
    } else {
        let describe = rd_ret(call(need("$s5Spike6AnimalC8describeSSyF"), &[], &[], dog, std::ptr::null_mut(), INTS));
        println!("C2d describe() (calls speak() virtually inside Swift)       = {describe:?}");
    }

    // C3/C4: stored properties.
    println!("C4 reflection of Dog (superclass fields included): {}", show_fields(dog_ty));
    let name_offset = fields(dog_ty).iter().find(|f| f.0 == "name").unwrap().2 as usize;
    let words = unsafe { *((dog + name_offset) as *const [u64; 2]) };
    println!("C3a name read straight from the instance at the reflected offset = {:?}", rd(words[0], words[1]));
    if !evolution {
        let global = unsafe { *(need("$s5Spike6AnimalC4nameSSvpWvd") as *const usize) };
        println!("C3b the exported field offset variable agrees: {} == {}", global, name_offset);
        let renamed = mk("Rexford");
        call(need("$s5Spike6AnimalC4nameSSvs"), &renamed, &[], dog, std::ptr::null_mut(), INTS); // the setter consumes its argument
        let describe = rd_ret(call(need("$s5Spike6AnimalC8describeSSyF"), &[], &[], dog, std::ptr::null_mut(), INTS));
        println!("C3c after name.setter(\"Rexford\")                         = {describe:?}");
    } else {
        let renamed = mk("Rexford");
        call(need("$s5Spike6AnimalC4nameSSvsTj"), &renamed, &[], dog, std::ptr::null_mut(), INTS);
        let getter = rd_ret(call(need("$s5Spike6AnimalC4nameSSvgTj"), &[], &[], dog, std::ptr::null_mut(), INTS));
        println!("C3c name setter and getter through their thunks            = {getter:?}");
    }
    unsafe { swift_release(dog); swift_release(animal) };
}

fn strukt(evolution: bool) {
    let null = std::ptr::null_mut();
    println!("S4 fields  Point: {}", show_fields(ty("5Spike5PointV").as_ptr()));
    println!("S4 fields  Tag:   {}", show_fields(ty("5Spike3TagV").as_ptr()));
    println!("S4 fields  Mixed: {}", show_fields(ty("5Spike5MixedV").as_ptr()));
    println!("S4 fields  Wide:  {} (size {})", show_fields(ty("5Spike4WideV").as_ptr()), ty("5Spike4WideV").size());

    if evolution {
        // A resilient (non-@frozen) struct has no fixed layout for other modules: everything goes by address.
        let point_ty = ty("5Spike5PointV");
        let mut storage = vec![0f64; point_ty.size().div_ceil(8)];
        call(need("$s5Spike5PointV1x1yACSd_SdtcfC"), &[], &[3.0, 4.0], 0, storage.as_mut_ptr().cast(), INDIRECT);
        let len = call(need("$s5Spike5PointV6lengthSdyF"), &[], &[], storage.as_ptr() as usize, null, FLOATS).fpr[0];
        println!("S6 resilient Point: built by address {:?} (size from the value witness table: {}); length() with self by address = {len}", storage, point_ty.size());
        return;
    }
    // S1/S2: Point is two Doubles: built and passed in floating-point registers.
    let p = call(need("$s5Spike5PointV1x1yACSd_SdtcfC"), &[], &[3.0, 4.0], 0, null, FLOATS);
    println!("S1 Point(x: 3, y: 4) comes back in d0, d1                    = ({}, {})", p.fpr[0], p.fpr[1]);
    let len = call(need("$s5Spike5PointV6lengthSdyF"), &[], &[p.fpr[0], p.fpr[1]], 0, null, FLOATS).fpr[0];
    println!("S2 point.length(): self passed as d0, d1                      = {len}");
    let scaled = call(need("$s5Spike5PointV6scaled2byACSd_tF"), &[], &[2.0, p.fpr[0], p.fpr[1]], 0, null, FLOATS);
    println!("S2 point.scaled(by: 2): argument d0, then self d1, d2         = ({}, {})", scaled.fpr[0], scaled.fpr[1]);
    let mut storage = [p.fpr[0], p.fpr[1]];
    call(need("$s5Spike5PointV4move2dxySd_tF"), &[], &[10.0], storage.as_mut_ptr() as usize, null, INTS);
    println!("S3 mutating point.move(dx: 10): self by address in the context register → {:?}", storage);

    // One word: where does self go?
    let one_in_x0 = call(need("$s5Spike3OneV5twiceSiyF"), &[21], &[], 0xdead, null, INTS).gpr[0];
    let one_in_ctx = call(need("$s5Spike3OneV5twiceSiyF"), &[0xdead], &[], 21, null, INTS).gpr[0];
    println!("S2 One(v: 21).twice(): self in x0 → {one_in_x0}; self in the context register → {one_in_ctx}");

    // Tag: Int + String = three integer words.
    let label = mk("seven");
    let tag = call(need("$s5Spike3TagV2id5labelACSi_SStcfC"), &[7, label[0], label[1]], &[], 0, null, INTS);
    println!("S1 Tag(id: 7, label: \"seven\") comes back in x0..x2           = ({}, {:?})", tag.gpr[0], rd(tag.gpr[1], tag.gpr[2]));
    let prefix = mk("tag");
    let shown = rd_ret(call(need("$s5Spike3TagV4show6prefixS2S_tF"), &[prefix[0], prefix[1], tag.gpr[0], tag.gpr[1], tag.gpr[2]], &[], 0, null, INTS));
    println!("S2 tag.show(prefix:): argument words, then self's three words = {shown:?}");

    // Mixed: integer and floating-point registers in one value.
    let ints = call(need("$s5Spike5MixedV1a1b1cACSi_SdSbtcfC"), &[1, 1], &[2.5], 0, null, INTS);
    let floats = call(need("$s5Spike5MixedV1a1b1cACSi_SdSbtcfC"), &[1, 1], &[2.5], 0, null, FLOATS);
    println!("S1 Mixed(a: 1, b: 2.5, c: true): a, c in x0, x1 = ({}, {}); b in d0 = {}   (two calls: one trampoline reads only one kind)",
             ints.gpr[0], ints.gpr[1] & 1, floats.fpr[0]);
    let sum = call(need("$s5Spike5MixedV3sumSdyF"), &[1, 1], &[2.5], 0, null, FLOATS).fpr[0];
    println!("S2 mixed.sum(): self as x0, d0, x1                             = {sum}");

    // Wide: by address; fields read through reflected offsets.
    let wide_ty = ty("5Spike4WideV");
    let mut wide = vec![0u64; wide_ty.size().div_ceil(8)];
    call(need("$s5Spike4WideVACycfC"), &[], &[], 0, wide.as_mut_ptr().cast(), INDIRECT);
    let note = fields(wide_ty.as_ptr()).iter().find(|f| f.0 == "note").unwrap().2 as usize / 8;
    println!("S5 Wide() built by address; a..d = {:?}; note at its reflected offset = {:?}", &wide[..4], rd(wide[note], wide[note + 1]));
}

#[no_mangle]
extern "C" fn rs_title(state: *const &'static str, out: *mut [u64; 2]) {
    unsafe { *out = mk(&format!("Rust<{}>", *state)) }
}
#[no_mangle]
extern "C" fn rs_greet(state: *const &'static str, name0: u64, name1: u64, out: *mut [u64; 2]) {
    unsafe { *out = mk(&format!("{} greets {} from Rust", *state, rd(name0, name1))) }
}

fn protocol(evolution: bool) {
    let null = std::ptr::null_mut();
    // P1: `any Greeter` is five words: three of inline storage, the type's metadata, the witness table.
    for kind in [0u64, 1] {
        let mut existential = [0usize; 5];
        call(need("$s5Spike10anyGreeteryAA0C0_pSiF"), &[kind], &[], 0, existential.as_mut_ptr().cast(), INDIRECT);
        let (metadata, witnesses) = (existential[3], existential[4] as *const usize);
        let this = existential.as_ptr() as usize; // both test types fit the inline buffer
        let title = rd_ret(call(unsafe { *witnesses.add(1) }, &[metadata as u64, witnesses as u64], &[], this, null, INTS));
        let name = mk("Rust");
        let greet = rd_ret(call(unsafe { *witnesses.add(2) }, &[name[0], name[1], metadata as u64, witnesses as u64], &[], this, null, INTS));
        println!("P1 anyGreeter({kind}) holds a {}: witness table entries → title {title:?}, greet {greet:?}", type_name(metadata as *const c_void));
        if evolution {
            let thunk = rd_ret(call(need("$s5Spike7GreeterP5greetyS2SFTj"), &[name[0], name[1], metadata as u64, witnesses as u64], &[], this, null, INTS));
            let index = (need("$s5Spike7GreeterP5greetyS2SFTq") - need("$s5Spike7GreeterTL")) / 8;
            println!("P2 the requirement's dispatch thunk → {thunk:?}; (method descriptor − base descriptor) / 8 = witness index {index}");
        }
        let used = rd_ret(call(need("$s5Spike10useGreeterySSAA0C0_pF"), &[existential.as_ptr() as u64], &[], 0, null, INTS));
        println!("P3 useGreeter(any Greeter): the existential passed by address  = {used:?}");
        unsafe { swift_bridgeObjectRelease(name[1]) };
    }

    // P5: a Greeter implemented in Rust. Self is UnsafeMutableRawPointer (pointing at Rust state); the witness
    // table is built by hand: [conformance descriptor, title getter, greet, greetAsync].
    let state: &'static str = "ferris";
    let pointer_metadata = need("$sSvN");
    let table = [0usize, unsafe { m3_title_witness() } as usize, unsafe { m3_greet_witness() } as usize, 0];
    let value = &state as *const &str as usize; // a value of type UnsafeMutableRawPointer
    let generic = rd_ret(call(need("$s5Spike10useGenericySSxAA7GreeterRzlF"), &[&value as *const usize as u64, pointer_metadata as u64, table.as_ptr() as u64], &[], 0, null, INTS));
    println!("P5 useGeneric<G: Greeter>(rust value, G = UnsafeMutableRawPointer, our table) = {generic:?}");
    let existential = [value, 0, 0, pointer_metadata, table.as_ptr() as usize];
    let any = rd_ret(call(need("$s5Spike10useGreeterySSAA0C0_pF"), &[existential.as_ptr() as u64], &[], 0, null, INTS));
    println!("P5 useGreeter(any Greeter built by hand around the Rust value)                 = {any:?}");
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let evolution = args.get(1).map(String::as_str) == Some("evolution");
    let lib = CString::new(format!("{}/lib{}.dylib", env!("CARGO_MANIFEST_DIR"), if evolution { "evolution" } else { "plain" })).unwrap();
    assert!(!unsafe { dlopen(lib.as_ptr(), 1) }.is_null(), "cannot load the spike library");
    match args.get(2).map(String::as_str) {
        Some("class") => class(evolution),
        Some("struct") => strukt(evolution),
        Some("protocol") => protocol(evolution),
        _ => eprintln!("usage: m3-spike <plain|evolution> <class|struct|protocol>"),
    }
}
