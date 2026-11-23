//! A lock needing neither allocation nor setup: `os_unfair_lock` (<os/lock.h>). On Apple platforms std's `Mutex` is
//! a pthread mutex, initialized and destroyed with each value: an async call's record had three of them.

use std::cell::UnsafeCell;
use std::ops::{Deref, DerefMut};

pub(crate) struct Lock<T> {
    lock: UnsafeCell<libc::os_unfair_lock>,
    value: UnsafeCell<T>,
}

unsafe impl<T: Send> Send for Lock<T> {}
unsafe impl<T: Send> Sync for Lock<T> {}

impl<T> Lock<T> {
    pub(crate) const fn new(value: T) -> Lock<T> {
        Lock { lock: UnsafeCell::new(libc::OS_UNFAIR_LOCK_INIT), value: UnsafeCell::new(value) }
    }

    /// Locks until the guard is dropped, on the thread that locked (as `os_unfair_lock_unlock` requires).
    #[inline]
    pub(crate) fn lock(&self) -> Guard<'_, T> {
        unsafe { libc::os_unfair_lock_lock(self.lock.get()) };
        Guard { lock: self, _not_send: std::marker::PhantomData }
    }

    #[inline]
    pub(crate) fn get_mut(&mut self) -> &mut T {
        self.value.get_mut()
    }
}

pub(crate) struct Guard<'a, T> {
    lock: &'a Lock<T>,
    _not_send: std::marker::PhantomData<*const ()>,
}

impl<T> Deref for Guard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        unsafe { &*self.lock.value.get() }
    }
}

impl<T> DerefMut for Guard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        unsafe { &mut *self.lock.value.get() }
    }
}

impl<T> Drop for Guard<'_, T> {
    #[inline]
    fn drop(&mut self) {
        unsafe { libc::os_unfair_lock_unlock(self.lock.lock.get()) };
    }
}
