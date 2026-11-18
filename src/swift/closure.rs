//! Rust closures passed to Swift as Swift closure values.
//!
//! A closure value is two words: the function and its context. The context is a Swift heap object holding the Rust
//! closure (sys/csrc/funclets.c, `ClosureBox`), so Swift retains, keeps and releases it like any closure context; the
//! Rust closure is dropped when the last reference goes. The function is one of apus's entries, which receive every
//! argument register: the Rust parameter and result types say which registers the Swift function type uses, as for
//! ordinary calls. `swift!` checks the closure against the declared function type before the call
//! (`CallError::ClosureMismatch`): async or sync, throws, `@MainActor`, and which values go by address.
//!
//! An async closure runs inside the Swift Task that calls it, as a Rust task of that Task (the same path as a spawned
//! task's future: calls it makes run in that Task, `.child()` calls in child Tasks, and cancelling the Task reaches it).
//! A panic in the closure aborts the process: Swift has no way to receive it.

use super::any::SwiftError;
use super::call::{CallError, Owned};
use super::func::{ArgLowering, SwiftArg, SwiftObject};
use super::{rt, selfcheck, task};
use std::ffi::c_void;
use std::future::Future;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::pin::Pin;
use std::ptr::NonNull;

use super::func::{FLOAT_REGISTERS, INT_REGISTERS};

type BoxFuture = Pin<Box<dyn Future<Output = ()> + Send>>;

/// A Rust closure as a Swift closure value, for a Swift parameter of function type.
///
/// ```ignore
/// apus::swift! {
///     #[swift = "Probe.applyConcrete(_: (Int) async -> Int) async -> Int"]
///     async fn apply_concrete(f: &SwiftClosure) -> i64;
/// }
/// let double = SwiftClosure::new_async(|x: i64| async move { x * 2 });
/// let n = apply_concrete(&double).await?;
/// ```
pub struct SwiftClosure {
    function: *const c_void,
    /// The context object (+1).
    context: NonNull<rt::ClosureBox>,
    shape: Shape,
}

unsafe impl Send for SwiftClosure {}
unsafe impl Sync for SwiftClosure {}

impl Drop for SwiftClosure {
    fn drop(&mut self) {
        unsafe { rt::swift_release(self.context.as_ptr().cast()) }
    }
}

impl std::fmt::Debug for SwiftClosure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SwiftClosure({})", self.shape.render())
    }
}

/// What the Rust closure looks like to Swift; compared with the declared function type.
#[derive(Clone, Debug)]
struct Shape {
    is_async: bool,
    throws: bool,
    main_actor: bool,
    /// Per parameter: passed by address.
    indirect: Vec<bool>,
    result: ResultKind,
}

impl Shape {
    /// The form `apus_mangle::closure_shapes` gives a declaration: `a(di)d`.
    fn render(&self) -> String {
        let mut out = String::from(if self.is_async { "a" } else { "s" });
        if self.throws {
            out.push('t');
        }
        if self.main_actor {
            out.push('m');
        }
        out.push('(');
        out.extend(self.indirect.iter().map(|&i| if i { 'i' } else { 'd' }));
        out.push(')');
        out.push(match self.result {
            ResultKind::Void => '-',
            ResultKind::Indirect => 'i',
            ResultKind::Ints | ResultKind::Floats => 'd',
        });
        out
    }

    /// `?` in the declared shape matches either.
    fn matches(&self, declared: &str) -> bool {
        let ours = self.render();
        ours.len() == declared.len() && ours.chars().zip(declared.chars()).all(|(a, b)| a == b || (b == '?' && matches!(a, 'i' | 'd')))
    }
}

/// How a closure result goes back to Swift.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResultKind {
    Void,
    Ints,
    Floats,
    /// Written to the address the caller passes.
    Indirect,
}

/// The arguments of one call, read in declaration order.
#[doc(hidden)]
pub struct ClosureArgs<'a> {
    frame: &'a rt::ClosureFrame,
    ints: usize,
    floats: usize,
}

impl ClosureArgs<'_> {
    pub fn int(&mut self) -> u64 {
        let value = self.frame.gpr[self.ints];
        self.ints += 1;
        value
    }

    pub fn float(&mut self) -> f64 {
        let value = self.frame.fpr[self.floats];
        self.floats += 1;
        value
    }
}

/// Where one call's result goes.
#[doc(hidden)]
pub struct ClosureOut<'a> {
    ret: &'a mut rt::RetRegs,
    indirect: *mut c_void,
}

/// A Rust type a Swift closure parameter can be read as. The value is borrowed from Swift (guaranteed) and copied out.
pub trait ClosureParam: Sized + Send + 'static {
    /// Integer and floating-point registers it takes (an address takes one integer register).
    #[doc(hidden)]
    const REGISTERS: (usize, usize);
    #[doc(hidden)]
    const INDIRECT: bool = false;
    #[doc(hidden)]
    unsafe fn take(args: &mut ClosureArgs<'_>) -> Self;
}

