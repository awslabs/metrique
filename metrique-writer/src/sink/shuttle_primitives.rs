// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Cfg-gated concurrency primitives used by [`super::background`] (crossbeam/std
//! vs. shuttle).
//!
//! Normally this re-exports the production crossbeam types. With `--cfg shuttle`
//! it substitutes shuttle-native equivalents: shuttle has no visibility into
//! crossbeam's (or plain `std::sync::atomic`'s) internal atomics -- also why
//! `Arc` itself isn't swapped: nothing depends on *when* its refcount hits
//! zero here. `ArrayQueue`'s substitute lives in
//! `metrique_writer_core::shuttle_test_support` (shared with `metrique-util`,
//! which needs the same shim); `Parker`/`Unparker` stay local, unique to this
//! crate.
//!
//! Gated on `feature = "_shuttle"` too, not `cfg(shuttle)` alone: `--cfg shuttle`
//! is set process-wide via RUSTFLAGS, so it also reaches builds of this crate
//! (e.g. as a dev-dependency with different requested features) that don't have
//! `_shuttle` enabled and therefore don't have the optional `shuttle` crate
//! linked at all.

#[cfg(not(all(shuttle, feature = "_shuttle")))]
pub(crate) use std::{
    sync::atomic::{AtomicBool, Ordering},
    sync::mpsc,
    thread,
};

#[cfg(not(all(shuttle, feature = "_shuttle")))]
pub(crate) use crossbeam_queue::ArrayQueue;
#[cfg(not(all(shuttle, feature = "_shuttle")))]
pub(crate) use crossbeam_utils::sync::{Parker, Unparker};

#[cfg(all(shuttle, feature = "_shuttle"))]
pub(crate) use shuttle::{
    sync::atomic::{AtomicBool, Ordering},
    sync::mpsc,
    thread,
};

#[cfg(all(shuttle, feature = "_shuttle"))]
pub(crate) use metrique_writer_core::shuttle_test_support::ArrayQueue;
#[cfg(all(shuttle, feature = "_shuttle"))]
pub(crate) use shuttle_impl::{Parker, Unparker};

#[cfg(all(shuttle, feature = "_shuttle"))]
mod shuttle_impl {
    use std::time::Instant;

    /// Shuttle-visible substitute for `crossbeam_utils::sync::{Parker, Unparker}`.
    /// Not built on `shuttle::thread::park`/`Thread::unpark`: those are bound to
    /// a specific OS thread, but `background.rs` constructs the `Parker` on one
    /// thread and parks on another. A `Mutex<bool>` + `Condvar` token shared via
    /// `Arc` matches crossbeam's actual (thread-identity-agnostic) semantics
    /// instead -- an earlier version bound to `shuttle::thread::current()` and
    /// deadlocked for exactly this reason.
    pub(crate) struct Parker {
        inner: std::sync::Arc<TokenState>,
    }

    struct TokenState {
        available: shuttle::sync::Mutex<bool>,
        condvar: shuttle::sync::Condvar,
    }

    impl Default for Parker {
        fn default() -> Self {
            Self {
                inner: std::sync::Arc::new(TokenState {
                    available: shuttle::sync::Mutex::new(false),
                    condvar: shuttle::sync::Condvar::new(),
                }),
            }
        }
    }

    impl Parker {
        pub(crate) fn unparker(&self) -> Unparker {
            Unparker {
                inner: self.inner.clone(),
            }
        }

        /// Shuttle doesn't model time, so a deadline-based park just parks
        /// unboundedly here (matching shuttle's own `thread::park_timeout`).
        /// Fine for what these tests check: push/pop/wake correctness with a
        /// `flush_interval` long enough that the real deadline never fires.
        /// Real-time behavior stays covered by the non-shuttle tests.
        pub(crate) fn park_deadline(&self, _deadline: Instant) {
            let mut available = self.inner.available.lock().unwrap();
            while !*available {
                available = self.inner.condvar.wait(available).unwrap();
            }
            *available = false;
        }
    }

    #[derive(Clone)]
    pub(crate) struct Unparker {
        inner: std::sync::Arc<TokenState>,
    }

    impl Unparker {
        pub(crate) fn unpark(&self) {
            let mut available = self.inner.available.lock().unwrap();
            *available = true;
            self.inner.condvar.notify_one();
        }
    }
}
