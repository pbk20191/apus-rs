//! What a declaration is, after parsing.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Struct,
    Class,
    /// An actor: mangled as a class.
    Actor,
    Enum,
    Protocol,
}

impl Kind {
    pub fn operator(self) -> char {
        match self {
            Kind::Struct => 'V',
            Kind::Class | Kind::Actor => 'C',
            Kind::Enum => 'O',
            Kind::Protocol => 'P',
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
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Ty {
    /// `Module.Outer.Inner<Args>`; `args` belong to the last path element.
    Nominal { module: String, path: Vec<(String, Kind)>, args: Vec<Ty> },
    Optional(Box<Ty>),
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
    /// `__owned`, `__shared`: the older spellings, which always show in the symbol.
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Subject {
    Param(GenericParam),
    Member(GenericParam, String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Requirement {
    Conforms(Subject, Protocol),
    SameType(Subject, Ty),
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

pub struct Decl {
    pub module: String,
    /// The types the declaration is a member of, outermost first.
    pub context: Vec<(String, Kind)>,
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
    /// The declaration's own requirements (inline constraints and its `where` clause).
    pub requirements: Vec<Requirement>,
    pub signature: Signature,
}
