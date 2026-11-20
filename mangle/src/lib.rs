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
/// … Predicate(struct)<Pack{T}> …   (a variadic generic type's pack argument)
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

/// The module a bare name in `ty` belongs to: its leading `Module.` (`Todos.Todo(class)`), else the standard library.
fn leading_module(ty: &str) -> &str {
    let ty = ty.trim_start();
    let end = ty.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).unwrap_or(ty.len());
    if end > 0 && ty[end..].starts_with('.') { &ty[..end] } else { "Swift" }
}

/// The mangled name of a concrete type written as in a declaration (`Todos.Todo(class)`,
/// `SwiftData.FetchResultsCollection(struct)<Todos.Todo(class)>`), in the form the Swift runtime resolves
/// (`swift_getTypeByMangledNameInContext`): `"5Todos4TodoC"`. A bare name belongs to the leading module.
pub fn type_name(ty: &str) -> Result<String, Error> {
    let declaration = format!("{}.__apus_type() -> {ty}", leading_module(ty));
    result_type(&declaration)
        .map_err(|Error(e)| Error(format!("`{ty}`: {e}")))?
        .ok_or_else(|| Error(format!("`{ty}`: the type must be concrete")))
}

/// A protocol's descriptor symbol (`…Mp`, documented mangling) from its name: `"$s9SwiftData15PersistentModelMp"` for
/// `SwiftData.PersistentModel`.
pub fn protocol_descriptor_of(protocol: &str) -> Result<String, Error> {
    let declaration = format!("{}.__apus_protocol<T: {protocol}>(_: T)", leading_module(protocol));
    let decl = parse::parse(&declaration).map_err(|e| Error(format!("`{protocol}`: {e}")))?;
    match decl.requirements.as_slice() {
        [ast::Requirement::Conforms(_, p)] => mangler::protocol_descriptor(p).map_err(Error),
        _ => Err(Error(format!("`{protocol}` is not one protocol"))),
    }
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
    /// A protocol's static member or initializer: the conforming type's metadata as `self`, then the type and its
    /// witness table after the arguments (an `OpenedType`).
    Conformance,
}

/// A protocol member's `Self` when its extension fixes it (`where Self == Bordered`).
fn fixed_self(decl: &ast::Decl) -> Option<&ast::Ty> {
    if !decl.context.last().is_some_and(|(_, kind)| *kind == ast::Kind::Protocol) {
        return None;
    }
    let this = ast::Subject::Param(ast::GenericParam { depth: 0, index: 0 });
    decl.extension.iter().flat_map(|e| e.requirements.iter()).find_map(|requirement| match requirement {
        ast::Requirement::SameType(subject, ty) if *subject == this => Some(ty),
        _ => None,
    })
}

/// A formal argument whose Rust type the declaration fixes, because Swift passes it as one pointer where `swift!`
/// would otherwise pass a value by address.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ArgumentKind {
    /// A value of a class-bound generic type (`T: SomeClass`, `T: AnyObject`): a class reference.
    ClassBound,
    /// A metatype (`T.Type`, a class's, `Any.Type`): the type's metadata, a `SwiftType`.
    Metatype,
    /// `@convention(c)`, optional or not: a C function pointer, of the shape given (see [`foreign_shape`]).
    CFunction(String),
    /// `@convention(block)`, optional or not: an Objective-C block, of the shape given.
    Block(String),
    /// `any Error`, optional or not: one error box reference, a `SwiftError`.
    Error,
    /// `any P`, optional or not: a class existential (its protocols class-bound) goes in registers, as its reference
    /// and witness tables; any other by address. Only the type's metadata tells which, at run time.
    Existential,
}

/// `any Error` or `(any Error)?`: Swift passes and returns it as one error box reference.
fn is_error(ty: &ast::Ty) -> bool {
    let ty = match ty {
        ast::Ty::Optional(wrapped) => wrapped,
        ty => ty,
    };
    matches!(ty, ast::Ty::Existential(p) if p.module == "Swift" && p.name == "Error")
}

/// A result whose Rust type the declaration fixes, for the same reason as [`ArgumentKind`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResultKind {
    /// A value of a class-bound generic type, returned as a +1 reference.
    ClassReference,
    /// A metatype, returned as the type's metadata.
    Metatype,
    /// `any Error`, optional or not: an error box reference (+1), a `SwiftError`.
    Error,
    /// A C function pointer.
    CFunction,
    /// An Objective-C block (+1).
    Block,
}

