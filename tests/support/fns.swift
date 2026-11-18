// M1 spike: ordinary Swift async functions, called from Rust by mangled symbol. No @_cdecl, no glue.
import Darwin

public enum DemoError: Error { case code(Int) }

@inline(never) public func addAsync(_ a: Int, _ b: Int) async -> Int {
    await Task.yield()
    return a + b
}
@inline(never) public func currentTaskBits() async -> Int {
    await Task.yield()
    return withUnsafeCurrentTask { t in Int(bitPattern: unsafeBitCast(t!, to: UInt.self)) }
}
@inline(never) public func greet(_ name: String, times: Int, sleepMs: Int) async throws -> String {
    try await Task.sleep(nanoseconds: UInt64(sleepMs) * 1_000_000)
    return String(repeating: name + " ", count: times)
}
@inline(never) public func half(_ x: Double, plus y: Int) async -> Double {
    await Task.yield()
    return x / 2 + Double(y)
}
@inline(never) public func failing(code: Int) async throws -> Int {
    await Task.yield()
    throw DemoError.code(code)
}
@inline(never) public func names(count: Int) async -> [String] {
    await Task.yield()
    return (0..<count).map { "name\($0)" }
}
@inline(never) public func maybe(_ x: Int) async -> Int? {
    await Task.yield()
    return x > 0 ? x * 10 : nil
}
@inline(never) public func isOnMain() async -> Bool {
    await Task.yield()
    return pthread_main_np() == 1
}
public final class Counter {
    var total = 0
    public init() {}
    @inline(never) public func bump(by n: Int) async -> Int {
        await Task.yield()
        total += n
        return total
    }
}
@inline(never) public func makeCounter() -> Counter { Counter() }
@inline(never) public func maybeName(_ x: Int) async -> String? {
    await Task.yield()
    return x > 0 ? "name\(x)" : nil
}
@inline(never) public func describe(_ flag: Bool, _ n: Int32, _ big: UInt64) async -> String {
    await Task.yield()
    return "\(flag) \(n) \(big)"
}
@inline(never) public func makeCounterAsync(start: Int) async -> Counter {
    await Task.yield()
    let c = Counter(); c.total = start
    return c
}
@inline(never) public func anyValue(_ x: Int) async -> Any {
    await Task.yield()
    return x * 2
}
@inline(never) public func nothing() async {
    await Task.yield()
}
@inline(never) public func isEven(_ x: Int) async -> Bool {
    await Task.yield()
    return x % 2 == 0
}
@inline(never) public func scores() async -> [String: Int] {
    await Task.yield()
    return ["a": 1, "b": 2]
}
// ---- M2: generics, metadata, witness tables, indirect values, synchronous calls ----
public struct Big {
    public var a: Int, b: Int, c: Int, d: Int
    public var name: String
    public var total: Int { a + b + c + d }                       // sync getter, self passed indirectly
    @inline(never) public func scaled(by k: Int) -> Big { Big(a: a * k, b: b * k, c: c * k, d: d * k, name: name + "x\(k)") }
    @inline(never) public static func make(n: Int) async -> Big { await Task.yield(); return Big(a: n, b: n + 1, c: n + 2, d: n + 3, name: "big\(n)") }
}
@inline(never) public func echo<T>(_ x: T) async -> T { await Task.yield(); return x }
@inline(never) public func joinAll<C: Collection>(_ c: C, sep: String) async -> String where C.Element == String {
    await Task.yield(); return c.joined(separator: sep)
}
@inline(never) public func describeIt<T: CustomStringConvertible>(_ x: T, times: Int) async throws -> String {
    await Task.yield(); return String(repeating: x.description, count: times)
}
@inline(never) public func bigSum(_ b: Big, plus: Int) async -> Int { await Task.yield(); return b.total + plus }
@inline(never) public func syncAdd(_ a: Int, _ b: Double) -> Double { Double(a) + b }
@inline(never) public func syncThrow(_ fail: Bool) throws -> Int { if fail { throw DemoError.code(9) }; return 1 }
@inline(never) public func sumAll(_ xs: [Int]) async -> Int { await Task.yield(); return xs.reduce(0, +) }
@inline(never) public func byteCount(_ s: String) -> Int { s.utf8.count }

