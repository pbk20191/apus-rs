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
/// [static] [(extension in Module [where …])] Module.Type(kind)[<T, …>]….name: Type { get [async] [throws] | set }
/// [static] [(extension in Module [where …])] Module.Type(kind)[<T, …>]….subscript[<U: P>]([label|_]: Type, …) -> Type { get [async] [throws] | set }
/// [static] [prefix | postfix] [Module.Type(kind)….]+(_: Type[, _: Type]) -> Type   (an operator: infix with two parameters)
/// [mutating | consuming | borrowing] … (how the method takes `self`; the symbol does not show it)
/// … ([label|_]: [_const] [inout | consuming | borrowing | __owned | __shared] Type, …)   (`_const`: AppIntents' compile-time constants)
/// @preconcurrency …   (its symbol leaves out @Sendable, global actors in function types and `Sendable` requirements)
/// [(extension in Module [where …])] Module.Type(kind)[<T, …>]….init[<U: P>]([label|_]: Type, …) [async] [throws] [-> Type?]
/// … T.Value(of P) …   (an associated type of a parameter with several protocols names its protocol)
/// Module.Outer(kind).(extension in Module [where …]) Inner(kind)….member   (a type declared in an extension that is
///     part of its context: a constrained one, or one in another module than the extended type's)
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
/// - Subscript labels are the ones the call site writes: `subscript(_: Int)` for `v[i]`.
/// - Not supported: non-ASCII names (and so non-ASCII operators),
///   a declaration inside more than one generic enclosing type, parameter packs, metatypes.
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

/// A formal argument whose Rust type the declaration fixes, because Swift passes it as one pointer where `swift!`
/// would otherwise pass a value by address.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArgumentKind {
    /// A value of a class-bound generic type (`T: SomeClass`, `T: AnyObject`): a class reference.
    ClassBound,
    /// A metatype (`T.Type`, a class's, `Any.Type`): the type's metadata, a `SwiftType`.
    Metatype,
}

/// A result whose Rust type the declaration fixes, for the same reason as [`ArgumentKind`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResultKind {
    /// A value of a class-bound generic type, returned as a +1 reference.
    ClassReference,
    /// A metatype, returned as the type's metadata.
    Metatype,
}

/// The generic parameters (the declaration's, its enclosing type's, its extension's) that are class-bound: Swift
/// passes their values as single references.
fn class_bound_params(decl: &ast::Decl) -> std::collections::HashSet<ast::GenericParam> {
    decl.requirements
        .iter()
        .chain(&decl.context_requirements)
        .chain(decl.extension.iter().flat_map(|e| e.requirements.iter()))
        .chain(decl.context_extension.iter().flat_map(|e| e.requirements.iter()))
        .filter_map(|requirement| match requirement {
            ast::Requirement::Superclass(ast::Subject::Param(param), _) | ast::Requirement::AnyObject(ast::Subject::Param(param)) => {
                Some(*param)
            }
            _ => None,
        })
        .collect()
}

