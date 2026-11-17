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
}

pub struct Param {
    pub label: Option<String>,
    pub ty: Ty,
    pub inout: bool,
    pub variadic: bool,
    /// `isolated`: the parameter is the function's actor isolation.
    pub isolated: bool,
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
    Function { params: Vec<Param>, result: Ty, is_async: bool, throws: bool },
    Getter(Ty),
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
    /// Whether the enclosing type's own declaration constrains its parameters (`Set<Element: Hashable>`).
    pub context_is_constrained: bool,
    pub extension: Option<Extension>,
    pub name: String,
    pub is_static: bool,
    /// `@MainActor` before the declaration.
    pub main_actor: bool,
    /// `nonisolated` before the declaration (an actor member callable from anywhere).
    pub nonisolated: bool,
    /// The declaration's own generic parameters.
    pub generic_params: usize,
    /// The declaration's own requirements (inline constraints and its `where` clause).
    pub requirements: Vec<Requirement>,
    pub signature: Signature,
}
