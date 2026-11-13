// The "Swift async API" side: plain Swift, called through @_cdecl.
import Darwin

public typealias RsDone = @convention(c) (UnsafeMutableRawPointer?, Int32, UInt8) -> Void  // ctx, cancelled, priority

final class Handle { let task: Task<Void, Never>; init(_ t: Task<Void, Never>) { task = t } }

/// Starts `Task { try await Task.sleep(ms) }` and reports back; returns a cancellable handle.
@_cdecl("swift_sleep_async")
public func swiftSleepAsync(_ ms: Int64, _ ctx: UnsafeMutableRawPointer?, _ done: RsDone) -> UnsafeMutableRawPointer {
    let bits = UInt(bitPattern: ctx)
    let t = Task {
        let prio = Task.currentPriority.rawValue
        do { try await Task.sleep(nanoseconds: UInt64(ms) * 1_000_000); done(UnsafeMutableRawPointer(bitPattern: bits), 0, prio) }
        catch { done(UnsafeMutableRawPointer(bitPattern: bits), 1, prio) }
    }
    return Unmanaged.passRetained(Handle(t)).toOpaque()
}
@_cdecl("swift_handle_cancel") public func swiftHandleCancel(_ h: UnsafeMutableRawPointer) {
    Unmanaged<Handle>.fromOpaque(h).takeUnretainedValue().task.cancel()
}
@_cdecl("swift_handle_release") public func swiftHandleRelease(_ h: UnsafeMutableRawPointer) {
    Unmanaged<Handle>.fromOpaque(h).release()
}
/// Traps unless the caller is isolated to the MainActor.
@_cdecl("swift_assume_main_actor") public func swiftAssumeMainActor() -> Bool {
    MainActor.assumeIsolated { pthread_main_np() == 1 }
}
