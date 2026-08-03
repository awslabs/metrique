// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Shuttle-only test support shared across this workspace, so sibling
//! crates don't each hand-roll the same shim. Internal only: this might
//! change or be removed in any version, regardless of semver.

use std::collections::VecDeque;

/// Shuttle-visible substitute for `crossbeam_queue::ArrayQueue`, backed by a
/// `shuttle::sync::Mutex` so the scheduler can explore interleavings of
/// concurrent `force_push`/`pop`.
#[doc(hidden)]
pub struct ArrayQueue<T> {
    capacity: usize,
    inner: shuttle::sync::Mutex<VecDeque<T>>,
}

impl<T> ArrayQueue<T> {
    #[doc(hidden)]
    pub fn new(capacity: usize) -> Self {
        assert!(capacity > 0, "capacity must be non-zero");
        Self {
            capacity,
            inner: shuttle::sync::Mutex::new(VecDeque::with_capacity(capacity)),
        }
    }

    /// Pushes `value`, evicting and returning the oldest entry if already at
    /// capacity -- matches `crossbeam_queue::ArrayQueue::force_push`.
    #[doc(hidden)]
    pub fn force_push(&self, value: T) -> Option<T> {
        let mut queue = self.inner.lock().unwrap();
        let evicted = if queue.len() >= self.capacity {
            queue.pop_front()
        } else {
            None
        };
        queue.push_back(value);
        evicted
    }

    #[doc(hidden)]
    pub fn pop(&self) -> Option<T> {
        self.inner.lock().unwrap().pop_front()
    }

    #[doc(hidden)]
    pub fn len(&self) -> usize {
        self.inner.lock().unwrap().len()
    }

    #[doc(hidden)]
    pub fn is_empty(&self) -> bool {
        self.inner.lock().unwrap().is_empty()
    }

    #[doc(hidden)]
    pub fn capacity(&self) -> usize {
        self.capacity
    }
}

/// Whether `now` has reached `deadline`. Real wall-clock time barely
/// advances during a fast shuttle iteration, so `now >= deadline` alone
/// would (almost) never fire.
#[doc(hidden)]
pub fn deadline_reached(now: std::time::Instant, deadline: std::time::Instant) -> bool {
    use shuttle::rand::Rng;
    now >= deadline || shuttle::rand::thread_rng().gen_bool(0.05)
}

/// Whether `threshold` has elapsed since `since`.
#[doc(hidden)]
pub fn threshold_elapsed(since: std::time::Instant, threshold: std::time::Duration) -> bool {
    deadline_reached(std::time::Instant::now(), since + threshold)
}

/// Shuttle-visible substitute for `crossbeam_utils::sync::{Parker, Unparker}`.
/// Not built on `shuttle::thread::park`/`Thread::unpark`: those are bound to
/// a specific OS thread, but callers construct the `Parker` on one thread and
/// park on another, so binding to `shuttle::thread::current()` here would
/// deadlock. A `Mutex<bool>` + `Condvar` token shared via `Arc` matches
/// crossbeam's actual (thread-identity-agnostic) semantics instead.
#[doc(hidden)]
pub struct Parker {
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
    #[doc(hidden)]
    pub fn unparker(&self) -> Unparker {
        Unparker {
            inner: self.inner.clone(),
        }
    }

    /// Shuttle doesn't model time, so a deadline-based park just parks
    /// unboundedly here (matching shuttle's own `thread::park_timeout`).
    #[doc(hidden)]
    pub fn park_deadline(&self, _deadline: std::time::Instant) {
        let mut available = self.inner.available.lock().unwrap();
        while !*available {
            available = self.inner.condvar.wait(available).unwrap();
        }
        *available = false;
    }
}

#[derive(Clone)]
#[doc(hidden)]
pub struct Unparker {
    inner: std::sync::Arc<TokenState>,
}

impl Unparker {
    #[doc(hidden)]
    pub fn unpark(&self) {
        let mut available = self.inner.available.lock().unwrap();
        *available = true;
        self.inner.condvar.notify_one();
    }
}

