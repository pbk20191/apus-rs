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