// ---- async sequences ----
@inline(never) public func countdown(from n: Int) -> AsyncStream<Int> {
    AsyncStream { continuation in
        Task {
            for i in stride(from: n, through: 1, by: -1) {
                continuation.yield(i)
                try? await Task.sleep(nanoseconds: 2_000_000)
            }
            continuation.finish()
        }
    }
}
@inline(never) public func failingStream(after n: Int) -> AsyncThrowingStream<String, Error> {
    AsyncThrowingStream { continuation in
        for i in 0..<n { continuation.yield("item\(i)") }
        continuation.finish(throwing: DemoError.code(7))
    }
}
/// A hand-written sequence whose iterator has state beyond one word.
public struct Ticks: AsyncSequence {
    public typealias Element = String
    let count: Int
    public struct Iterator: AsyncIteratorProtocol {
        var produced = 0
        let count: Int
        var log: [String] = []
        public mutating func next() async -> String? {
            guard produced < count else { return nil }
            await Task.yield()
            produced += 1
            log.append("t\(produced)")
            return "tick\(produced) of \(count), after \(log.count - 1) earlier"
        }
    }
    public func makeAsyncIterator() -> Iterator { Iterator(count: count) }
}
@inline(never) public func ticks(_ n: Int) -> Ticks { Ticks(count: n) }
nonisolated(unsafe) var endlessLock = os_unfair_lock()
nonisolated(unsafe) var endlessTerminated = Set<Int>()
/// Yields 0, 1, 2, … every millisecond until its consumer goes away; `tag` identifies it to the test.
@inline(never) public func endless(tag: Int) -> AsyncStream<Int> {
    AsyncStream { continuation in
        let producer = Task {
            var i = 0
            while !Task.isCancelled {
                continuation.yield(i)
                i += 1
                try? await Task.sleep(nanoseconds: 1_000_000)
            }
        }
        continuation.onTermination = { _ in
            producer.cancel()
            os_unfair_lock_lock(&endlessLock)
            endlessTerminated.insert(tag)
            os_unfair_lock_unlock(&endlessLock)
        }
    }
}

// ObjC-backed class instances (NSObject subclasses) for the objc2 feature and SwiftObject's reference counting.
import Foundation

public final class Tracked: NSObject {
    // Deinits counted per value: tests run in parallel and each uses its own value.
    nonisolated(unsafe) static var deinits: [Int: Int] = [:]
    static let lock = NSLock()
    public let value: Int
    public init(value: Int) { self.value = value }
    deinit { Tracked.lock.withLock { Tracked.deinits[value, default: 0] += 1 } }
    @inline(never) public func doubled() -> Int { value * 2 }
}
@inline(never) public func makeTracked(_ value: Int) -> Tracked { Tracked(value: value) }
@inline(never) public func trackedDeinits(_ value: Int) -> Int { Tracked.lock.withLock { Tracked.deinits[value, default: 0] } }
@inline(never) public func trackedValue(_ object: Tracked) -> Int { object.value }
@inline(never) public func className(_ object: NSObject) -> String { String(describing: type(of: object)) }
@inline(never) public func echoObject(_ object: NSObject) -> NSObject { object }

// Isolation (tests/swift_isolation.rs).
@inline(never) public func withIsolation(_ x: Int, isolation: isolated (any Actor)? = #isolation) async -> Int {
    guard let isolation else { return x }
    return x + (isolation === MainActor.shared ? 1000 : 2000)
}
@inline(never) public func isolationKind(isolation: isolated (any Actor)? = #isolation) -> Int {
    guard let isolation else { return 0 }
    return isolation === MainActor.shared ? 1 : 2
}

// Actor and @MainActor sync members (tests/swift_isolation.rs). preconditionIsolated traps off the actor.
public actor Tally {
    var n = 0
    public init() {}
    @inline(never) public func bump(by k: Int) -> Int { preconditionIsolated(); n += k; return n }
    public var count: Int { preconditionIsolated(); return n }
    @inline(never) public nonisolated func label() -> String { "tally" }
}
@inline(never) public func makeTally() -> Tally { Tally() }

