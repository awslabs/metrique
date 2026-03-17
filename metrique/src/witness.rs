// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! An atomically swappable shared value where each cloned handle captures a
//! snapshot on first read.

use std::sync::{Arc, OnceLock};

use crate::CloseValue;

/// An atomically swappable shared value where each cloned handle captures a
/// snapshot on first read.
///
/// In services with shared state that changes at runtime (feature flags,
/// config reloads, routing tables), request handlers need to both read the
/// current value and emit metrics reflecting what they saw. `Witness`
/// ensures the value captured for metrics matches the value used during
/// processing, even if a background task swaps in a new value mid-request.
///
/// # Usage
///
/// Put a `Witness<T>` in your metrics struct. Background tasks call
/// [`store`](Witness::store) to swap in new values; each request
/// [`clone`](Clone::clone)s a handle, and the snapshot is captured
/// automatically when the metric is closed for emission. You only
/// need to call [`load`](Witness::load) explicitly if your request
/// handler needs the value for its own logic (e.g. branching on a
/// feature flag); calling `load` early also pins the snapshot to
/// that point rather than emission time.
///
/// ```rust,ignore
/// // Background task refreshes config on a loop.
/// // Each request clones the handle; emitted metrics reflect the
/// // config that was current when the request first read it.
/// #[metrics(rename_all = "PascalCase")]
/// struct RequestMetrics {
///     operation: &'static str,
///     #[metrics(flatten)]
///     app_config: Witness<AppConfig>,
/// }
/// ```
///
/// See the [global-state example] for a complete working version.
///
/// [global-state example]: https://github.com/awslabs/metrique/blob/main/metrique/examples/global-state.rs
///
/// # How it works
///
/// All clones of a `Witness` share the same underlying value. Each clone
/// has its own snapshot slot: the first call to [`load`](Witness::load)
/// captures the current value, and all subsequent loads on that handle
/// return the same `Arc<T>`. Calling [`clone`](Clone::clone) produces a
/// fresh handle with an empty snapshot slot.
///
/// The typical pattern is: keep one long-lived `Witness` for background
/// writers to [`store`](Witness::store) into, and [`clone`](Clone::clone)
/// a handle per request so each request gets its own snapshot.
///
/// For hot paths that need the latest value (bypassing the snapshot and
/// `Arc` clone), use [`shared_ref`](Witness::shared_ref).
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
/// // Background task updates the shared state.
/// shared.store(Arc::new(String::from("v2")));
///
/// // The request handle still sees "v1".
/// assert_eq!(*request.load(), "v1");
///
/// // shared_ref always sees the latest.
/// assert_eq!(*request.shared_ref(), "v2");
///
/// // A new clone captures the updated value.
/// let next_request = shared.clone();
/// assert_eq!(*next_request.load(), "v2");
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

    /// Atomically replace the shared value.
    ///
    /// All handles (including this one) will see the new value on their
    /// next [`load`](Witness::load), unless they have already captured a
    /// snapshot. A handle that has already called `load` is unaffected;
    /// its snapshot is immutable.
    pub fn store(&self, val: Arc<T>) {
        self.swap.store(val);
    }

    /// Load the value. The first call captures a snapshot; subsequent
    /// calls return the same `Arc<T>`.
    pub fn load(&self) -> Arc<T> {
        self.snapshot.get_or_init(|| self.swap.load_full()).clone()
    }

    /// Get a cheap guard for the latest shared value, bypassing the snapshot.
    /// The returned value may differ from what [`load`](Witness::load) returns
    /// (and from what metrics will emit on close).
    ///
    /// Use [`load`](Witness::load) for a `Send` handle or when you need
    /// snapshot consistency.
    ///
    /// Returns a [`SharedRef`] that derefs to `T`.
    pub fn shared_ref(&self) -> SharedRef<T> {
        SharedRef(self.swap.load())
    }
}

impl<T> Clone for Witness<T> {
    /// Clone produces a fresh handle to the same shared value, without a
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
        x.store(Arc::new(100));
        // Still returns the captured value.
        assert_eq!(*x.load(), 42);
    }

    #[test]
    fn store_before_load() {
        let x = Witness::new(42u64);
        x.store(Arc::new(100));
        assert_eq!(*x.load(), 100);
    }

    #[test]
    fn store_after_load_updates_shared() {
        let x = Witness::new(42u64);
        x.load();
        x.store(Arc::new(100));
        // Snapshot is unchanged.
        assert_eq!(*x.load(), 42);
        // But a fresh clone sees the new value.
        assert_eq!(*x.clone().load(), 100);
    }

    #[test]
    fn shared_ref_sees_latest() {
        let x = Witness::new(42u64);
        x.store(Arc::new(100));
        assert_eq!(*x.shared_ref(), 100);
    }

    #[test]
    fn clone_gets_fresh_snapshot() {
        let x = Witness::new(42u64);
        x.load(); // capture 42

        let writer = x.clone();
        writer.store(Arc::new(100));

        let reader = x.clone();
        assert_eq!(*reader.load(), 100);
        // Original still has 42.
        assert_eq!(*x.load(), 42);
    }

    #[test]
    fn clone_shares_swap() {
        let x = Witness::new(42u64);
        let cloned = x.clone();
        x.store(Arc::new(100));
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
