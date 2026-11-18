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
/// [(extension in Module [where …])] Module.Type(kind)[<T, …>]….init[<U: P>]([label|_]: Type, …) [async] [throws] [-> Type?]
/// case Module.Type(kind)[<T, …>]….Enum(enum).name[([label:] Type, …)]
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
/// - Function types: `(Int, String) async throws -> String`, with `@escaping`, `@Sendable`, `@MainActor`.
/// - Initializers mangle to their allocating entry point (`…fC`); the result is the type itself unless written
///   (`-> Level(enum)?` for a failable one). Protocol initializers are not supported.
/// - `case …` gives an enum case's tag symbol (`…WC`), which library-evolution modules export.
/// - Not supported: operators, subscripts, setters, non-ASCII names,
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

/// What a call passes as `self` for the type itself: a static member or an initializer gets its type's metatype.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TypeSelf {
    /// An instance member or a free function.
    NotStatic,
    /// A struct's or enum's metatype is thin: nothing is passed.
    Thin,
    /// A class's or actor's metatype is its metadata, passed as `self`.
    Thick,
}

/// A declaration parsed once, for a caller that asks several of the questions below about it (`swift!`).
pub struct Declaration(ast::Decl);

impl Declaration {
    pub fn parse(declaration: &str) -> Result<Self, Error> {
        parse::parse(declaration).map(Declaration).map_err(Error)
    }

    /// See [`isolation`].
    pub fn isolation(&self) -> Isolation {
        let decl = &self.0;
        let sync = match &decl.signature {
            ast::Signature::Getter(_) => true,
            ast::Signature::Function { is_async, .. } => !is_async,
            ast::Signature::Case(_) => return Isolation::None,
        };
        let actor_member = !decl.is_static && decl.name != "init" && decl.context.last().is_some_and(|(_, kind)| *kind == ast::Kind::Actor);
        match () {
            _ if !sync || decl.nonisolated => Isolation::None,
            _ if decl.main_actor => Isolation::MainActor,
            _ if actor_member => Isolation::Actor,
            _ => Isolation::None,
        }
    }

    /// See [`TypeSelf`].
    pub fn type_self(&self) -> TypeSelf {
        let decl = &self.0;
        if !decl.is_static && decl.name != "init" {
            return TypeSelf::NotStatic;
        }
        match decl.context.last() {
            Some((_, ast::Kind::Class | ast::Kind::Actor)) => TypeSelf::Thick,
            Some((_, ast::Kind::Struct | ast::Kind::Enum)) => TypeSelf::Thin,
            _ => TypeSelf::NotStatic,
        }
    }

    /// See [`typed_throws`].
    pub fn typed_throws(&self) -> Result<bool, Error> {
        match &self.0.signature {
            ast::Signature::Function { typed_error: Some(ast::Ty::Param(_) | ast::Ty::Member(..)), .. } => Ok(true),
            ast::Signature::Function { typed_error: Some(_), .. } => Err(Error(CONCRETE_TYPED_THROWS.into())),
            _ => Ok(false),
        }
    }

    /// See [`check_callable`].
    pub fn check_callable(&self) -> Result<(), Error> {
        if let ast::Signature::Case(_) = &self.0.signature {
            return Err(Error("an enum case is not callable: `mangle!` gives its tag symbol, for `SwiftEnumCase`".into()));
        }
        if self.0.is_static && self.0.context.last().is_some_and(|(_, kind)| *kind == ast::Kind::Protocol) {
            return Err(Error("static protocol requirements are not supported".into()));
        }
        if let ast::Signature::Function { params, .. } = &self.0.signature {
            if params.iter().any(|p| p.isolated && !matches!(p.ty, ast::Ty::Optional(_))) {
                return Err(Error("only `isolated (any Actor)?` parameters are supported: apus passes nil where the call is not on the main actor".into()));
            }
        }
        Ok(())
    }

    /// See [`nonsending`].
    pub fn nonsending(&self) -> bool {
        self.0.nonsending
    }

    /// See [`formal_count`].
    pub fn formal_count(&self) -> usize {
        match &self.0.signature {
            ast::Signature::Function { params, .. } => params.len(),
            ast::Signature::Getter(_) | ast::Signature::Case(_) => 0,
        }
    }

    /// See [`closure_shapes`].
    pub fn closure_shapes(&self) -> Result<Vec<Option<String>>, Error> {
        let ast::Signature::Function { params, .. } = &self.0.signature else { return Ok(Vec::new()) };
        params
            .iter()
            .map(|param| match &param.ty {
                ast::Ty::Function(function) => closure_shape(function).map(Some).map_err(Error),
                _ => Ok(None),
            })
            .collect()
    }

