//! What a declaration is, after parsing.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Struct,
    Class,
    /// An actor: mangled as a class.
    Actor,
    Enum,
    Protocol,
    /// An imported C type named by a typedef, mangled under that name (`So9NSDecimala`): an anonymous struct
    /// (`typedef struct {…} NSDecimal`) or a Core Foundation class (`CGColorRef`, written `CGColor` in Swift).
    TypedefStruct,
    TypedefClass,
}

impl Kind {
    pub fn operator(self) -> char {
        match self {
            Kind::Struct => 'V',
            Kind::Class | Kind::Actor => 'C',
            Kind::Enum => 'O',
            Kind::Protocol => 'P',
            Kind::TypedefStruct | Kind::TypedefClass => 'a',
        }
    }
}

/// A generic parameter: `depth` 0 for the parameters of the enclosing type (or of a free function), 1 for a
/// method's own parameters inside a generic type.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct GenericParam {
    pub depth: usize,
    pub index: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Protocol {
    pub module: String,
    pub name: String,
    /// Written `P(AnyObject)`: a class-bound protocol. Not part of the symbol.
    pub class_bound: bool,
}

impl Ty {
    /// The `split` of a nominal type whose arguments (`count` of them) all belong to path element `level` of `depth`.
    pub fn split(depth: usize, level: usize, count: usize) -> Vec<usize> {
        if level + 1 == depth {
            return Vec::new();
        }
        let mut split = vec![0; depth];
        split[level] = count;
        split
    }
}

