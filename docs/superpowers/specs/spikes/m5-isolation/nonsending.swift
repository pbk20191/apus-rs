@inline(never) nonisolated(nonsending) public func inherits(_ x: Int) async -> Int { x + 1 }
@inline(never) public func plain(_ x: Int) async -> Int { x + 1 }
