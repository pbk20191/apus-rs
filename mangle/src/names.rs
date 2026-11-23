//! Names the mangling abbreviates (include/swift/Demangling/StandardTypesMangling.def) and the standard
//! library names the declaration syntax accepts without a module.

use crate::ast::Kind;

/// `S<letter>` types of the Swift module.
pub fn standard_type(name: &str) -> Option<(&'static str, Kind)> {
    Some(match name {
        "Array" => ("a", Kind::Struct),
        "Bool" => ("b", Kind::Struct),
        "Dictionary" => ("D", Kind::Struct),
        "Double" => ("d", Kind::Struct),
        "Float" => ("f", Kind::Struct),
        "Set" => ("h", Kind::Struct),
        "DefaultIndices" => ("I", Kind::Struct),
        "Int" => ("i", Kind::Struct),
        "Character" => ("J", Kind::Struct),
        "ClosedRange" => ("N", Kind::Struct),
        "Range" => ("n", Kind::Struct),
        "ObjectIdentifier" => ("O", Kind::Struct),
        "UnsafePointer" => ("P", Kind::Struct),
        "UnsafeMutablePointer" => ("p", Kind::Struct),
        "UnsafeBufferPointer" => ("R", Kind::Struct),
        "UnsafeMutableBufferPointer" => ("r", Kind::Struct),
        "String" => ("S", Kind::Struct),
        "Substring" => ("s", Kind::Struct),
        "UInt" => ("u", Kind::Struct),
        "UnsafeRawPointer" => ("V", Kind::Struct),
        "UnsafeMutableRawPointer" => ("v", Kind::Struct),
        "UnsafeRawBufferPointer" => ("W", Kind::Struct),
        "UnsafeMutableRawBufferPointer" => ("w", Kind::Struct),
        "Optional" => ("q", Kind::Enum),
        _ => return None,
    })
}

/// `Sc<letter>` types of the _Concurrency module.
pub fn concurrency_type(name: &str) -> Option<(&'static str, Kind)> {
    Some(match name {
        "CheckedContinuation" => ("cC", Kind::Struct),
        "UnsafeContinuation" => ("cc", Kind::Struct),
        "CancellationError" => ("cE", Kind::Struct),
        "UnownedSerialExecutor" => ("ce", Kind::Struct),
        "TaskGroup" => ("cG", Kind::Struct),
        "ThrowingTaskGroup" => ("cg", Kind::Struct),
        "UnownedJob" => ("cJ", Kind::Struct),
        "MainActor" => ("cM", Kind::Class),
        "TaskPriority" => ("cP", Kind::Struct),
        "AsyncStream" => ("cS", Kind::Struct),
        "AsyncThrowingStream" => ("cs", Kind::Struct),
        "Task" => ("cT", Kind::Struct),
        "UnsafeCurrentTask" => ("ct", Kind::Struct),
        _ => return None,
    })
}

/// `S<letter>` protocols of the Swift module.
pub fn standard_protocol(name: &str) -> Option<&'static str> {
    Some(match name {
        "BinaryFloatingPoint" => "B",
        "Encodable" => "E",
        "Decodable" => "e",
        "FloatingPoint" => "F",
        "RandomNumberGenerator" => "G",
        "Hashable" => "H",
        "Numeric" => "j",
        "BidirectionalCollection" => "K",
        "RandomAccessCollection" => "k",
        "Comparable" => "L",
        "Collection" => "l",
        "MutableCollection" => "M",
        "RangeReplaceableCollection" => "m",
        "Equatable" => "Q",
        "Sequence" => "T",
        "IteratorProtocol" => "t",
        "UnsignedInteger" => "U",
        "RangeExpression" => "X",
        "Strideable" => "x",
        "RawRepresentable" => "Y",
        "StringProtocol" => "y",
        "SignedInteger" => "Z",
        "BinaryInteger" => "z",
        _ => return None,
    })
}

