// Ground truth for the enum-case declarations in mangle/tests/corpus.rs: compiled with library evolution as module
// `Cases`, where each case exports its tag symbol (`…WC`).
public enum Shape { case none, circle(Double), rect(Double, Double); indirect case group([Shape]) }
public enum Labeled { case io(code: Int32, path: String), unit }
public enum Wrap<T> { case one(T), nothing }
public enum Pair<A, B> { case both(A, B), left(A) }
public struct Outer { public enum Inner { case x, y(Int) } }
public enum Fn { case call((Int) -> String) }
// A single labeled payload is a one-element tuple; a nested enum of a generic type binds the outer arguments.
public enum Notice { case token(token: String), url(destination: Int?) }
public enum GenOuter<T> { public enum Inner { case a, b(Int) } }
// Cases of an enum whose generic parameters are constrained: the enum's whole generic signature is in the symbol.
public protocol Tagged {}
public enum Keyed<K: Hashable> { case one(K), none }
public enum Both<A: Tagged, B: Sendable> { case a(A), b }
public struct Sorting<E: Hashable> { public enum Order { case up, down(E) } }
// Types declared in a constrained extension: the extension is part of their context.
public struct Holder<T> { public init() {} }
extension Holder where T: Tagged {
    public enum Mode { case on, off(T) }
    public func mode() -> Mode { .on }
    public struct Item { public init() {}; public func size() -> Int { 0 }; public func same(_ i: Item) -> Bool { true } }
}
extension Holder where T == Int { public enum Plain { case a } }
extension Holder { public enum Free { case z } }
