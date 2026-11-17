@inline(never) public func applyConcrete(_ f: (Int) async -> Int) async -> Int { await f(20) + 1 }
@inline(never) public func applyEscaping(_ f: @escaping @Sendable (Int, String) async throws -> String) async throws -> String { try await f(2, "x") }
@inline(never) public func applyGeneric<T>(_ x: T, _ f: (T) async -> T) async -> T { await f(x) }
@inline(never) public func applySync(_ f: (Int) -> Int) -> Int { f(3) }
@inline(never) public func applyMain(_ f: @MainActor (Int) async -> Int) async -> Int { await f(1) }