    /// See [`generic_plan`].
    pub fn generic_plan(&self) -> Result<Option<GenericPlan>, Error> {
        generic_plan_of(&self.0)
    }
}

/// The isolation a call to `declaration` needs (see [`Isolation`]).
pub fn isolation(declaration: &str) -> Result<Isolation, Error> {
    Ok(Declaration::parse(declaration)?.isolation())
}

const CONCRETE_TYPED_THROWS: &str =
    "concrete typed throws is not supported yet: its error may come back in registers or through a slot depending on layout";

/// Whether the declaration has typed throws (`throws(E)`), which apus calls with an error slot. Only a generic or
/// associated error type is accepted: a concrete one comes back in registers or through a slot depending on its
/// layout, which the declaration does not tell.
pub fn typed_throws(declaration: &str) -> Result<bool, Error> {
    Declaration::parse(declaration)?.typed_throws()
}

/// Refuses declarations apus would call with the wrong arguments: an `isolated` parameter of a type other than
/// `(any Actor)?` (apus passes nil outside the main actor, which a non-optional actor parameter cannot hold).
pub fn check_callable(declaration: &str) -> Result<(), Error> {
    Declaration::parse(declaration)?.check_callable()
}

/// Whether the declaration is `nonisolated(nonsending)`: apus passes the caller's isolation as a hidden first argument.
/// Only the declaration can say so; the symbol is the same as a plain async function's.
pub fn nonsending(declaration: &str) -> Result<bool, Error> {
    Ok(Declaration::parse(declaration)?.nonsending())
}

/// The symbol apus calls for `declaration`: the mangled name, or for a protocol requirement its dispatch thunk
/// (`…Tj`), which library-evolution modules (Apple's frameworks) export instead of the requirement itself.
pub fn call_symbol(declaration: &str) -> Result<String, Error> {
    let decl = parse::parse(declaration).map_err(Error)?;
    let symbol = mangler::mangle(&decl).map_err(Error)?;
    let requirement = decl.extension.is_none() && decl.context.last().is_some_and(|(_, kind)| *kind == ast::Kind::Protocol);
    Ok(if requirement { format!("{symbol}Tj") } else { symbol })
}

/// The number of formal parameters of a function declaration (0 for a getter).
pub fn formal_count(declaration: &str) -> Result<usize, Error> {
    Ok(Declaration::parse(declaration)?.formal_count())
}

/// Per formal parameter, the shape a closure passed for it must have, or `None` for a parameter that is not a
/// function type: `a`/`s` (async or sync), then `t` (throws) and `m` (`@MainActor`) when declared, then the
/// parameters and the result in parentheses and after them, each `i` (passed by address: a generic parameter,
/// `Any`), `d` (in registers: standard scalars, `String`, class references, `any Error`), `?` (the Rust types decide),
/// and `-` for a `Void` result: `"a(di)d"`.
pub fn closure_shapes(declaration: &str) -> Result<Vec<Option<String>>, Error> {
    Declaration::parse(declaration)?.closure_shapes()
}

fn closure_shape(function: &ast::FunctionTy) -> Result<String, String> {
    fn code(ty: &ast::Ty) -> Result<char, String> {
        use ast::Ty;
        fn mentions_param(ty: &Ty) -> bool {
            match ty {
                Ty::Param(_) | Ty::Member(..) => true,
                Ty::Optional(wrapped) => mentions_param(wrapped),
                Ty::Tuple(elements) => elements.iter().any(|(_, element)| mentions_param(element)),
                Ty::Nominal { args, .. } => args.iter().any(mentions_param),
                Ty::Function(f) => f.params.iter().any(mentions_param) || mentions_param(&f.result),
                Ty::Any | Ty::AnyObject | Ty::Existential(_) => false,
            }
        }
        const DIRECT: [&str; 13] =
            ["Int", "UInt", "Int8", "Int16", "Int32", "Int64", "UInt8", "UInt16", "UInt32", "UInt64", "Double", "Float", "Bool"];
        Ok(match ty {
            Ty::Param(_) | Ty::Member(..) | Ty::Any => 'i',
            // `any Error` is a single box reference; another `any P` is a container by address unless P is
            // class-bound, which the declaration does not show.
            Ty::Existential(protocol) if protocol.module == "Swift" && protocol.name == "Error" => 'd',
            Ty::Existential(_) => '?',
            Ty::AnyObject => 'd',
            Ty::Nominal { module, path, args } if module == "Swift" && path.len() == 1 && args.is_empty()
                && (DIRECT.contains(&path[0].0.as_str()) || path[0].0 == "String") => 'd',
            Ty::Nominal { path, args, .. } if args.is_empty() && matches!(path.last(), Some((_, ast::Kind::Class | ast::Kind::Actor))) => 'd',
            ty if mentions_param(ty) => {
                return Err("a closure parameter or result that involves a generic parameter must be the parameter itself".into())
            }
            _ => '?',
        })
    }
    let mut shape = String::from(if function.is_async { "a" } else { "s" });
    if function.throws {
        shape.push('t');
    }
    if function.main_actor {
        shape.push('m');
    }
    shape.push('(');
    for param in &function.params {
        shape.push(code(param)?);
    }
    shape.push(')');
    shape.push(if function.result == ast::Ty::Tuple(Vec::new()) { '-' } else { code(&function.result)? });
    Ok(shape)
}