/// A `@convention(c)` or `@convention(block)` function type, itself or optional.
fn foreign_function(ty: &ast::Ty) -> Option<&ast::FunctionTy> {
    match ty {
        ast::Ty::Optional(wrapped) => match &**wrapped {
            ast::Ty::Function(function) if function.convention != ast::Convention::Swift => Some(function),
            _ => None,
        },
        ast::Ty::Function(function) if function.convention != ast::Convention::Swift => Some(function),
        _ => None,
    }
}

/// The parameter and result types of a C function or block as Objective-C type encodings, which the Rust function
/// pointer or block is checked against: `(id)q` takes an `Int32` and a `Double` and returns an `Int`. `?` is any
/// other type (a pointer, an object), `v` no result.
fn foreign_shape(function: &ast::FunctionTy) -> String {
    fn code(ty: &ast::Ty) -> char {
        match ty {
            ast::Ty::Tuple(elements) if elements.is_empty() => 'v',
            ast::Ty::Nominal { module, path, args, .. } if module == "Swift" && path.len() == 1 && args.is_empty() => {
                match path[0].0.as_str() {
                    "Int" | "Int64" => 'q',
                    "UInt" | "UInt64" => 'Q',
                    "Int32" => 'i',
                    "UInt32" => 'I',
                    "Int16" => 's',
                    "UInt16" => 'S',
                    "Int8" => 'c',
                    "UInt8" => 'C',
                    "Double" => 'd',
                    "Float" => 'f',
                    "Bool" => 'B',
                    _ => '?',
                }
            }
            _ => '?',
        }
    }
    let params: String = function.params.iter().map(code).collect();
    format!("({params}){}", code(&function.result))
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
            ast::Requirement::Conforms(ast::Subject::Param(param), protocol) if protocol.class_bound => Some(*param),
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
            // A C function or block is one pointer as a parameter or result, or the optional of one (null is nil),
            // and as a closure's parameter or result; elsewhere `swift!` would not lower it.
            Ty::Function(function) if function.convention != ast::Convention::Swift => {
                if !matches!(at, At::Top | At::Optional | At::Closure | At::Argument) {
                    return Err("a C function or block is supported only as a parameter or result itself, or its optional".into());
                }
                function.params.iter().chain([&function.result]).try_for_each(|ty| walk(ty, At::Closure, class_bound))
            }
            Ty::Optional(wrapped) if at == At::Optional && matches!(&**wrapped, Ty::Function(f) if f.convention != ast::Convention::Swift) => {
                Err("a C function or block is supported only as a parameter or result itself, or its optional".into())
            }
            Ty::Optional(wrapped) => walk(wrapped, inner(if at == At::Top { At::Optional } else { at }), class_bound),
            Ty::Tuple(elements) => elements.iter().try_for_each(|(_, element)| walk(element, inner(At::Tuple), class_bound)),
            Ty::Function(function) => {
                function.params.iter().chain([&function.result]).try_for_each(|ty| walk(ty, At::Closure, class_bound))
            }
            Ty::Nominal { args, .. } => args.iter().try_for_each(|arg| walk(arg, inner(At::Argument), class_bound)),
            Ty::Pack(elements) => elements.iter().try_for_each(|element| walk(element, inner(At::Argument), class_bound)),
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

    /// The mangled name (no `$s`) of the type a static member or initializer belongs to, as the runtime resolves it:
    /// `"5Probe6MintedC"`. `None` for a generic type (its name needs the generic arguments) or a free function.
    pub fn context_type_name(&self) -> Option<String> {
        let decl = &self.0;
        let ty = match fixed_self(decl) {
            Some(ty) => ty.clone(),
            None if decl.context.is_empty() || decl.context_params > 0 => return None,
            None => ast::Ty::Nominal {
                module: decl.module.clone(),
                path: decl.context.clone(),
                args: Vec::new(),
                split: Vec::new(),
                extension: decl.context_extension.clone().map(Box::new),
            },
        };
        mangler::type_name(&ty).ok()
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
        // `where Self == Concrete`: a member of that type (observed: no Self metadata or witness table is passed).
        let kind = match fixed_self(decl) {
            Some(ast::Ty::Nominal { path, .. }) => path.last().map(|(_, kind)| *kind),
            Some(_) => None,
            None => decl.context.last().map(|(_, kind)| *kind),
        };
        match kind {
            Some(ast::Kind::Class | ast::Kind::Actor | ast::Kind::TypedefClass) => TypeSelf::Thick,
            Some(ast::Kind::Struct | ast::Kind::Enum | ast::Kind::TypedefStruct) => TypeSelf::Thin,
            Some(ast::Kind::Protocol) => TypeSelf::Conformance,
            None => TypeSelf::NotStatic,
        }
    }

    /// Whether the result is exactly the protocol's `Self` (a requirement's or a protocol extension's): returned by
    /// address, with the type the call is made on.
    pub fn returns_self(&self) -> bool {
        let decl = &self.0;
        let result = match &decl.signature {
            ast::Signature::Function { result, .. } | ast::Signature::Getter(result) => result,
            ast::Signature::Subscript { result, setter: false, .. } => result,
            _ => return false,
        };
        decl.context.last().is_some_and(|(_, kind)| *kind == ast::Kind::Protocol)
            && fixed_self(decl).is_none()
            && *result == ast::Ty::Param(ast::GenericParam { depth: 0, index: 0 })
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

    /// What a member of a generic struct or enum is passed besides its arguments (see [`ValueContext`]).
    pub fn value_context(&self) -> ValueContext {
        let decl = &self.0;
        let value = matches!(decl.context.last(), Some((_, ast::Kind::Struct | ast::Kind::Enum)));
        // An extension that fixes every parameter (`where Success == Never, Failure == Never`) makes it concrete.
        let fixed: std::collections::HashSet<ast::GenericParam> = extension_requirements(decl)
            .filter_map(|r| match r {
                ast::Requirement::SameType(ast::Subject::Param(param), _) => Some(*param),
                _ => None,
            })
            .collect();
        let generic = decl.context_params > fixed.len();
        match (value && generic, decl.is_static || decl.name == "init") {
            (false, _) => ValueContext::None,
            (true, false) => ValueContext::SelfMetadata,
            (true, true) => ValueContext::GenericArguments,
        }
    }

    /// For an instance member of a generic struct or enum whose result is one of the type's own (outermost)
    /// parameters: its index among `Self`'s generic arguments, where the result's type is read at run time.
    pub fn result_from_self(&self) -> Option<usize> {
        let decl = &self.0;
        let result = match &decl.signature {
            ast::Signature::Function { result, .. } | ast::Signature::Getter(result) => result,
            ast::Signature::Subscript { result, setter: false, .. } => result,
            _ => return None,
        };
        match result {
            ast::Ty::Param(param) if param.depth == 0 && self.value_context() == ValueContext::SelfMetadata => {
                (!class_bound_params(decl).contains(param)).then_some(param.index)
            }
            _ => None,
        }
    }

    /// See [`check_callable`].
    pub fn check_callable(&self) -> Result<(), Error> {
        if let ast::Signature::Case(_) = &self.0.signature {
            return Err(Error("an enum case is not callable: `mangle!` gives its tag symbol, for `SwiftEnumCase`".into()));
        }
        // A conformance an extension adds (`where T: P`) is one more witness table after `Self`'s metadata.
        let added = extension_requirements(&self.0).any(|r| match r {
            ast::Requirement::Conforms(_, protocol) => !protocol.is_marker() && !self.0.context_requirements.contains(r),
            _ => false,
        });
        if added && self.value_context() == ValueContext::SelfMetadata {
            return Err(Error(
                "a member of a generic struct or enum in an extension that adds a conformance (`where T: P`) is not \
                 supported: it takes that conformance's witness table too"
                    .into(),
            ));
        }
        let value = matches!(self.0.context.last(), Some((_, ast::Kind::Struct | ast::Kind::Enum)));
        if value && self.0.module == "Swift" && !self.0.is_static && self.0.name != "init" {
            return Err(Error(
                "an instance member of a standard library struct or enum is not supported: the type is frozen, and Swift \
                 passes `self` in registers (an array as its one reference), not by address"
                    .into(),
            ));
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
            ty if is_error(ty) => Some(ArgumentKind::Error),
            ast::Ty::Existential(_) => Some(ArgumentKind::Existential),
            ast::Ty::Optional(wrapped) if matches!(**wrapped, ast::Ty::Existential(_)) => Some(ArgumentKind::Existential),
            ty => foreign_function(ty).map(|function| match function.convention {
                ast::Convention::C => ArgumentKind::CFunction(foreign_shape(function)),
                _ => ArgumentKind::Block(foreign_shape(function)),
            }),
        };
        Ok(match &decl.signature {
            ast::Signature::Function { params, .. } => {
                // `isolated (any Actor)?` is the call's isolation (`Isolation`); `any P...` an array.
                params.iter().map(|p| if p.inout || p.isolated || p.variadic { None } else { kind(&p.ty) }).collect()
            }
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
            ty if is_error(ty) => Some(ResultKind::Error),
            ty => foreign_function(ty).map(|function| match function.convention {
                ast::Convention::C => ResultKind::CFunction,
                _ => ResultKind::Block,
            }),
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
                ast::Ty::Function(function) if function.convention == ast::Convention::Swift => {
                    closure_shape(function).map(Some).map_err(Error)
                }
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

/// The dispatch thunk (`…Tj`) of a class's own member, which apus calls when the process has it: a library-evolution
/// module exports a non-final member of a class only as its thunk, which dispatches to the override as Swift would
/// (docs/ABI/Mangling.rst). A final member has none, and is called by its own symbol. `None` for anything else.
pub fn dispatch_thunk(declaration: &str) -> Result<Option<String>, Error> {
    let decl = parse::parse(declaration).map_err(Error)?;
    let symbol = mangler::mangle(&decl).map_err(Error)?;
    let member = decl.extension.is_none() && decl.context.last().is_some_and(|(_, kind)| *kind == ast::Kind::Class);
    Ok((member && decl.name != "init").then(|| format!("{symbol}Tj")))
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
                Ty::Pack(elements) => elements.iter().any(mentions_param),
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
            // A C function pointer or block: one pointer.
            ty if foreign_function(ty).is_some() => 'd',
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
    /// Per witness table, what the protocol's descriptor must say for the declared lowering to be Swift's.
    pub witness_layouts: Vec<ProtocolLayout>,
}

/// The requirements of the extension a declaration is in.
fn extension_requirements(decl: &ast::Decl) -> impl Iterator<Item = &ast::Requirement> {
    decl.extension.iter().flat_map(|e| e.requirements.iter())
}

/// What a member of a generic struct or enum takes besides its arguments (observed in what swiftc emits, as Apple's
/// frameworks are compiled; the calling-convention documents do not spell it out).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValueContext {
    /// Not a member of a generic value type.
    None,
    /// An instance member: the metadata of `Self` (the value's type), after the formal arguments and before the
    /// member's own generic arguments. apus takes it from the receiver.
    SelfMetadata,
    /// A static member or initializer: the type's generic arguments (each parameter's metadata, then witness tables),
    /// after the formal arguments, which the caller passes explicitly.
    GenericArguments,
}

/// Whether a required protocol must be class-bound: a parameter is passed as a reference exactly when Swift knows its
/// conforming types are classes, which for a protocol only its descriptor tells (checked before the call).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProtocolLayout {
    /// Marked `(AnyObject)`: the protocol must be class-bound.
    Class,
    /// The parameter goes by address: the protocol must not be class-bound.
    Any,
    /// The parameter is class-bound anyway (a superclass, `AnyObject`): either.
    Either,
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
    let mut witness_layouts = Vec::new();
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
                witness_layouts.push(match (protocol.class_bound, class_bound.contains(&param)) {
                    (true, _) => ProtocolLayout::Class,
                    (false, false) => ProtocolLayout::Any,
                    (false, true) => ProtocolLayout::Either,
                });
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
    Ok(Some(GenericPlan { formal_count: params.len(), metadata_from, witnesses, error_slot_from, result_from, type_functions, witness_layouts }))
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
        assert_eq!(of("static M.Shape(protocol).make() -> Self"), TypeSelf::Conformance);
        assert_eq!(of("M.Shape(protocol).init(n: Int)"), TypeSelf::Conformance);
        assert!(Declaration::parse("static M.Shape(protocol).make() -> Self").unwrap().returns_self());
        assert!(!Declaration::parse("M.Shape(protocol).area() -> Double").unwrap().returns_self());
        // `where Self == Concrete` (SwiftUI's `.bordered`): a member of that type, not generic over Self.
        let bordered = "static (extension in M where Self == Bordered(struct)) M.Style(protocol).bordered: Self { get }";
        assert_eq!(of(bordered), TypeSelf::Thin);
        assert!(!Declaration::parse(bordered).unwrap().returns_self());
        assert_eq!(super::result_type(bordered).unwrap().as_deref(), Some("1M8BorderedV"));
        assert_eq!(of("static (extension in M where Self == Klass(class)) M.Style(protocol).klass: Self { get }"), TypeSelf::Thick);
        assert_eq!(of("M.Tally(actor).init()"), TypeSelf::Thick);
        let name = |d: &str| Declaration::parse(d).unwrap().context_type_name();
        assert_eq!(name("M.Built(class).init(n: Int)").as_deref(), Some("1M5BuiltC"));
        assert_eq!(name("M.Crate(class)<T>.init(_: T)"), None, "a generic class has no one name");
        assert_eq!(of("M.Crate(class)<T>.init(_: T)"), TypeSelf::Thick);
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
    fn generic_value_types_pass_their_metadata() {
        use super::{Declaration, ValueContext};
        let context = |d: &str| Declaration::parse(d).unwrap().value_context();
        // An instance member of a generic struct or enum takes `Self`'s metadata after its formal arguments; a
        // static member or initializer takes the type's generic arguments, which the caller passes explicitly.
        for member in [
            "M.D(struct)<T: P>.v: Int { get }",
            "M.D(struct)<T: P>.v: Int { set }",
            "mutating M.D(struct)<T>.bump(_: Int)",
            "M.D(struct)<T>.subscript(_: Int) -> Int { get }",
            "M.K(enum)<T>.tag: Int { get }",
            "(extension in N) M.D(struct)<T>.f()",
        ] {
            assert_eq!(context(member), ValueContext::SelfMetadata, "{member}");
        }
        // A result of the type's own parameter: its metadata is among `Self`'s generic arguments.
        let result = |d: &str| Declaration::parse(d).unwrap().result_from_self();
        assert_eq!(result("M.D(struct)<K: Hashable, V>.subscript(_: Int) -> V { get }"), Some(1));
        assert_eq!(result("M.D(struct)<T>.first: T { get }"), Some(0));
        assert_eq!(result("M.D(struct)<T>.first: T? { get }"), None);
        assert_eq!(result("M.D(struct)<T: AnyObject>.first: T { get }"), None);
        assert_eq!(result("M.C(class)<T>.first: T { get }"), None);
        assert_eq!(context("static M.D(struct)<T: P>.make(_: Int) -> Int"), ValueContext::GenericArguments);
        // An extension that fixes every parameter makes the member concrete: nothing more is passed. One that fixes
        // some still takes `Self`; one that adds a conformance would take its witness table too: refused.
        let sleep = "static (extension in _Concurrency where Success == Never, Failure == Never) \
                     _Concurrency.Task<Success, Failure>.sleep(nanoseconds: UInt64) async throws";
        assert_eq!(context(sleep), ValueContext::None);
        assert_eq!(context("(extension in M where T == Int, U == String) M.D(struct)<T, U>.a: Int { get }"), ValueContext::None);
        assert_eq!(context("(extension in M where T == Int) M.D(struct)<T, U>.b: Int { get }"), ValueContext::SelfMetadata);
        let error = super::check_callable("(extension in M where T: P) M.D(struct)<T>.c: Int { get }").unwrap_err().to_string();
        assert!(error.contains("witness"), "{error}");
        assert!(super::check_callable("(extension in M where T: P) M.D(struct)<T: P>.c: Int { get }").is_ok());
        assert_eq!(context("M.D(struct)<T>.init(x: Int)"), ValueContext::GenericArguments);
        for plain in ["M.S(struct).v: Int { get }", "M.C(class)<T>.f()", "M.f<T>(_: T)", "M.P(protocol).f()"] {
            assert_eq!(context(plain), ValueContext::None, "{plain}");
        }
        // A standard-library struct or enum is frozen: Swift passes `self` in registers (an array as its one
        // reference), not by address.
        for frozen in ["Swift.Array<Element>.count: Int { get }", "Swift.Optional<Wrapped>.f()", "Swift.String.count: Int { get }"] {
            let error = super::check_callable(frozen).unwrap_err().to_string();
            assert!(error.contains("standard library"), "{frozen}: {error}");
        }
        assert!(super::check_callable("static Swift.Array<Element>.f() -> Int").is_ok());
    }

    #[test]
    fn class_members_have_dispatch_thunks() {
        use super::dispatch_thunk;
        // A class's own members may be dispatched (non-final members of a library-evolution class export only their
        // dispatch thunk); initializers, extension members, and other kinds' members are not.
        assert_eq!(
            dispatch_thunk("SwiftData.ModelContext(class).save() throws").unwrap().as_deref(),
            Some("$s9SwiftData12ModelContextC4saveyyKFTj")
        );
        assert!(dispatch_thunk("M.C(class).count: Int { get }").unwrap().is_some());
        for direct in ["M.C(class).init(x: Int)", "(extension in N) M.C(class).f()", "M.S(struct).f()", "M.f()", "M.A(actor).f()"] {
            assert_eq!(dispatch_thunk(direct).unwrap(), None, "{direct}");
        }
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
        // An existential parameter: passed in registers when its protocols are class-bound, which only its metadata
        // tells; a generic parameter's value is passed by address whatever its type.
        let existential = parsed("M.f<T>(_: any P, _: (any P)?, _: T, _: [any P])");
        assert_eq!(existential.argument_kinds().unwrap(), vec![Some(ArgumentKind::Existential), Some(ArgumentKind::Existential), None, None]);
        assert_eq!(parsed("M.f(_: isolated (any Actor)?, _: any P...)").argument_kinds().unwrap(), vec![None, None]);
        // `any Error` is one box reference: a `SwiftError`.
        let error = parsed("M.f(_: any Error, _: (any Error)?) -> (any Error)?");
        assert_eq!(error.argument_kinds().unwrap(), vec![Some(ArgumentKind::Error), Some(ArgumentKind::Error)]);
        assert_eq!(error.result_kind(), Some(ResultKind::Error));
        // A class-bound protocol (`protocol PersistentModel: AnyObject`) is marked `(AnyObject)`, which the
        // declaration cannot show otherwise; its symbol is the unmarked one's. The witness entry says what the
        // protocol's descriptor must say at run time.
        let insert = parsed("SwiftData.ModelContext(class).insert<T: PersistentModel(AnyObject)>(_: T)");
        assert_eq!(insert.argument_kinds().unwrap(), vec![Some(ArgumentKind::ClassBound)]);
        assert_eq!(
            super::mangle("SwiftData.ModelContext(class).insert<T: PersistentModel(AnyObject)>(_: T)").unwrap(),
            "$s9SwiftData12ModelContextC6insertyyxAA010PersistentC0RzlF"
        );
        let plan = insert.generic_plan().unwrap().unwrap();
        assert_eq!(plan.type_functions, vec!["__object_type_of"]);
        assert_eq!(plan.witness_layouts, vec![super::ProtocolLayout::Class]);
        let plain = parsed("M.f<T: Hashable>(_: T)").generic_plan().unwrap().unwrap();
        assert_eq!(plain.witness_layouts, vec![super::ProtocolLayout::Any]);
        let based = parsed("M.f<T: Base(class) & Hashable>(_: T)").generic_plan().unwrap().unwrap();
        assert_eq!(based.witness_layouts, vec![super::ProtocolLayout::Either]);
        let error = super::check_callable("M.f(_: any P(AnyObject))").unwrap_err().to_string();
        assert!(error.contains("AnyObject"), "{error}");
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
    fn sending_parameters_are_consumed() {
        use super::Declaration;
        let parsed = Declaration::parse("M.f(_: sending Base(class), _: Int) -> sending Base(class)").unwrap();
        assert_eq!(parsed.consumed(), vec![true, false]);
        let error = super::mangle("M.f(_: ((sending Base(class)) -> Void))").unwrap_err().to_string();
        assert!(error.contains("sending"), "{error}");
    }

    #[test]
    fn c_functions_and_blocks_are_single_pointers_of_a_checked_shape() {
        use super::{ArgumentKind, Declaration, ResultKind};
        let parsed = |d: &str| Declaration::parse(d).unwrap();
        let f = parsed("M.f(_: @convention(c) (Int32, Double) -> Int, _: (@convention(block) (String) -> Bool)?, _: () -> Int)");
        assert_eq!(
            f.argument_kinds().unwrap(),
            vec![Some(ArgumentKind::CFunction("(id)q".into())), Some(ArgumentKind::Block("(?)B".into())), None]
        );
        assert_eq!(f.closure_shapes().unwrap(), vec![None, None, Some("s()d".into())]);
        assert_eq!(parsed("M.f() -> @convention(c) () -> Void").result_kind(), Some(ResultKind::CFunction));
        assert_eq!(parsed("M.f() -> (@convention(block) () -> Void)?").result_kind(), Some(ResultKind::Block));
        // An autoclosure is a Swift closure.
        assert_eq!(parsed("M.f(_: @autoclosure () -> Int)").closure_shapes().unwrap(), vec![Some("s()d".into())]);
        // Passed one pointer as themselves or optional; deeper, apus would not lower them.
        for declaration in ["M.f(_: (Int, @convention(c) () -> Void))", "M.f() -> (@convention(c) () -> Void)??"] {
            let error = super::check_callable(declaration).unwrap_err().to_string();
            assert!(error.contains("C function or block"), "{declaration}: {error}");
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
