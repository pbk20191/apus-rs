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

/// A type written as in a declaration, parsed (bare names belong to the leading module).
fn parse_type(ty: &str) -> Result<ast::Ty, Error> {
    let declaration = format!("{}.__apus_type() -> {ty}", leading_module(ty));
    match parse::parse(&declaration).map_err(|e| Error(format!("`{ty}`: {e}")))?.signature {
        ast::Signature::Function { result, .. } => Ok(result),
        _ => unreachable!("a function declaration"),
    }
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

/// A typed wrapper's Swift type (`swift! { class Todo = "Todos.Todo(class)"; }`): its mangled name and its qualified
/// name without generic arguments (`Todos.Todo`). `kind` is `class`, `struct` or `enum`, which the type must be; it
/// must be concrete.
pub fn wrapper_type(kind: &str, ty: &str) -> Result<(String, String), Error> {
    let parsed = parse_type(ty)?;
    let Some(nominal) = nominal_ref(&parsed).filter(|n| !n.optional && !n.array) else {
        return Err(Error(format!("`{ty}` is not a class, struct or enum")));
    };
    let ast::Ty::Nominal { path, .. } = &parsed else { unreachable!("a nominal type") };
    let matches = match path.last().map(|(_, k)| *k) {
        Some(ast::Kind::Class | ast::Kind::Actor | ast::Kind::TypedefClass) => kind == "class",
        Some(ast::Kind::Struct | ast::Kind::TypedefStruct) => kind == "struct",
        Some(ast::Kind::Enum) => kind == "enum",
        _ => false,
    };
    if !matches {
        return Err(Error(format!("`{ty}` is not a {kind}")));
    }
    let mangled = nominal.mangled.ok_or_else(|| Error(format!("`{ty}`: a wrapper's type must be concrete")))?;
    Ok((mangled, nominal.name))
}

/// A class, struct or enum a declaration names outside the standard library, as a typed wrapper may stand for it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NominalRef {
    /// Its mangled name when it is concrete.
    pub mangled: Option<String>,
    /// Its qualified name without generic arguments (`SwiftData.FetchDescriptor`).
    pub name: String,
    /// Written `T?`.
    pub optional: bool,
    /// Written `[T]`.
    pub array: bool,
    /// A class (or actor).
    pub class: bool,
}

fn nominal_ref(ty: &ast::Ty) -> Option<NominalRef> {
    match ty {
        ast::Ty::Optional(wrapped) => nominal_ref(wrapped).filter(|n| !n.optional && !n.array).map(|n| NominalRef { optional: true, ..n }),
        ast::Ty::Nominal { module, path, args, .. } if module == "Swift" => match (path.as_slice(), args.as_slice()) {
            ([(name, _)], [element]) if name == "Array" => {
                nominal_ref(element).filter(|n| !n.optional && !n.array).map(|n| NominalRef { array: true, ..n })
            }
            _ => None,
        },
        // A C value is no wrapper's: it goes as its C layout (`CValue`).
        ty if c_value(ty).is_some() => None,
        ast::Ty::Nominal { module, path, .. } => Some(NominalRef {
            mangled: mangler::is_concrete_type(ty).then(|| mangler::type_name(ty).ok()).flatten(),
            name: std::iter::once(module.as_str()).chain(path.iter().map(|(name, _)| name.as_str())).collect::<Vec<_>>().join("."),
            optional: false,
            array: false,
            class: matches!(path.last(), Some((_, ast::Kind::Class | ast::Kind::Actor | ast::Kind::TypedefClass))),
        }),
        _ => None,
    }
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

/// A type's kind, which a declaration writes after its name (`(class)`) unless a [`KindResolver`] knows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TypeKind {
    Struct,
    Class,
    Actor,
    Enum,
    Protocol,
}

/// What a declaration leaves out and a symbol graph knows: the kind of a type named without one, and whether a
/// protocol is class-bound (`protocol P: AnyObject`, written `P(AnyObject)`). `None` where it does not know.
pub trait KindResolver: Sync {
    /// The kind of the type at `path` (`["FetchDescriptor"]`, outermost first) in `module`.
    fn kind(&self, module: &str, path: &[&str]) -> Option<TypeKind>;
    fn class_bound(&self, module: &str, protocol: &str) -> Option<bool>;
    /// The module a top-level type of `module` was defined in before it moved (`@_originallyDefinedIn`), which its
    /// mangled name keeps: CoreFoundation's `CGFloat` is mangled as CoreGraphics'.
    fn original_module(&self, _module: &str, _name: &str) -> Option<String> {
        None
    }
    /// Whether the top-level enum `name` of `module` is `@objc` (`POSIXErrorCode`): its value is its raw integer.
    fn objc_enum(&self, _module: &str, _name: &str) -> bool {
        false
    }
}

/// See [`KindResolver::original_module`].
pub(crate) fn original_module(module: &str, name: &str) -> Option<String> {
    kind_resolver()?.original_module(module, name)
}

static KIND_RESOLVER: std::sync::OnceLock<&'static dyn KindResolver> = std::sync::OnceLock::new();

/// Resolves kinds left out in every declaration parsed from now on in this process (`swift!`'s proc macros set
/// their symbol graphs'). Only the first call takes effect.
pub fn set_kind_resolver(resolver: &'static dyn KindResolver) {
    let _ = KIND_RESOLVER.set(resolver);
}

fn kind_resolver() -> Option<&'static dyn KindResolver> {
    KIND_RESOLVER.get().copied()
}

/// How a call checks an instance member's receiver against the declaration's type, by its runtime type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReceiverCheck {
    /// A class's object: an instance of the class named (mangled), or of a subclass.
    Instance(String),
    /// A struct's or enum's value: of exactly the type named (mangled).
    Exactly(String),
    /// A generic struct's or enum's value: its type's qualified name, generic arguments left out, is this one
    /// (`SwiftData.FetchResultsCollection`).
    Named(String),
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
    /// A value of a type imported from C (see [`Declaration::imported_values`]); `true` for an optional one.
    CValue(bool),
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
    /// A value of a type imported from C or of an `@objc` enum; `true` if optional.
    CValue(bool),
}

