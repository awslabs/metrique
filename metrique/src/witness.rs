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
/// need to call [`snapshot`](Witness::snapshot) explicitly if your
/// request handler needs the value for its own logic (e.g. branching
/// on a feature flag); calling `snapshot` early also pins the captured
/// value to that point rather than emission time.
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
/// has its own snapshot slot: the first call to [`snapshot`](Witness::snapshot)
/// captures the current value, and all subsequent calls on that handle
/// return the same `Arc<T>`. Calling [`clone`](Clone::clone) produces a
/// fresh handle with an empty snapshot slot.
///
/// The typical pattern is: keep one long-lived `Witness` for background
/// writers to [`store`](Witness::store) into, and [`clone`](Clone::clone)
/// a handle per request so each request gets its own snapshot.
///
/// For hot paths that need the latest value (bypassing the snapshot and
/// `Arc` clone), use [`latest`](Witness::latest).
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
/// // First snapshot captures "v1".
/// assert_eq!(*request.snapshot(), "v1");
///
/// // Background task updates the shared state.
/// shared.store(Arc::new(String::from("v2")));
///
/// // The request handle still sees "v1".
/// assert_eq!(*request.snapshot(), "v1");
///
/// // latest() always sees the current value.
/// assert_eq!(*request.latest(), "v2");
///
/// // A new clone captures the updated value.
/// let next_request = shared.clone();
/// assert_eq!(*next_request.snapshot(), "v2");
/// ```
pub struct Witness<T> {
    swap: Arc<arc_swap::ArcSwap<T>>,
    snap: OnceLock<Arc<T>>,
}

/// A cheap, short-lived reference returned by [`Witness::latest`].
///
/// Always reads the latest value (bypasses the snapshot). This means
/// that the guarded value might differ from metrics emitted by its related
/// [`Witness`].
///
/// Derefs to `T` without cloning. Not `Send`; for cross-task use, call
/// [`snapshot`](Witness::snapshot) instead.
pub struct LatestRef<T>(arc_swap::Guard<Arc<T>>);

impl<T> std::ops::Deref for LatestRef<T> {
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
            snap: OnceLock::new(),
        }
    }

    /// Atomically replace the shared value.
    ///
    /// All handles (including this one) will see the new value on their
    /// next [`snapshot`](Witness::snapshot), unless they have already
    /// captured one. A handle that has already called `snapshot` is
    /// unaffected; its captured value is immutable.
    pub fn store(&self, val: Arc<T>) {
        self.swap.store(val);
    }

    /// Capture and return a snapshot of the current value.
    ///
    /// The first call captures the value; subsequent calls return the
    /// same `Arc<T>`.
    pub fn snapshot(&self) -> Arc<T> {
        self.snap.get_or_init(|| self.swap.load_full()).clone()
    }

    /// Get a cheap guard for the latest shared value, bypassing the snapshot.
    /// The returned value may differ from what [`snapshot`](Witness::snapshot)
    /// returns (and from what metrics will emit on close).
    ///
    /// Use [`snapshot`](Witness::snapshot) for a `Send` handle or when you
    /// need snapshot consistency.
    ///
    /// Returns a [`LatestRef`] that derefs to `T`.
    pub fn latest(&self) -> LatestRef<T> {
        LatestRef(self.swap.load())
    }
}

impl<T> Clone for Witness<T> {
    /// Clone produces a fresh handle to the same shared value, without a
    /// captured snapshot.
    fn clone(&self) -> Self {
        Self {
            swap: self.swap.clone(),
            snap: OnceLock::new(),
        }
    }
}

impl<T: std::fmt::Debug> std::fmt::Debug for Witness<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut d = f.debug_struct("Witness");
        d.field("current", &*self.swap.load());
        if let Some(snap) = self.snap.get() {
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
        Arc::unwrap_or_clone(self.snapshot()).close()
    }
}

#[diagnostic::do_not_recommend]
impl<T> CloseValue for &'_ Witness<T>
where
    T: Clone + CloseValue,
{
    type Closed = T::Closed;

    fn close(self) -> Self::Closed {
        Arc::unwrap_or_clone(self.snapshot()).close()
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
    fn first_snapshot_captures() {
        let x = Witness::new(42u64);
        assert_eq!(*x.snapshot(), 42);
        x.store(Arc::new(100));
        // Still returns the captured value.
        assert_eq!(*x.snapshot(), 42);
    }

    #[test]
    fn store_before_snapshot() {
        let x = Witness::new(42u64);
        x.store(Arc::new(100));
        assert_eq!(*x.snapshot(), 100);
    }

    #[test]
    fn store_after_snapshot_updates_shared() {
        let x = Witness::new(42u64);
        x.snapshot();
        x.store(Arc::new(100));
        // Snapshot is unchanged.
        assert_eq!(*x.snapshot(), 42);
        // But a fresh clone sees the new value.
        assert_eq!(*x.clone().snapshot(), 100);
    }

    #[test]
    fn latest_sees_current() {
        let x = Witness::new(42u64);
        x.store(Arc::new(100));
        assert_eq!(*x.latest(), 100);
    }

    #[test]
    fn clone_gets_fresh_snapshot() {
        let x = Witness::new(42u64);
        x.snapshot(); // capture 42

        let writer = x.clone();
        writer.store(Arc::new(100));

        let reader = x.clone();
        assert_eq!(*reader.snapshot(), 100);
        // Original still has 42.
        assert_eq!(*x.snapshot(), 42);
    }

    #[test]
    fn clone_shares_swap() {
        let x = Witness::new(42u64);
        let cloned = x.clone();
        x.store(Arc::new(100));
        assert_eq!(*x.latest(), 100);
        assert_eq!(*cloned.latest(), 100);
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
        x.snapshot();
        let dbg = format!("{:?}", x);
        assert!(dbg.contains("snapshot"));
    }
}