/// Shuttle-visible substitute for a slot that can be set at most once, then
/// read back -- the shuttle side of a cfg-swap whose non-shuttle side is a
/// zero-cost `std::sync::OnceLock` wrapper (kept local to each crate, since
/// that side is real production logic). Backed by a `shuttle::sync::Mutex`
/// so the scheduler sees `get`/`set` as real interleaving points. `with`
/// clones the value out and releases the lock *before* calling `f` -- not
/// while holding it -- so that `f` (which in practice calls arbitrary user
/// code) never runs with this lock held; that's why `T: Clone` is required
/// here but not on the non-shuttle side.
#[doc(hidden)]
pub struct OnceSlot<T>(shuttle::sync::Mutex<Option<T>>);

impl<T: Clone> OnceSlot<T> {
    #[doc(hidden)]
    pub fn new() -> Self {
        Self(shuttle::sync::Mutex::new(None))
    }

    #[doc(hidden)]
    pub fn with<R>(&self, f: impl FnOnce(Option<&T>) -> R) -> R {
        let value = self.0.lock().unwrap().clone();
        f(value.as_ref())
    }

    #[doc(hidden)]
    pub fn set(&self, value: T) -> Result<(), T> {
        let mut guard = self.0.lock().unwrap();
        if guard.is_some() {
            Err(value)
        } else {
            *guard = Some(value);
            Ok(())
        }
    }
}

impl<T: Clone> Default for OnceSlot<T> {
    fn default() -> Self {
        Self::new()
    }
}

struct GuardState<T> {
    strong: usize,
    weak: usize,
    value: Option<T>,
}

/// Shuttle-visible substitute for `Arc<Mutex<Option<T>>>` in a hand-rolled
/// "release when the last strong ref drops" protocol -- the shuttle side of
/// a cfg-swap whose non-shuttle side is exactly that real `Arc`/`Mutex`/
/// `Weak` (kept local to each crate, since that side is real production
/// logic). Plain `Arc`/`Weak`'s own atomics aren't shuttle-instrumented, so
/// a release race that hinges on the *last* strong ref's drop wouldn't be
/// shuttle-testable with the real types -- this tracks `strong`/`weak`
/// counts itself under a single `shuttle::sync::Mutex` instead, so the
/// scheduler can see and interleave every increment/decrement.
#[doc(hidden)]
pub struct GuardArc<T>(std::sync::Arc<shuttle::sync::Mutex<GuardState<T>>>);

#[doc(hidden)]
pub struct GuardWeak<T>(std::sync::Arc<shuttle::sync::Mutex<GuardState<T>>>);

impl<T> GuardArc<T> {
    #[doc(hidden)]
    pub fn new(value: T) -> Self {
        Self(std::sync::Arc::new(shuttle::sync::Mutex::new(GuardState {
            strong: 1,
            weak: 0,
            value: Some(value),
        })))
    }

    #[doc(hidden)]
    pub fn downgrade(this: &Self) -> GuardWeak<T> {
        this.0.lock().unwrap().weak += 1;
        GuardWeak(this.0.clone())
    }

    #[doc(hidden)]
    pub fn is_present(&self) -> bool {
        self.0.lock().unwrap().value.is_some()
    }

    #[doc(hidden)]
    pub fn take(&self) -> Option<T> {
        self.0.lock().unwrap().value.take()
    }
}

impl<T> Clone for GuardArc<T> {
    fn clone(&self) -> Self {
        self.0.lock().unwrap().strong += 1;
        Self(self.0.clone())
    }
}

impl<T> Drop for GuardArc<T> {
    fn drop(&mut self) {
        let mut state = self.0.lock().unwrap();
        state.strong -= 1;
        if state.strong == 0 {
            state.value = None;
        }
    }
}

impl<T> GuardWeak<T> {
    #[doc(hidden)]
    pub fn upgrade(&self) -> Option<GuardArc<T>> {
        let mut state = self.0.lock().unwrap();
        if state.strong == 0 {
            None
        } else {
            state.strong += 1;
            Some(GuardArc(self.0.clone()))
        }
    }
}

