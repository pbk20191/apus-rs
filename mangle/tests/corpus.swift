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
public extension Pair { init(b: B) where A == Int { self.init() } ; init(a: A) where B == [A] { self.init() }
  static func tag(_ b: B) -> Int where A == Int { 0 } ; func sum() -> Int where A == Int, B == Int { 0 } }
public extension Box { init(flag: Bool) where T == Bool { self.init() } }
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
// Ownership modifiers: only a convention other than the default shows (`n` in a function, `h` in an initializer).
public final class Own {}
public struct Owner { public var x = 0
  public consuming func eat() -> Int { x }
  public borrowing func look() -> Int { x }
  public init(consuming c: Own) {}
  public init(borrowing b: borrowing Own) {}
}
public func own1(_ a: consuming Own) {}
public func own2(_ a: borrowing Own) {}
public func own3(_ a: __owned Own) {}
public func own4(_ a: __shared Own) {}
public func own5(_ a: consuming String, _ b: Int) {}
// A constrained extension whose requirement names a type with its kind.
extension Box where T == Big { public func onlyBig() -> Int { 0 } }
// An initializer of a type nested in a generic type; @preconcurrency drops concurrency annotations from the symbol.
public struct GenBox<T> { public struct Item { public init(x: Int) {} } }
@preconcurrency public func pre(_ f: @escaping @Sendable (Int) -> Bool) {}
@preconcurrency public func preMain(_ f: @escaping @MainActor () -> Void) {}
// A generic type nested in a non-generic one: bound arguments go to their level (`y_xq_G`); `_const` parameters.
public enum PE { public struct Pair<A, B> { public init(a: A, b: B) {} } }
public func usePair(_ p: PE.Pair<Int, String>) {}
public func constant(_ id: _const String, _ n: Int) {}
// Opaque types: a `some P` result mangles as `Qr` and has a descriptor (`…QOMQ`); a `some P` parameter is an
// anonymous generic parameter.
public struct Sq2: Shape { public func area() -> Double { 1 } }
public func makeSome(_ s: Double) -> some Shape { Sq2() }
public func takesSome(_ s: some Shape, _ t: some Shape & Hashable) -> Double { 0 }
public var someVar: some Shape { Sq2() }
public struct Holder2 { public var shape: some Shape { Sq2() } ; public static func make() -> some Shape { Sq2() } }
public struct GBox<T> { public func shape() -> some Shape { Sq2() } }
// Imported C types named by a typedef (an anonymous struct, a CF class) are mangled under the typedef's name (`a`).
import Foundation
import CoreGraphics
public func cTypedefs(_ d: Decimal, _ c: CGColor) -> CGColor? { nil }
// `@preconcurrency`: marker protocols (Sendable) leave the symbol's generic signature and existentials.
public struct PQ<T> {}
@preconcurrency public func pw<T1: Sendable, T2: Sendable & Hashable>(_ a: PQ<T1>, _ b: T2, _ c: any Sendable) {}
// An associated type of a parameter with several protocols names its protocol.
public protocol Opening { associatedtype Value }
public protocol Leafy {}
public protocol Showing {}
extension Showing where Self: Opening, Self.Value: Leafy { public func perform() {} }
public func mixw<T: Opening>(_ t: T, _ v: T.Value) where T: Leafy, T.Value: Leafy {}
public func seqw<T: Sequence>(_ t: T, _ e: T.Element) where T: Hashable, T.Element: Hashable {}
public func amark<T: Opening & Sendable>(_ t: T, _ v: T.Value) {}
// A member type of a generic type: each level of the path takes its own arguments.
public struct GOuter<T> { public struct Inner { public init() {} }; public enum E<U> { case a } }
public struct GTwo<A, B> { public struct W {} }
public func gf1(_ x: GOuter<Int>.Inner) {}
public func gf2<V>(_ x: GOuter<V>.Inner, _ y: GOuter<V>.Inner) {}
public func gf3(_ x: GOuter<Int>.E<String>) {}
public func gf4(_ x: GTwo<Int, String>.W) {}
public func gf6(_ x: [GOuter<Int>.Inner]) -> GOuter<Int>.Inner? { nil }
// Class-bound generic parameters: a superclass (`Rb`) or AnyObject (`Rl…C`), before the protocols.
open class SBase { public init() {} }
public protocol SP {}
public func s1<T: SBase>(_ t: T) -> T { t }
public func s2<T: NSObject>(_ t: T, _ n: Int) -> [T] { [t] }
public func s3<T>(_ t: T) where T: SBase, T: SP {}
public func s4<T: AnyObject>(_ t: T) -> T { t }
public func s5<T: AnyObject & SP>(_ t: T) {}
// Metatypes: `T.Type` (`xm`), a class's, `Any.Type` (`ypXp`).
public func mt1<T>(_ t: T.Type) -> Int { 0 }
public func mt2<T: Decodable>(_ t: T.Type, from n: Int) -> T? { nil }
public func mt3(_ b: SBase.Type) -> Int { 0 }
public func mt5() -> SBase.Type { SBase.self }
public func mt6<T>(_ t: T.Type) -> T.Type { t }
public func mt8(_ a: Any.Type) -> Int { 0 }
public func mt9(_ t: Int.Type?, _ s: Big.Type) -> Int { 0 }
public protocol MP {}
public func mt7(_ p: any MP.Type) -> Int { 0 }
// `where Self == Concrete` (SwiftUI's `.bordered`): the result `Self` is the concrete type.
public protocol PStyle {}
public struct PBordered: PStyle { public init() {} }
extension PStyle where Self == PBordered { public static var bordered: Self { PBordered() } }
// Variadic generic types (`Predicate<each Input>`): a pack argument is a type list, then `QP`.
public struct Pk<each T> { public init() {} }
public struct PMixed<A, each T> { public init() {} }
public func k1(_ p: Pk<Int>) {}
public func k2(_ p: Pk<Int, String>) {}
public func k0(_ p: Pk< >) {}
public func k3<V>(_ p: Pk<V>, _ q: Pk<V>) {}
public func k4(_ p: PMixed<Int, String, Bool>) {}
public func k5(_ p: [Pk<Int>]) -> Pk<Int>? { nil }
public func k6<V>(_ p: Predicate<V>) {}

