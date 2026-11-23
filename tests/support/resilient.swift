// A library-evolution module (like Apple's frameworks): protocol requirements are called through dispatch thunks.
import Foundation
public protocol Shape {
    func area() -> Double
    func label() -> String
    func grown(by factor: Double) async -> Double
    mutating func stretch() -> Double
}

/// Fits the existential's three-word inline buffer.
public struct Square: Shape {
    var side: Double
    public init(side: Double) { self.side = side }
    public mutating func stretch() -> Double { side += 1; return area() }
    public func area() -> Double { side * side }
    public func label() -> String { "square" }
    public func grown(by factor: Double) async -> Double { await Task.yield(); return side * factor * side * factor }
}

/// Five words: boxed in the existential.
public struct Polygon: Shape {
    var a: Double, b: Double, c: Double, d: Double, e: Double
    public init() { a = 1; b = 2; c = 3; d = 4; e = 5 }
    public mutating func stretch() -> Double { a += 10; return area() }
    public func area() -> Double { a + b + c + d + e }
    public func label() -> String { "polygon" }
    public func grown(by factor: Double) async -> Double { await Task.yield(); return (a + b + c + d + e) * factor }
}

@inline(never) public func makeSquare(side: Double) -> any Shape { Square(side: side) }
@inline(never) public func makePolygon() -> any Shape { Polygon() }

// Enum cases by tag (tests/swift_enum.rs): built for library evolution, so each case exports its tag symbol.
public enum Figure {
    case none
    case circle(Double)
    case label(String)
    indirect case group([Figure])
    case empty
}
public func figure(_ i: Int) -> Figure {
    switch i {
    case 0: return .none
    case 1: return .circle(2.5)
    case 2: return .label("hi")
    case 3: return .group([.circle(1), .label("inner")])
    default: return .empty
    }
}

// A raw-value enum: `init?(rawValue:)` and `rawValue` through their symbols (tests/swift_init.rs).
public enum Grade: Int { case low = 1, high = 5 }

// Setters, subscripts, mutating methods and inout parameters (tests/swift_accessors.rs).
public struct Bag {
    public var count: Int
    public var name: String
    public init() { count = 0; name = "" }
    public subscript(_ i: Int) -> Int { get { count * i } set { count = newValue / i } }
    public mutating func add(_ n: Int) { count += n }
    nonisolated(unsafe) public static var total = 0
}
public func swapInts(_ a: inout Int, _ b: inout Int) { swap(&a, &b) }
public final class Shelf {
    public var label = ""
    var items: [String: String] = [:]
    public init() {}
    public subscript(_ key: String) -> String { get { items[key] ?? "" } set { items[key] = newValue } }
}

// Effectful getters and operators (tests/swift_accessors.rs).
public enum MeterError: Error { case negative }
public struct Meter {
    public var value: Double
    public init(_ value: Double) { self.value = value }
    public var slowDouble: Double { get async { await Task.yield(); return value * 2 } }
    public var checked: Double { get throws { if value < 0 { throw MeterError.negative }; return value } }
    public subscript(scale s: Double) -> Double { get async throws { await Task.yield(); if s == 0 { throw MeterError.negative }; return value * s } }
    public static func + (a: Meter, b: Meter) -> Meter { Meter(a.value + b.value) }
    public static prefix func - (a: Meter) -> Meter { Meter(-a.value) }
}
infix operator ~~: ComparisonPrecedence
public func ~~ (a: Meter, b: Meter) -> Bool { abs(a.value - b.value) < 0.001 }

// Ownership modifiers (tests/swift_ownership.rs): a Ticket records its deinit, so a reference released once too
// often (or never) shows.
nonisolated(unsafe) var goneTickets: Set<Int> = []
let ticketsLock = NSLock()
public final class Ticket {
    public let id: Int
    public init(id: Int) { self.id = id }
    deinit { ticketsLock.withLock { _ = goneTickets.insert(id) } }
}
public func ticketGone(_ id: Int) -> Bool { ticketsLock.withLock { goneTickets.contains(id) } }
nonisolated(unsafe) var keptTickets: [Ticket] = []
nonisolated(unsafe) let keptLock = NSLock()
public func keep(_ t: consuming Ticket) { let t = t; keptLock.withLock { keptTickets.append(t) } }
// The tickets are released outside the lock: their deinit takes ticketsLock.
public func releaseKept() { let released = keptLock.withLock { let all = keptTickets; keptTickets = []; return all }; _ = released }
public struct Stub {
    public var ticket: Ticket?
    public init(borrowing t: borrowing Ticket) { ticket = copy t }
}
public struct Pass {
    var ticket: Ticket
    public init(_ ticket: Ticket) { self.ticket = ticket }
    public consuming func redeem() -> Int { ticket.id }
}