/// How `swift!` fills a generic function's generic arguments itself (see [`generic_plan`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GenericPlan {
    /// The declaration's formal parameters (what a Rust signature lists in automatic mode).
    pub formal_count: usize,
    /// Per generic parameter, in order: the formal parameter whose value gives its type metadata.
    pub metadata_from: Vec<usize>,
    /// Witness tables in the ABI's canonical requirement order: (formal parameter giving the type, protocol descriptor).
    pub witnesses: Vec<(usize, String)>,
    /// Typed throws of a generic error type: the formal parameter giving that type.
    pub error_slot_from: Option<usize>,
    /// A result whose type is exactly a generic parameter: the formal parameter giving that type.
    pub result_from: Option<usize>,
}

/// The generic arguments a call to `declaration` needs and where their values come from, or `None` when it is not
/// generic. Errors name what automatic arguments cannot handle; such a declaration takes them explicitly.
pub fn generic_plan(declaration: &str) -> Result<Option<GenericPlan>, Error> {
    Declaration::parse(declaration)?.generic_plan()
}

fn generic_plan_of(decl: &ast::Decl) -> Result<Option<GenericPlan>, Error> {
    use ast::{GenericParam, Requirement, Subject, Ty};
    let explicit = |why: String| Err(Error(format!("{why}: pass the generic arguments explicitly (type metadata, then witness tables)")));
    let ast::Signature::Function { params, typed_error, result, .. } = &decl.signature else { return Ok(None) };
    // A member of a generic type gets its type's generic arguments through `self` / `Self`: nothing to fill unless
    // it has generic parameters of its own.
    if decl.generic_params == 0 {
        return Ok(None);
    }
    if decl.context_params > 0 {
        return explicit("generic members of generic types".into());
    }
    let mut metadata_from = Vec::new();
    for index in 0..decl.generic_params {
        let param = GenericParam { depth: 0, index };
        match params.iter().position(|p| !p.inout && !p.variadic && p.ty == Ty::Param(param)) {
            Some(formal) => metadata_from.push(formal),
            None => return explicit(format!("generic parameter {} is not the type of one of its parameters", index + 1)),
        }
    }
    // Marker protocols take no witness table.
    let marker = |p: &ast::Protocol| p.module == "Swift" && matches!(p.name.as_str(), "Sendable" | "SendableMetatype" | "Copyable" | "Escapable" | "BitwiseCopyable");
    let mut witnesses = Vec::new();
    let mut conformances_per_param = vec![0usize; decl.generic_params];
    for requirement in mangler::sorted_requirements(&decl.requirements) {
        match requirement {
            Requirement::Conforms(_, ref protocol) if marker(protocol) => {}
            Requirement::Conforms(Subject::Param(param), protocol) => {
                conformances_per_param[param.index] += 1;
                if conformances_per_param[param.index] > 1 {
                    // The canonical signature drops a conformance another one implies; only the explicit form is sure.
                    return explicit(format!("generic parameter {} has more than one protocol requirement", param.index + 1));
                }
                if protocol.module == "__C" {
                    return explicit("Objective-C protocol requirements (they take no witness table)".into());
                }
                witnesses.push((metadata_from[param.index], mangler::protocol_descriptor(&protocol).map_err(Error)?));
            }
            Requirement::Conforms(Subject::Member(..), _) => return explicit("requirements on associated types".into()),
            Requirement::SameType(Subject::Param(_), _) => return explicit("generic parameters fixed to a type".into()),
            Requirement::SameType(Subject::Member(..), _) => {}
        }
    }
    let error_slot_from = match typed_error {
        Some(Ty::Param(param)) => Some(metadata_from[param.index]),
        Some(Ty::Member(..)) => return explicit("typed throws of an associated type".into()),
        Some(_) => return Err(Error(CONCRETE_TYPED_THROWS.into())),
        None => None,
    };
    let result_from = match result {
        Ty::Param(param) => Some(metadata_from[param.index]),
        _ => None,
    };
    Ok(Some(GenericPlan { formal_count: params.len(), metadata_from, witnesses, error_slot_from, result_from }))
}

