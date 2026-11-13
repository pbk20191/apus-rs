@_cdecl("swift_add") public func swiftAdd(_ a: Int, _ b: Int) -> Int { a &+ b }
/// Calls back into Rust: after cross-language inlining this is a single multiply-add.
@_cdecl("swift_mul_add") public func swiftMulAdd(_ a: Int, _ b: Int, _ c: Int) -> Int { rust_mul(a, b) &+ c }