/// A Rust type a Swift closure result can be given as. `Result<R, SwiftError>` makes the closure throwing.
pub trait ClosureResult: Send + 'static {
    #[doc(hidden)]
    const KIND: ResultKind;
    #[doc(hidden)]
    const THROWS: bool = false;
    #[doc(hidden)]
    unsafe fn put(self, out: &mut ClosureOut<'_>);
}

macro_rules! int_param {
    ($($t:ty),*) => {$(
        impl ClosureParam for $t {
            const REGISTERS: (usize, usize) = (1, 0);
            unsafe fn take(args: &mut ClosureArgs<'_>) -> Self {
                args.int() as $t
            }
        }
        impl ClosureResult for $t {
            const KIND: ResultKind = ResultKind::Ints;
            unsafe fn put(self, out: &mut ClosureOut<'_>) {
                out.ret.gpr[0] = self as u64;
            }
        }
    )*};
}
int_param!(i64, u64, i32, u32, i16, u16, i8, u8, isize, usize);

impl ClosureParam for bool {
    const REGISTERS: (usize, usize) = (1, 0);
    unsafe fn take(args: &mut ClosureArgs<'_>) -> Self {
        args.int() & 1 == 1
    }
}

impl ClosureResult for bool {
    const KIND: ResultKind = ResultKind::Ints;
    unsafe fn put(self, out: &mut ClosureOut<'_>) {
        out.ret.gpr[0] = self as u64;
    }
}

impl ClosureParam for f64 {
    const REGISTERS: (usize, usize) = (0, 1);
    unsafe fn take(args: &mut ClosureArgs<'_>) -> Self {
        args.float()
    }
}

impl ClosureResult for f64 {
    const KIND: ResultKind = ResultKind::Floats;
    unsafe fn put(self, out: &mut ClosureOut<'_>) {
        out.ret.fpr[0] = self;
    }
}

/// A `Float` uses the low half of its floating-point register.
impl ClosureParam for f32 {
    const REGISTERS: (usize, usize) = (0, 1);
    unsafe fn take(args: &mut ClosureArgs<'_>) -> Self {
        f32::from_bits(args.float().to_bits() as u32)
    }
}

impl ClosureResult for f32 {
    const KIND: ResultKind = ResultKind::Floats;
    unsafe fn put(self, out: &mut ClosureOut<'_>) {
        out.ret.fpr[0] = f64::from_bits(self.to_bits() as u64);
    }
}

impl ClosureParam for String {
    const REGISTERS: (usize, usize) = (2, 0);
    unsafe fn take(args: &mut ClosureArgs<'_>) -> Self {
        let string = rt::SwiftString { words: [args.int(), args.int()] };
        rt::string_to_rust(&string)
    }
}

/// Returned owned (+1), as Swift expects.
impl ClosureResult for String {
    const KIND: ResultKind = ResultKind::Ints;
    unsafe fn put(self, out: &mut ClosureOut<'_>) {
        let string = rt::string_from_utf8(&self);
        out.ret.gpr[0] = string.words[0];
        out.ret.gpr[1] = string.words[1];
    }
}

impl ClosureParam for SwiftObject {
    const REGISTERS: (usize, usize) = (1, 0);
    unsafe fn take(args: &mut ClosureArgs<'_>) -> Self {
        SwiftObject::from_raw(rt::swift_unknownObjectRetain(args.int() as *mut c_void))
    }
}

impl ClosureResult for SwiftObject {
    const KIND: ResultKind = ResultKind::Ints;
    unsafe fn put(self, out: &mut ClosureOut<'_>) {
        out.ret.gpr[0] = self.into_raw() as u64;
    }
}

impl ClosureResult for () {
    const KIND: ResultKind = ResultKind::Void;
    unsafe fn put(self, _out: &mut ClosureOut<'_>) {}
}

/// A throwing closure: an `Err` is thrown to the Swift caller.
impl<R: ClosureResult> ClosureResult for Result<R, SwiftError> {
    const KIND: ResultKind = R::KIND;
    const THROWS: bool = true;
    unsafe fn put(self, out: &mut ClosureOut<'_>) {
        match self {
            Ok(value) => value.put(out),
            Err(error) => out.ret.error = error.into_raw(),
        }
    }
}

/// A value Swift passes by address: a parameter or result whose declared type is a generic parameter (`(T) async -> T`),
/// read and written as `T`'s Rust layout. `T` must have the layout of the Swift type the caller uses, and nothing to
/// retain or release (`Int`, `Double`, `Bool` as `i64`, `f64`, `bool`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ByAddress<T>(pub T);

impl<T: Copy + Send + 'static> ClosureParam for ByAddress<T> {
    const REGISTERS: (usize, usize) = (1, 0);
    const INDIRECT: bool = true;
    unsafe fn take(args: &mut ClosureArgs<'_>) -> Self {
        ByAddress(std::ptr::read(args.int() as *const T))
    }
}