@MainActor @inline(never) public func onMainThread() -> Bool { MainActor.assumeIsolated { Thread.isMainThread } }

public final class QueueExecutor: SerialExecutor, @unchecked Sendable {
    let queue = DispatchQueue(label: "probe.custom")
    public func enqueue(_ job: consuming ExecutorJob) {
        let job = UnownedJob(job)
        queue.async { job.runSynchronously(on: self.asUnownedSerialExecutor()) }
    }
    public func asUnownedSerialExecutor() -> UnownedSerialExecutor { UnownedSerialExecutor(ordinary: self) }
}
public actor Custom {
    nonisolated let executor = QueueExecutor()
    public nonisolated var unownedExecutor: UnownedSerialExecutor { executor.asUnownedSerialExecutor() }
    public init() {}
    @inline(never) public func onOwnQueue() -> Bool {
        preconditionIsolated()
        return String(cString: __dispatch_queue_get_label(nil)) == "probe.custom"
    }
}
@inline(never) public func makeCustom() -> Custom { Custom() }

// Typed throws with a generic error type (tests/swift_isolation.rs).
// Five words: returned by address, as apus receives a SwiftValue result.
public struct Boom: Error {
    public let code: Int
    let padding: (Int, Int, Int, Int) = (0, 0, 0, 0)
    public init(code: Int) { self.code = code }
}
@inline(never) public func makeBoom(code: Int) -> Boom { Boom(code: code) }
@inline(never) public func errorCode<E: Error>(_ error: E) -> Int { (error as? Boom)?.code ?? -1 }
@inline(never) public func genericTyped<E: Error>(_ fail: Bool, _ error: E) async throws(E) -> Int {
    await Task.yield()
    if fail { throw error }
    return 42
}
@inline(never) public func genericTypedSync<E: Error>(_ fail: Bool, _ error: E) throws(E) -> Int {
    if fail { throw error }
    return 42
}
/// Like failingStream, with a concrete Failure: next(isolation:) writes a DemoError into the error slot.
public struct TypedFailing: AsyncSequence {
    public typealias Element = String
    public typealias Failure = DemoError
    let count: Int
    public struct Iterator: AsyncIteratorProtocol {
        var produced = 0
        let count: Int
        public mutating func next() async throws(DemoError) -> String? {
            defer { produced += 1 }
            if produced < count { return "item\(produced)" }
            if produced == count { throw DemoError.code(7) }
            return nil
        }
    }
    public func makeAsyncIterator() -> Iterator { Iterator(count: count) }
}
@inline(never) public func failingTypedStream(after n: Int) -> TypedFailing { TypedFailing(count: n) }

// nonisolated(nonsending): a hidden isolation argument comes first (tests/swift_isolation.rs).
@inline(never) nonisolated(nonsending) public func inherits(_ x: Int) async -> Int { x + 1 }
// (`#isolation === MainActor.shared` directly crashes swiftc 6.4.2.)
@inline(never) nonisolated(nonsending) public func inheritsMain() async -> Bool {
    let isolation = #isolation
    return isolation.map { $0 === MainActor.shared } ?? false
}

// A hop call abandoned mid-flight (tests/swift_isolation.rs): the actor's last reference is the call's. Its deinit must
// run after the Task has left the actor's executor, never inside one of its jobs.
public final class FlaggingExecutor: SerialExecutor, @unchecked Sendable {
    let queue = DispatchQueue(label: "probe.flagging")
    nonisolated(unsafe) static var inJob = false
    public func enqueue(_ job: consuming ExecutorJob) {
        let job = UnownedJob(job)
        queue.async {
            FlaggingExecutor.inJob = true
            job.runSynchronously(on: self.asUnownedSerialExecutor())
            FlaggingExecutor.inJob = false
        }
    }
    public func asUnownedSerialExecutor() -> UnownedSerialExecutor { UnownedSerialExecutor(ordinary: self) }
}
nonisolated(unsafe) var slowDeinitInJob = -1   // -1: not deinitialized yet, 1: inside a job, 0: outside
public actor Slow {
    nonisolated let executor = FlaggingExecutor()
    public nonisolated var unownedExecutor: UnownedSerialExecutor { executor.asUnownedSerialExecutor() }
    public init() {}
    deinit { slowDeinitInJob = FlaggingExecutor.inJob ? 1 : 0 }
    @inline(never) public func nap(ms: Int) -> Int { usleep(UInt32(ms) * 1000); return ms }
}
@inline(never) public func makeSlow() -> Slow { Slow() }
@inline(never) public func slowDeinitState() -> Int { slowDeinitInJob }

