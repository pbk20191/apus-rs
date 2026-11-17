//! Computes the mangled symbol of a Swift declaration, the name it has in a binary.
//!
//! ```
//! let symbol = apus_mangle::mangle("Greeter.greet(_: String, times: Int) async throws -> String").unwrap();
//! assert_eq!(symbol, "$s7Greeter5greet_5timesS2S_SitYaKF");
//! ```
//!
//! The declaration is written as in Swift, fully qualified, with parameter names left out and the kind of each
//! type outside the standard library spelled out: `Shop.Cart(struct).total(for: Customer(class)) -> Double`.
//! See [`mangle`] for the grammar and its limits.

mod ast;
mod mangler;
mod names;
mod parse;

/// Why a declaration could not be mangled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error(String);

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

/// Mangles a function, method or property getter.
///
/// ```text
/// [static] [(extension in Module [where …])] Module.Type(kind)[<T, …>]….name[<U: P>]([label|_]: Type, …)
///     [async] [throws] [-> Type] [where …]
/// [static] [(extension in Module [where …])] Module.Type(kind)[<T, …>]….name: Type { get }
/// ```
///
/// - Types: standard library names (`Int`, `String`, `Bool`, `Double`, `UInt8`, …), `[T]`, `[K: V]`, `Set<T>`,
///   `T?`, tuples, `Any`, `AnyObject`, `any Proto`, generic parameters and `T.Associated`.
/// - Other types name their kind: `Cart(struct)`, `Customer(class)`, `Color(enum)`; `(actor)` is accepted too.
///   A bare name belongs to the declaration's module; `Other.Name(kind)` names another module; nested types are
///   `Outer(kind).Inner(kind)`. Generic arguments follow the kind: `Box(struct)<Int>`.
/// - Members of a generic type name its parameters: `Shop.Box(struct)<T>.get() -> T?`,
///   `Swift.Array<Element>.first: Element? { get }`. Write the type's own constraints with them
///   (`Keyed(struct)<K: Hashable>`). The declaration's own parameters come after its name.
/// - Protocol members: `Shop.Shape(protocol).area() -> Double`; the conforming type is `Self`.
/// - `(extension in Module)` marks a member declared in an extension that is mangled as one: an extension in
///   another module than the type's, any constrained extension (`(extension in M where Element == Int)`), and
///   every protocol extension. Inside it, bare names belong to the extension's module.
/// - Not supported: function-typed parameters, operators, initializers, subscripts, setters, non-ASCII names,
///   isolation and ownership attributes, more than one generic enclosing type, parameter packs.
pub fn mangle(declaration: &str) -> Result<String, Error> {
    let decl = parse::parse(declaration).map_err(Error)?;
    mangler::mangle(&decl).map_err(Error)
}

/// The mangled name of the declaration's result type, in the form the Swift runtime resolves
/// (`swift_getTypeByMangledNameInContext`): `"SaySSG"` for `[String]`. `None` if it depends on a generic parameter.
pub fn result_type(declaration: &str) -> Result<Option<String>, Error> {
    let decl = parse::parse(declaration).map_err(Error)?;
    mangler::result_type(&decl).map_err(Error)
}

/// Where a call to a synchronous declaration must run: apus hops there first and back afterwards. An async
/// declaration hops by itself, a `nonisolated` member needs nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Isolation {
    None,
    /// The `self` actor (a sync member of an `(actor)` type).
    Actor,
    /// The main actor (`@MainActor`).
    MainActor,
}

/// The isolation a call to `declaration` needs (see [`Isolation`]).
pub fn isolation(declaration: &str) -> Result<Isolation, Error> {
    let decl = parse::parse(declaration).map_err(Error)?;
    let sync = match &decl.signature {
        ast::Signature::Getter(_) => true,
        ast::Signature::Function { is_async, .. } => !is_async,
    };
    let actor_member = !decl.is_static && decl.context.last().is_some_and(|(_, kind)| *kind == ast::Kind::Actor);
    Ok(match () {
        _ if !sync || decl.nonisolated => Isolation::None,
        _ if decl.main_actor => Isolation::MainActor,
        _ if actor_member => Isolation::Actor,
        _ => Isolation::None,
    })
}