public func cv0(_ f: @autoclosure () -> Int) {}
public func cv1(_ f: @escaping @autoclosure () -> Int) {}
public func cv2(_ f: @autoclosure () throws -> Int) throws {}
public func cv3(_ f: @autoclosure @Sendable () async -> String) async {}
public func cv4(_ f: @convention(c) (Int32, Double) -> Int32) {}
public func cv5(_ f: (@convention(c) () -> Void)?) {}
public func cv6() -> @convention(c) (Int) -> Int { { $0 } }
public func cv7(_ f: @convention(block) (Int) -> Int) {}
public func cv8(_ f: @escaping @Sendable @convention(block) () -> Void) {}
public func cv9(_ f: (@convention(block) (String) -> Bool)?) -> @convention(block) () -> Int { { 0 } }
public func cv10(_ f: (@convention(c) (Int) -> Int, Int) -> Int) {}
extension Made { public init(cv: @escaping @autoclosure () -> Int, b: @escaping @convention(block) () -> Void) { self.init(x: 0) } }

public func sd0(_ a: sending Counter, _ b: sending String) {}
public func sd1<T>(_ x: sending T) -> sending T? { nil }
public func sd2() -> sending Counter { Counter() }
public func sd3(_ f: @escaping @MainActor () async throws -> sending Counter) {}
public func sd4(_ f: @escaping @Sendable () -> sending Counter) -> sending Counter { f() }
extension Made { public init(sd: sending Counter) { self.init(x: 0) } }
// A class-bound protocol, marked `(AnyObject)`: the symbol is the unmarked one's.
public protocol Owning: AnyObject {}
public func cb0<T: Owning>(_ x: T) {}
public func cb1<T: Owning & Hashable>(_ x: T, _ y: [T]) -> T { x }