// Opaque results (tests/swift_opaque.rs): the caller learns the actual type from the runtime.
public func makeSomeShape(side: Double) -> some Shape { Square(side: side) }
public func makeSomeShapeLater(side: Double) async -> some Shape { await Task.yield(); return Square(side: side) }
public struct ShapeMaker { public init() {} ; public var polygon: some Shape { Polygon() } }
extension Shape { public func enlarged() -> some Shape { Square(side: area()) } }

// Class-bound generic parameters and metatypes (tests/swift_class_bound.rs): a class-bound value is one reference,
// a thick metatype one metadata pointer.
open class Animal { public let name: String; public required init(name: String) { self.name = name } }
public final class Dog: Animal {}
public func dog(name: String) -> Dog { Dog(name: name) }
public func renamed<T: Animal>(_ a: T, suffix: String) -> T { T(name: a.name + suffix) }
public func nameOf<T: Animal>(_ a: T) -> String { "\(T.self) \(a.name)" }
public func same<T: AnyObject>(_ x: T) -> T { x }
public func describeType<T>(_ t: T.Type) -> String { "\(t)" }
public func make<T: Animal>(_ t: T.Type, name: String) -> T { t.init(name: name) }
public func dogType() -> Animal.Type { Dog.self }
public func anyTypeName(_ t: Any.Type) -> String { "\(t)" }
public func zero<T: AdditiveArithmetic>(_ t: T.Type) -> T { T.zero }
// Generic parameters bound to concrete types in `swift!` (T is no argument's own type); `Nope` conforms to nothing.
public func sumText<T: AdditiveArithmetic>(_ values: [T]) -> String { "\(values.reduce(.zero, +))" }
public func sumOf<T: AdditiveArithmetic>(_ values: [T]) -> T { values.reduce(.zero, +) }
public func doubled<T: AdditiveArithmetic>(_ x: T) -> T { x + x }
public struct Nope {}
public func nopeType() -> Any.Type { Nope.self }
// Sequences (SwiftIter): an Array of class instances.
public func dogs(_ names: [String]) -> [Dog] { names.map { Dog(name: $0) } }
public func meters(_ value: Double) -> Measurement<UnitLength> { Measurement(value: value, unit: .meters) }
public func unitSymbol(_ unit: Unit) -> String { unit.symbol }
// A class-bound protocol (`Named: AnyObject`): its parameter is one reference; `Tagged` is not class-bound.
public protocol Named: AnyObject { var label: String { get } }
public protocol Tagged {}
extension Dog: Named, Tagged { public var label: String { "named \(name)" } }
public func labelOf<T: Named>(_ x: T) -> String { x.label }
public func isTagged<T: Tagged>(_ x: T) -> Bool { true }

// Static protocol requirements and protocol initializers (tests/swift_protocol_static.rs): called on a type and its
// conformance; `Self` results come back by address.
public protocol Maker {
    static func make(_ n: Int) -> Self
    static var label: String { get }
    init(n: Int)
    static func count() async -> Int
    var n: Int { get }
    func doubled() -> Self
}
public struct Thing: Maker {
    public var n: Int
    public init(n: Int) { self.n = n }
    public static func make(_ n: Int) -> Thing { Thing(n: n) }
    public static var label: String { "thing" }
    public static func count() async -> Int { await Task.yield(); return 7 }
    public func doubled() -> Thing { Thing(n: n * 2) }
}
public func thingMetadata() -> Any.Type { Thing.self }