/// Whether the declaration has typed throws (`throws(E)`), which apus calls with an error slot. Only a generic or
/// associated error type is accepted: a concrete one comes back in registers or through a slot depending on its
/// layout, which the declaration does not tell.
pub fn typed_throws(declaration: &str) -> Result<bool, Error> {
    let decl = parse::parse(declaration).map_err(Error)?;
    match &decl.signature {
        ast::Signature::Function { typed_error: Some(ast::Ty::Param(_) | ast::Ty::Member(..)), .. } => Ok(true),
        ast::Signature::Function { typed_error: Some(_), .. } => Err(Error(
            "concrete typed throws is not supported yet: its error may come back in registers or through a slot depending on layout"
                .into(),
        )),
        _ => Ok(false),
    }
}

/// Refuses declarations apus would call with the wrong arguments: an `isolated` parameter of a type other than
/// `(any Actor)?` (apus passes nil outside the main actor, which a non-optional actor parameter cannot hold).
pub fn check_callable(declaration: &str) -> Result<(), Error> {
    let decl = parse::parse(declaration).map_err(Error)?;
    if let ast::Signature::Function { params, .. } = &decl.signature {
        if params.iter().any(|p| p.isolated && !matches!(p.ty, ast::Ty::Optional(_))) {
            return Err(Error("only `isolated (any Actor)?` parameters are supported: apus passes nil where the call is not on the main actor".into()));
        }
    }
    Ok(())
}

/// Whether the declaration is `nonisolated(nonsending)`: apus passes the caller's isolation as a hidden first argument.
/// Only the declaration can say so; the symbol is the same as a plain async function's.
pub fn nonsending(declaration: &str) -> Result<bool, Error> {
    Ok(parse::parse(declaration).map_err(Error)?.nonsending)
}

#[cfg(test)]
mod isolation_tests {
    use super::{isolation, Isolation};

    #[test]
    fn sync_members_of_actors_and_the_main_actor_need_a_hop() {
        let of = |d: &str| isolation(d).unwrap();
        assert_eq!(of("M.Tally(actor).bump(by: Int) -> Int"), Isolation::Actor);
        assert_eq!(of("M.Tally(actor).count: Int { get }"), Isolation::Actor);
        assert_eq!(of("M.Tally(actor).bump(by: Int) async -> Int"), Isolation::None);
        assert_eq!(of("nonisolated M.Tally(actor).label() -> String"), Isolation::None);
        assert_eq!(of("static M.Tally(actor).make() -> Tally(actor)"), Isolation::None);
        assert_eq!(of("@MainActor M.onMain() -> Bool"), Isolation::MainActor);
        assert_eq!(of("@MainActor M.onMain() async -> Bool"), Isolation::None);
        assert_eq!(of("M.Counter(class).bump(by: Int) -> Int"), Isolation::None);
    }

    #[test]
    fn only_optional_isolated_parameters_are_callable() {
        use super::check_callable;
        assert!(check_callable("M.f(_: Int, isolation: isolated (any Actor)? = #isolation) async -> Int").is_ok());
        let error = check_callable("M.f(_: isolated any Actor, _: Int) -> Int").unwrap_err().to_string();
        assert!(error.contains("isolated (any Actor)?"), "{error}");
    }

    #[test]
    fn only_generic_typed_errors_are_callable() {
        use super::typed_throws;
        assert_eq!(typed_throws("M.f<E: Error>(_: E) async throws(E) -> Int").unwrap(), true);
        assert_eq!(typed_throws("M.f() throws(any Error) -> Int").unwrap(), false);
        assert_eq!(typed_throws("M.f() throws -> Int").unwrap(), false);
        assert!(typed_throws("M.f() throws(Boom(struct)) -> Int").unwrap_err().to_string().contains("concrete typed throws"));
    }
}
