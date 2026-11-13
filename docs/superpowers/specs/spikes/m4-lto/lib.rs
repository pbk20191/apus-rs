extern "C" {
    fn swift_add(a: isize, b: isize) -> isize;
    fn swift_mul_add(a: isize, b: isize, c: isize) -> isize;
}
#[no_mangle]
pub extern "C" fn rust_mul(a: isize, b: isize) -> isize { a.wrapping_mul(b) }

#[no_mangle]
#[inline(never)]
pub extern "C" fn rust_calls_swift(a: isize, b: isize) -> isize { unsafe { swift_add(a, b) } }
#[no_mangle]
#[inline(never)]
pub extern "C" fn rust_swift_rust(a: isize, b: isize, c: isize) -> isize { unsafe { swift_mul_add(a, b, c) } }

#[no_mangle]
pub extern "C" fn main() -> i32 {
    let n = std::env::args().count() as isize;
    println!("{} {}", rust_calls_swift(n, 41), rust_swift_rust(n + 2, 7, 21));
    0
}