#[cfg(test)]
mod isolation_tests {
    use super::{isolation, Isolation};

    #[test]
    fn closure_shapes_follow_the_declared_convention() {
        let shapes =
            super::closure_shapes("M.f(_: (Int, Any) async throws -> String, _: (any Error) -> Void, _: (any P) -> Any, _: Int)").unwrap();
        // `any Error` is one box pointer in a register; `any P` may be class-bound (registers) or not (address).
        assert_eq!(shapes, vec![Some("at(di)d".into()), Some("s(d)-".into()), Some("s(?)i".into()), None]);
    }

    #[test]
    fn statics_and_initializers_take_a_class_as_self() {
        use super::{Declaration, TypeSelf};
        let of = |d: &str| Declaration::parse(d).unwrap().type_self();
        assert_eq!(of("M.Made(struct).init(x: Int)"), TypeSelf::Thin);
        assert_eq!(of("static M.Level(enum).all() -> [Level(enum)]"), TypeSelf::Thin);
        assert_eq!(of("M.Built(class).init(n: Int)"), TypeSelf::Thick);
        assert_eq!(of("static M.Built(class).make() -> Built(class)"), TypeSelf::Thick);
        assert_eq!(of("M.Tally(actor).init()"), TypeSelf::Thick);
        assert_eq!(of("M.Built(class).value: Int { get }"), TypeSelf::NotStatic);
        assert_eq!(of("M.free() -> Int"), TypeSelf::NotStatic);
        assert!(super::check_callable("static M.Shape(protocol).make() -> Self").is_err(), "a static requirement");
        assert_eq!(of("M.Tally(actor).init()"), TypeSelf::Thick);
        assert_eq!(super::isolation("M.Tally(actor).init()").unwrap(), Isolation::None, "an actor's init is nonisolated");
    }

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
    fn generic_plan_gives_metadata_sources_and_witness_order() {
        use super::generic_plan;
        assert!(generic_plan("M.f(_: Int) -> Int").unwrap().is_none(), "not generic");
        assert!(generic_plan("M.Box(struct)<T>.get() -> T").unwrap().is_none(), "the type's arguments come with self");
        let plan = generic_plan("M.describeIt<T: CustomStringConvertible>(_: T, times: Int) async throws -> String").unwrap().unwrap();
        assert_eq!(plan.formal_count, 2);
        assert_eq!(plan.metadata_from, vec![0]);
        assert_eq!(plan.witnesses, vec![(0, "$ss23CustomStringConvertibleMp".to_string())]);
        assert_eq!(plan.error_slot_from, None);
        // Canonical order: by parameter. Marker protocols take no witness table.
        let plan = generic_plan("M.two<A: Hashable, B: Equatable & Sendable>(_: B, _: A) -> Int").unwrap().unwrap();
        assert_eq!(plan.metadata_from, vec![1, 0]);
        assert_eq!(plan.witnesses, vec![(1, "$sSHMp".into()), (0, "$sSQMp".into())]);
        let plan = generic_plan("M.g<E: Error>(_: Bool, _: E) async throws(E) -> Int").unwrap().unwrap();
        assert_eq!(plan.error_slot_from, Some(1));
        assert_eq!(plan.witnesses, vec![(1, "$ss5ErrorMp".into())]);
        // Unsupported for automatic arguments: the message points at passing them explicitly.
        for declaration in [
            "M.make<T>() -> T",
            "M.join<C: Collection>(_: C) -> Int where C.Element: Hashable",
            "M.Box(struct)<T>.map<U>(_: U) -> U",
            // Two conformances of one parameter: Swift drops one implied by the other (Hashable implies Equatable).
            "M.h<T: Equatable & Hashable>(_: T) -> Int",
        ] {
            let error = generic_plan(declaration).unwrap_err().to_string();
            assert!(error.contains("explicitly"), "{declaration}: {error}");
        }
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
        // `throws(Never)` cannot throw: Swift mangles and calls it as a function without `throws`.
        assert_eq!(super::mangle("M.f() throws(Never) -> Int").unwrap(), "$s1M1fSiyF");
        assert_eq!(super::mangle("M.g() async throws(Never) -> Int").unwrap(), "$s1M1gSiyYaF");
        assert_eq!(typed_throws("M.f() throws(Never) -> Int").unwrap(), false);
        let concrete = super::generic_plan("M.g<T>(_: T) throws(Boom(struct)) -> Int").unwrap_err().to_string();
        assert!(concrete.contains("concrete typed throws") && !concrete.contains("associated"), "{concrete}");
    }
}