/// Refuses types whose lowering `swift!` does not follow: Swift passes a value of a class-bound generic type and a
/// thick metatype as one pointer (observed type lowering; the owner accepted it), which apus does at a parameter's or
/// result's top level only. A struct's or enum's metatype is thin (nothing is passed), an optional metatype is a
/// byte, `any P.Type` two words.
fn check_lowering(decl: &ast::Decl) -> Result<(), String> {
    use ast::Ty;
    #[derive(Clone, Copy, PartialEq)]
    enum At {
        Top,
        Optional,
        Tuple,
        Closure,
        Argument,
    }
    let class_bound = class_bound_params(decl);
    fn walk(ty: &Ty, at: At, class_bound: &std::collections::HashSet<ast::GenericParam>) -> Result<(), String> {
        let inner = |next: At| if at == At::Closure { At::Closure } else { next };
        match ty {
            Ty::Metatype(instance) => {
                let thick = match &**instance {
                    Ty::Param(_) | Ty::Any => true,
                    Ty::Nominal { path, .. } => {
                        matches!(path.last(), Some((_, ast::Kind::Class | ast::Kind::Actor | ast::Kind::TypedefClass)))
                    }
                    _ => false,
                };
                match at {
                    At::Argument => Ok(()),
                    At::Top if thick => Ok(()),
                    At::Top => Err("a metatype is passed as type metadata only for a generic parameter, a class or `Any`: \
                                    a struct's or enum's is thin (nothing is passed), `any P.Type` takes a witness table too"
                        .into()),
                    _ => Err("a metatype is supported only as a parameter or result itself".into()),
                }
            }
            Ty::Param(param) if class_bound.contains(param) => match at {
                At::Top | At::Argument => Ok(()),
                _ => Err("a class-bound generic parameter's value is supported only as a parameter or result itself".into()),
            },
            Ty::Optional(wrapped) => walk(wrapped, inner(if at == At::Top { At::Optional } else { at }), class_bound),
            Ty::Tuple(elements) => elements.iter().try_for_each(|(_, element)| walk(element, inner(At::Tuple), class_bound)),
            Ty::Function(function) => {
                function.params.iter().chain([&function.result]).try_for_each(|ty| walk(ty, At::Closure, class_bound))
            }
            Ty::Nominal { args, .. } => args.iter().try_for_each(|arg| walk(arg, inner(At::Argument), class_bound)),
            Ty::Param(_) | Ty::Member(..) | Ty::Any | Ty::AnyObject | Ty::Existential(_) | Ty::Opaque(_) => Ok(()),
        }
    }
    let check = |ty: &Ty| walk(ty, At::Top, &class_bound);
    match &decl.signature {
        ast::Signature::Function { params, result, typed_error, .. } => {
            params.iter().try_for_each(|p| check(&p.ty))?;
            check(result)?;
            match typed_error {
                Some(Ty::Param(param)) if class_bound.contains(param) => Err("a class-bound typed error is not supported".into()),
                _ => Ok(()),
            }
        }
        ast::Signature::Getter(ty) | ast::Signature::Setter(ty) => check(ty),
        ast::Signature::Subscript { params, result, .. } => {
            params.iter().try_for_each(|p| check(&p.ty))?;
            check(result)
        }
        ast::Signature::Case(_) => Ok(()),
    }
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
            ast::Signature::Getter(_) | ast::Signature::Setter(_) | ast::Signature::Subscript { .. } => !decl.accessor_async,
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
            Some((_, ast::Kind::Class | ast::Kind::Actor | ast::Kind::TypedefClass)) => TypeSelf::Thick,
            Some((_, ast::Kind::Struct | ast::Kind::Enum | ast::Kind::TypedefStruct)) => TypeSelf::Thin,
            _ => TypeSelf::NotStatic,
        }
    }

    /// Per formal parameter, whether the callee consumes it (+1) instead of borrowing it: every parameter of an
    /// initializer and a setter's new value (Swift's defaults, SE-0377), or any marked `consuming`/`__owned`.
    pub fn consumed(&self) -> Vec<bool> {
        let by_default = self.0.name == "init";
        match &self.0.signature {
            ast::Signature::Function { params, .. } => params.iter().map(|p| ast::Ownership::consumes(p.ownership, by_default)).collect(),
            // The new value comes first, then a subscript's indices (borrowed unless marked).
            ast::Signature::Setter(_) => vec![true],
            ast::Signature::Subscript { params, setter: true, .. } => {
                std::iter::once(true).chain(params.iter().map(|p| ast::Ownership::consumes(p.ownership, false))).collect()
            }
            ast::Signature::Subscript { params, .. } => params.iter().map(|p| ast::Ownership::consumes(p.ownership, false)).collect(),
            _ => vec![false; self.formal_count()],
        }
    }

    /// Whether the method consumes `self` (`consuming func`): apus passes a copy for the callee to consume.
    pub fn consumes_self(&self) -> bool {
        self.0.consumes_self
    }

    /// Whether the callee is async: an `async` function, or a `{ get async }` getter.
    pub fn is_async(&self) -> bool {
        match &self.0.signature {
            ast::Signature::Function { is_async, .. } => *is_async,
            _ => self.0.accessor_async,
        }
    }

    /// Whether the callee throws `any Error` (`throws`, or `{ get throws }`); typed throws is [`typed_throws`].
    pub fn throws_any_error(&self) -> bool {
        match &self.0.signature {
            ast::Signature::Function { throws, typed_error, .. } => *throws && typed_error.is_none(),
            _ => self.0.accessor_throws,
        }
    }

    /// Per formal parameter, whether it is `inout` (passed as the address of a value the callee changes in place).
    pub fn inout_params(&self) -> Vec<bool> {
        match &self.0.signature {
            ast::Signature::Function { params, .. } => params.iter().map(|p| p.inout).collect(),
            _ => vec![false; self.formal_count()],
        }
    }

    /// Whether the call changes `self` in place: a `mutating` method, or a setter, of a struct or enum (`self` is
    /// passed `inout`: the value's own address).
    pub fn mutates_self(&self) -> bool {
        let decl = &self.0;
        let setter = matches!(decl.signature, ast::Signature::Setter(_) | ast::Signature::Subscript { setter: true, .. });
        let value_type = decl.context.last().is_some_and(|(_, kind)| matches!(kind, ast::Kind::Struct | ast::Kind::Enum | ast::Kind::TypedefStruct));
        !decl.is_static && value_type && (decl.mutating || setter)
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
        check_lowering(&self.0).map_err(Error)?;
        if let ast::Signature::Function { params, .. } = &self.0.signature {
            if params.iter().any(|p| p.isolated && !matches!(p.ty, ast::Ty::Optional(_))) {
                return Err(Error("only `isolated (any Actor)?` parameters are supported: apus passes nil where the call is not on the main actor".into()));
            }
        }
        Ok(())
    }

    /// How each formal argument must be passed, where the declaration fixes it (see [`ArgumentKind`]), in the order
    /// the Rust signature lists them.
    pub fn argument_kinds(&self) -> Result<Vec<Option<ArgumentKind>>, Error> {
        let decl = &self.0;
        let class_bound = class_bound_params(decl);
        let kind = |ty: &ast::Ty| match ty {
            ast::Ty::Param(param) if class_bound.contains(param) => Some(ArgumentKind::ClassBound),
            ast::Ty::Metatype(_) => Some(ArgumentKind::Metatype),
            _ => None,
        };
        Ok(match &decl.signature {
            ast::Signature::Function { params, .. } => params.iter().map(|p| if p.inout { None } else { kind(&p.ty) }).collect(),
            ast::Signature::Getter(_) | ast::Signature::Case(_) => Vec::new(),
            ast::Signature::Setter(ty) => vec![kind(ty)],
            ast::Signature::Subscript { params, result, setter } => {
                let new_value = setter.then(|| kind(result));
                new_value.into_iter().chain(params.iter().map(|p| kind(&p.ty))).collect()
            }
        })
    }

    /// What the result must be taken as, where the declaration fixes it (see [`ResultKind`]).
    pub fn result_kind(&self) -> Option<ResultKind> {
        let decl = &self.0;
        let result = match &decl.signature {
            ast::Signature::Function { result, .. } | ast::Signature::Getter(result) => result,
            ast::Signature::Subscript { result, setter: false, .. } => result,
            _ => return None,
        };
        match result {
            ast::Ty::Param(param) if class_bound_params(decl).contains(param) => Some(ResultKind::ClassReference),
            ast::Ty::Metatype(_) => Some(ResultKind::Metatype),
            _ => None,
        }
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
            ast::Signature::Setter(_) => 1,
            ast::Signature::Subscript { params, setter, .. } => params.len() + usize::from(*setter),
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

/// The symbol of the opaque type descriptor of a declaration with a `some P` result (`…QOMQ`, named after the
/// function, property or subscript itself), or `None`. The runtime gives the result's actual type from it
/// (`swift_getOpaqueTypeMetadata2`).
pub fn opaque_descriptor(declaration: &str) -> Result<Option<String>, Error> {
    let parsed = Declaration::parse(declaration)?;
    let result = match &parsed.0.signature {
        ast::Signature::Function { result, .. } => result,
        ast::Signature::Getter(ty) | ast::Signature::Subscript { result: ty, setter: false, .. } => ty,
        _ => return Ok(None),
    };
    if result.opaque_count() == 0 {
        return Ok(None);
    }
    let symbol = mangler::mangle(&parsed.0).map_err(Error)?;
    // A getter's descriptor is the variable's or subscript's: `…vg` -> `…vp`, `…ig` -> `…ip` (before a static `Z`).
    let (body, is_static) = symbol.strip_suffix('Z').map_or((symbol.as_str(), ""), |body| (body, "Z"));
    let entity = match &parsed.0.signature {
        ast::Signature::Getter(_) => body.strip_suffix("vg").map(|stem| format!("{stem}vp")),
        ast::Signature::Subscript { .. } => body.strip_suffix("ig").map(|stem| format!("{stem}ip")),
        _ => Some(body.to_string()),
    }
    .ok_or_else(|| Error("unexpected accessor symbol".into()))?;
    Ok(Some(format!("{entity}{is_static}QOMQ")))
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
/// `(any Actor)?` (apus passes nil outside the main actor, which a non-optional actor parameter cannot hold); a
/// class-bound generic parameter (its values are references, not passed by address).
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
                Ty::Param(_) | Ty::Member(..) | Ty::Opaque(_) => true,
                Ty::Optional(wrapped) | Ty::Metatype(wrapped) => mentions_param(wrapped),
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
            Ty::Nominal { module, path, args, .. } if module == "Swift" && path.len() == 1 && args.is_empty()
                && (DIRECT.contains(&path[0].0.as_str()) || path[0].0 == "String") => 'd',
            Ty::Nominal { path, args, .. } if args.is_empty() && matches!(path.last(), Some((_, ast::Kind::Class | ast::Kind::Actor | ast::Kind::TypedefClass))) => 'd',
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
    /// A result whose type is exactly a generic parameter (not a class-bound one, which comes back as a reference):
    /// the formal parameter giving that type.
    pub result_from: Option<usize>,
    /// Per formal parameter, the function of `apus::swift` that gives the Swift type it names or holds:
    /// `__type_of` (a value), `__object_type_of` (a class reference), `__metatype_of` (a metatype).
    pub type_functions: Vec<&'static str>,
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
    let class_bound = class_bound_params(decl);
    let type_functions = params
        .iter()
        .map(|p| match &p.ty {
            Ty::Metatype(_) => "__metatype_of",
            Ty::Param(param) if class_bound.contains(param) => "__object_type_of",
            _ => "__type_of",
        })
        .collect();
    let mut metadata_from = Vec::new();
    for index in 0..decl.generic_params {
        let param = GenericParam { depth: 0, index };
        // `T.Type` names T exactly; a value gives its own (for a class reference, its dynamic) type.
        let named = |ty: &Ty| !matches!(ty, Ty::Metatype(instance) if **instance == Ty::Param(param));
        let source = params
            .iter()
            .position(|p| !p.inout && !p.variadic && !named(&p.ty))
            .or_else(|| params.iter().position(|p| !p.inout && !p.variadic && p.ty == Ty::Param(param)));
        match source {
            Some(formal) => metadata_from.push(formal),
            None => return explicit(format!("generic parameter {} is not the type of one of its parameters", index + 1)),
        }
    }
    // Marker protocols take no witness table.
    let marker = ast::Protocol::is_marker;
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
            // No witness table: a class-bound parameter's value is a reference (see `check_lowering`).
            Requirement::Superclass(..) | Requirement::AnyObject(_) => {}
        }
    }
    let error_slot_from = match typed_error {
        Some(Ty::Param(param)) => Some(metadata_from[param.index]),
        Some(Ty::Member(..)) => return explicit("typed throws of an associated type".into()),
        Some(_) => return Err(Error(CONCRETE_TYPED_THROWS.into())),
        None => None,
    };
    let result_from = match result {
        Ty::Param(param) if !class_bound.contains(param) => Some(metadata_from[param.index]),
        _ => None,
    };
    Ok(Some(GenericPlan { formal_count: params.len(), metadata_from, witnesses, error_slot_from, result_from, type_functions }))
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

    #[test]
    fn class_bound_values_are_references() {
        use super::{ArgumentKind, Declaration, ResultKind};
        let parsed = |d: &str| Declaration::parse(d).unwrap();
        // A value of a class-bound generic type is passed and returned as a single reference; its type comes from
        // the object.
        let keep = parsed("M.keep<T: Base(class)>(_: T, _: Int) -> T");
        assert_eq!(keep.argument_kinds().unwrap(), vec![Some(ArgumentKind::ClassBound), None]);
        assert_eq!(keep.result_kind(), Some(ResultKind::ClassReference));
        let plan = keep.generic_plan().unwrap().unwrap();
        assert_eq!((plan.metadata_from, plan.result_from), (vec![0], None));
        assert_eq!(plan.type_functions, vec!["__object_type_of", "__type_of"]);
        assert_eq!(parsed("M.f<T: AnyObject & Hashable>(_: T)").generic_plan().unwrap().unwrap().witnesses, vec![(0, "$sSHMp".into())]);
        // An enclosing type's class-bound parameter (`Measurement<UnitType: NSUnit>`) is one too.
        let unit = parsed("Foundation.Measurement(struct)<UnitType: __C.NSUnit(class)>.unit: UnitType { get }");
        assert!(unit.check_callable().is_ok());
        assert_eq!(unit.result_kind(), Some(ResultKind::ClassReference));
        let init = parsed("Foundation.Measurement(struct)<UnitType: __C.NSUnit(class)>.init(value: Double, unit: UnitType)");
        assert_eq!(init.argument_kinds().unwrap(), vec![None, Some(ArgumentKind::ClassBound)]);
        let constrained = parsed("(extension in M where T: Base(class)) M.Box(struct)<T>.put(_: T)");
        assert_eq!(constrained.argument_kinds().unwrap(), vec![Some(ArgumentKind::ClassBound)]);
        // Where Swift would lower it otherwise, refused.
        for declaration in [
            "M.f<T: AnyObject>(_: T?)",
            "M.f<T: AnyObject>(_: T) -> T?",
            "M.f<T: AnyObject>(_: (T, Int))",
            "M.f<T: AnyObject>(_: T, _: (T) -> Void)",
            "M.Box(struct)<T: AnyObject>.get() -> T?",
        ] {
            let error = super::check_callable(declaration).unwrap_err().to_string();
            assert!(error.contains("class-bound"), "{declaration}: {error}");
        }
    }

    #[test]
    fn metatypes_are_metadata_pointers() {
        use super::{ArgumentKind, Declaration, ResultKind};
        let parsed = |d: &str| Declaration::parse(d).unwrap();
        // `T.Type` gives T's metadata; a class's metatype and `Any.Type` are its metadata too.
        let decode = parsed("M.decode<T: Decodable>(_: T.Type, from: Int) -> T");
        assert_eq!(decode.argument_kinds().unwrap(), vec![Some(ArgumentKind::Metatype), None]);
        let plan = decode.generic_plan().unwrap().unwrap();
        assert_eq!((plan.metadata_from, plan.result_from), (vec![0], Some(0)));
        assert_eq!(plan.type_functions[0], "__metatype_of");
        assert_eq!(parsed("M.f(_: Base(class).Type, _: Any.Type)").argument_kinds().unwrap(), vec![Some(ArgumentKind::Metatype); 2]);
        assert_eq!(parsed("M.f<T>(_: T.Type) -> T.Type").result_kind(), Some(ResultKind::Metatype));
        // A struct's or enum's metatype is thin (nothing is passed), an optional one a byte, `any P.Type` two words.
        for declaration in [
            "M.f(_: Int.Type)",
            "M.f(_: Big(struct).Type)",
            "M.f() -> Kind(enum).Type",
            "M.f(_: Base(class).Type?)",
            "M.f(_: any P.Type)",
            "M.f(_: (Any.Type) -> Void)",
        ] {
            let error = super::check_callable(declaration).unwrap_err().to_string();
            assert!(error.contains("metatype"), "{declaration}: {error}");
        }
    }

    #[test]
    fn a_metatype_is_not_a_member_type() {
        // `T.Type` is T's metatype (`xm`), never an associated type named `Type`.
        assert_eq!(super::mangle("M.f<T>(_: T.Type) -> Int").unwrap(), "$s1M1fySixmlF");
        let error = super::mangle("case M.E(enum)<T>.boom(T.Protocol)").unwrap_err().to_string();
        assert!(error.contains("metatype"), "{error}");
    }
}