// Function conventions (tests/swift_conventions.rs): an `@autoclosure` is a Swift closure; a `@convention(c)` function
// is a C function pointer; a `@convention(block)` function an Objective-C block (borrowed as an argument, +1 as a result,
// consumed by an initializer).
public func autoValue(_ f: @autoclosure () -> Int) -> Int { f() + 1 }
public func autoLater(_ f: @escaping @autoclosure () -> String) -> String { let g = f; return g() + "!" }
public func callC(_ f: @convention(c) (Int32, Double) -> Int32, _ x: Int32) -> Int32 { f(x, 0.5) }
public func callMaybeC(_ f: (@convention(c) (Int) -> Int)?) -> Int { f?(20) ?? -1 }
public func cDoubler() -> @convention(c) (Int) -> Int { { $0 * 2 } }
public func callBlock(_ f: @convention(block) (Int) -> Int) -> Int { f(21) }
nonisolated(unsafe) var keptBlock: (@convention(block) () -> Int)?
public func keepBlock(_ f: (@convention(block) () -> Int)?) { keptBlock = f }
public func callKeptBlock() -> Int { keptBlock?() ?? -1 }
public func adder(_ k: Int) -> @convention(block) (Int) -> Int { { $0 + k } }
public struct BlockHolder {
    let f: @convention(block) () -> Int
    public init(f: @escaping @convention(block) () -> Int) { self.f = f }
    public func call() -> Int { f() }
}

// A `sending` parameter is consumed (tests/swift_conventions.rs): the caller hands its reference over.
nonisolated(unsafe) var adopted: Dog?
nonisolated(unsafe) weak var adoptedWatch: Dog?
public func adopt(_ d: sending Dog) { adopted = d; adoptedWatch = d }
public func releaseAdopted() { adopted = nil }
public func adoptedAlive() -> Bool { adoptedWatch != nil }

// Members of a generic struct (tests/swift_generic_auto.rs): an instance member takes `Self`'s metadata after its
// arguments; a static member the type's generic arguments. `count`'s offset depends on T, so the callee needs them.
public struct Tray<T> {
    public var item: T
    public var count: Int
    public var doubled: Int { count * 2 }
    public mutating func add(_ n: Int) { count += n }
    public func describe<U>(_ u: U, _ n: Int) -> String { "\(T.self) \(U.self) \(count + n)" }
    public func later(_ n: Int) async -> Int { count + n }
    public static func size(plus n: Int) -> Int { MemoryLayout<T>.size + n }
}
public func pearTray(count: Int) -> Tray<String> { Tray(item: "pear", count: count) }
// Specialized generics (tests/swift_generic_auto.rs): every generic parameter bound (`T = "…"`), the type's too.
public final class Holder<T> {
    public let item: T
    public init(item: T) { self.item = item }
    public func show<U>(_ u: U) -> String { "\(item) \(U.self)" }
    public static func kind() -> String { "Holder<\(T.self)>" }
}
public func holdInt(_ n: Int) -> Holder<Int> { Holder(item: n) }
// A conformance an extension or a member adds to the type's parameter: its witness table is passed too.
public protocol Weighed { var weight: Int { get } }
extension Int: Weighed { public var weight: Int { self * 10 } }
extension Holder where T: Weighed {
    public func weighed<U: CustomStringConvertible>(_ u: U) -> String { "\(item.weight) \(u)" }
    public static func unit() -> Int { T.self == Int.self ? 1 : 0 }
}
extension Tray where T: Weighed { public func weight() -> Int { item.weight * count } }
extension Tray { public func heaviest() -> Int where T: Comparable { count } }
public func intTray(count: Int) -> Tray<Int> { Tray(item: 4, count: count) }
// A member's `where` that fixes the type's parameters: they are no generic arguments (`init(second:)` takes `B`'s
// metadata only, `sum()` not even `Self`'s).
public struct Pair<A, B> {
    public var first: A
    public var second: B
    public init(second: B) where A == Int { first = 7; self.second = second }
    public static func tag(_ b: B) -> String where A == Int { "\(A.self) \(B.self) \(b)" }
    public func sum() -> Int where A == Int, B == Int { first + second }
    public var text: String { "\(first) \(second)" }
}
// Counts its live instances: a consumed argument must be a copy, leaving the caller's (swift_generic_auto).
public final class Kept {
    nonisolated(unsafe) static var alive = 0
    init() { Kept.alive += 1 }
    deinit { Kept.alive -= 1 }
}
public func makeKept() -> [Kept] { [Kept()] }
public func keptAlive() -> Int { Kept.alive }
public func makeOneKept() -> Kept { Kept() }
@inline(never) public func nameOf(_ animal: Animal) -> String { animal.name }
public func stringLength(_ s: String) -> Int { s.utf8.count }
public func echoSome<T: Equatable>(_ t: T) -> some Equatable { t }
public func laterMax<T: Comparable>(_ a: T, _ b: T) async -> T { await Task.yield(); return Swift.max(a, b) }
nonisolated(unsafe) var keptStringValue = ""
@inline(never) public func keepString(_ s: consuming String) { keptStringValue = s }
public func keptString() -> String { keptStringValue }
public func stringSumLater(_ s: String) async -> Int { await Task.yield(); return s.utf8.reduce(0) { $0 + Int($1) } }
@inline(never) public func consumeKept(_ k: consuming Kept) -> Int { _ = consume k; return 1 }
public struct Flag<V> {
    public var value: V
    public init() where V == Bool { value = true }
    public var text: String { "\(value)" }
}
public func rank<T: Hashable & Comparable>(_ a: T, _ b: T) -> Int { a < b ? -1 : (a == b ? 0 : 1) }
public func joined<S: Sequence>(_ s: S) -> String where S.Element == String { s.joined(separator: ",") }
public func described<S: Sequence>(_ s: S) -> String where S.Element: CustomStringConvertible { s.map(\.description).joined(separator: "+") }
public func flattened<S: Sequence>(_ s: S) -> String where S.Element: Sequence, S.Element.Element == String { s.flatMap { $0 }.joined(separator: ",") }
// A class-bound existential (tests/swift_class_bound.rs): Swift passes and returns it as its reference and witness
// table, in registers.
public func anyNamed(_ name: String) -> any Named { Dog(name: name) }
public func labelOfAny(_ x: any Named) -> String { x.label }
public func maybeNamed(_ name: String) -> (any Named)? { name.isEmpty ? nil : Dog(name: name) }
public func labelOfMaybe(_ x: (any Named)?) -> String { x?.label ?? "nobody" }
// Opened from Rust: a class-bound protocol's requirements and extension methods take `self` as the reference itself.
extension Named { public func shout() -> String { label + "!" } }

