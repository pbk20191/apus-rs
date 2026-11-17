//! Mangles each declaration and checks the result against the symbols swiftc actually produced for
//! tests/corpus.swift. Needs `xcrun swiftc` and `nm`.
use apus_mangle::mangle;
use std::collections::HashSet;
use std::process::Command;

const DECLARATIONS: &[&str] = &[
    // methods, statics, nested types, getters
    "Probe.Big(struct).scaled(by: Int) -> Big(struct)",
    "Probe.Big(struct).two(_: Big(struct), _: Big(struct)) -> Big(struct)",
    "static Probe.Big(struct).make(n: Int) async -> Big(struct)",
    "Probe.Big(struct).Inner(struct).f() -> Big(struct).Inner(struct)",
    "Probe.Big(struct).Inner(struct).g(_: Big(struct)) -> Big(struct)",
    "Probe.Big(struct).total: Int { get }",
    "static Probe.Big(struct).shared: Big(struct) { get }",
    "Probe.Counter(class).bump(by: Int) async -> Int",
    "Probe.Counter(class).me() -> Counter(class)",
    "Probe.Counter(class).name: String { get }",
    "Probe.Color(enum).next(_: Color(enum)) -> Color(enum)",
    "Probe.globalCount: Int { get }",
    // free functions: substitutions, collections, optionals, tuples
    "Probe.f1(_: Big(struct), _: Big(struct)) -> Big(struct)",
    "Probe.f2(_: [String], _: [String]) -> [String]",
    "Probe.f3(_: Int?, _: Int?) -> [Int?]",
    "Probe.f4(_: [String: Int], _: Set<Int>) -> [String: [Int]]",
    "Probe.f5(_: Counter(class), _: Color(enum)) -> Counter(class)?",
    "Probe.f6(_: any Shape) -> Any",
    "Probe.f7(_: (Int, String), named: (a: Int, b: Bool)) -> (Int, Int)",
    "Probe.f8(_: UInt8, _: Int32, _: UInt64, _: Float, _: Character, _: UInt)",
    "Probe.f9(counterMaker: Counter(class), makeCounter: Int, counter: Int)",
    "Probe.fa(_: Big(struct).Inner(struct), y: Big(struct).Inner(struct)) -> Big(struct)",
    "Probe.fb(_: Double, _: Double, _: Double, _: Bool) -> Double",
    "Probe.fh(_: Int, b: Int, _: inout Int, d: Int...)",
    "Probe.fj() throws",
    "Probe.fk() async",
    "Probe.fl(_: any Error, _: AnyObject, _: UnsafeRawPointer, _: UnsafeMutablePointer<Int>)",
    "Probe.fm(_: any Shape, _: any Shape, _: Box(struct)<Int>, _: Box(struct)<Int>, _: Box(struct)<String>)",
    "Probe.fn(_: Big(struct), _: Counter(class), _: Big(struct), _: Counter(class), _: Color(enum)) -> Color(enum)",
    "Probe.fq(_: (Int, Int), _: (Int, Int), _: Int?, _: Int??) -> Int?",
    "Probe.fr(aLongLabelName: Int, anotherLongLabelName: Int, labelName: Int, name: Int) -> Int",
    "Probe.fs(_: Any, _: Any, _: AnyObject, _: AnyObject, _: any Error, _: any Error)",
    // generics
    "Probe.fc<T>(_: T, _: T) -> [T]",
    "Probe.fd<T, U>(_: T, _: U) -> U",
    "Probe.fe<T: Hashable, U: Shape>(_: T, _: U) -> [T: U]",
    "Probe.ff<S: Sequence>(_: S) -> [S.Element] where S.Element: Equatable",
    "Probe.fg<C: Collection>(_: C) async throws -> Int where C.Element == String",
    "Probe.fo<T: Comparable & Hashable, U>(_: T, _: U, _: T) -> T where U: Sequence, U.Element == T",
    "Probe.fp<T: CustomStringConvertible>(_: T, times: Int) async throws -> String",
    // many substitutions (indices past 26), merged in every way
    "Probe.many(_: S01(struct), _: S02(struct), _: S03(struct), _: S04(struct), _: S05(struct), _: S06(struct), _: S07(struct), \
     _: S08(struct), _: S09(struct), _: S10(struct), _: S11(struct), _: S12(struct), _: S13(struct), _: S14(struct), _: S15(struct), \
     _: S15(struct), _: S14(struct), _: S13(struct), _: S01(struct), _: S13(struct), _: S13(struct)) -> S14(struct)",
    // word substitutions inside identifiers
    "Probe.URLSessionTask(struct).resumeURLSession(urlSession: Int, sessionTask: Int, task: Int)",
    "Probe.my_snake_case(snake_case: Int, my_snake: Int, case_: Int)",
    "Probe.fooFooFoo(foo: Int, Foo: Int, fooFoo: Int)",
    "Probe.x2y(a1: Int, a1b2: Int, _a: Int, a_: Int, ab: Int, abAb: Int)",
    "Probe.HTTPSProxy(httpsProxy: Int, proxyHTTPS: Int, HTTPS: Int, proxy: Int)",
    // concurrency types, nested collections, tuple parameters
    "Probe.t1(_: Task<Int, Never>, _: AsyncStream<Int>, _: TaskPriority, _: TaskPriority) async",
    "Probe.t2(_: [[Int]], _: [[Int]], _: [Int: [String: Int?]]?, _: Set<String>?) -> [[Int]]?",
    "Probe.t3(_: (Int, Int))",
    "Probe.t4(pair: (Int, Int))",
    "Probe.t5(_: ())",
    "Probe.t6(_: Int32?, _: Int32, _: UInt8?, _: [UInt8])",
    // more generics
    "Probe.g1<A, B, C>(_: A, _: B, _: C) -> (A, B, C)",
    "Probe.g2<T: Sendable & Shape>(_: T) async",
    "Probe.g3<S: Sequence>(_: S, _: S.Element) -> Int where S.Element == Int",
    "Probe.g4<S: Sequence>(_: S) -> Int where S.Element: Hashable",
    "Probe.g5<K: Hashable, V>(_: [K: V], _: K) -> V?",
    "Probe.g6<T: Collection, U: Collection>(_: T, _: U) where T.Element == U.Element, T.Element: Equatable",
    "Probe.g7<T>(_: T?, _: [T?]) -> T?",
    "Probe.g8<T: Sequence, U: Sequence>(_: T, _: U, _: U.Element) -> [T.Element] where U.Element == T.Element",
    "Probe.g9<S: Sequence, T>(_: S, _: S.Element) -> T where S.Element == T",
    "Probe.t7(_: Box(struct)<()>, _: ()?) -> ()?",
    // an extension in the type's own module mangles like a member; deep nesting
    "Probe.Counter(class).extra(_: Int) -> Int",
    "static Probe.Outer(enum).Mid(enum).Leaf(struct).deep(_: Outer(enum), _: Outer(enum).Mid(enum)) -> Outer(enum).Mid(enum).Leaf(struct)",
    // extensions of other modules' types
    "(extension in Probe) Swift.Int.doubled() -> Int",
    "static (extension in Probe) Swift.Int.make(_: String) -> Int",
    "(extension in Probe) Swift.Int.half: Int { get }",
    "(extension in Probe) Swift.String.shout(times: Int) -> String",
    "(extension in Probe) Swift.Array<Element>.firstTwo() -> [Element]",
    "(extension in Probe) Swift.Array<Element>.second: Element? { get }",
    "(extension in Probe) Swift.Array<Element>.pair(with: [Element]) -> [[Element]]",
    "(extension in Probe) Swift.Optional<Wrapped>.orThrow() throws -> Wrapped",
    "(extension in Probe) Swift.Dictionary<Key, Value>.keysArray() -> [Key]",
    "(extension in Probe) Swift.Dictionary<Key, Value>.value(for: Key) -> Value?",
    "(extension in Probe) _Concurrency.Task<Success, Failure>.poke()",
    "(extension in Probe) _Concurrency.AsyncStream<Element>.tag() -> Int",
    // constrained extensions
    "(extension in Probe where Element == Int) Swift.Array<Element>.total() -> Int",
    "(extension in Probe where Element == Int) Swift.Array<Element>.firstOr(_: Element) -> Element",
    "(extension in Probe where Element: Hashable) Swift.Array<Element>.unique() -> [Element]",
    "(extension in Probe where Wrapped: Shape) Swift.Optional<Wrapped>.areaOrZero() -> Double",
    "(extension in Probe where Value: Equatable) Swift.Dictionary<Key, Value>.hasEqual() -> Bool",
    "(extension in Probe where Key == String) Swift.Dictionary<Key, Value>.stringKeys() -> [Key]",
    "(extension in Probe where Element == Int) Swift.Set<Element>.sum() -> Int",
    "(extension in Probe where T: Shape) Probe.Box(struct)<T>.areas() -> Double",
    "(extension in Probe where A == Int, B: Hashable) Probe.Pair(struct)<A, B>.keyed() -> [B: Int]",
    "(extension in Probe where K == String) Probe.Keyed(struct)<K: Hashable>.name() -> String",
    "static (extension in Probe where Success == Never, Failure == Never) _Concurrency.Task<Success, Failure>.nap(_: UInt64) async throws",
    // members of generic types
    "Probe.Box(struct)<T>.get() -> T?",
    "Probe.Box(struct)<T>.put(_: T, at: Int)",
    "Probe.Box(struct)<T>.count: Int { get }",
    "static Probe.Box(struct)<T>.empty() -> Box(struct)<T>",
    "Probe.Box(struct)<T>.map<U>(_: U) -> Box(struct)<U>",
    "Probe.Box(struct)<T>.same(_: Box(struct)<T>) -> Box(struct)<T>",
    "Probe.Box(struct)<T>.contextual() -> Int where T: Hashable",
    "Probe.Box(struct)<T>.two<U, V>(_: U, _: V) where U: Hashable",
    "Probe.Pair(struct)<A, B>.swap() -> Pair(struct)<B, A>",
    "Probe.Pair(struct)<A, B>.first(_: A) -> B?",
    "Probe.Pair(struct)<A, B>.Inner(struct).f(_: A) -> B?",
    "Probe.Keyed(struct)<K: Hashable>.key() -> K?",
    // protocol extensions
    "(extension in Probe) Probe.Shape(protocol).twice() -> Double",
    "(extension in Probe) Probe.Shape(protocol).same(as: Self) -> Self",
    "(extension in Probe) Swift.Collection.countTwice() -> Int",
    "(extension in Probe) Swift.Collection.firstElement() -> Self.Element?",
    "(extension in Probe where Self.Element == String) Swift.Sequence.joinedTwice() -> String",
    // the functions the runtime tests call
    "Probe.addAsync(_: Int, _: Int) async -> Int",
    "Probe.greet(_: String, times: Int, sleepMs: Int) async throws -> String",
    "Probe.makeCounter() -> Counter(class)",
    "Probe.names(count: Int) async -> [String]",
    "Probe.currentTaskBits() async -> Int",
    "Probe.scores() async -> [String: Int]",
    "Probe.describe(_: Bool, _: Int32, _: UInt64) async -> String",
    // isolation
    "Probe.withIsolation(_: Int, isolation: isolated (any Actor)? = #isolation) async -> Int",
    "Probe.isolatedTo(_: isolated any Actor, _: Int) -> Int",
    "Probe.Tally(actor).bump(by: Int) -> Int",
    "Probe.Tally(actor).count: Int { get }",
    "nonisolated Probe.Tally(actor).label() -> String",
    "@MainActor Probe.onMainThread() -> Bool",
];

