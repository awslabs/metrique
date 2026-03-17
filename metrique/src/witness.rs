// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! An atomically swappable value with snapshot-on-first-read semantics.

use std::sync::{Arc, OnceLock};

use crate::CloseValue;

/// An atomically swappable value. The first call to [`load`](Witness::load)
/// captures the current value; all subsequent loads return that same
/// `Arc<T>`, even if the underlying value is updated.
///
/// This makes `Witness` suitable to use in metrics structs: the emitted
/// metric always reflects the state that was seen during processing.
///
/// For hot paths that need to see the latest value and avoid overhead,
/// (bypassing the snapshot, Arc clone, etc), use [`.shared_ref()`](Witness::shared_ref).
///
/// ```
/// use std::sync::Arc;
/// use metrique::Witness;
///
/// let shared = Witness::new(String::from("v1"));
///
/// // Clone for a per-request handle.
/// let request = shared.clone();
///
/// // First load captures "v1".
/// assert_eq!(*request.load(), "v1");
///
/// // Update the shared state.
/// shared.store(Arc::new(String::from("v2"))).unwrap();
///
/// // The request handle still sees "v1".
/// assert_eq!(*request.load(), "v1");
///
/// // shared_ref always sees the latest.
/// assert_eq!(*request.shared_ref(), "v2");
/// ```
pub struct Witness<T> {
    swap: Arc<arc_swap::ArcSwap<T>>,
    snapshot: OnceLock<Arc<T>>,
}

/// A cheap, short-lived reference returned by [`Witness::shared_ref`].
///
/// Always reads the latest value (bypasses the snapshot). This means
/// that the guarded value might differ from metrics emitted by its related
/// [`Witness`].
///
/// Derefs to `T` without cloning. Not `Send`; for cross-task use, call
/// [`load`](Witness::load) instead.
pub struct SharedRef<T>(arc_swap::Guard<Arc<T>>);

impl<T> std::ops::Deref for SharedRef<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.0
    }
}

impl<T> Witness<T> {
    /// Create a new `Witness` from an initial value.
    pub fn new(val: T) -> Self {
        Self {
            swap: Arc::new(arc_swap::ArcSwap::from_pointee(val)),
            snapshot: OnceLock::new(),
        }
    }

    /// Atomically replace the value. Returns `Err(val)` if this handle
    /// has already captured a snapshot via [`load`](Witness::load).
    pub fn store(&self, val: Arc<T>) -> Result<(), Arc<T>> {
        if self.snapshot.get().is_some() {
            return Err(val);
        }
        self.swap.store(val);
        Ok(())
    }

    /// Load the value. The first call captures a snapshot; subsequent
    /// calls return the same `Arc<T>`.
    pub fn load(&self) -> Arc<T> {
        self.snapshot.get_or_init(|| self.swap.load_full()).clone()
    }

    /// Get a cheap guard for the underlying value, bypassing the snapshot.
    /// This might be different from emitted metrics for the value on close.
    ///
    /// Use [`Self::load`] for a `Send` handle or if you need snapshot properties.
    ///
    /// Returns a [`SharedRef`] that derefs to `T`.
    pub fn shared_ref(&self) -> SharedRef<T> {
        SharedRef(self.swap.load())
    }
}

impl<T> Clone for Witness<T> {
    /// Clone produces a fresh handle to the same swap, without a
    /// captured snapshot.
    fn clone(&self) -> Self {
        Self {
            swap: self.swap.clone(),
            snapshot: OnceLock::new(),
        }
    }
}

impl<T: std::fmt::Debug> std::fmt::Debug for Witness<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut d = f.debug_struct("Witness");
        d.field("current", &*self.swap.load());
        if let Some(snap) = self.snapshot.get() {
            d.field("snapshot", snap);
        }
        d.finish()
    }
}

#[diagnostic::do_not_recommend]
impl<T> CloseValue for Witness<T>
where
    T: Clone + CloseValue,
{
    type Closed = T::Closed;

    fn close(self) -> Self::Closed {
        Arc::unwrap_or_clone(self.load()).close()
    }
}

#[diagnostic::do_not_recommend]
impl<T> CloseValue for &'_ Witness<T>
where
    T: Clone + CloseValue,
{
    type Closed = T::Closed;

    fn close(self) -> Self::Closed {
        Arc::unwrap_or_clone(self.load()).close()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::CloseValue;

    use super::Witness;

    #[derive(Clone, Debug)]
    struct Closeable;
    impl CloseValue for Closeable {
        type Closed = u64;
        fn close(self) -> u64 {
            42
        }
    }

    #[test]
    fn close_ref() {
        let x = Witness::new(Closeable);
        assert_eq!((&x).close(), 42);
    }

    #[test]
    fn close_owned() {
        let x = Witness::new(Closeable);
        assert_eq!(x.close(), 42);
    }

    #[test]
    fn first_load_captures() {
        let x = Witness::new(42u64);
        assert_eq!(*x.load(), 42);
        x.store(Arc::new(100)).unwrap_err();
        // Still returns the captured value.
        assert_eq!(*x.load(), 42);
    }

    #[test]
    fn store_before_load() {
        let x = Witness::new(42u64);
        x.store(Arc::new(100)).unwrap();
        assert_eq!(*x.load(), 100);
    }

    #[test]
    fn store_after_load_returns_err() {
        let x = Witness::new(42u64);
        x.load();
        assert!(x.store(Arc::new(100)).is_err());
    }

    #[test]
    fn shared_ref_sees_latest() {
        let x = Witness::new(42u64);
        x.store(Arc::new(100)).unwrap();
        assert_eq!(*x.shared_ref(), 100);
    }

    #[test]
    fn clone_gets_fresh_snapshot() {
        let x = Witness::new(42u64);
        x.load(); // capture 42

        // Store through a fresh clone (no snapshot yet).
        let writer = x.clone();
        writer.store(Arc::new(100)).unwrap();

        let reader = x.clone();
        assert_eq!(*reader.load(), 100);
        // Original still has 42.
        assert_eq!(*x.load(), 42);
    }

    #[test]
    fn clone_shares_swap() {
        let x = Witness::new(42u64);
        let cloned = x.clone();
        x.store(Arc::new(100)).unwrap();
        assert_eq!(*x.shared_ref(), 100);
        assert_eq!(*cloned.shared_ref(), 100);
    }

    #[test]
    fn debug_without_snapshot() {
        let x = Witness::new(42u64);
        let dbg = format!("{:?}", x);
        assert!(dbg.contains("42"));
        assert!(!dbg.contains("snapshot"));
    }

    #[test]
    fn debug_with_snapshot() {
        let x = Witness::new(42u64);
        x.load();
        let dbg = format!("{:?}", x);
        assert!(dbg.contains("snapshot"));
    }
}
