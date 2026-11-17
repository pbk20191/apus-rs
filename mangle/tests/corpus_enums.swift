// Ground truth for the enum-case declarations in mangle/tests/corpus.rs: compiled with library evolution as module
// `Cases`, where each case exports its tag symbol (`…WC`).
public enum Shape { case none, circle(Double), rect(Double, Double); indirect case group([Shape]) }
public enum Labeled { case io(code: Int32, path: String), unit }
public enum Wrap<T> { case one(T), nothing }
public enum Pair<A, B> { case both(A, B), left(A) }
public struct Outer { public enum Inner { case x, y(Int) } }
public enum Fn { case call((Int) -> String) }