fn compiled_symbols() -> HashSet<String> {
    let dir = std::env::temp_dir().join(format!("apus-mangle-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let lib = dir.join("libcorpus.dylib");
    let source = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/corpus.swift");
    let status = Command::new("xcrun")
        .args(["swiftc", "-parse-as-library", "-emit-library", "-module-name", "Probe", source, "-o"])
        .arg(&lib)
        .status()
        .expect("xcrun swiftc");
    assert!(status.success(), "swiftc failed on corpus.swift");
    let out = Command::new("nm").arg("-gU").arg(&lib).output().expect("nm");
    String::from_utf8(out.stdout)
        .unwrap()
        .lines()
        .filter_map(|l| l.split_whitespace().last())
        .map(|s| s.trim_start_matches('_').to_string())
        .collect()
}

#[test]
fn declarations_mangle_to_the_symbols_swiftc_emits() {
    let symbols = compiled_symbols();
    let mut failures = Vec::new();
    for decl in DECLARATIONS {
        match mangle(decl) {
            Ok(symbol) if symbols.contains(&symbol) => {}
            Ok(symbol) => failures.push(format!("{decl}\n    mangled to {symbol}, which swiftc did not emit")),
            Err(e) => failures.push(format!("{decl}\n    error: {e}")),
        }
    }
    assert!(failures.is_empty(), "{} of {} declarations wrong:\n{}", failures.len(), DECLARATIONS.len(), failures.join("\n"));
}

#[test]
fn framework_symbols() {
    // Real symbols, checked against the SDK in the main crate's tests.
    assert_eq!(
        mangle("static StoreKit.Product(struct).products<I: Collection>(for: I) async throws -> [Product(struct)] where I.Element == String").unwrap(),
        "$s8StoreKit7ProductV8products3forSayACGx_tYaKSlRzSS7ElementRtzlFZ"
    );
    assert_eq!(mangle("StoreKit.Product(struct).displayName: String { get }").unwrap(), "$s8StoreKit7ProductV11displayNameSSvg");
    assert_eq!(mangle("StoreKit.Product(struct).id: String { get }").unwrap(), "$s8StoreKit7ProductV2idSSvg");
    assert_eq!(
        mangle("static StoreKit.Transaction(struct).updates: Transaction(struct).Transactions(struct) { get }").unwrap(),
        "$s8StoreKit11TransactionV7updatesAC12TransactionsVvgZ"
    );
    assert_eq!(mangle("StoreKit.Transaction(struct).finish() async").unwrap(), "$s8StoreKit11TransactionV6finishyyYaF");
    // `Task.sleep(nanoseconds:)`: a constrained extension inside the type's own module.
    assert_eq!(
        mangle(
            "static (extension in _Concurrency where Success == Never, Failure == Never) \
             _Concurrency.Task<Success, Failure>.sleep(nanoseconds: UInt64) async throws"
        )
        .unwrap(),
        "$sScTss5NeverORszABRs_rlE5sleep11nanosecondsys6UInt64V_tYaKFZ"
    );
}

#[test]
fn errors_are_reported() {
    for bad in ["", "Probe", "Probe.f(", "Probe.f(_: Mystery) -> Int", "Probe.f(_ Int)", "Probe.Big.scaled()"] {
        assert!(mangle(bad).is_err(), "{bad:?} should not mangle");
    }
}

#[test]
fn result_types() {
    use apus_mangle::result_type;
    assert_eq!(result_type("Probe.f() -> [String]").unwrap().as_deref(), Some("SaySSG"));
    assert_eq!(result_type("Probe.Big(struct).scaled(by: Int) -> Big(struct)").unwrap().as_deref(), Some("5Probe3BigV"));
    assert_eq!(result_type("StoreKit.f() async throws -> [Product(struct)]").unwrap().as_deref(), Some("Say8StoreKit7ProductVG"));
    assert_eq!(result_type("Probe.Counter(class).name: String { get }").unwrap().as_deref(), Some("SS"));
    assert_eq!(result_type("Probe.f()").unwrap().as_deref(), Some("yt"));
    assert_eq!(result_type("Probe.f<T>(_: T) -> [T]").unwrap(), None);
    assert_eq!(result_type("Probe.f<S: Sequence>(_: S) -> S.Element where S.Element == Int").unwrap().as_deref(), Some("Si"));
}