/// A value of a type imported from C (`__C.CGRect(struct)`, `__C.NSURLResourceKey(typedef struct)`) or of an `@objc`
/// enum (`Darwin.POSIXErrorCode(enum)`, a C enum to Objective-C, its value its raw integer: owner-accepted, observed):
/// `Some(true)` for an optional one. Swift passes it by swiftcall's lowering of its C layout.
fn c_value(ty: &ast::Ty) -> Option<bool> {
    let (ty, optional) = match ty {
        ast::Ty::Optional(wrapped) => (&**wrapped, true),
        ty => (ty, false),
    };
    match ty {
        ast::Ty::Nominal { module, path, .. }
            if module == "__C" && matches!(path.last(), Some((_, ast::Kind::Struct | ast::Kind::Enum | ast::Kind::TypedefStruct))) =>
        {
            Some(optional)
        }
        ast::Ty::Nominal { module, path, .. }
            if matches!(path.as_slice(), [(name, ast::Kind::Enum)] if kind_resolver().is_some_and(|r| r.objc_enum(module, name))) =>
        {
            Some(optional)
        }
        _ => None,
    }
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

    /// Like [`parse`](Self::parse), with names written without a kind resolved by `resolver`.
    pub fn parse_resolving(declaration: &str, resolver: &dyn KindResolver) -> Result<Self, Error> {
        parse::parse_resolving(declaration, Some(resolver)).map(Declaration).map_err(Error)
    }

    /// The declaration's own symbol (`$s…`), as [`mangle`] gives it.
    pub fn symbol(&self) -> Result<String, Error> {
        mangler::mangle(&self.0).map_err(Error)
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

    /// The names of the generic parameters [`bind`](Self::bind) binds: the enclosing type's (but those an extension
    /// fixes) and the declaration's own. None for a protocol's member, whose `Self` is not bound.
    pub fn generic_parameter_names(&self) -> Vec<String> {
        self.bindable().into_iter().map(|(_, name)| name.to_string()).collect()
    }

    fn bindable(&self) -> Vec<(ast::GenericParam, &str)> {
        use ast::GenericParam;
        let decl = &self.0;
        if decl.context.last().is_some_and(|(_, kind)| *kind == ast::Kind::Protocol) {
            return Vec::new();
        }
        // An extension or the member's `where` that fixes a parameter (`where T == Int`) leaves nothing to bind for it.
        let fixed = fixed_params(decl);
        let own_depth = usize::from(decl.context_params > 0);
        (decl.context_names.iter().enumerate())
            .map(|(index, name)| (GenericParam { depth: 0, index }, name.as_str()))
            .filter(|(param, _)| !fixed.contains(param))
            .chain(decl.generic_names.iter().enumerate().map(|(index, name)| (GenericParam { depth: own_depth, index }, name.as_str())))
            .collect()
    }

    /// Binds every generic parameter of the declaration (its enclosing type's and its own), by name, to concrete
    /// types written as in a declaration (`[("T", "Todos.Todo(class)")]`). The function stays generic: its values of
    /// type `T` go by address, and its generic arguments are the bound types' metadata and witness tables, found
    /// when the call is made, as are the requirements the call checks (a superclass, an associated type).
    pub fn bind(&self, bindings: &[(&str, &str)]) -> Result<ConcretePlan, Error> {
        use ast::{GenericParam, Requirement, Subject, Ty};
        let decl = &self.0;
        let (params, result, typed_error): (&[ast::Param], &Ty, &Option<Ty>) = match &decl.signature {
            ast::Signature::Function { params, result, typed_error, .. } => (params, result, typed_error),
            ast::Signature::Getter(ty) | ast::Signature::Subscript { result: ty, setter: false, .. } => {
                let params = match &decl.signature {
                    ast::Signature::Subscript { params, .. } => params.as_slice(),
                    _ => &[],
                };
                (params, ty, &None)
            }
            _ => return Err(Error("only a function's, getter's or subscript getter's generic parameters can be bound".into())),
        };
        let protocol_member = decl.context.last().is_some_and(|(_, kind)| *kind == ast::Kind::Protocol);
        if protocol_member {
            return Err(Error("a protocol member's generic parameters cannot be bound: call it on an `Opened` value".into()));
        }
        // An extension or the member's `where` that fixes a parameter (`where T == Int`) leaves nothing to bind for it.
        let fixed = fixed_params(decl);
        let own_depth = usize::from(decl.context_params > 0);
        let named = self.bindable();
        if named.is_empty() {
            return Err(Error("the declaration is not generic: it takes no bound types".into()));
        }
        for (i, (_, name)) in named.iter().enumerate() {
            if named[..i].iter().any(|(_, other)| other == name) && *name != "_" {
                return Err(Error(format!("`{name}` names two generic parameters (the type's and the member's): rename one to bind it")));
            }
        }
        for (name, _) in bindings {
            if !named.iter().any(|(_, n)| n == name) {
                return Err(Error(format!("the declaration has no generic parameter `{name}`")));
            }
        }
        let class_bound = class_bound_params(decl);
        let mut bound: Vec<(GenericParam, Ty)> = Vec::new();
        for (param, name) in &named {
            // An opaque parameter (`_: some P`) is an unnamed generic parameter, which no binding can name.
            if *name == "_" {
                return Err(Error("an opaque `some` parameter cannot be bound: declare it as a named generic parameter".into()));
            }
            let Some((_, ty)) = bindings.iter().find(|(n, _)| n == name) else {
                return Err(Error(format!("`{name}` is not bound: write `{name} = \"Module.Type(kind)\"`")));
            };
            let parsed = parse_type(ty)?;
            if !mangler::is_concrete_type(&parsed) {
                return Err(Error(format!("`{ty}`: a bound type must be concrete")));
            }
            let class = matches!(&parsed, Ty::Nominal { path, .. }
                if matches!(path.last(), Some((_, ast::Kind::Class | ast::Kind::Actor | ast::Kind::TypedefClass))));
            if class_bound.contains(param) && !class {
                return Err(Error(format!("`{name}` is class-bound, and `{ty}` is not a class")));
            }
            bound.push((*param, parsed));
        }
        let name_of = |ty: &Ty| -> Result<String, Error> {
            match mangler::bound_type_name(decl, ty, &bound).map_err(Error)? {
                Some(name) => Ok(name),
                None => Err(Error(format!("`{ty:?}` is not concrete with the bindings"))),
            }
        };
        let bound_name = |param: GenericParam| name_of(&Ty::Param(param));
        // What the call passes: a value type's instance member gets its type's arguments from `Self`'s metadata, a
        // class's members from `self`; a value type's static member or initializer takes them all.
        // A parameter fixed to a type is no generic argument: a static member or initializer takes the others only.
        let value_static = self.value_context() == ValueContext::GenericArguments;
        let passed = |param: &GenericParam| value_static || param.depth == own_depth;
        let types = bound.iter().filter(|(param, _)| passed(param)).map(|(param, _)| bound_name(*param)).collect::<Result<Vec<_>, _>>()?;
        let requirements: Vec<&Requirement> = decl
            .context_requirements
            .iter()
            .chain(decl.context_extension.iter().flat_map(|e| e.requirements.iter()))
            .chain(extension_requirements(decl))
            .chain(&decl.requirements)
            .collect();
        let subject_param = |subject: &Subject| match subject {
            Subject::Param(param) | Subject::Member(param, _) => *param,
        };
        // The protocol declaring an associated type: the one written (`T.Value(of P)`), or the parameter's one.
        let associated = |param: GenericParam, member: &str| -> Result<BoundType, Error> {
            let protocol = match decl.associated_protocols.iter().find(|((p, n), _)| *p == param && n == member) {
                Some((_, protocol)) => protocol.clone(),
                None => {
                    let mut protocols = requirements.iter().filter_map(|r| match r {
                        Requirement::Conforms(Subject::Param(p), protocol) if *p == param && !protocol.is_marker() => Some(protocol),
                        _ => None,
                    });
                    match (protocols.next(), protocols.next()) {
                        (Some(protocol), None) => protocol.clone(),
                        _ => return Err(Error(format!("`{member}`: name the protocol that declares it, `.{member}(of P)`"))),
                    }
                }
            };
            let (base, descriptor) = mangler::associated_type_symbols(&protocol, member).map_err(Error)?;
            Ok(BoundType::Associated { ty: bound_name(param)?, protocol: mangler::protocol_descriptor(&protocol).map_err(Error)?, base, descriptor })
        };
        let mut witnesses = Vec::new();
        let mut superclasses = Vec::new();
        let mut same_types = Vec::new();
        let owned: Vec<Requirement> = requirements.iter().map(|r| (*r).clone()).collect();
        for requirement in mangler::sorted_requirements(&owned) {
            // What `self` does not give: the member's own, and a conformance added to its type's parameters.
            let takes_witness = self.added(&requirement)
                || passed(&subject_param(match &requirement {
                    Requirement::Conforms(subject, _) | Requirement::SameType(subject, _) | Requirement::Superclass(subject, _) => subject,
                    Requirement::AnyObject(subject) => subject,
                }));
            match requirement {
                Requirement::Conforms(_, ref protocol) if protocol.is_marker() => {}
                Requirement::Conforms(subject, protocol) => {
                    if protocol.module == "__C" {
                        return Err(Error("an Objective-C protocol requirement takes no witness table: not supported".into()));
                    }
                    if !takes_witness {
                        continue;
                    }
                    let (on, layout) = match subject {
                        Subject::Param(param) => (
                            BoundType::Named(bound_name(param)?),
                            match (protocol.class_bound, class_bound.contains(&param)) {
                                (true, _) => ProtocolLayout::Class,
                                (false, false) => ProtocolLayout::Any,
                                (false, true) => ProtocolLayout::Either,
                            },
                        ),
                        Subject::Member(param, member) => (associated(param, &member)?, ProtocolLayout::Either),
                    };
                    witnesses.push((on, mangler::protocol_descriptor(&protocol).map_err(Error)?, layout));
                }
                Requirement::SameType(Subject::Param(param), _) if fixed.contains(&param) => {}
                Requirement::SameType(Subject::Param(_), _) => {
                    return Err(Error("a generic parameter fixed to another type cannot be bound: bind the type it is fixed to".into()))
                }
                Requirement::SameType(Subject::Member(param, member), ty) => {
                    let expected = match &ty {
                        Ty::Member(..) => return Err(Error("an associated type the same as another associated type cannot be bound".into())),
                        ty => name_of(ty)?,
                    };
                    same_types.push((associated(param, &member)?, expected));
                }
                Requirement::Superclass(Subject::Param(param), class) => superclasses.push((bound_name(param)?, name_of(&class)?)),
                Requirement::Superclass(Subject::Member(..), _) | Requirement::AnyObject(Subject::Member(..)) => {
                    return Err(Error("a class requirement on an associated type cannot be bound".into()))
                }
                // Checked against the binding above: a class.
                Requirement::AnyObject(Subject::Param(_)) => {}
            }
        }
        let error_type = match typed_error {
            None => None,
            Some(Ty::Param(param)) => Some(bound_name(*param)?),
            Some(Ty::Member(..)) => return Err(Error("typed throws of an associated type cannot be bound".into())),
            Some(_) => return Err(Error(CONCRETE_TYPED_THROWS.into())),
        };
        // Where Swift abstracts a generic parameter, the bound type's own lowering is not the generic one: only a
        // parameter or result that is `T` itself (by address, or a reference if class-bound) or names `T` among a
        // nominal type's arguments (whose layout does not depend on it) keeps to what apus lowers.
        fn placed(ty: &Ty, top: bool) -> Result<(), Error> {
            match ty {
                Ty::Param(_) if top => Ok(()),
                Ty::Param(_) | Ty::Member(..) => {
                    Err(Error("a bound generic parameter is supported only as a parameter or result itself, or among a type's generic arguments".into()))
                }
                Ty::Nominal { args, .. } => args.iter().try_for_each(|arg| nominal_argument(arg)),
                Ty::Optional(wrapped) => placed(wrapped, false),
                Ty::Metatype(instance) => match &**instance {
                    Ty::Param(_) if top => Ok(()),
                    other => placed(other, false),
                },
                Ty::Pack(elements) => elements.iter().try_for_each(|e| placed(e, false)),
                Ty::Tuple(elements) => elements.iter().try_for_each(|(_, e)| placed(e, false)),
                Ty::Function(function) => function.params.iter().chain([&function.result]).try_for_each(|t| placed(t, false)),
                Ty::Any | Ty::AnyObject | Ty::Existential(_) | Ty::Opaque(_) => Ok(()),
            }
        }
        fn nominal_argument(ty: &Ty) -> Result<(), Error> {
            match ty {
                Ty::Param(_) => Ok(()),
                other => placed(other, false),
            }
        }
        params.iter().try_for_each(|p| placed(&p.ty, true))?;
        placed(result, true)?;
        let abstract_value = |ty: &Ty| matches!(ty, Ty::Param(param) if !class_bound.contains(param));
        let self_type = match decl.context_params {
            0 => None,
            _ => Some(name_of(&decl.context_type())?),
        };
        let receiver = match (&self_type, decl.is_static || decl.name == "init", decl.context.last()) {
            (Some(ty), false, Some((_, ast::Kind::Class | ast::Kind::Actor | ast::Kind::TypedefClass))) => Some(ReceiverCheck::Instance(ty.clone())),
            (Some(ty), false, Some(_)) => Some(ReceiverCheck::Exactly(ty.clone())),
            _ => self.receiver_check(),
        };
        Ok(ConcretePlan {
            receiver,
            types,
            witnesses,
            superclasses,
            same_types,
            error_type,
            self_type,
            result: mangler::bound_type_name(decl, result, &bound).map_err(Error)?,
            // A variadic `T...` is passed as one `[T]`, an `inout T` by address as `&mut SwiftValue`.
            by_address: params.iter().map(|p| abstract_value(&p.ty) && !p.variadic && !p.inout).collect(),
            result_by_address: abstract_value(result),
            argument_types: params
                .iter()
                .map(|p| match mangler::is_concrete_type(&p.ty) {
                    true => Ok(None),
                    false => mangler::bound_type_name(decl, &p.ty, &bound).map_err(Error),
                })
                .collect::<Result<_, _>>()?,
            nominal_params: params.iter().map(|p| nominal_ref(&mangler::bound_type(decl, &p.ty, &bound))).collect(),
            nominal_result: nominal_ref(&mangler::bound_type(decl, result, &bound)),
        })
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

    /// How a call checks that an instance member's receiver has the declaration's type (see [`ReceiverCheck`]);
    /// `None` for a free function, a static member or initializer, a protocol's member, or a generic class's.
    pub fn receiver_check(&self) -> Option<ReceiverCheck> {
        let decl = &self.0;
        if decl.is_static || decl.name == "init" {
            return None;
        }
        let kind = match fixed_self(decl) {
            Some(ast::Ty::Nominal { path, .. }) => path.last().map(|(_, kind)| *kind)?,
            Some(_) => return None,
            None => decl.context.last().map(|(_, kind)| *kind)?,
        };
        let concrete = self.context_type_name();
        match kind {
            ast::Kind::Class | ast::Kind::Actor | ast::Kind::TypedefClass => concrete.map(ReceiverCheck::Instance),
            ast::Kind::Struct | ast::Kind::Enum | ast::Kind::TypedefStruct => Some(match concrete {
                Some(name) => ReceiverCheck::Exactly(name),
                None => ReceiverCheck::Named(
                    std::iter::once(decl.module.as_str()).chain(decl.context.iter().map(|(name, _)| name.as_str())).collect::<Vec<_>>().join("."),
                ),
            }),
            ast::Kind::Protocol => None,
        }
    }

    /// The class, struct or enum each formal parameter is, where it is one outside the standard library (see
    /// [`NominalRef`]).
    pub fn nominal_params(&self) -> Vec<Option<NominalRef>> {
        match &self.0.signature {
            ast::Signature::Function { params, .. } | ast::Signature::Subscript { params, .. } => params.iter().map(|p| nominal_ref(&p.ty)).collect(),
            ast::Signature::Setter(ty) => vec![nominal_ref(ty)],
            _ => Vec::new(),
        }
    }

    /// The class, struct or enum the result is, where it is one outside the standard library (see [`NominalRef`]).
    pub fn nominal_result(&self) -> Option<NominalRef> {
        match &self.0.signature {
            ast::Signature::Function { result, .. } | ast::Signature::Getter(result) => nominal_ref(result),
            ast::Signature::Subscript { result, setter: false, .. } => nominal_ref(result),
            _ => None,
        }
    }

    /// The modules whose binaries may define the declaration's symbol: its own, and the modules of the extensions it is
    /// declared in (`(extension in _Concurrency) Swift.Task…` is in `_Concurrency`).
    pub fn binary_modules(&self) -> Vec<String> {
        let decl = &self.0;
        let mut modules = vec![decl.module.clone()];
        let extensions = decl.extension.iter().map(|e| &e.module).chain(decl.context_extension.iter().map(|e| &e.module));
        for module in extensions {
            if !modules.contains(module) {
                modules.push(module.clone());
            }
        }
        modules
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

    /// Per formal parameter, and for the result, whether its type is a value imported from C (`__C.CGPoint(struct)`,
    /// `__C.NSURLResourceKey(typedef struct)`, or an optional of one): passed by swiftcall's lowering of its C layout.
    pub fn imported_values(&self) -> (Vec<bool>, bool) {
        let imported = |ty: &ast::Ty| c_value(ty).is_some();
        match &self.0.signature {
            ast::Signature::Function { params, result, .. } => (params.iter().map(|p| imported(&p.ty)).collect(), imported(result)),
            ast::Signature::Getter(ty) => (Vec::new(), imported(ty)),
            ast::Signature::Setter(ty) => (vec![imported(ty)], false),
            ast::Signature::Subscript { params, result, setter } => {
                let indices = params.iter().map(|p| imported(&p.ty));
                match setter {
                    true => (std::iter::once(imported(result)).chain(indices).collect(), false),
                    false => (indices.collect(), imported(result)),
                }
            }
            ast::Signature::Case(_) => (Vec::new(), false),
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

    /// What a member of a generic struct or enum is passed besides its arguments (see [`ValueContext`]).
    pub fn value_context(&self) -> ValueContext {
        let decl = &self.0;
        let value = matches!(decl.context.last(), Some((_, ast::Kind::Struct | ast::Kind::Enum)));
        // An extension or a `where` that fixes every parameter (`where Success == Never, Failure == Never`) makes it
        // concrete: not even `Self`'s metadata is passed.
        let fixed: std::collections::HashSet<ast::GenericParam> = fixing_requirements(decl)
            .filter_map(|r| match r {
                ast::Requirement::SameType(ast::Subject::Param(param), _) if param.depth == 0 => Some(*param),
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
        self.check(false)
    }

    /// [`check_callable`](Self::check_callable) for a declaration whose generic parameters are all bound
    /// ([`bind`](Self::bind)): a conformance added to its type's parameters is then passed too.
    pub fn check_bound_callable(&self) -> Result<(), Error> {
        self.check(true)
    }

    /// A conformance an extension or the member itself adds to its type's parameters (`where T: P`), which `self`
    /// does not give: one more witness table, in canonical order among the member's own generic arguments (observed,
    /// accepted 2026-11-21: `(Box<T> metadata, U, T.P, U.Q, self)`, a class's `(U, T.P, U.Q, self)`).
    fn added(&self, requirement: &ast::Requirement) -> bool {
        use ast::{Requirement, Subject};
        let decl = &self.0;
        match requirement {
            Requirement::Conforms(Subject::Param(param) | Subject::Member(param, _), protocol) => {
                param.depth == 0
                    && !protocol.is_marker()
                    && !decl.context_requirements.contains(requirement)
                    && !decl.context_extension.iter().any(|e| e.requirements.contains(requirement))
            }
            _ => false,
        }
    }

    fn adds_conformance(&self) -> bool {
        let decl = &self.0;
        let member = decl.requirements.iter().filter(|_| decl.context_params > 0);
        extension_requirements(decl).chain(member).any(|r| self.added(r))
    }

    fn check(&self, bound: bool) -> Result<(), Error> {
        if let ast::Signature::Case(_) = &self.0.signature {
            return Err(Error("an enum case is not callable: `mangle!` gives its tag symbol, for `SwiftEnumCase`".into()));
        }
        // An added conformance's witness table is found from the bound type; a struct's or enum's static member or
        // initializer takes its whole signature's generic arguments, the added conformance's among them. A protocol's
        // `Self` is not bound.
        let value_static = self.value_context() == ValueContext::GenericArguments;
        let protocol = self.0.context.last().is_some_and(|(_, kind)| *kind == ast::Kind::Protocol);
        if self.adds_conformance() && !value_static && (!bound || protocol) {
            return Err(Error(
                if protocol {
                    "a protocol extension's member that adds a conformance (`where Self: P`) is not supported: it takes that conformance's witness table too"
                } else {
                    "a member of a generic type that adds a conformance (`where T: P`) takes that conformance's witness table too: bind its generic parameters (`T = \"…\"`) to call it"
                }
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
            ty if c_value(ty).is_some() => c_value(ty).map(ArgumentKind::CValue),
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
            ty if c_value(ty).is_some() => c_value(ty).map(ResultKind::CValue),
            ty => foreign_function(ty).map(|function| match function.convention {
                ast::Convention::C => ResultKind::CFunction,
                _ => ResultKind::Block,
            }),
        }
    }

    /// Whether Swift returns the result in registers whatever module calls it, so that it cannot be read by address
    /// (`SwiftValue`): a standard library `Array`, `Dictionary`, `Set` or `String` (frozen: one or two words), a class
    /// instance, or an `Optional` of one of these.
    pub fn result_in_registers(&self) -> bool {
        fn direct(ty: &ast::Ty) -> bool {
            match ty {
                ast::Ty::Optional(wrapped) => direct(wrapped),
                ast::Ty::AnyObject => true,
                ast::Ty::Nominal { module, path, .. } => match path.last() {
                    Some((_, ast::Kind::Class | ast::Kind::Actor)) => true,
                    Some((name, _)) => module == "Swift" && path.len() == 1 && ["Array", "Dictionary", "Set", "String"].contains(&name.as_str()),
                    None => false,
                },
                _ => false,
            }
        }
        match &self.0.signature {
            ast::Signature::Function { result: ty, .. } | ast::Signature::Getter(ty) => direct(ty),
            ast::Signature::Subscript { result, setter: false, .. } => direct(result),
            _ => false,
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

/// How a class's own member dispatches to an override: its dispatch thunk (`…Tj`) where the module has one (library
/// evolution), else the class's vtable, through `swift_lookUpClassMethod` with the member's method descriptor
/// (`…Tq`) and the declaring class's nominal type descriptor (`…Mn`, the unbound one for a generic class)
/// (docs/ABI/Mangling.rst; include/swift/Runtime/Metadata.h). A final member has neither, and is called by its own
/// symbol.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClassMethod {
    pub thunk: String,
    pub method: String,
    pub class: String,
    /// A static (`class`) member: `self` is the class's metadata, not an instance.
    pub class_side: bool,
}

/// See [`ClassMethod`]; `None` for what is not a class's own member, as for [`dispatch_thunk`].
pub fn class_method(declaration: &str) -> Result<Option<ClassMethod>, Error> {
    let decl = parse::parse(declaration).map_err(Error)?;
    let Some(thunk) = dispatch_thunk(declaration)? else { return Ok(None) };
    let class = ast::Ty::Nominal {
        module: decl.module.clone(),
        path: decl.context.clone(),
        args: Vec::new(),
        split: Vec::new(),
        extension: decl.context_extension.clone().map(Box::new),
    };
    let class = mangler::type_name(&class).map_err(Error)?;
    let method = format!("{}Tq", thunk.strip_suffix("Tj").unwrap_or(&thunk));
    Ok(Some(ClassMethod { thunk, method, class: format!("$s{class}Mn"), class_side: decl.is_static }))
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

/// A generic declaration's parameters fixed to concrete types (`swift!`'s `#[swift = "…", T = "Type"]`): what
/// [`Declaration::bind`] computes, all known at compile time but the lookups themselves.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConcretePlan {
    /// The type metadata the call passes, in order: the mangled name (no `$s`) of each bound type.
    pub types: Vec<String>,
    /// Witness tables in canonical order: (the conforming type, protocol descriptor, layout).
    pub witnesses: Vec<(BoundType, String, ProtocolLayout)>,
    /// Superclass requirements, checked when the call is made: (bound type, the class it must be or inherit from).
    pub superclasses: Vec<(String, String)>,
    /// Same-type requirements on associated types, checked when the call is made: (associated type, the type it must be).
    pub same_types: Vec<(BoundType, String)>,
    /// Typed throws of a generic error type: the type bound to it, whose error slot the call passes.
    pub error_type: Option<String>,
    /// A member of a generic type: that type with its parameters bound, which a class's static member or
    /// initializer passes as `self`.
    pub self_type: Option<String>,
    /// How an instance member's receiver is checked: against the bound type where the declaration's is generic.
    pub receiver: Option<ReceiverCheck>,
    /// The result type with the bindings substituted, when it is concrete then.
    pub result: Option<String>,
    /// Per formal parameter: its type is exactly a generic parameter that is not class-bound, which a generic function
    /// takes by address whatever the type bound to it (the Rust argument must be a `&SwiftValue`).
    pub by_address: Vec<bool>,
    /// The result is exactly such a parameter, returned by address (`SwiftValue`).
    pub result_by_address: bool,
    /// Per formal parameter whose type names a bound parameter: that type with the bindings substituted, which the
    /// argument's own type must be (with automatic generic arguments the argument gives the type; here it does not).
    pub argument_types: Vec<Option<String>>,
    /// [`Declaration::nominal_params`] with the bindings substituted.
    pub nominal_params: Vec<Option<NominalRef>>,
    /// [`Declaration::nominal_result`] with the bindings substituted.
    pub nominal_result: Option<NominalRef>,
}

/// A type a bound declaration's generic argument or check is about.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BoundType {
    /// A bound type, by its mangled name.
    Named(String),
    /// An associated type of the bound type `ty`, found through its conformance to `protocol` (a descriptor symbol)
    /// by the protocol's requirement base (`…TL`) and the associated type's descriptor (`…Tl`).
    Associated { ty: String, protocol: String, base: String, descriptor: String },
}

/// The requirements of the extension a declaration is in.
fn extension_requirements(decl: &ast::Decl) -> impl Iterator<Item = &ast::Requirement> {
    decl.extension.iter().flat_map(|e| e.requirements.iter())
}

/// The requirements that can fix the type's parameters: its own extension's, that of the extension an enclosing
/// type is declared in (`extension Holder where T == Int { struct Fixed }`), and, for a member of a generic type,
/// its own `where`'s.
fn fixing_requirements(decl: &ast::Decl) -> impl Iterator<Item = &ast::Requirement> {
    let member = decl.requirements.iter().filter(|_| decl.context_params > 0);
    extension_requirements(decl).chain(decl.context_extension.iter().flat_map(|e| e.requirements.iter())).chain(member)
}

/// The type's parameters an extension or the member's own `where` fixes to a type (`where T == Int`, `B == [A]`):
/// they drop out of the generic signature, so no generic argument is passed for them (observed, accepted 2026-11-22:
/// `Two<A, B>.init(b:) where A == Int` takes `B`'s metadata only).
fn fixed_params(decl: &ast::Decl) -> Vec<ast::GenericParam> {
    use ast::{Requirement, Subject, Ty};
    fixing_requirements(decl)
        .filter_map(|r| match r {
            Requirement::SameType(Subject::Param(param), ty) if param.depth == 0 && !matches!(ty, Ty::Param(_) | Ty::Member(..)) => Some(*param),
            _ => None,
        })
        .collect()
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

/// The witness tables a generic function's own parameters take, in the ABI's canonical requirement order: (generic
/// parameter index, protocol descriptor, what the descriptor must say). Errors name what neither automatic nor bound
/// generic arguments handle.
fn witness_requirements(decl: &ast::Decl) -> Result<Vec<(usize, String, ProtocolLayout)>, Error> {
    use ast::{Requirement, Subject};
    let explicit = |why: String| Err(Error(format!("{why}: pass the generic arguments explicitly (type metadata, then witness tables)")));
    let class_bound = class_bound_params(decl);
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
                let layout = match (protocol.class_bound, class_bound.contains(&param)) {
                    (true, _) => ProtocolLayout::Class,
                    (false, false) => ProtocolLayout::Any,
                    (false, true) => ProtocolLayout::Either,
                };
                witnesses.push((param.index, mangler::protocol_descriptor(&protocol).map_err(Error)?, layout));
            }
            Requirement::Conforms(Subject::Member(..), _) => return explicit("requirements on associated types".into()),
            // The type's parameter fixed to a type: no generic argument (see `fixed_params`).
            Requirement::SameType(Subject::Param(param), _) if fixed_params(decl).contains(&param) => {}
            Requirement::SameType(Subject::Param(_), _) => return explicit("generic parameters fixed to a type".into()),
            Requirement::SameType(Subject::Member(..), _) => {}
            // No witness table: a class-bound parameter's value is a reference (see `check_lowering`).
            Requirement::Superclass(..) | Requirement::AnyObject(_) => {}
        }
    }
    Ok(witnesses)
}

fn generic_plan_of(decl: &ast::Decl) -> Result<Option<GenericPlan>, Error> {
    use ast::{GenericParam, Ty};
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
    let mut witnesses = Vec::new();
    let mut witness_layouts = Vec::new();
    for (param, descriptor, layout) in witness_requirements(decl)? {
        witnesses.push((metadata_from[param], descriptor));
        witness_layouts.push(layout);
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
    fn kinds_come_from_a_resolver() {
        use super::{Declaration, KindResolver, TypeKind};
        struct Graph;
        impl KindResolver for Graph {
            fn kind(&self, module: &str, path: &[&str]) -> Option<TypeKind> {
                match (module, path) {
                    ("SwiftData", ["ModelContext"]) => Some(TypeKind::Class),
                    ("SwiftData", ["FetchDescriptor"]) => Some(TypeKind::Struct),
                    ("SwiftData", ["PersistentModel"]) => Some(TypeKind::Protocol),
                    _ => None,
                }
            }
            fn class_bound(&self, module: &str, protocol: &str) -> Option<bool> {
                Some(module == "SwiftData" && protocol == "PersistentModel")
            }
        }
        let symbol = |d: &str| Declaration::parse_resolving(d, &Graph).and_then(|d| d.symbol()).map_err(|e| e.to_string());
        let kinded = "SwiftData.ModelContext(class).fetch<T: PersistentModel(AnyObject)>(_: FetchDescriptor(struct)<T>) throws -> [T]";
        let bare = "SwiftData.ModelContext.fetch<T: PersistentModel>(_: FetchDescriptor<T>) throws -> [T]";
        assert_eq!(symbol(bare), symbol(kinded));
        let bound = [("T", "Todos.Todo(class)")];
        let plan = Declaration::parse_resolving(bare, &Graph).unwrap().bind(&bound).unwrap();
        assert_eq!(plan, Declaration::parse(kinded).unwrap().bind(&bound).unwrap(), "class-bound from the resolver");
        assert!(symbol("SwiftData.Unknown.f()").unwrap_err().contains("needs its kind"));
    }

    #[test]
    fn bind_fixes_generic_parameters() {
        use super::{ConcretePlan, Declaration, ProtocolLayout};
        let bind = |d: &str, b: &[(&str, &str)]| Declaration::parse(d).unwrap().bind(b);
        let fetch = "SwiftData.ModelContext(class).fetch<T: PersistentModel(AnyObject)>(_: FetchDescriptor(struct)<T>, \
                     batchSize: Int) throws -> FetchResultsCollection(struct)<T>";
        assert_eq!(
            bind(fetch, &[("T", "Todos.Todo(class)")]).unwrap(),
            ConcretePlan {
                types: vec!["5Todos4TodoC".into()],
                witnesses: vec![(super::BoundType::Named("5Todos4TodoC".into()), "$s9SwiftData15PersistentModelMp".into(), ProtocolLayout::Class)],
                superclasses: vec![],
                same_types: vec![],
                error_type: None,
                self_type: None,
                receiver: Some(super::ReceiverCheck::Instance("9SwiftData12ModelContextC".into())),
                result: Some("9SwiftData22FetchResultsCollectionVy5Todos4TodoCG".into()),
                by_address: vec![false, false],
                result_by_address: false,
                argument_types: vec![Some("9SwiftData15FetchDescriptorVy5Todos4TodoCG".into()), None],
                nominal_params: vec![
                    Some(super::NominalRef {
                        mangled: Some("9SwiftData15FetchDescriptorVy5Todos4TodoCG".into()),
                        name: "SwiftData.FetchDescriptor".into(),
                        optional: false,
                        array: false,
                        class: false,
                    }),
                    None,
                ],
                nominal_result: Some(super::NominalRef {
                    mangled: Some("9SwiftData22FetchResultsCollectionVy5Todos4TodoCG".into()),
                    name: "SwiftData.FetchResultsCollection".into(),
                    optional: false,
                    array: false,
                    class: false,
                }),
            }
        );
        let zero = bind("Res.zeroOf<T: AdditiveArithmetic>() -> T", &[("T", "Swift.Int")]).unwrap();
        assert_eq!(zero.types, ["Si"]);
        assert_eq!(zero.result.as_deref(), Some("Si"));
        assert!(zero.result_by_address, "a generic result is returned by address");
        assert!(bind("M.f<T>(_: T) -> Int", &[("T", "Swift.Int")]).unwrap().by_address == [true]);
        let error = |d: &str, b: &[(&str, &str)]| bind(d, b).unwrap_err().to_string();
        assert!(error("M.f<T>(_: T) -> Int", &[]).contains("`T` is not bound"));
        assert!(error("M.f<T>(_: T) -> Int", &[("T", "Swift.Int"), ("U", "Swift.Int")]).contains("no generic parameter `U`"));
        assert!(error("M.f<T>(_: T) -> Int", &[("T", "some Hashable")]).contains("concrete"));
        assert!(error("M.f<T>(_: T?) -> Int", &[("T", "Swift.Int")]).contains("only as a parameter or result itself"));
        assert!(error("M.f<T>(_: Int) -> (T, Int)", &[("T", "Swift.Int")]).contains("only as a parameter or result itself"));
        assert!(error("M.f(_: Int)", &[("T", "Swift.Int")]).contains("not generic"));
        assert!(error("M.f<T>(_: T, _: some Hashable)", &[("T", "Swift.Int")]).contains("opaque `some` parameter"));
        // A variadic `T...` is one `[T]`, an `inout T` a `&mut SwiftValue`: neither is the by-address `&SwiftValue`.
        assert_eq!(bind("M.f<T>(_: T..., _: inout T) -> Int", &[("T", "Swift.Int")]).unwrap().by_address, [false, false]);
        assert!(error("M.g<S: AsyncSequence>(_: [S]) throws(S.Failure) -> Int", &[("S", "Swift.Int")]).contains("typed throws"));
        assert!(error("M.h<T: AnyObject>(_: T)", &[("T", "Swift.Int")]).contains("not a class"));
        // Each argument whose type names a bound parameter: its type then, which the call checks the value against.
        let fetch = bind(fetch, &[("T", "Todos.Todo(class)")]).unwrap();
        assert_eq!(fetch.argument_types, [Some("9SwiftData15FetchDescriptorVy5Todos4TodoCG".to_string()), None]);
    }

    #[test]
    fn bind_specializes_generic_types_and_requirements() {
        use super::{BoundType, Declaration, ProtocolLayout};
        let bind = |d: &str, b: &[(&str, &str)]| Declaration::parse(d).unwrap().bind(b);
        let error = |d: &str, b: &[(&str, &str)]| bind(d, b).unwrap_err().to_string();
        let named = |n: &str| BoundType::Named(n.into());
        // A generic struct's static member takes the type's and its own parameters' metadata, then every witness
        // table in canonical order; an instance member only its own, after `Self`'s metadata (from the receiver).
        let make = bind("static M.Box(struct)<T: Hashable>.make<U: P>(_: U) -> Int", &[("T", "Swift.Int"), ("U", "M.S(struct)")]).unwrap();
        assert_eq!(make.types, ["Si", "1M1SV"]);
        assert_eq!(make.witnesses, [(named("Si"), "$sSHMp".into(), ProtocolLayout::Any), (named("1M1SV"), "$s1M1PMp".into(), ProtocolLayout::Any)]);
        assert_eq!(make.self_type.as_deref(), Some("1M3BoxVySiG"));
        let map = bind("M.Box(struct)<T: Hashable>.map<U: P>(_: U) -> T", &[("T", "Swift.Int"), ("U", "M.S(struct)")]).unwrap();
        assert_eq!(map.types, ["1M1SV"]);
        assert_eq!(map.witnesses, [(named("1M1SV"), "$s1M1PMp".into(), ProtocolLayout::Any)]);
        assert_eq!((map.result.as_deref(), map.result_by_address, map.by_address.as_slice()), (Some("Si"), true, &[true][..]));
        // A generic class's members take their own parameters only; the bound class is `self` or the receiver's.
        let class = bind("M.GBox(class)<T>.map<U: P>(_: U) -> Int", &[("T", "Swift.Int"), ("U", "M.S(struct)")]).unwrap();
        assert_eq!((class.types.as_slice(), class.self_type.as_deref()), (&["1M1SV".to_string()][..], Some("1M4GBoxCySiG")));
        // The receiver is checked against the bound type: a class's object by cast, a value exactly.
        use super::ReceiverCheck;
        assert_eq!(class.receiver, Some(ReceiverCheck::Instance("1M4GBoxCySiG".into())));
        assert_eq!(map.receiver, Some(ReceiverCheck::Exactly("1M3BoxVySiG".into())));
        assert_eq!(make.receiver, None);
        // Several protocols: the symbol's order.
        let two = bind("M.two<T: Hashable & Comparable>(_: T) -> Int", &[("T", "Swift.Int")]).unwrap();
        assert_eq!(two.witnesses, [(named("Si"), "$sSLMp".into(), ProtocolLayout::Any), (named("Si"), "$sSHMp".into(), ProtocolLayout::Any)]);
        // Associated types: a conformance's witness table, or the type checked.
        let element = BoundType::Associated { ty: "Say1M1SVG".into(), protocol: "$sSTMp".into(), base: "$sSTTL".into(), descriptor: "$s7ElementSTTl".into() };
        let conforming = bind("M.f<S: Sequence>(_: S) -> Int where S.Element: P", &[("S", "[M.S(struct)]")]).unwrap();
        assert_eq!(conforming.witnesses, [(named("Say1M1SVG"), "$sSTMp".into(), ProtocolLayout::Any), (element.clone(), "$s1M1PMp".into(), ProtocolLayout::Either)]);
        let same = bind("M.f<S: Sequence>(_: S) -> Int where S.Element == String", &[("S", "[M.S(struct)]")]).unwrap();
        assert_eq!(same.same_types, [(element, "SS".to_string())]);
        // A superclass, checked when the call is made; a typed error's type.
        let sup = bind("M.k<T: Animal(class)>(_: T) -> Int", &[("T", "M.Dog(class)")]).unwrap();
        assert_eq!(sup.superclasses, [("1M3DogC".to_string(), "1M6AnimalC".to_string())]);
        let thrown = bind("M.g<E: Error>(_: Int) throws(E) -> Int", &[("E", "M.Boom(struct)")]).unwrap();
        assert_eq!(thrown.error_type.as_deref(), Some("1M4BoomV"));
        // Every parameter is bound, the type's too; a protocol's `Self` is not.
        assert!(error("M.Box(struct)<T>.map<U>(_: U) -> Int", &[("U", "Swift.Int")]).contains("`T` is not bound"));
        assert!(error("M.Box(struct)<T>.map<T>(_: T) -> Int", &[("T", "Swift.Int")]).contains("names two generic parameters"));
        assert!(error("M.P(protocol).f<U>(_: U) -> Int", &[("U", "Swift.Int")]).contains("protocol"));
    }

    #[test]
    fn bind_passes_added_conformances() {
        use super::{BoundType, Declaration, ProtocolLayout};
        let parse = |d: &str| Declaration::parse(d).unwrap();
        let named = |n: &str| BoundType::Named(n.into());
        // A conformance an extension or the member adds to the type's parameters is one more witness table, in
        // canonical order among the member's own (observed, accepted 2026-11-21): `(Box<T>, U, T.P, U.Q, self)`.
        let b = parse("(extension in M where T: P) M.Box(struct)<T>.b<U: Q>(_: U) -> Int");
        let plan = b.bind(&[("T", "Swift.Int"), ("U", "M.S(struct)")]).unwrap();
        assert_eq!(plan.types, ["1M1SV"]);
        assert_eq!(
            plan.witnesses,
            [(named("Si"), "$s1M1PMp".into(), ProtocolLayout::Any), (named("1M1SV"), "$s1M1QMp".into(), ProtocolLayout::Any)]
        );
        // The type's own requirement comes with `self`; only the added one is passed. A member's own `where`, too.
        let c = parse("(extension in M where T: Q) M.GBox(class)<T: P>.c() -> Int").bind(&[("T", "Swift.Int")]).unwrap();
        assert_eq!(c.witnesses, [(named("Si"), "$s1M1QMp".into(), ProtocolLayout::Any)]);
        let d = parse("M.Box(struct)<T>.d() -> Int where T: Q").bind(&[("T", "Swift.Int")]).unwrap();
        assert_eq!(d.witnesses, [(named("Si"), "$s1M1QMp".into(), ProtocolLayout::Any)]);
        // Bound, they are callable; unbound, nothing would pass the witness table.
        for added in [
            "(extension in M where T: P) M.Box(struct)<T>.b<U: Q>(_: U) -> Int",
            "M.Box(struct)<T>.d() -> Int where T: Q",
            "static (extension in M where T: P) M.GBox(class)<T>.s() -> Int",
        ] {
            assert!(parse(added).check_bound_callable().is_ok(), "{added}");
            let error = parse(added).check_callable().unwrap_err().to_string();
            assert!(error.contains("bind"), "{added}: {error}");
        }
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
        // A generic class's member and a protocol extension's take the added conformance's witness table as well
        // (`(T.P, self)`, observed): refused alike. A struct's static member takes the whole signature's arguments.
        for added in [
            "(extension in M where T: P) M.C(class)<T>.c() -> Int",
            "static (extension in M where T: P) M.C(class)<T>.s() -> Int",
            "(extension in M where Self: Q) M.P(protocol).f() -> Int",
            "(extension in M where Self.Element: Q) M.P(protocol).f() -> Int",
        ] {
            let error = super::check_callable(added).unwrap_err().to_string();
            assert!(error.contains("witness"), "{added}: {error}");
        }
        assert!(super::check_callable("static (extension in M where T: P) M.D(struct)<T>.s() -> Int").is_ok());
        assert_eq!(context("M.D(struct)<T>.init(x: Int)"), ValueContext::GenericArguments);
        // A member's own `where` fixing the type's parameters drops them as an extension's does (observed, accepted
        // 2026-11-22): `One<V>.init() where V == Bool` takes nothing, `get() where V == Bool` not even `Self`.
        assert_eq!(context("M.One(struct)<V>.init() where V == Bool"), ValueContext::None);
        assert_eq!(context("M.One(struct)<V>.get() -> Int where V == Bool"), ValueContext::None);
        assert_eq!(context("M.Two(struct)<A, B>.both() -> Int where A == Int"), ValueContext::SelfMetadata);
        assert_eq!(context("M.Two(struct)<A, B>.init(b: B) where A == Int"), ValueContext::GenericArguments);
        let names = |d: &str| Declaration::parse(d).unwrap().generic_parameter_names();
        assert_eq!(names("M.Two(struct)<A, B>.init(b: B) where A == Int"), ["B"]);
        assert_eq!(names("M.Two(struct)<A, B>.init(a: A) where B == [A]"), ["A"]);
        // A static member or initializer takes only the parameters left free, an extension's alike (`Two.init(b:)`
        // takes `B`'s metadata; `init(p:) where B == String, A: Hashable` takes `A`, `A.Hashable`).
        let bind = |d: &str, b: &[(&str, &str)]| Declaration::parse(d).unwrap().bind(b).map(|plan| (plan.types, plan.witnesses.len()));
        assert_eq!(bind("M.Two(struct)<A, B>.init(b: B) where A == Int", &[("B", "Swift.String")]).unwrap(), (vec!["SS".into()], 0));
        assert_eq!(bind("static M.Two(struct)<A, B>.tag(_: B) -> Int where A == Int", &[("B", "Swift.String")]).unwrap(), (vec!["SS".into()], 0));
        assert_eq!(bind("(extension in M where A == Int) M.Two(struct)<A, B>.init(b: B)", &[("B", "Swift.String")]).unwrap(), (vec!["SS".into()], 0));
        assert_eq!(bind("M.Two(struct)<A, B>.init(p: A) where B == String, A: Hashable", &[("A", "Swift.Int")]).unwrap(), (vec!["Si".into()], 1));
        // A type declared in an extension that fixes its enclosing type's parameter (`extension Holder where T ==
        // Int { struct Fixed }`) is concrete the same way; one that constrains it takes the conformance.
        let fixed = "M.Holder(struct)<T>.(extension in M where T == Int) Fixed(struct)";
        assert_eq!(context(&format!("{fixed}.get() -> Int")), ValueContext::None);
        assert_eq!(context(&format!("{fixed}.init(x: Int)")), ValueContext::None);
        assert!(names(&format!("{fixed}.init(x: Int)")).is_empty());
        let tagged = "M.Holder(struct)<T>.(extension in M where T: Tagged) Mode(struct)";
        assert_eq!(context(&format!("{tagged}.get() -> Int")), ValueContext::SelfMetadata);
        assert_eq!(bind(&format!("static {tagged}.make() -> Int"), &[("T", "Swift.Int")]).unwrap(), (vec!["Si".into()], 1));
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
        // A receiver is checked by its type: a class's object by cast, a value's type exactly or by name.
        use super::ReceiverCheck::*;
        let receiver = |d: &str| super::Declaration::parse(d).unwrap().receiver_check();
        assert_eq!(receiver("Todos.Todo(class).title: String { get }"), Some(Instance("5Todos4TodoC".into())));
        assert_eq!(receiver("Res.Meter(struct).value: Double { get }"), Some(Exactly("3Res5MeterV".into())));
        assert_eq!(receiver("SwiftData.FetchResultsCollection(struct)<Element>.endIndex: Int { get }"), Some(Named("SwiftData.FetchResultsCollection".into())));
        for unchecked in ["M.f()", "static M.S(struct).f()", "M.C(class).init()", "M.C(class)<T>.f()", "M.P(protocol).f()"] {
            assert_eq!(receiver(unchecked), None, "{unchecked}");
        }
        // What a typed wrapper may stand for: a class, struct or enum outside the standard library.
        let decl = super::Declaration::parse("M.f(_: M.C(class), _: Int, _: M.S(struct)<M.C(class)>) -> [M.C(class)]?").unwrap();
        let c = super::NominalRef { mangled: Some("1M1CC".into()), name: "M.C".into(), optional: false, array: false, class: true };
        let s_ = super::NominalRef { mangled: Some("1M1SVyAA1CCG".into()), name: "M.S".into(), optional: false, array: false, class: false };
        assert_eq!(decl.nominal_params(), [Some(c.clone()), None, Some(s_)]);
        assert_eq!(super::Declaration::parse("M.f() -> [M.C(class)]").unwrap().nominal_result(), Some(super::NominalRef { array: true, ..c.clone() }));
        assert_eq!(decl.nominal_result(), None, "an optional array is no wrapper's");
        assert_eq!(super::wrapper_type("class", "Todos.Todo(class)").unwrap(), ("5Todos4TodoC".into(), "Todos.Todo".into()));
        assert!(super::wrapper_type("struct", "Todos.Todo(class)").is_err());
        assert!(super::wrapper_type("struct", "M.S(struct)<T>").is_err());
        // Results Swift returns in registers, which a by-address `SwiftValue` would read as garbage.
        let in_registers = |d: &str| super::Declaration::parse(d).unwrap().result_in_registers();
        for direct in ["M.f() -> [M.S(struct)]", "M.f() -> [String: Int]?", "M.f() -> Set<Int>", "M.f() -> M.C(class)?", "M.v: String { get }"] {
            assert!(in_registers(direct), "{direct}");
        }
        for by_address in ["M.f() -> M.S(struct)", "M.f() -> M.S(struct)?", "M.f<T>() -> T", "M.f() -> (Int, Int)", "M.f() -> any Hashable"] {
            assert!(!in_registers(by_address), "{by_address}");
        }
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
        // Without a thunk (a module without library evolution), the class's vtable: the member's method descriptor
        // and the declaring class's descriptor (the unbound one for a generic class).
        use super::{class_method, ClassMethod};
        assert_eq!(
            class_method("static M.C(class)<T>.kind() -> Int").unwrap(),
            Some(ClassMethod {
                thunk: "$s1M1CC4kindSiyFZTj".into(),
                method: "$s1M1CC4kindSiyFZTq".into(),
                class: "$s1M1CCMn".into(),
                class_side: true,
            })
        );
        assert_eq!(class_method("M.C(class).count: Int { get }").unwrap().map(|m| m.method), Some("$s1M1CC5countSivgTq".into()));
        assert_eq!(class_method("M.f()").unwrap(), None);
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
