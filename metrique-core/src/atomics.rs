// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::sync::Arc;

use core::sync::atomic::{AtomicBool, AtomicU8, AtomicU16, AtomicU32, AtomicU64, AtomicUsize};

use crate::CloseValue;

/// A thin wrapper around `AtomicU64` that implements [`CloseValue`](crate::CloseValue).
///
/// This is provided for convenience to avoid the need to specify an ordering. However,
/// all other atomics also implement [`CloseValue`] and can be used directly.
#[derive(Default, Debug)]
pub struct Counter(pub AtomicU64);
impl Counter {
    /// Create a new [`Counter`], initialized a specific value
    pub const fn new(starting_count: u64) -> Self {
        Self(AtomicU64::new(starting_count))
    }

    /// Add 1 to this counter
    pub fn increment(&self) {
        self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    /// Increments the count by 1, returning a guard that decrements the count
    /// on drop, and the new value. Useful for tracking in-flight operations.
    pub fn increment_scoped(&self) -> (CounterGuard<'_>, u64) {
        let count = self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        (CounterGuard(&self.0), count)
    }

    /// Increase the value of this counter by `i`
    pub fn add(&self, i: u64) {
        self.0.fetch_add(i, std::sync::atomic::Ordering::Relaxed);
    }

    /// Set this counter to `i`, discarding the previous value
    pub fn set(&self, i: u64) {
        self.0.store(i, std::sync::atomic::Ordering::SeqCst);
    }

    /// Increments the count by 1, returning an owned guard that decrements the
    /// count on drop, and the new value.
    ///
    /// Unlike [`increment_scoped`](Self::increment_scoped), the returned
    /// [`OwnedCounterGuard`] can be moved across async boundaries or stored
    /// in structs without lifetime constraints.
    pub fn increment_owned(self: &Arc<Self>) -> (OwnedCounterGuard, u64) {
        let count = self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        (
            OwnedCounterGuard {
                counter: Arc::clone(self),
            },
            count,
        )
    }
}

/// A guard that decrements a [`Counter`] when dropped.
///
/// Returned by [`Counter::increment_scoped`].
#[must_use]
pub struct CounterGuard<'a>(&'a AtomicU64);

impl Drop for CounterGuard<'_> {
    fn drop(&mut self) {
        self.0
            .fetch_update(
                std::sync::atomic::Ordering::Relaxed,
                std::sync::atomic::Ordering::Relaxed,
                |v| Some(v.saturating_sub(1)),
            )
            .ok();
    }
}

#[diagnostic::do_not_recommend]
impl CloseValue for &CounterGuard<'_> {
    type Closed = u64;

    fn close(self) -> Self::Closed {
        self.0.load(std::sync::atomic::Ordering::Relaxed)
    }
}

#[diagnostic::do_not_recommend]
impl CloseValue for CounterGuard<'_> {
    type Closed = u64;

    fn close(self) -> Self::Closed {
        (&self).close()
    }
}

/// An owned guard that decrements a [`Counter`] when dropped.
///
/// Unlike [`CounterGuard`], this guard can be moved across async boundaries
/// or stored in structs without lifetime constraints.
///
/// Returned by [`Counter::increment_owned`].
#[must_use]
pub struct OwnedCounterGuard {
    counter: Arc<Counter>,
}

impl Drop for OwnedCounterGuard {
    fn drop(&mut self) {
        self.counter
            .0
            .fetch_update(
                std::sync::atomic::Ordering::Relaxed,
                std::sync::atomic::Ordering::Relaxed,
                |v| Some(v.saturating_sub(1)),
            )
            .ok();
    }
}

#[diagnostic::do_not_recommend]
impl CloseValue for &OwnedCounterGuard {
    type Closed = u64;

    fn close(self) -> Self::Closed {
        self.counter.0.load(std::sync::atomic::Ordering::Relaxed)
    }
}

#[diagnostic::do_not_recommend]
impl CloseValue for OwnedCounterGuard {
    type Closed = u64;

    fn close(self) -> Self::Closed {
        (&self).close()
    }
}

impl CloseValue for &'_ Counter {
    type Closed = u64;

    fn close(self) -> Self::Closed {
        <&AtomicU64>::close(&self.0)
    }
}

impl CloseValue for Counter {
    type Closed = u64;

    fn close(self) -> Self::Closed {
        self.0.close()
    }
}

