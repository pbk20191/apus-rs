// Ground truth for mangle/tests/corpus.rs: compiled with swiftc as module `Probe`; every declaration line in
// corpus.rs must mangle to a symbol this file exports.
public struct Big { public var a = 0
  public func scaled(by k: Int) -> Big { self }
  public func two(_ x: Big, _ y: Big) -> Big { self }
  public static func make(n: Int) async -> Big { Big() }
  public var total: Int { 0 }
  public static var shared: Big { Big() }
  public struct Inner { public func f() -> Inner { self } ; public func g(_ b: Big) -> Big { b } }
}
public final class Counter { public init() {} ; public func bump(by n: Int) async -> Int { 0 } ; public func me() -> Counter { self }
  public var name: String { "" } }
public enum Color { case red ; public func next(_ c: Color) -> Color { c } }
public protocol Shape { func area() -> Double }
public struct Box<T> { public init() {} }
public var globalCount: Int { 0 }
public func f1(_ a: Big, _ b: Big) -> Big { a }
public func f2(_ a: [String], _ b: [String]) -> [String] { a }
public func f3(_ a: Int?, _ b: Int?) -> [Int?] { [] }
public func f4(_ a: [String: Int], _ b: Set<Int>) -> [String: [Int]] { [:] }
public func f5(_ c: Counter, _ k: Color) -> Counter? { nil }
public func f6(_ s: any Shape) -> Any { 0 }
public func f7(_ t: (Int, String), named: (a: Int, b: Bool)) -> (Int, Int) { (0, 0) }
public func f8(_ a: UInt8, _ b: Int32, _ c: UInt64, _ d: Float, _ e: Character, _ f: UInt) {}
public func f9(counterMaker: Counter, makeCounter: Int, counter: Int) {}
public func fa(_ x: Big.Inner, y: Big.Inner) -> Big { Big() }
public func fb(_ a: Double, _ b: Double, _ c: Double, _ d: Bool) -> Double { 0 }
public func fc<T>(_ x: T, _ y: T) -> [T] { [] }
public func fd<T, U>(_ x: T, _ y: U) -> U { y }
public func fe<T: Hashable, U: Shape>(_ x: T, _ y: U) -> [T: U] { [:] }
public func ff<S: Sequence>(_ s: S) -> [S.Element] where S.Element: Equatable { [] }
public func fg<C: Collection>(_ c: C) async throws -> Int where C.Element == String { 0 }
public func fh(_ a: Int, b: Int, _ c: inout Int, d: Int...) {}
public func fj() throws {}
public func fk() async {}
public func fl(_ a: Swift.Error, _ b: AnyObject, _ c: UnsafeRawPointer, _ d: UnsafeMutablePointer<Int>) {}
public func fm(_ a: any Shape, _ b: any Shape, _ c: Box<Int>, _ d: Box<Int>, _ e: Box<String>) {}
public func fn(_ a: Big, _ b: Counter, _ c: Big, _ d: Counter, _ e: Color) -> Color { e }
public func fo<T: Comparable & Hashable, U>(_ x: T, _ y: U, _ z: T) -> T where U: Sequence, U.Element == T { x }
public func fp<T: CustomStringConvertible>(_ x: T, times: Int) async throws -> String { "" }
public func fq(_ a: (Int, Int), _ b: (Int, Int), _ c: Int?, _ d: Int??) -> Int? { nil }
public func fr(aLongLabelName: Int, anotherLongLabelName: Int, labelName: Int, name: Int) -> Int { 0 }
public func addAsync(_ a: Int, _ b: Int) async -> Int { 0 }
public func greet(_ name: String, times: Int, sleepMs: Int) async throws -> String { "" }
public func makeCounter() -> Counter { Counter() }
public func names(count: Int) async -> [String] { [] }
public func currentTaskBits() async -> Int { 0 }
public func scores() async -> [String: Int] { [:] }
public func describe(_ flag: Bool, _ n: Int32, _ big: UInt64) async -> String { "" }
public func fs(_ a: Any, _ b: Any, _ c: AnyObject, _ d: AnyObject, _ e: any Error, _ f: any Error) {}
// ---- second batch: word substitutions, many substitutions, concurrency types, more generics ----
public struct S01 {} ; public struct S02 {} ; public struct S03 {} ; public struct S04 {} ; public struct S05 {}
public struct S06 {} ; public struct S07 {} ; public struct S08 {} ; public struct S09 {} ; public struct S10 {}
public struct S11 {} ; public struct S12 {} ; public struct S13 {} ; public struct S14 {} ; public struct S15 {}
public func many(_ a: S01, _ b: S02, _ c: S03, _ d: S04, _ e: S05, _ f: S06, _ g: S07, _ h: S08, _ i: S09, _ j: S10,
                 _ k: S11, _ l: S12, _ m: S13, _ n: S14, _ o: S15, _ p: S15, _ q: S14, _ r: S13, _ s: S01, _ t: S13, _ u: S13) -> S14 { n }