// Errors (tests/swift_errors.rs).
public struct StructError: Error {
    public let code: Int
    public let detail: String
    public init(code: Int, detail: String) { self.code = code; self.detail = detail }
}
@inline(never) public func throwStruct(code: Int) throws -> Int { throw StructError(code: code, detail: "bad") }
@inline(never) public func throwNSError(code: Int) throws -> Int { throw NSError(domain: "probe.domain", code: code) }
@inline(never) public func structErrorCode<E: Error>(_ error: E) -> Int { (error as? StructError)?.code ?? -1 }

// Rust closures as Swift closures (tests/swift_closures.rs).
@inline(never) public func applyConcrete(_ f: (Int) async -> Int) async -> Int { await f(20) + 1 }
@inline(never) public func applyGeneric<T>(_ x: T, _ f: (T) async -> T) async -> T { await f(x) }
@inline(never) public func applySync(_ f: (Int) -> Int) -> Int { f(3) * 10 }
@inline(never) public func applySyncDouble(_ f: (Double, Int) -> Double) -> Double { f(1.5, 2) }
@inline(never) public func applyMain(_ f: @MainActor (Int) async -> Int) async -> Int { await f(1) }
@inline(never) public func applyEscaping(_ f: @escaping @Sendable (Int, String) async throws -> String) async throws -> String {
    async let a = f(1, "a")
    async let b = f(2, "b")
    return try await a + b
}
@inline(never) public func applyCancelled(_ f: @escaping @Sendable () async -> Bool) async -> Bool {
    let task = Task { await f() }
    task.cancel()
    return await task.value
}
nonisolated(unsafe) var keptClosure: (@Sendable () -> Int)?
@inline(never) public func keep(_ f: @escaping @Sendable () -> Int) { keptClosure = f }
@inline(never) public func callKept() -> Int { keptClosure?() ?? -1 }
@inline(never) public func dropKept() { keptClosure = nil }
@inline(never) public func applySyncGeneric<T>(_ x: T, _ f: (T) -> T) -> T { f(x) }
@inline(never) public func makeAnyError() -> any Error { StructError(code: 1, detail: "x") }

// An enum of a module without library evolution: no tag symbols, tags by declaration order (tests/swift_enum.rs).
public enum Light { case red, amber(Int), green }
// Its metadata, kept alive for the test (nothing else in the binary refers to the type).
public func lightMetadata() -> UnsafeRawPointer { unsafeBitCast(Light.self as Any.Type, to: UnsafeRawPointer.self) }

// A class's initializer and static method take the class (its metadata) as `self` (tests/swift_init.rs).
public final class Minted {
    public let n: Int
    public init(n: Int) { self.n = n }
    public static func twice(_ n: Int) -> Minted { Minted(n: n * 2) }
    public var value: Int { n }
    nonisolated(unsafe) static var gone: Set<Int> = []
    static let goneLock = NSLock()
    deinit { Minted.goneLock.withLock { _ = Minted.gone.insert(n) } }
}
public func mintedGone(_ n: Int) -> Bool { Minted.goneLock.withLock { Minted.gone.contains(n) } }
// An initializer consumes its arguments (they are passed +1): a String and a class reference to keep.
public final class Holder {
    public let s: String
    public let o: Minted
    public init(s: String, o: Minted) { self.s = s; self.o = o }
    public var text: String { s }
}
