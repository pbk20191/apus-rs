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