public struct URLSessionTask { public func resumeURLSession(urlSession: Int, sessionTask: Int, task: Int) {} }
public func my_snake_case(snake_case: Int, my_snake: Int, case_: Int) {}
public func fooFooFoo(foo: Int, Foo: Int, fooFoo: Int) {}
public func x2y(a1: Int, a1b2: Int, _a: Int, a_: Int, ab: Int, abAb: Int) {}
public func HTTPSProxy(httpsProxy: Int, proxyHTTPS: Int, HTTPS: Int, proxy: Int) {}
public func t1(_ t: Task<Int, Never>, _ s: AsyncStream<Int>, _ p: TaskPriority, _ q: TaskPriority) async {}
public func t2(_ a: [[Int]], _ b: [[Int]], _ c: [Int: [String: Int?]]?, _ d: Set<String>?) -> [[Int]]? { nil }
public func t3(_ a: (Int, Int)) {}
public func t4(pair: (Int, Int)) {}
public func t5(_ a: ()) {}
public func t6(_ a: Int32?, _ b: Int32, _ c: UInt8?, _ d: [UInt8]) {}
public func g1<A, B, C>(_ a: A, _ b: B, _ c: C) -> (A, B, C) { (a, b, c) }
public func g2<T: Sendable & Shape>(_ t: T) async {}
public func g3<S: Sequence>(_ s: S, _ e: S.Element) -> Int where S.Element == Int { 0 }
public func g4<S: Sequence>(_ s: S) -> Int where S.Element: Hashable { 0 }
public func g5<K: Hashable, V>(_ d: [K: V], _ k: K) -> V? { nil }
public func g6<T: Collection, U: Collection>(_ t: T, _ u: U) where T.Element == U.Element, T.Element: Equatable {}
public func g7<T>(_ t: T?, _ u: [T?]) -> T? { nil }
public extension Counter { func extra(_ x: Int) -> Int { x } }
public enum Outer { public enum Mid { public struct Leaf { public static func deep(_ o: Outer, _ m: Mid) -> Leaf { Leaf() } } } }
public func g8<T: Sequence, U: Sequence>(_ t: T, _ u: U, _ e: U.Element) -> [T.Element] where U.Element == T.Element { [] }
public func g9<S: Sequence, T>(_ s: S, _ e: S.Element) -> T where S.Element == T { e }
public func t7(_ b: Box<()>, _ c: ()?) -> ()? { nil }
// ---- third batch: extensions, members of generic types, protocol extensions ----
public extension Int { func doubled() -> Int { self * 2 } ; static func make(_ s: String) -> Int { 0 } ; var half: Int { self / 2 } }
public extension String { func shout(times: Int) -> String { self } }
public extension Array { func firstTwo() -> [Element] { [] } ; var second: Element? { nil } ; func pair(with other: [Element]) -> [[Element]] { [] } }
public extension Array where Element == Int { func total() -> Int { 0 } ; func firstOr(_ d: Element) -> Element { d } }
public extension Array where Element: Hashable { func unique() -> [Element] { self } }
public extension Optional { func orThrow() throws -> Wrapped { fatalError() } }
public extension Optional where Wrapped: Shape { func areaOrZero() -> Double { 0 } }
public extension Dictionary { func keysArray() -> [Key] { [] } ; func value(for k: Key) -> Value? { nil } }
public extension Dictionary where Value: Equatable { func hasEqual() -> Bool { false } }
public extension Dictionary where Key == String { func stringKeys() -> [Key] { [] } }
public extension Set where Element == Int { func sum() -> Int { 0 } }
public extension Box { func get() -> T? { nil } ; func put(_ t: T, at i: Int) {} ; var count: Int { 0 } ; static func empty() -> Box<T> { Box() }
  func map<U>(_ u: U) -> Box<U> { Box<U>() } ; func same(_ b: Box<T>) -> Box<T> { b }
  func contextual() -> Int where T: Hashable { 0 } ; func two<U, V>(_ u: U, _ v: V) where U: Hashable {} }
