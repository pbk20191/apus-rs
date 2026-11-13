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