impl<T> Drop for GuardWeak<T> {
    fn drop(&mut self) {
        self.0.lock().unwrap().weak -= 1;
    }
}

#[doc(hidden)]
pub use shuttle::sync::mpsc::Sender;
#[doc(hidden)]
pub use shuttle::thread;

/// Generates the `pct`/`determinism` shuttle test pair for the `fn` it
/// wraps, nested in a module named after it: `mod $name { fn $name() {..}
/// #[test] fn pct() {..} #[test] fn determinism() {..} }`. Calls
/// `shuttle::check_pct`/`shuttle::check_uncontrolled_nondeterminism` with the
/// same iteration count -- the pattern every shuttle test in this workspace
/// follows.
///
/// ```ignore
/// shuttle_test! {
///     num_iters = 2_000, depth = 3;
///     fn round_trip_no_loss() { /* ... */ }
/// }
/// ```
///
/// `num_iters` and `depth` are required, in that order. Add
/// `, should_panic = "..."` after `depth = ...` for tests expecting a panic.
///
/// Nesting in `mod $name` (instead of generating sibling
/// `${name}_pct`/`${name}_determinism` functions) since
/// `macro_rules!` can't synthesize a new identifier by concatenation on
/// stable Rust, so `pct`/`determinism` stay fixed literal names,
/// disambiguated by the enclosing module instead.
#[macro_export]
macro_rules! shuttle_test {
    (
        num_iters = $num_iters:expr, depth = $depth:expr $(, should_panic = $msg:literal)?;
        $(#[$meta:meta])*
        fn $name:ident() $body:block
    ) => {
        mod $name {
            // Not every test uses every name `use super::*` brings in scope.
            #[allow(unused_imports)]
            use super::*;

            $(#[$meta])*
            fn $name() $body

            #[test]
            $(#[should_panic(expected = $msg)])?
            fn pct() {
                ::shuttle::check_pct($name, $num_iters, $depth);
            }

            #[test]
            $(#[should_panic(expected = $msg)])?
            fn determinism() {
                ::shuttle::check_uncontrolled_nondeterminism($name, $num_iters);
            }
        }
    };
}

/// Shuttle-visible substitute for `std::sync::mpsc::channel()`, wrapping the
/// receiver half so `recv_timeout` can actually return `Timeout` -- shuttle's
/// own `recv_timeout` never times out, a known gap documented in Shuttle
/// itself:
/// https://github.com/awslabs/shuttle/blob/c8a46d3965048df3207ec920dae066bc9c4d9d89/shuttle-std/src/sync/mpsc.rs#L433.
/// Mostly checks non-blockingly, occasionally really blocks instead.
#[doc(hidden)]
pub fn channel<T>() -> (Sender<T>, RecvTimeoutReceiver<T>) {
    let (tx, rx) = shuttle::sync::mpsc::channel();
    (tx, RecvTimeoutReceiver { inner: rx })
}

#[doc(hidden)]
pub struct RecvTimeoutReceiver<T> {
    inner: shuttle::sync::mpsc::Receiver<T>,
}

impl<T> RecvTimeoutReceiver<T> {
    #[doc(hidden)]
    pub fn recv_timeout(
        &self,
        _timeout: std::time::Duration,
    ) -> Result<T, std::sync::mpsc::RecvTimeoutError> {
        use shuttle::rand::Rng;
        use std::sync::mpsc::{RecvTimeoutError, TryRecvError};

        if shuttle::rand::thread_rng().gen_bool(0.8) {
            match self.inner.try_recv() {
                Ok(val) => Ok(val),
                Err(TryRecvError::Empty) => Err(RecvTimeoutError::Timeout),
                Err(TryRecvError::Disconnected) => Err(RecvTimeoutError::Disconnected),
            }
        } else {
            self.inner
                .recv()
                .map_err(|_| RecvTimeoutError::Disconnected)
        }
    }
}