public extension Box where T: Shape { func areas() -> Double { 0 } }
public struct Pair<A, B> { public init() {} ; public func swap() -> Pair<B, A> { Pair<B, A>() } ; public func first(_ a: A) -> B? { nil }
  public struct Inner { public func f(_ a: A) -> B? { nil } } }
public extension Pair where A == Int, B: Hashable { func keyed() -> [B: Int] { [:] } }
public struct Keyed<K: Hashable> { public init() {} ; public func key() -> K? { nil } }
public extension Keyed where K == String { func name() -> String { "" } }
public extension Shape { func twice() -> Double { 0 } ; func same(as other: Self) -> Self { other } }
public extension Collection { func countTwice() -> Int { 0 } ; func firstElement() -> Element? { nil } }
public extension Sequence where Element == String { func joinedTwice() -> String { "" } }
public extension Task where Success == Never, Failure == Never { static func nap(_ n: UInt64) async throws {} }
public extension Task { func poke() {} }
public extension AsyncStream { func tag() -> Int { 0 } }

// isolation
public func withIsolation(_ x: Int, isolation: isolated (any Actor)? = #isolation) async -> Int { x }
public func isolatedTo(_ actor: isolated any Actor, _ n: Int) -> Int { n }
public actor Tally { var n = 0; public func bump(by k: Int) -> Int { n }; public var count: Int { n }; public nonisolated func label() -> String { "" } }
@MainActor public func onMainThread() -> Bool { true }
public func genericTyped<E: Error>(_ fail: Bool, _ error: E) async throws(E) -> Int { 0 }
public func anyTyped() throws(any Error) -> Int { 0 }
@inline(never) nonisolated(nonsending) public func inherits(_ x: Int) async -> Int { x + 1 }
// function-typed parameters
public func applyConcrete(_ f: (Int) async -> Int) async -> Int { 0 }
public func applyEscaping(_ f: @escaping @Sendable (Int, String) async throws -> String) async throws -> String { "" }
public func applyGeneric<T>(_ x: T, _ f: (T) async -> T) async -> T { x }
public func applySync(_ f: (Int) -> Int) -> Int { 0 }
public func applyMain(_ f: @MainActor (Int) async -> Int) async -> Int { 0 }
public func applyVoid(_ f: @escaping () async -> Void) async {}
public func applyLabeled(count: Int, transform f: (Double) -> Double) -> Double { 0 }
// Initializers: the allocating entry point (`fC`).
public enum Level: Int { case low = 1, high = 5 }
public struct Made { public var x = 0
  public init(x: Int) { self.x = x }
  public init(_ a: Int, _ b: String) {}
  public init(parse: String) throws {}
  public init(slow: Int) async {}
  public init<T>(any: T) {}
  public init?(maybe: Int) {}
}
public final class Built { public init(n: Int) {} ; public static func make() -> Built { Built(n: 1) } }
// Setters and subscripts (`vs`, `ig`, `is`); `mutating` changes no symbol.
public struct Prop { public var x = 0 ; public static var shared = 0
  public subscript(_ i: Int) -> Int { get { i } set {} }
  public subscript(_ k: String, d: Int) -> String { get { k } set {} }
  public subscript<T>(t t: T) -> T { get { t } set {} }
  public mutating func bump(by n: Int) { x += n }
}
public final class KProp { public var y = 0 ; public subscript(_ i: Int) -> Int { get { i } set {} } }
public struct GProp<T> { public var v: T ; public subscript(_ i: Int) -> T { get { v } set {} } }
// Effectful getters (same symbols as plain ones) and operators (`oi`, `op`, `oP`).
public struct Ops { public var x = 0
  public var slow: Int { get async { 1 } }
  public var both: String { get async throws { "" } }
  public subscript(_ i: Int) -> Int { get async throws { i } }
  public static func + (a: Ops, b: Ops) -> Ops { a }
  public static func == (a: Ops, b: Ops) -> Bool { true }
  public static prefix func - (a: Ops) -> Ops { a }
  public static postfix func ++ (a: Ops) -> Ops { a }
  public static func <*> (a: Ops, b: Int) -> Ops { a }
  public static func += (a: inout Ops, b: Ops) {}
  public static func ..< (a: Ops, b: Ops) -> Int { 0 }
}
infix operator <*>: AdditionPrecedence
postfix operator ++
public func % (a: Ops, b: Ops) -> Ops { a }