impl Protocol {
    /// A marker protocol: no witness table, and it does not make an associated type ambiguous.
    pub fn is_marker(&self) -> bool {
        self.module == "Swift"
            && matches!(self.name.as_str(), "Sendable" | "SendableMetatype" | "Copyable" | "Escapable" | "BitwiseCopyable")
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Ty {
    /// `Module.Outer<A>.Inner<B>`: `args` of all levels, outermost first; `split` how many each path element takes,
    /// empty when they all belong to the last (see `Ty::split`). `extension`: one element of the path is declared in
    /// an extension that is part of its context.
    Nominal { module: String, path: Vec<(String, Kind)>, args: Vec<Ty>, split: Vec<usize>, extension: Option<Box<ContextExtension>> },
    Optional(Box<Ty>),
    /// `Pack{A, B}`: a variadic generic type's pack argument (`Predicate<each Input>`), written as Swift's
    /// demangler prints it; only among a nominal type's arguments.
    Pack(Vec<Ty>),
    /// `T.Type`: a type's metatype (`Any.Type`, `any P.Type`: an existential metatype).
    Metatype(Box<Ty>),
    Tuple(Vec<(Option<String>, Ty)>),
    Any,
    AnyObject,
    /// `any P`
    Existential(Protocol),
    Param(GenericParam),
    /// `T.Name`: an associated type of a generic parameter.
    Member(GenericParam, String),
    /// A closure type: `@escaping @Sendable @MainActor (A, B) async throws -> R`.
    Function(Box<FunctionTy>),
    /// `some P & Q` as the declaration's result: its opaque result type (`Qr`). As a parameter it is an anonymous
    /// generic parameter instead (the parser rewrites it). With its protocol's primary associated types
    /// (`some Collection<Int>`), which a result's symbol leaves out and a parameter's makes requirements.
    Opaque(Vec<Protocol>, Vec<PrimaryArg>),
}

/// An argument of `some P<…>`: the type a primary associated type of the `protocol`-th protocol is, named (`Element
/// == Int`) or in the protocol's order (`Int`).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PrimaryArg {
    pub protocol: usize,
    pub name: Option<String>,
    pub ty: Ty,
}

impl Ty {
    /// How many `some P` this type holds.
    pub fn opaque_count(&self) -> usize {
        match self {
            Ty::Opaque(..) => 1,
            Ty::Optional(wrapped) | Ty::Metatype(wrapped) => wrapped.opaque_count(),
            Ty::Pack(elements) => elements.iter().map(Ty::opaque_count).sum(),
            Ty::Tuple(elements) => elements.iter().map(|(_, e)| e.opaque_count()).sum(),
            Ty::Nominal { args, .. } => args.iter().map(Ty::opaque_count).sum(),
            Ty::Function(f) => f.params.iter().map(Ty::opaque_count).sum::<usize>() + f.result.opaque_count(),
            Ty::Any | Ty::AnyObject | Ty::Existential(_) | Ty::Param(_) | Ty::Member(..) => 0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct FunctionTy {
    pub params: Vec<Ty>,
    pub result: Ty,
    pub is_async: bool,
    pub throws: bool,
    pub sendable: bool,
    pub main_actor: bool,
    /// `@escaping` was written. A closure type is non-escaping only as a parameter without it; anywhere else
    /// (inside an optional, a result) it escapes.
    pub escaping: bool,
    /// `@autoclosure`: a parameter's closure the caller writes as an expression; passed like any closure.
    pub autoclosure: bool,
    pub convention: Convention,
    /// `-> sending R`: the result is in a disconnected region (`YT`); returned as any result is.
    pub sending_result: bool,
}

/// A function type's representation (`@convention(…)`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Convention {
    /// A Swift closure: the function and its context.
    Swift,
    /// `@convention(c)`: a C function pointer.
    C,
    /// `@convention(block)`: an Objective-C block object.
    Block,
}

pub struct Param {
    pub label: Option<String>,
    pub ty: Ty,
    pub inout: bool,
    pub variadic: bool,
    /// `isolated`: the parameter is the function's actor isolation.
    pub isolated: bool,
    /// `consuming`/`__owned` or `borrowing`/`__shared`, as written (`None`: the default).
    pub ownership: Option<Ownership>,
    /// `_const`: a compile-time constant argument (`Yt`); passed like any other.
    pub compile_time_const: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ownership {
    Consuming,
    Borrowing,
    /// `__owned`, `__shared`: the older spellings, which always show in the symbol. `sending` is `__owned`
    /// (SE-0430: the callee consumes it).
    Owned,
    Shared,
}

impl Ownership {
    /// Whether the callee consumes the argument, given whether it does by default (an initializer's).
    pub fn consumes(ownership: Option<Ownership>, by_default: bool) -> bool {
        match ownership {
            None => by_default,
            Some(Ownership::Consuming | Ownership::Owned) => true,
            Some(Ownership::Borrowing | Ownership::Shared) => false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Subject {
    Param(GenericParam),
    Member(GenericParam, String),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Requirement {
    Conforms(Subject, Protocol),
    SameType(Subject, Ty),
    /// `T: SomeClass`: a superclass.
    Superclass(Subject, Ty),
    /// `T: AnyObject`: the class layout.
    AnyObject(Subject),
}


pub enum Signature {
    /// `typed_error`: `throws(E)` with an error type other than `any Error`.
    Function { params: Vec<Param>, result: Ty, is_async: bool, throws: bool, typed_error: Option<Ty> },
    Getter(Ty),
    /// `name: T { set }`: the property's setter, which takes the new value.
    Setter(Ty),
    /// `subscript(…) -> T { get }` or `{ set }` (`setter`), whose setter takes the new value before the indices.
    Subscript { params: Vec<Param>, result: Ty, setter: bool },
    /// `case Module.Enum(enum).name(payload…)`: an enum case, mangled to its tag symbol (`…WC`). The payload's
    /// labels count: a single labeled value is a one-element tuple.
    Case(Vec<(Option<String>, Ty)>),
}

/// `(extension in Module where …)`: the declaration lives in an extension that needs its own mangling
/// (another module's, a constrained one, or a protocol's).
pub struct Extension {
    pub module: String,
    pub requirements: Vec<Requirement>,
}

/// A type declared in an extension that Swift makes part of its context (a constrained one, or one in another module
/// than the extended type's): `Outer<T>.(extension in M where T: P) Inner`. ASTMangler::appendContext.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ContextExtension {
    /// The index in the path of the type declared in the extension.
    pub at: usize,
    pub module: String,
    pub requirements: Vec<Requirement>,
    /// The extended type's generic parameters, and whether its own declaration constrains them: how the
    /// extension's generic signature ends.
    pub extended_params: usize,
    pub extended_constrained: bool,
}

impl Decl {
    /// The enclosing type, bound to its own generic parameters (`Outer<T>.Inner`).
    pub fn context_type(&self) -> Ty {
        let args: Vec<Ty> = (0..self.context_params).map(|index| Ty::Param(GenericParam { depth: 0, index })).collect();
        let level = self.context_generic_level.unwrap_or(self.context.len().saturating_sub(1));
        Ty::Nominal {
            module: self.module.clone(),
            path: self.context.clone(),
            split: Ty::split(self.context.len(), level, args.len()),
            args,
            extension: self.context_extension.clone().map(Box::new),
        }
    }
}

pub struct Decl {
    pub module: String,
    /// The types the declaration is a member of, outermost first.
    pub context: Vec<(String, Kind)>,
    /// An enclosing type declared in an extension that is part of its context.
    pub context_extension: Option<ContextExtension>,
    /// Generic parameters of the enclosing type (1 for a protocol: `Self`).
    pub context_params: usize,
    /// Which enclosing type (index into `context`) declares those parameters: a type nested in a generic one binds
    /// them at that level (`Outer<T>.Inner` is `yx_G`).
    pub context_generic_level: Option<usize>,
    /// `@preconcurrency`: the symbol leaves out `@Sendable` and global actors in function types.
    pub preconcurrency: bool,
    /// Whether the enclosing type's own declaration constrains its parameters (`Set<Element: Hashable>`).
    pub context_is_constrained: bool,
    pub extension: Option<Extension>,
    pub name: String,
    pub is_static: bool,
    /// `@MainActor` before the declaration.
    pub main_actor: bool,
    /// `nonisolated` before the declaration (an actor member callable from anywhere).
    pub nonisolated: bool,
    /// `nonisolated(nonsending)`: an async function that runs on its caller's isolation, passed as a hidden first
    /// argument. The symbol does not show it.
    pub nonsending: bool,
    /// A getter's (or a subscript getter's) effects: `{ get async throws }`. The symbol does not show them.
    pub accessor_async: bool,
    pub accessor_throws: bool,
    /// An operator's fixity in the symbol: `oi` (infix), `op` (prefix), `oP` (postfix).
    pub operator: Option<&'static str>,
    /// `consuming`/`__consuming` before the declaration: the method consumes `self`. The symbol does not show it.
    pub consumes_self: bool,
    /// `mutating`: the method changes `self` (a struct's or enum's, passed `inout`). The symbol does not show it.
    pub mutating: bool,
    /// The declaration's own generic parameters.
    pub generic_params: usize,
    /// Their names, in order (`some P` parameters are unnamed: `_`).
    pub generic_names: Vec<String>,
    /// The enclosing generic type's parameter names, in order (none for a protocol's `Self`).
    pub context_names: Vec<String>,
    /// The enclosing generic type's constraints (`Outer<T: P>`).
    pub context_requirements: Vec<Requirement>,
    /// `T.Value(of P)`: which protocol declares an associated type.
    pub associated_protocols: Vec<((GenericParam, String), Protocol)>,
    /// The nested associated types written `(mangled P)` (`T.A.B(mangled P)`): their symbol names that protocol.
    pub mangled_associated: Vec<(GenericParam, String)>,
    /// The declaration's own requirements (inline constraints and its `where` clause); for an enum case, the enum's.
    pub requirements: Vec<Requirement>,
    pub signature: Signature,
}