impl<T: Copy + Send + 'static> ClosureResult for ByAddress<T> {
    const KIND: ResultKind = ResultKind::Indirect;
    unsafe fn put(self, out: &mut ClosureOut<'_>) {
        std::ptr::write(out.indirect as *mut T, self.0);
    }
}

/// The Rust closure behind a context object: reads the arguments, and either runs (sync) or returns the future that
/// writes the result when it completes (async).
struct Payload {
    call: Box<dyn Fn(*mut rt::ClosureFrame) -> Option<BoxFuture> + Send + Sync>,
    main: bool,
}

/// A frame pointer the async closure's future writes its result through: the frame lives in Swift's callee context,
/// which stays until the closure returns.
struct FramePtr(*mut rt::ClosureFrame);
unsafe impl Send for FramePtr {}

fn abort_on_panic<T>(f: impl FnOnce() -> T) -> T {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or_else(|_| {
        eprintln!("apus: a Rust closure called by Swift panicked; Swift cannot receive a panic, aborting");
        std::process::abort()
    })
}

/// Rust closures of one arity, async or sync.
#[doc(hidden)]
pub trait ClosureFn<Args>: Send + Sync + 'static {
    type Output;
    fn params() -> Vec<(bool, (usize, usize))>;
    /// Reads the arguments and calls the closure: its result, or its future.
    unsafe fn invoke(&self, args: &mut ClosureArgs<'_>) -> Self::Output;
}

macro_rules! closure_arity {
    ($($a:ident)*) => {
        impl<Func, Ret, $($a: ClosureParam),*> ClosureFn<($($a,)*)> for Func
        where
            Func: Fn($($a),*) -> Ret + Send + Sync + 'static,
        {
            type Output = Shaped<Ret>;
            fn params() -> Vec<(bool, (usize, usize))> {
                vec![$(($a::INDIRECT, $a::REGISTERS)),*]
            }
            #[allow(unused_variables, non_snake_case)]
            unsafe fn invoke(&self, args: &mut ClosureArgs<'_>) -> Shaped<Ret> {
                $(let $a = $a::take(args);)*
                Shaped(self($($a),*))
            }
        }
    };
}

/// The closure's own return value, before it is known to be a result or a future.
#[doc(hidden)]
pub struct Shaped<R>(R);

closure_arity!();
closure_arity!(A);
closure_arity!(A B);
closure_arity!(A B C);
closure_arity!(A B C D);
closure_arity!(A B C D E);
closure_arity!(A B C D E F);

impl SwiftClosure {
    /// An async Swift closure. `f` takes the parameters as [`ClosureParam`] types and returns a future of a
    /// [`ClosureResult`]; it may be called many times, at once, from any thread.
    pub fn new_async<Args, F, Fut>(f: F) -> SwiftClosure
    where
        F: ClosureFn<Args, Output = Shaped<Fut>>,
        Fut: Future + Send + 'static,
        Fut::Output: ClosureResult,
    {
        let shape = Self::shape::<Args, F>(true, Fut::Output::KIND, Fut::Output::THROWS);
        let call = move |frame: *mut rt::ClosureFrame| -> Option<BoxFuture> {
            let future = abort_on_panic(|| unsafe {
                // An indirect result's address comes first, in the first integer register.
                let indirect = Fut::Output::KIND == ResultKind::Indirect;
                let mut args = ClosureArgs { frame: &*frame, ints: indirect as usize, floats: 0 };
                f.invoke(&mut args).0
            });
            let frame = FramePtr(frame);
            Some(Box::pin(async move {
                let frame = frame;
                let output = futures_catch(future).await;
                unsafe {
                    let frame = &mut *frame.0;
                    let indirect = frame.gpr[0] as *mut c_void;
                    output.put(&mut ClosureOut { ret: &mut frame.ret, indirect });
                }
            }))
        };
        Self::build(shape, Box::new(call), unsafe { rt::apus_closure_async_fn() })
    }