// Benchmarks (benches/calls.rs): each target, and a loop calling it from Swift (the same module: native Swift's best
// case, which knows `BenchPoint`'s layout).
public struct BenchPoint { public var x: Double; public var y: Double; public init(x: Double, y: Double) { self.x = x; self.y = y } }
@inline(never) public func benchAdd(_ a: Int, _ b: Int) -> Int { a &+ b }
@inline(never) public func benchLength(_ s: String) -> Int { s.utf8.count }
@inline(never) public func benchPoint(_ x: Double) -> BenchPoint { BenchPoint(x: x, y: x) }
@inline(never) public func benchMax<T: Comparable>(_ a: T, _ b: T) -> T { Swift.max(a, b) }
@inline(never) public func benchAddAsync(_ a: Int, _ b: Int) async -> Int { a &+ b }
@inline(never) public func benchLoopAdd(_ n: Int) -> Int { var s = 0; for i in 0..<n { s = benchAdd(s, i) }; return s }
@inline(never) public func benchLoopLength(_ n: Int) -> Int { var s = 0; for _ in 0..<n { s &+= benchLength("hello, apus") }; return s }
@inline(never) public func benchLoopPoint(_ n: Int) -> Double { var s = 0.0; for i in 0..<n { s += benchPoint(Double(i)).y }; return s }
@inline(never) public func benchLoopMax(_ n: Int) -> Int { var s = 0; for i in 0..<n { s &+= benchMax(i, s) }; return s }
@inline(never) public func benchLoopAddAsync(_ n: Int) async -> Int { var s = 0; for i in 0..<n { s = await benchAddAsync(s, i) }; return s }
public class BenchBox { public var n: Int; public init(n: Int) { self.n = n } }
public func benchMakeBox(_ n: Int) -> BenchBox { BenchBox(n: n) }
@inline(never) public func benchBoxed(_ b: BenchBox) -> Int { b.n }
@inline(never) public func benchLoopBoxed(_ n: Int) -> Int { let b = BenchBox(n: 1); var s = 0; for _ in 0..<n { s &+= benchBoxed(b) }; return s }
@inline(never) public func benchPointX(_ p: BenchPoint) -> Double { p.x }
@inline(never) public func benchLoopPointX(_ n: Int) -> Double { let p = BenchPoint(x: 1, y: 2); var s = 0.0; for _ in 0..<n { s += benchPointX(p) }; return s }
