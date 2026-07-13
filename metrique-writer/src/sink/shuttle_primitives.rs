// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Cfg-gated concurrency primitives used by [`super::background`] (crossbeam/std vs.
//! shuttle).
//!
//! Under normal compilation this re-exports the production, crossbeam-backed types.
//! With `--cfg shuttle` (and the `_shuttle` feature) it substitutes shuttle-native
//! equivalents instead, so Shuttle's scheduler has visibility into every point where
//! the background queue's threads can interleave. Shuttle only instruments the sync
//! primitives it re-implements itself; it has no visibility into crossbeam's internal
//! atomics, so testing `background.rs` under `--cfg shuttle` without this swap would
//! not actually explore any interleavings inside the queue or the parker.
//!
//! Gated on `feature = "_shuttle"` as well as `cfg(shuttle)`, not `cfg(shuttle)`
//! alone: `--cfg shuttle` is set process-wide via RUSTFLAGS, so it reaches
//! *every* crate `cargo test` compiles, including feature-resolution units of
//! this very crate that don't have `_shuttle` enabled (e.g. reached only as a
//! dev-dependency edge with different requested features, such as
//! `metrique-writer-core`'s own dev-dependency on this crate) and therefore
//! don't have the optional `shuttle` dependency linked at all. Without the
//! feature check, those units would try to name a crate that isn't there.

#[cfg(not(all(shuttle, feature = "_shuttle")))]
pub(crate) use std::{sync::mpsc, thread};

#[cfg(not(all(shuttle, feature = "_shuttle")))]
pub(crate) use crossbeam_queue::ArrayQueue;
#[cfg(not(all(shuttle, feature = "_shuttle")))]
pub(crate) use crossbeam_utils::sync::{Parker, Unparker};

#[cfg(all(shuttle, feature = "_shuttle"))]
pub(crate) use shuttle::{sync::mpsc, thread};

#[cfg(all(shuttle, feature = "_shuttle"))]
pub(crate) use shuttle_impl::{ArrayQueue, Parker, Unparker};

#[cfg(all(shuttle, feature = "_shuttle"))]
mod shuttle_impl {
    use std::{collections::VecDeque, time::Instant};

    /// Shuttle-visible substitute for `crossbeam_queue::ArrayQueue`, backed by a
    /// `shuttle::sync::Mutex` so the scheduler can explore interleavings of
    /// concurrent `force_push`/`pop`. Implements only the surface `background.rs`
    /// actually calls; mirrors `dial9-core`'s `BoundedQueue` shuttle shim.
    pub(crate) struct ArrayQueue<T> {
        capacity: usize,
        inner: shuttle::sync::Mutex<VecDeque<T>>,
    }

    impl<T> ArrayQueue<T> {
        pub(crate) fn new(capacity: usize) -> Self {
            assert!(capacity > 0, "capacity must be non-zero");
            Self {
                capacity,
                inner: shuttle::sync::Mutex::new(VecDeque::with_capacity(capacity)),
            }
        }

        /// Pushes `value`, evicting and returning the oldest entry if already at
        /// capacity — matches `crossbeam_queue::ArrayQueue::force_push`.
        pub(crate) fn force_push(&self, value: T) -> Option<T> {
            let mut queue = self.inner.lock().unwrap();
            let evicted = if queue.len() >= self.capacity {
                queue.pop_front()
            } else {
                None
            };
            queue.push_back(value);
            evicted
        }

        pub(crate) fn pop(&self) -> Option<T> {
            self.inner.lock().unwrap().pop_front()
        }

        pub(crate) fn len(&self) -> usize {
            self.inner.lock().unwrap().len()
        }

        pub(crate) fn capacity(&self) -> usize {
            self.capacity
        }
    }

    /// Shuttle-visible substitute for `crossbeam_utils::sync::{Parker, Unparker}`.
    ///
    /// This is *not* built on `shuttle::thread::park`/`Thread::unpark`: those are
    /// bound to a specific OS thread's identity, but crossbeam's `Parker` isn't --
    /// it's shared token/condvar state that works correctly regardless of which
    /// thread later calls `park()`, which matters here because `background.rs`
    /// constructs the `Parker` on the *calling* thread inside `do_build` and only
    /// moves it into the `Receiver` to be parked on by the spawned background
    /// thread. Binding to `shuttle::thread::current()` at construction time (an
    /// earlier version of this shim did) captures the wrong thread's identity and
    /// deadlocks: `unpark()` would wake a park token nobody was ever waiting on.
    /// A `Mutex<bool>` + `Condvar` token, shared via `Arc` between `Parker` and
    /// every `Unparker` cloned from it, has no such thread-identity dependency.
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

        /// Shuttle does not model time (see `shuttle::thread::park_timeout`, which
        /// shuttle itself implements as a plain `park()` for the same reason), so a
        /// deadline-based park behaves like an unbounded park here. This is fine for
        /// what the shuttle tests check: they exercise push/pop/wake correctness
        /// with a `flush_interval` long enough that the real periodic-flush
        /// deadline never fires during a single (fast, in-memory) test run. The
        /// production crossbeam path — and its real-time behavior — stays covered
        /// by the existing non-shuttle tests.
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
