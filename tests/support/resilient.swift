// A library-evolution module (like Apple's frameworks): protocol requirements are called through dispatch thunks.
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
