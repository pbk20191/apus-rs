//! A Swift `AsyncSequence` consumed from Rust.
//!
//! Works for any conforming type through the protocols' dispatch thunks: `makeAsyncIterator()` once, then
//! `next()` per element, each a Swift call in the current Task like any other (see call.rs). The associated
//! types (iterator, element) come from the conformances' witness tables.

use super::any::FromSwiftAny;
use super::call::{CallError, SwiftCall};
use super::func::SwiftAsyncFn;
use super::sync::SwiftFn;
use super::value::{SwiftProtocol, SwiftType, SwiftValue, WitnessTable};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

static ASYNC_SEQUENCE: SwiftProtocol = SwiftProtocol::new("$sSciMp");
static ASYNC_ITERATOR_PROTOCOL: SwiftProtocol = SwiftProtocol::new("$sScIMp");
/// `AsyncSequence.makeAsyncIterator()` (dispatch thunk): `__consuming`, self by address, result by address.
static MAKE_ASYNC_ITERATOR: SwiftFn = SwiftFn::new("$sSci17makeAsyncIterator0bC0QzyFTj");
/// `AsyncIteratorProtocol.next() async throws -> Element?` (dispatch thunk): `mutating`, result by address.
static NEXT: SwiftAsyncFn = SwiftAsyncFn::new("$sScI4next7ElementQzSgyYaKFTj");

/// The iterator, mutated in place by `next()`. Only the one call in flight touches it.
struct IteratorCell(SwiftValue);
unsafe impl Sync for IteratorCell {}

/// The elements of a Swift `AsyncSequence`.
///
/// ```ignore
/// let mut updates = SwiftStream::new(sequence)?;          // any value whose type is an AsyncSequence
/// while let Some(update) = updates.next().await {
///     let update: SwiftValue = update?;                   // Err: the sequence threw; it is finished then
/// }
/// ```
///
/// Each element is awaited in the current Swift Task, so the task's cancellation ends a cancellable sequence.
/// Dropping the stream destroys the iterator, which is how Swift sequences are told to stop producing.
pub struct SwiftStream {
    iterator: Arc<IteratorCell>,
    iterator_type: SwiftType,
    iterator_conformance: WitnessTable,
    element: SwiftType,
    in_flight: Option<SwiftCall<SwiftValue>>,
    child: bool,
    finished: bool,
}

impl SwiftStream {
    /// Starts iterating `sequence`. Fails with `InvalidArgument` if its type is not an `AsyncSequence`.
    pub fn new(sequence: SwiftValue) -> Result<SwiftStream, CallError> {
        let sequence_type = sequence.ty();
        let conformance = sequence_type.conformance(&ASYNC_SEQUENCE).ok_or(CallError::InvalidArgument("the value's type is not an AsyncSequence"))?;
        let unavailable = CallError::Unavailable("AsyncSequence runtime support");
        let Some(iterator_type) = sequence_type.associated_type(conformance, "$sSciTL", "$s13AsyncIteratorSciTl") else { return Err(unavailable) };
        let Some(iterator_conformance) = iterator_type.conformance(&ASYNC_ITERATOR_PROTOCOL) else { return Err(unavailable) };
        let Some(element) = iterator_type.associated_type(iterator_conformance, "$sScITL", "$s7ElementScITl") else { return Err(unavailable) };
        // Witness method arguments: Self's metadata, then the conformance's witness table.
        let iterator = MAKE_ASYNC_ITERATOR.call_returning(iterator_type, (sequence_type, conformance)).on_consumed(sequence).invoke()?;
        Ok(SwiftStream { iterator: Arc::new(IteratorCell(iterator)), iterator_type, iterator_conformance, element, in_flight: None, child: false, finished: false })
    }

    /// The sequence's `Element` type.
    pub fn element_type(&self) -> SwiftType {
        self.element
    }

    /// Awaits each element in a child Task instead of this Task, so that other futures polled alongside
    /// (`select`, [`within`](super::within)) keep running while an element is pending.
    pub fn child(mut self) -> SwiftStream {
        self.child = true;
        self
    }

    /// The next element; `None` when the sequence is finished. A thrown error is the last item.
    /// Dropping the returned future keeps the pending element for the next call.
    pub fn next(&mut self) -> impl Future<Output = Option<Result<SwiftValue, CallError>>> + '_ {
        std::future::poll_fn(|cx| self.poll_next_element(cx))
    }

    /// Like [`next`](Self::next), with the element copied out as `T` (`InvalidArgument` if it is not one).
    pub async fn next_value<T: FromSwiftAny>(&mut self) -> Option<Result<T, CallError>> {
        Some(match self.next().await? {
            Ok(element) => element.get::<T>().ok_or(CallError::InvalidArgument("the element is not of the requested type")),
            Err(error) => Err(error),
        })
    }

    pub fn poll_next_element(&mut self, cx: &mut Context<'_>) -> Poll<Option<Result<SwiftValue, CallError>>> {
        if self.finished {
            return Poll::Ready(None);
        }
        let (iterator, iterator_type, conformance, element, child) = (&self.iterator, self.iterator_type, self.iterator_conformance, self.element, self.child);
        let call = self.in_flight.get_or_insert_with(|| {
            let call = NEXT.call_returning(element.optional(), (iterator_type, conformance)).throws().on_inout(iterator.0.as_ptr(), iterator.clone());
            if child { call.child() } else { call }
        });
        let Poll::Ready(result) = Pin::new(call).poll(cx) else { return Poll::Pending };
        self.in_flight = None;
        Poll::Ready(match result {
            Ok(optional) => match optional.into_optional_payload(element) {
                Some(value) => Some(Ok(value)),
                None => {
                    self.finished = true;
                    None
                }
            },
            Err(error) => {
                // A sequence that threw is over; other errors (no current task) leave it untouched.
                self.finished = matches!(error, CallError::Swift(_));
                Some(Err(error))
            }
        })
    }
}

#[cfg(feature = "stream")]
impl futures_core::Stream for SwiftStream {
    type Item = Result<SwiftValue, CallError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.get_mut().poll_next_element(cx)
    }
}
