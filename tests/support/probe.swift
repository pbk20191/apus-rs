// Test-only: plain Swift code asking the runtime about "the current Task".
import Darwin
enum TL { @TaskLocal static var name: String = "<none>" }
@_cdecl("probe_has_task") public func probeHasTask() -> Bool { withUnsafeCurrentTask { $0 != nil } }
@_cdecl("probe_is_cancelled") public func probeIsCancelled() -> Bool { Task.isCancelled }
@_cdecl("probe_priority") public func probePriority() -> UInt8 { Task.currentPriority.rawValue }
@_cdecl("probe_assume_main") public func probeAssumeMain() -> Bool { MainActor.assumeIsolated { pthread_main_np() == 1 } }
@_cdecl("probe_with_local")
public func probeWithLocal(_ v: UnsafePointer<CChar>, _ body: @convention(c) () -> Void) {
    TL.$name.withValue(String(cString: v)) { body() }
}
@_cdecl("probe_read_local") public func probeReadLocal(_ buf: UnsafeMutablePointer<CChar>, _ n: Int) { _ = strlcpy(buf, TL.name, n) }
public typealias PrioCb = @convention(c) (UnsafeMutableRawPointer?, UInt8) -> Void
@_cdecl("probe_spawn_swift_task")
public func probeSpawnSwiftTask(_ ctx: UnsafeMutableRawPointer?, _ cb: PrioCb) {
    let bits = UInt(bitPattern: ctx)
    Task { cb(UnsafeMutableRawPointer(bitPattern: bits), Task.currentPriority.rawValue) }
}

// ---- M0: the user-side recipe (what README will show) ----
public final class AsyncSwiftClosure {
    let body: () async throws -> Any
    init(_ body: @escaping () async throws -> Any) { self.body = body }
}
/// Hands a Swift async closure to Rust as an owned (+1) pointer.
public func asyncSwiftClosure(_ body: @escaping () async throws -> Any) -> UnsafeMutableRawPointer {
    Unmanaged.passRetained(AsyncSwiftClosure(body)).toOpaque()
}
// ---- M0 probe closures ----
@_cdecl("probe_closure_identity") public func probeClosureIdentity() -> UnsafeMutableRawPointer {
    asyncSwiftClosure { withUnsafeCurrentTask { t in Int(bitPattern: unsafeBitCast(t!, to: UInt.self)) } }
}
@_cdecl("probe_closure_sleep") public func probeClosureSleep(_ ms: Int) -> UnsafeMutableRawPointer {
    asyncSwiftClosure { try await Task.sleep(nanoseconds: UInt64(ms) * 1_000_000); return "slept" }
}
@_cdecl("probe_closure_throw_eio") public func probeClosureThrowEIO() -> UnsafeMutableRawPointer {
    asyncSwiftClosure { throw ProbeError.io(code: EIO) }
}
@_cdecl("probe_closure_void") public func probeClosureVoid() -> UnsafeMutableRawPointer {
    asyncSwiftClosure { () }
}
@_cdecl("probe_closure_read_local") public func probeClosureReadLocal() -> UnsafeMutableRawPointer {
    asyncSwiftClosure { "\(TL.name)/0x\(String(Task.currentPriority.rawValue, radix: 16))" }
}
@_cdecl("probe_any_describe") public func probeAnyDescribe(_ p: UnsafeRawPointer) -> UnsafeMutablePointer<CChar> {
    strdup(String(describing: p.load(as: Any.self)))!
}
@_cdecl("probe_any_destroy") public func probeAnyDestroy(_ p: UnsafeMutableRawPointer) {
    p.assumingMemoryBound(to: Any.self).deinitialize(count: 1)
}
@_cdecl("probe_error_describe") public func probeErrorDescribe(_ p: UnsafeRawPointer) -> UnsafeMutablePointer<CChar> {
    var bits = p
    let e = withUnsafeBytes(of: &bits) { $0.load(as: Error.self) }   // copy: +1, balanced by Swift
    return strdup("\(type(of: e)): \(e)")!
}

enum ProbeError: Error { case io(code: Int32) }
@_cdecl("probe_closure_block_ms") public func probeClosureBlockMs(_ ms: Int) -> UnsafeMutableRawPointer {
    asyncSwiftClosure { usleep(UInt32(ms) * 1000); return "blocked \(ms)ms" }   // ignores cancellation
}

