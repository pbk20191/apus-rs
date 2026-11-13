// M3 spike: Swift classes, structs and protocols seen from Rust. Plain Swift, no @_cdecl.
import Darwin

// ---------- classes ----------
open class Animal {
    public var name: String
    public private(set) var legs: Int = 4
    public init(name: String) { self.name = name }
    open func speak() -> String { "..." }
    public func describe() -> String { "\(name) says \(speak())" }
}
public final class Dog: Animal {
    public var tricks: [String] = []
    public override func speak() -> String { "Woof" }
    public func learn(_ trick: String) async -> Int { await Task.yield(); tricks.append(trick); return tricks.count }
}
@inline(never) public func makeDog(_ name: String) -> Animal { Dog(name: name) }

// ---------- structs ----------
public struct Point {                      // two Doubles: travels in d0, d1
    public var x: Double, y: Double
    public init(x: Double, y: Double) { self.x = x; self.y = y }
    @inline(never) public func length() -> Double { (x * x + y * y).squareRoot() }
    @inline(never) public func scaled(by k: Double) -> Point { Point(x: x * k, y: y * k) }
    @inline(never) public mutating func move(dx: Double) { x += dx }
}
public struct Tag {                        // Int + String: three integer words
    public var id: Int, label: String
    public init(id: Int, label: String) { self.id = id; self.label = label }
    @inline(never) public func show(prefix: String) -> String { "\(prefix)#\(id) \(label)" }
}
public struct Mixed {                      // Int, Double, Bool: integer and float registers together
    public var a: Int, b: Double, c: Bool
    public init(a: Int, b: Double, c: Bool) { self.a = a; self.b = b; self.c = c }
    @inline(never) public func sum() -> Double { Double(a) + b + (c ? 1 : 0) }
}
public struct One {                        // a single word
    public var v: Int
    public init(v: Int) { self.v = v }
    @inline(never) public func twice() -> Int { v * 2 }
}
public struct Wide {                       // five words: by address
    public var a = 1, b = 2, c = 3, d = 4
    public var note = "wide"
    public var owner: Animal? = nil
    public init() {}
}

// ---------- protocols ----------
public protocol Greeter {
    var title: String { get }
    func greet(_ name: String) -> String
    func greetAsync(_ times: Int) async -> String
}
public struct English: Greeter {
    public var title: String { "Mx" }
    public func greet(_ name: String) -> String { "Hello, \(name)" }
    public func greetAsync(_ times: Int) async -> String { await Task.yield(); return String(repeating: "hi ", count: times) }
}
public final class Robot: Greeter {
    var serial = 7
    public var title: String { "Unit \(serial)" }
    public func greet(_ name: String) -> String { "BEEP \(name)" }
    public func greetAsync(_ times: Int) async -> String { await Task.yield(); return String(repeating: "beep ", count: times) }
}
@inline(never) public func anyGreeter(_ kind: Int) -> any Greeter { kind == 0 ? English() : Robot() }
@inline(never) public func useGreeter(_ g: any Greeter) -> String { g.greet("you") + " / " + g.title }
@inline(never) public func useGeneric<G: Greeter>(_ g: G) -> String { g.greet("generic") + " / " + g.title }
@inline(never) public func useGenericAsync<G: Greeter>(_ g: G) async -> String { await g.greetAsync(2) + g.title }
