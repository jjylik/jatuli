//! A minimal spinlock for guarding shared state.
//!
//! `GlobalAlloc`'s methods take `&self`, so the allocator needs interior
//! mutability and must be `Sync`. On our single core with interrupts disabled
//! this lock never actually contends, but it is the smallest sound primitive
//! and we will reuse it as the kernel grows.

use core::cell::UnsafeCell;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{AtomicBool, Ordering};

/// A spinlock wrapping a value of type `A`.
pub struct Locked<A> {
    locked: AtomicBool,
    inner: UnsafeCell<A>,
}

// SAFETY: all access to `inner` goes through `lock()`, which serializes callers
// via the `locked` flag, so the wrapped value can be shared across contexts.
unsafe impl<A: Send> Sync for Locked<A> {}

impl<A> Locked<A> {
    /// Create a new, unlocked `Locked<A>`.
    pub const fn new(inner: A) -> Self {
        Self {
            locked: AtomicBool::new(false),
            inner: UnsafeCell::new(inner),
        }
    }

    /// Acquire the lock, spinning until it is free, and return a guard.
    pub fn lock(&self) -> LockGuard<'_, A> {
        while self
            .locked
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            core::hint::spin_loop();
        }
        LockGuard { lock: self }
    }
}

/// RAII guard returned by [`Locked::lock`]; releases the lock when dropped.
pub struct LockGuard<'a, A> {
    lock: &'a Locked<A>,
}

impl<A> Deref for LockGuard<'_, A> {
    type Target = A;

    fn deref(&self) -> &A {
        // SAFETY: holding the guard means we hold the lock, so the reference is exclusive.
        unsafe { &*self.lock.inner.get() }
    }
}

impl<A> DerefMut for LockGuard<'_, A> {
    fn deref_mut(&mut self) -> &mut A {
        // SAFETY: holding the guard means we hold the lock, so the reference is exclusive.
        unsafe { &mut *self.lock.inner.get() }
    }
}

impl<A> Drop for LockGuard<'_, A> {
    fn drop(&mut self) {
        self.lock.locked.store(false, Ordering::Release);
    }
}