// ---- Task 7: Any / Error values handed to Rust ----
final class DeinitFlag { let flag: UnsafeMutablePointer<Bool>; init(_ f: UnsafeMutablePointer<Bool>) { flag = f }; deinit { flag.pointee = true } }
@_cdecl("probe_any_int") public func probeAnyInt(_ out: UnsafeMutableRawPointer) { out.initializeMemory(as: Any.self, repeating: 42, count: 1) }
@_cdecl("probe_any_string") public func probeAnyString(_ out: UnsafeMutableRawPointer) { out.initializeMemory(as: Any.self, repeating: "héllo👋", count: 1) }
@_cdecl("probe_any_optional_nil") public func probeAnyOptionalNil(_ out: UnsafeMutableRawPointer) { let v: Int? = nil; out.initializeMemory(as: Any.self, repeating: v as Any, count: 1) }
@_cdecl("probe_any_optional_some") public func probeAnyOptionalSome(_ out: UnsafeMutableRawPointer) { let v: Int? = 7; out.initializeMemory(as: Any.self, repeating: v as Any, count: 1) }
@_cdecl("probe_any_void") public func probeAnyVoid(_ out: UnsafeMutableRawPointer) { out.initializeMemory(as: Any.self, repeating: (), count: 1) }
@_cdecl("probe_any_object") public func probeAnyObject(_ out: UnsafeMutableRawPointer, _ flag: UnsafeMutablePointer<Bool>) { out.initializeMemory(as: Any.self, repeating: DeinitFlag(flag), count: 1) }
@_cdecl("probe_make_error") public func probeMakeError(_ out: UnsafeMutableRawPointer) { out.initializeMemory(as: Error.self, repeating: ProbeError.io(code: EIO), count: 1) }

// ---- Task 8 ----
@_cdecl("probe_closure_deinit_flag") public func probeClosureDeinitFlag(_ flag: UnsafeMutablePointer<Bool>) -> UnsafeMutableRawPointer {
    let captured = DeinitFlag(flag)
    return asyncSwiftClosure { _ = captured; return () }
}

// ---- Task 9 ----
@_cdecl("probe_closure_sleep_flag_cancel") public func probeClosureSleepFlagCancel(_ ms: Int, _ cancelled: UnsafeMutablePointer<Bool>) -> UnsafeMutableRawPointer {
    let bits = UInt(bitPattern: cancelled)
    return asyncSwiftClosure {
        do { try await Task.sleep(nanoseconds: UInt64(ms) * 1_000_000); return "slept" }
        catch { UnsafeMutablePointer<Bool>(bitPattern: bits)!.pointee = true; throw error }
    }
}

// ---- collections handed to Rust as Any ----
@_cdecl("probe_any_int_array") public func probeAnyIntArray(_ out: UnsafeMutableRawPointer) { out.initializeMemory(as: Any.self, repeating: [1, 2, 3], count: 1) }
@_cdecl("probe_any_string_array") public func probeAnyStringArray(_ out: UnsafeMutableRawPointer) { out.initializeMemory(as: Any.self, repeating: ["a", "héllo👋", "", String(repeating: "long", count: 20)], count: 1) }
@_cdecl("probe_any_empty_array") public func probeAnyEmptyArray(_ out: UnsafeMutableRawPointer) { out.initializeMemory(as: Any.self, repeating: [Int](), count: 1) }
@_cdecl("probe_any_nested_array") public func probeAnyNestedArray(_ out: UnsafeMutableRawPointer) { out.initializeMemory(as: Any.self, repeating: [[1], [], [2, 3]], count: 1) }
@_cdecl("probe_any_optional_array") public func probeAnyOptionalArray(_ out: UnsafeMutableRawPointer) { let v: [Int?] = [1, nil, 3]; out.initializeMemory(as: Any.self, repeating: v, count: 1) }
@_cdecl("probe_any_bool_double_array") public func probeAnyBoolDoubleArray(_ out: UnsafeMutableRawPointer, _ bools: Bool) {
    if bools { out.initializeMemory(as: Any.self, repeating: [true, false, true], count: 1) }
    else { out.initializeMemory(as: Any.self, repeating: [1.5, -2.0], count: 1) }
}
@_cdecl("probe_any_dict") public func probeAnyDict(_ out: UnsafeMutableRawPointer) { out.initializeMemory(as: Any.self, repeating: ["one": 1, "two": 2, "three": 3], count: 1) }
@_cdecl("probe_any_big_dict") public func probeAnyBigDict(_ out: UnsafeMutableRawPointer) {
    var d = [Int: String](); for i in 0..<200 { d[i] = "v\(i)" }
    out.initializeMemory(as: Any.self, repeating: d, count: 1)
}
@_cdecl("probe_any_empty_dict") public func probeAnyEmptyDict(_ out: UnsafeMutableRawPointer) { out.initializeMemory(as: Any.self, repeating: [String: Int](), count: 1) }
@_cdecl("probe_any_dict_of_arrays") public func probeAnyDictOfArrays(_ out: UnsafeMutableRawPointer) { out.initializeMemory(as: Any.self, repeating: ["a": [1, 2], "b": []], count: 1) }