    /// A synchronous Swift closure: `f` runs on the calling thread.
    pub fn new_sync<Args, F, R>(f: F) -> SwiftClosure
    where
        F: ClosureFn<Args, Output = Shaped<R>>,
        R: ClosureResult,
    {
        let shape = Self::shape::<Args, F>(false, R::KIND, R::THROWS);
        let call = move |frame: *mut rt::ClosureFrame| -> Option<BoxFuture> {
            abort_on_panic(|| unsafe {
                let frame = &mut *frame;
                let output = f.invoke(&mut ClosureArgs { frame, ints: 0, floats: 0 }).0;
                output.put(&mut ClosureOut { indirect: frame.indirect, ret: &mut frame.ret });
            });
            None
        };
        Self::build(shape, Box::new(call), unsafe {
            rt::apus_closure_sync_fn(match R::KIND {
                ResultKind::Floats => 1,
                ResultKind::Indirect => 2,
                ResultKind::Void | ResultKind::Ints => 0,
            })
        })
    }

    /// Marks the closure `@MainActor`: an async one runs on the main actor (Swift's caller does not hop for it), and
    /// it matches only a `@MainActor` function type. Panics once the closure has been passed to Swift and Swift
    /// kept it: the change would reach Swift's copy while it may be running.
    pub fn on_main_actor(mut self) -> SwiftClosure {
        let context = self.context.as_ptr();
        assert!(
            unsafe { rt::swift_isUniquelyReferenced_nonNull_native(context.cast()) },
            "apus: on_main_actor on a closure Swift already holds"
        );
        self.shape.main_actor = true;
        unsafe {
            (*context).flags |= rt::CLOSURE_MAIN_ACTOR;
            (*((*context).payload as *mut Payload)).main = true;
        }
        self
    }

    fn shape<Args, F: ClosureFn<Args>>(is_async: bool, result: ResultKind, throws: bool) -> Shape {
        let params = F::params();
        let indirect_result = is_async && result == ResultKind::Indirect;
        let ints: usize = params.iter().map(|(_, (i, _))| i).sum::<usize>() + indirect_result as usize;
        let floats: usize = params.iter().map(|(_, (_, f))| f).sum();
        assert!(
            ints <= INT_REGISTERS && floats <= FLOAT_REGISTERS,
            "apus: a closure's arguments must fit the argument registers ({INT_REGISTERS} integer, {FLOAT_REGISTERS} floating-point)"
        );
        Shape { is_async, throws, main_actor: false, indirect: params.iter().map(|(i, _)| *i).collect(), result }
    }

    fn build(shape: Shape, call: Box<dyn Fn(*mut rt::ClosureFrame) -> Option<BoxFuture> + Send + Sync>, function: *const c_void) -> SwiftClosure {
        selfcheck::ensure();
        let payload = Box::into_raw(Box::new(Payload { call, main: false }));
        let context = unsafe { rt::apus_closure_new(payload.cast(), 0) };
        SwiftClosure { function, context: NonNull::new(context.cast()).expect("swift_allocObject"), shape }
    }
}

/// Awaits `future`, aborting on a panic (see `abort_on_panic`).
async fn futures_catch<F: Future>(future: F) -> F::Output {
    let mut future = std::pin::pin!(future);
    std::future::poll_fn(|cx| abort_on_panic(|| future.as_mut().poll(cx))).await
}

impl SwiftArg for &SwiftClosure {
    /// Two words, the context passed guaranteed: held until the call is over.
    fn lower(&self, into: &mut ArgLowering) {
        into.int(self.function as u64);
        let context = unsafe { rt::swift_retain(self.context.as_ptr().cast()) };
        into.owned.push(Owned::Native(context));
        into.int(context as u64);
    }
}

/// A closure argument with the shape `swift!` read from the declared function type.
#[doc(hidden)]
pub struct DeclaredClosure<'a>(pub &'a SwiftClosure, pub &'static str);

impl SwiftArg for DeclaredClosure<'_> {
    fn lower(&self, into: &mut ArgLowering) {
        if !self.0.shape.matches(self.1) {
            into.fail(CallError::ClosureMismatch(format!(
                "the closure is `{}`, the declared function type `{}` (a async, s sync, t throws, m @MainActor; i by address, d in registers)",
                self.0.shape.render(),
                self.1
            )));
        }
        (&self.0).lower(into);
    }
}

#[export_name = apus_sys::boundary_name!(rs_closure_start)]
extern "C" fn rs_closure_start(payload: *mut c_void, frame: *mut rt::ClosureFrame) -> *mut c_void {
    let payload = unsafe { &*(payload as *const Payload) };
    let future = (payload.call)(frame).expect("an async closure's entry");
    task::closure_task(payload.main, future)
}

#[export_name = apus_sys::boundary_name!(rs_closure_call_sync)]
extern "C" fn rs_closure_call_sync(payload: *mut c_void, frame: *mut rt::ClosureFrame) {
    let payload = unsafe { &*(payload as *const Payload) };
    (payload.call)(frame);
}

#[export_name = apus_sys::boundary_name!(rs_closure_drop)]
extern "C" fn rs_closure_drop(payload: *mut c_void) {
    abort_on_panic(|| drop(unsafe { Box::from_raw(payload as *mut Payload) }));
}
