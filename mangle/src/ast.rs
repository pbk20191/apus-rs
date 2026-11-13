//! What a declaration is, after parsing.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Struct,
    Class,
    Enum,
}

impl Kind {
    pub fn operator(self) -> char {
        match self {
            Kind::Struct => 'V',
            Kind::Class => 'C',
            Kind::Enum => 'O',
        }
    }
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
    /// A generic parameter, by index (depth 0).
    Param(usize),
    /// `T.Name`: an associated type of a generic parameter.
    Member(usize, String),
}

pub struct Param {
    pub label: Option<String>,
    pub ty: Ty,
    pub inout: bool,
    pub variadic: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Subject {
    Param(usize),
    Member(usize, String),
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

pub struct Decl {
    pub module: String,
    /// The types the declaration is a member of, outermost first.
    pub context: Vec<(String, Kind)>,
    pub name: String,
    pub is_static: bool,
    pub generic_params: usize,
    pub requirements: Vec<Requirement>,
    pub signature: Signature,
}
