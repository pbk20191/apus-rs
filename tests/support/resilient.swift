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