macro_rules! close_value_atomic {
    (atomic: $atomic: ty, inner: $inner: ty) => {
        impl $crate::CloseValue for &'_ $atomic {
            type Closed = $inner;

            fn close(self) -> Self::Closed {
                self.load(std::sync::atomic::Ordering::Relaxed)
            }
        }

        impl $crate::CloseValue for $atomic {
            type Closed = $inner;

            fn close(self) -> Self::Closed {
                self.load(std::sync::atomic::Ordering::Relaxed)
            }
        }
    };
}

close_value_atomic!(atomic: AtomicU64, inner: u64);
close_value_atomic!(atomic: AtomicU32, inner: u32);
close_value_atomic!(atomic: AtomicU16, inner: u16);
close_value_atomic!(atomic: AtomicU8, inner: u8);
close_value_atomic!(atomic: AtomicUsize, inner: usize);

close_value_atomic!(atomic: AtomicBool, inner: bool);

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    #[test]
    fn increment_scoped() {
        let counter = Counter::new(0);
        let (guard, count) = counter.increment_scoped();
        assert_eq!(count, 1);
        drop(guard);
        assert_eq!(counter.0.load(std::sync::atomic::Ordering::Relaxed), 0);
    }

    #[test]
    fn increment_scoped_static() {
        static COUNTER: Counter = Counter::new(0);
        let (guard, count) = COUNTER.increment_scoped();
        assert_eq!(count, 1);
        drop(guard);
        assert_eq!(COUNTER.0.load(std::sync::atomic::Ordering::Relaxed), 0);
    }

    #[test]
    fn counter_guard_close_value() {
        let counter = Counter::new(0);
        let (guard, _) = counter.increment_scoped();
        // CloseValue reads the current count (1) without decrementing.
        assert_eq!((&guard).close(), 1);
        // Guard still decrements on drop.
        drop(guard);
        assert_eq!(counter.0.load(std::sync::atomic::Ordering::Relaxed), 0);
    }

    #[test]
    fn owned_counter_guard_increment_and_drop() {
        let counter = Arc::new(Counter::new(0));
        let (guard, count) = counter.increment_owned();
        assert_eq!(count, 1);
        assert_eq!(counter.0.load(std::sync::atomic::Ordering::Relaxed), 1);
        drop(guard);
        assert_eq!(counter.0.load(std::sync::atomic::Ordering::Relaxed), 0);
    }

    #[test]
    fn owned_counter_guard_saturates_at_zero() {
        let counter = Arc::new(Counter::new(0));
        let (guard, _) = counter.increment_owned();
        // Manually set to 0 to test saturating_sub
        counter.0.store(0, std::sync::atomic::Ordering::Relaxed);
        drop(guard);
        assert_eq!(counter.0.load(std::sync::atomic::Ordering::Relaxed), 0);
    }

    #[test]
    fn owned_counter_guard_close_value() {
        let counter = Arc::new(Counter::new(0));
        let (guard, _) = counter.increment_owned();
        assert_eq!((&guard).close(), 1);
        // Guard still decrements on drop.
        drop(guard);
        assert_eq!(counter.0.load(std::sync::atomic::Ordering::Relaxed), 0);
    }

    #[test]
    fn owned_counter_guard_move_across_threads() {
        let counter = Arc::new(Counter::new(0));
        let (guard, count) = counter.increment_owned();
        assert_eq!(count, 1);
        let counter_clone = Arc::clone(&counter);
        let handle = std::thread::spawn(move || {
            assert_eq!(
                counter_clone.0.load(std::sync::atomic::Ordering::Relaxed),
                1
            );
            drop(guard);
        });
        handle.join().unwrap();
        assert_eq!(counter.0.load(std::sync::atomic::Ordering::Relaxed), 0);
    }

    #[test]
    fn owned_counter_guard_multiple_guards() {
        let counter = Arc::new(Counter::new(0));
        let (g1, c1) = counter.increment_owned();
        let (g2, c2) = counter.increment_owned();
        assert_eq!(c1, 1);
        assert_eq!(c2, 2);
        drop(g1);
        assert_eq!(counter.0.load(std::sync::atomic::Ordering::Relaxed), 1);
        drop(g2);
        assert_eq!(counter.0.load(std::sync::atomic::Ordering::Relaxed), 0);
    }
}