/// The protocol a substitution (`T` of `ST`, `ci` of `Sci`) stands for: `standard_protocol` and
/// `concurrency_protocol` the other way.
pub fn substituted_protocol(substitution: &str) -> Option<&'static str> {
    const NAMES: [&str; 29] = [
        "BinaryFloatingPoint", "Encodable", "Decodable", "FloatingPoint", "RandomNumberGenerator", "Hashable", "Numeric",
        "BidirectionalCollection", "RandomAccessCollection", "Comparable", "Collection", "MutableCollection",
        "RangeReplaceableCollection", "Equatable", "Sequence", "IteratorProtocol", "UnsignedInteger", "RangeExpression",
        "Strideable", "RawRepresentable", "StringProtocol", "SignedInteger", "BinaryInteger", "Actor", "Executor",
        "SerialExecutor", "TaskExecutor", "AsyncIteratorProtocol", "AsyncSequence",
    ];
    NAMES.into_iter().find(|name| standard_protocol(name).or_else(|| concurrency_protocol(name)) == Some(substitution))
}

/// The primary associated types of the Swift and _Concurrency modules' protocols (`protocol Collection<Element>`),
/// as their `.swiftinterface` declares them, each with the protocol declaring it (a symbol names that one:
/// `Collection`'s `Element` is `Sequence`'s).
pub fn primary_associated_types(name: &str) -> Option<&'static [(&'static str, &'static str)]> {
    Some(match name {
        "Sequence" | "Collection" | "BidirectionalCollection" | "RandomAccessCollection" | "MutableCollection"
        | "RangeReplaceableCollection" => &[("Element", "Sequence")],
        "IteratorProtocol" => &[("Element", "IteratorProtocol")],
        "SetAlgebra" => &[("Element", "SetAlgebra")],
        "AsyncSequence" => &[("Element", "AsyncSequence"), ("Failure", "AsyncSequence")],
        "AsyncIteratorProtocol" => &[("Element", "AsyncIteratorProtocol"), ("Failure", "AsyncIteratorProtocol")],
        "Identifiable" => &[("ID", "Identifiable")],
        "RangeExpression" => &[("Bound", "RangeExpression")],
        "RawRepresentable" => &[("RawValue", "RawRepresentable")],
        "Strideable" => &[("Stride", "Strideable")],
        "SIMD" => &[("Scalar", "SIMDStorage")],
        "InstantProtocol" => &[("Duration", "InstantProtocol")],
        "Clock" => &[("Duration", "Clock")],
        _ => return None,
    })
}

/// `Sc<letter>` protocols of the _Concurrency module.
pub fn concurrency_protocol(name: &str) -> Option<&'static str> {
    Some(match name {
        "Actor" => "cA",
        "Executor" => "cF",
        "SerialExecutor" => "cf",
        "TaskExecutor" => "ch",
        "AsyncIteratorProtocol" => "cI",
        "AsyncSequence" => "ci",
        _ => return None,
    })
}

/// Swift-module types without an abbreviation that may still be written bare.
pub fn swift_type(name: &str) -> Option<Kind> {
    Some(match name {
        "Int8" | "Int16" | "Int32" | "Int64" | "UInt8" | "UInt16" | "UInt32" | "UInt64" | "Float16" | "AnyHashable" | "Duration"
        | "StaticString" => Kind::Struct,
        "Never" => Kind::Enum,
        _ => return None,
    })
}

/// Swift-module protocols without an abbreviation that may still be written bare.
pub fn swift_protocol(name: &str) -> bool {
    matches!(
        name,
        "Error"
            | "Sendable"
            | "CustomStringConvertible"
            | "CustomDebugStringConvertible"
            | "Identifiable"
            | "CaseIterable"
            | "AdditiveArithmetic"
            | "LosslessStringConvertible"
            | "TextOutputStream"
            | "Clock"
            | "InstantProtocol"
            | "DurationProtocol"
    )
}
