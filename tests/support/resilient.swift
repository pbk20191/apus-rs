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
public func meters(_ value: Double) -> Measurement<UnitLength> { Measurement(value: value, unit: .meters) }
public func unitSymbol(_ unit: Unit) -> String { unit.symbol }
