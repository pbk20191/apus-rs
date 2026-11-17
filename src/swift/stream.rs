//! A Swift `AsyncSequence` consumed from Rust.
//!
//! Works for any conforming type through the protocols' dispatch thunks: `makeAsyncIterator()` once, then
//! `next(isolation:)` (macOS 15) or `next()` per element, each a Swift call in the current Task like any other (see
//! call.rs). The associated types (iterator, element, failure) come from the conformances' witness tables.

use super::any::{FromSwiftAny, SwiftError};
use super::func::ErrorSlot;
use super::isolation::Isolation;
use super::rt;
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
/// `AsyncIteratorProtocol.next(isolation:) async throws(Failure) -> Element?` (macOS 15, dispatch thunk): the caller's
/// isolation, then the error slot; `mutating`, result by address.
static NEXT_ISOLATED: SwiftAsyncFn = SwiftAsyncFn::new("$sScI4next9isolation7ElementQzSgScA_pSgYi_tYa7FailureQzYKFTj");

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
    /// The iterator's `Failure` when `next(isolation:)` is used (macOS 15), else `None` (`next()`).
    failure: Option<SwiftType>,
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
        let failure = NEXT_ISOLATED
            .is_available()
            .then(|| iterator_type.associated_type(iterator_conformance, "$sScITL", "$s7FailureScITl"))
            .flatten();
        let iterator = MAKE_ASYNC_ITERATOR.call_returning(iterator_type, (sequence_type, conformance)).on_consumed(sequence).invoke()?;
        let iterator = Arc::new(IteratorCell(iterator));
        Ok(SwiftStream { iterator, iterator_type, iterator_conformance, element, failure, in_flight: None, child: false, finished: false })
    }

    /// Whether elements come from `next(isolation:)` (macOS 15) rather than `next()`.
    #[doc(hidden)]
    pub fn uses_isolated_next(&self) -> bool {
        self.failure.is_some()
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
        let failure = self.failure;
        let call = self.in_flight.get_or_insert_with(|| {
            let call = match failure {
                Some(failure) => {
                    let args = (Isolation::current(), ErrorSlot::of(failure), iterator_type, conformance);
                    NEXT_ISOLATED.call_returning(element.optional(), args)
                }
                None => NEXT.call_returning(element.optional(), (iterator_type, conformance)).throws(),
            };
            let call = call.on_inout(iterator.0.as_ptr(), iterator.clone());
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
                // A typed error is boxed as `any Error`, as `next()` throws it.
                let error = match (error, failure) {
                    (CallError::Typed(value), Some(failure)) => CallError::Swift(box_error(value, failure)),
                    (other, _) => other,
                };
                // A sequence that threw is over; other errors (no current task) leave it untouched.
                self.finished = matches!(error, CallError::Swift(_));
                Some(Err(error))
            }
        })
    }
}

/// A thrown `Failure` as an `any Error` box: the box itself when `Failure` is `any Error`, else a new box.
fn box_error(value: SwiftValue, failure: SwiftType) -> SwiftError {
    let any_error = SwiftType::from_mangled_name("s5Error_p");
    unsafe {
        if Some(failure) == any_error {
            let boxed = *(value.as_ptr() as *const *mut std::ffi::c_void);
            drop(value.into_uninit()); // the box moves out; only the storage is ours
            return SwiftError::from_raw(boxed);
        }
        let conformance = rt::swift_conformsToProtocol(failure.as_ptr(), &rt::ERROR_PROTOCOL as *const u8 as *const std::ffi::c_void);
        assert!(!conformance.is_null(), "an AsyncIteratorProtocol Failure conforms to Error");
        let boxed = rt::swift_allocError(failure.as_ptr(), conformance, value.as_ptr(), true);
        drop(value.into_uninit()); // taken into the box
        SwiftError::from_raw(boxed.object)
    }
}

#[cfg(feature = "stream")]
impl futures_core::Stream for SwiftStream {
    type Item = Result<SwiftValue, CallError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.get_mut().poll_next_element(cx)
    }
}
