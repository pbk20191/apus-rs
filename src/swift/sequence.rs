//! A Swift `Sequence` iterated from Rust: [`SwiftStream`](super::SwiftStream)'s synchronous counterpart.
//!
//! Works for any conforming type (an `Array`, bridged or not, a `Set`, a `Dictionary`, a framework's collection)
//! through the protocols' dispatch thunks: `makeIterator()` once, then `next()` per element. The associated types
//! (iterator, element) come from the conformances' witness tables (`swift_getAssociatedTypeWitness`).

use super::call::CallError;
use super::sync::SwiftFn;
use super::value::{SwiftProtocol, SwiftType, SwiftValue, WitnessTable};

static SEQUENCE: SwiftProtocol = SwiftProtocol::new("$sSTMp");
static ITERATOR_PROTOCOL: SwiftProtocol = SwiftProtocol::new("$sStMp");
/// `Sequence.makeIterator()` (dispatch thunk): `__consuming`, self by address, result by address.
static MAKE_ITERATOR: SwiftFn = SwiftFn::new("$sST12makeIterator0B0QzyFTj");
/// `IteratorProtocol.next() -> Element?` (dispatch thunk): `mutating`, result by address.
static NEXT: SwiftFn = SwiftFn::new("$sSt4next7ElementQzSgyFTj");

/// The elements of a Swift `Sequence`, in order.
///
/// ```ignore
/// for todo in SwiftIter::new(todos)? {                    // any value whose type is a Sequence
///     let todo = todo.into_object().expect("a class instance");
/// }
/// ```
pub struct SwiftIter {
    iterator: SwiftValue,
    iterator_type: SwiftType,
    conformance: WitnessTable,
    element: SwiftType,
    finished: bool,
}

impl SwiftIter {
    /// Starts iterating `sequence`. Fails with `InvalidArgument` if its type is not a `Sequence`.
    pub fn new(sequence: SwiftValue) -> Result<SwiftIter, CallError> {
        let sequence_type = sequence.ty();
        let conformance = sequence_type.conformance(&SEQUENCE).ok_or(CallError::InvalidArgument("the value's type is not a Sequence"))?;
        let unavailable = CallError::Unavailable("Sequence runtime support");
        if !NEXT.is_available() {
            return Err(unavailable);
        }
        let Some(iterator_type) = sequence_type.associated_type(conformance, "$sSTTL", "$s8IteratorSTTl") else { return Err(unavailable) };
        let Some(iterator_conformance) = iterator_type.conformance(&ITERATOR_PROTOCOL) else { return Err(unavailable) };
        let Some(element) = iterator_type.associated_type(iterator_conformance, "$sStTL", "$s7ElementStTl") else { return Err(unavailable) };
        // Witness method arguments: Self's metadata, then the conformance's witness table.
        let iterator = MAKE_ITERATOR.call_returning(iterator_type, (sequence_type, conformance)).on_consumed(sequence).invoke()?;
        Ok(SwiftIter { iterator, iterator_type, conformance: iterator_conformance, element, finished: false })
    }

    /// The sequence's `Element` type.
    pub fn element_type(&self) -> SwiftType {
        self.element
    }
}

impl Iterator for SwiftIter {
    type Item = SwiftValue;

    fn next(&mut self) -> Option<SwiftValue> {
        if self.finished {
            return None;
        }
        let next = NEXT.call_returning(self.element.optional(), (self.iterator_type, self.conformance));
        // `next()` neither throws nor fails to lower: `new` found it, and its arguments are two pointers.
        let optional = next.on_inout_value(&mut self.iterator).invoke().expect("IteratorProtocol.next()");
        let element = optional.into_optional_payload(self.element);
        self.finished = element.is_none();
        element
    }
}
