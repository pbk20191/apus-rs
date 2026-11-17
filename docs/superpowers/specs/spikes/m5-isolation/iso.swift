public struct Boom: Error { public let code: Int }

@inline(never) public func withIsolation(_ x: Int, isolation: isolated (any Actor)? = #isolation) async -> Int {
    x + (isolation == nil ? 0 : 1000)
}

public actor Tally {
    var n = 0
    public init() {}
    @inline(never) public func bump(by k: Int) -> Int { n += k; return n }
}

@inline(never) public func concreteTyped(_ fail: Bool) async throws(Boom) -> Int {
    if fail { throw Boom(code: 7) }
    return 42
}

@inline(never) public func genericTyped<E: Error>(_ error: E?) async throws(E) -> Int {
    if let error { throw error }
    return 42
}
