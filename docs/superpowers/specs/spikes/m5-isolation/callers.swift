@inline(never) public func pull<I: AsyncIteratorProtocol>(_ it: inout I) async throws(I.Failure) -> I.Element? {
    try await it.next(isolation: nil)
}
public actor Tally2 {
    var n = 0
    public init() {}
    @inline(never) public func bump(by k: Int) -> Int { n += k; return n }
}
@inline(never) public func callBump(_ t: Tally2) async -> Int { await t.bump(by: 1) }
