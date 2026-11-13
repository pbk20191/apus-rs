//! Computes the mangled symbol of a Swift declaration, the name it has in a binary.
//!
//! ```
//! let symbol = async_swift_rs_mangle::mangle("Greeter.greet(_: String, times: Int) async throws -> String").unwrap();
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
/// [static] Module.Type(kind)….name[<T: P, U>]([label|_]: Type, …) [async] [throws] [-> Type] [where …]
/// [static] Module.Type(kind)….name: Type { get }
/// ```
///
/// - Types: standard library names (`Int`, `String`, `Bool`, `Double`, `UInt8`, …), `[T]`, `[K: V]`, `Set<T>`,
///   `T?`, tuples, `Any`, `AnyObject`, `any Proto`, generic parameters and `T.Associated`.
/// - Other types name their kind: `Cart(struct)`, `Customer(class)`, `Color(enum)`; `(actor)` is accepted too.
///   A bare name belongs to the declaration's module; `Other.Name(kind)` names another module; nested types are
///   `Outer(kind).Inner(kind)`. Generic arguments follow the kind: `Box(struct)<Int>`.
/// - Not supported: members of extensions or of generic types, function-typed parameters, operators,
///   initializers, subscripts, setters, non-ASCII names, isolation and ownership attributes.
pub fn mangle(declaration: &str) -> Result<String, Error> {
    let decl = parse::parse(declaration).map_err(Error)?;
    mangler::mangle(&decl).map_err(Error)
}
