// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Arc-based abstraction to allow "children" to keep the parent alive longer
//!
//! - [`Parent`]: Provides mutable access
//! - [`Guard`]: Prevents inner drop from being called
//! - [`DropAll`]: Ignores all existing `Guard`s
//!
//! NOTE: This similar (but not identical) to the (currently nightly-only) `Unique(A)Rc` API.
//!
//! The main difference is that our additional references are actually strong (but cannot be mutated through).

use core::{
    cell::UnsafeCell,
    ops::{Deref, DerefMut, Drop},
};
use std::{fmt::Debug, sync::Arc};

// `Arc`/`Weak`'s own atomics aren't shuttle-instrumented, so a `DropAll`
// racing ordinary `Guard`s (which only involves `Arc`/`Weak`) wouldn't be
// Shuttle-testable with plain `Arc`/`Weak`.
mod guard_rc {
    #[cfg(not(all(shuttle, feature = "_shuttle")))]
    mod imp {
        use std::sync::{Arc, Mutex, Weak};

        pub(crate) struct GuardArc<T>(Arc<Mutex<Option<T>>>);
        pub(crate) struct GuardWeak<T>(Weak<Mutex<Option<T>>>);

        // `#[inline]`: these just forward to the same `Arc`/`Mutex`/`Weak`
        // calls the pre-`GuardArc` code made directly, so this wrapper
        // should compile down to zero overhead
        impl<T> GuardArc<T> {
            #[inline]
            pub(crate) fn new(value: T) -> Self {
                Self(Arc::new(Mutex::new(Some(value))))
            }

            #[inline]
            pub(crate) fn downgrade(this: &Self) -> GuardWeak<T> {
                GuardWeak(Arc::downgrade(&this.0))
            }

            #[inline]
            pub(crate) fn is_present(&self) -> bool {
                self.0.lock().unwrap().is_some()
            }

            #[inline]
            pub(crate) fn take(&self) -> Option<T> {
                self.0.lock().unwrap().take()
            }
        }

        impl<T> Clone for GuardArc<T> {
            #[inline]
            fn clone(&self) -> Self {
                Self(self.0.clone())
            }
        }

        impl<T> GuardWeak<T> {
            #[inline]
            pub(crate) fn upgrade(&self) -> Option<GuardArc<T>> {
                self.0.upgrade().map(GuardArc)
            }
        }
    }

    #[cfg(all(shuttle, feature = "_shuttle"))]
    mod imp {
        pub(crate) use metrique_writer_core::shuttle_test_support::{GuardArc, GuardWeak};
    }

    pub(crate) use imp::{GuardArc, GuardWeak};
}

use guard_rc::{GuardArc, GuardWeak};

/// [`Parent`] owner
///
/// The [`Parent`] provides exclusive mutable access to its inner value.
///
/// You can delay the primary value being dropped by calling [`new_guard`](Parent::new_guard).
/// As long as guards exist, the value backed by primary will not be dropped.
#[derive(Debug)]
pub(crate) struct Parent<T> {
    // SAFETY: `value` MUST only be mutated through `Parent`. This is safe because:
    // 1. `Parent` does NOT implement `Clone` or `Copy`, meaning only 1 `Parent` may exist.
    // 2. `Guard` does NOT provide any access of any kind into value other than drop.
    // 3. `drop` is protected by the atomic within the `Arc`.
    value: Arc<UnsafeCell<T>>,
    guard: Guard,
}

/// Safety: If `T` is `Send`, then `Arc<T>` is `Send`
unsafe impl<T> Send for Parent<T> where T: Send {}
/// Safety: If `T` is `Sync`, then `Arc<T>` is `Sync`
unsafe impl<T> Sync for Parent<T> where T: Sync {}

// Why all these layers?
// They exist to make `DropAll` possible. We want a single switch to make all of the existing guards
// release their handle on `Parent` to allow the inner value to Drop.
//
// 1. GuardArc: The switch itself. `take()`s the payload out, whichever of "last `Guard` dropped" or
//    "a `DropAll` dropped" gets there first.
// 2. Box<dyn FnOnce()...>: Function pointer (which inside of it holds a reference to the main `Arc`)
//    Why the Function pointer? It allows us to erase the generic in the guard.
type GuardPayload = Box<dyn FnOnce() + Send + Sync>;

/// Any guards that remain alive will prevent the `value` within `Parent` from being dropped
pub(crate) struct Guard {
    _value: GuardArc<GuardPayload>,
}

impl Debug for Guard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let is_open = self._value.is_present();
        f.debug_struct("Guard").field("open", &is_open).finish()
    }
}

/// If a `DropAll` is created, dropping the `DropAll` will effectively ignore the existence of all `Guards`.
///
/// Dropping a `DropAll` will cause `value` to drop if and only if the `Parent` has been dropped already.
/// Keeping a `DropAll` alive will NOT prevent the `Parent` from being dropped, if it and all standard guards have
/// already been dropped.
pub(crate) struct DropAll(GuardWeak<GuardPayload>);
impl Drop for DropAll {
    fn drop(&mut self) {
        if let Some(guard) = self.0.upgrade() {
            if let Some(f) = guard.take() {
                (f)()
            }
        }
    }
}

impl<T: Send + Sync + 'static> Parent<T> {
    pub(crate) fn new(value: T) -> Self {
        let value: Arc<UnsafeCell<T>> = Arc::new(value.into());
        struct AssertSendSync<T>(T);
        // Safety: `T` is `Send` and `Sync`
        // It will not be mutated through the `Guard`
        unsafe impl<T> Send for AssertSendSync<T> {}
        unsafe impl<T> Sync for AssertSendSync<T> {}
        let guard_value = AssertSendSync(value.clone());
        let guard = Guard {
            _value: GuardArc::new(Box::new(|| drop(guard_value))),
        };
        Self { value, guard }
    }

    /// Creates a new `Guard`
    ///
    /// The inner value will not be dropped until either:
    /// 1. This (and all other guards are dropped)
    /// 2. A [`Self::force_drop_guard`] is created and dropped
    ///
    /// And:
    /// 3. The `Parent` is also dropped.
    pub(crate) fn new_guard(&self) -> Guard {
        Guard {
            _value: self.guard._value.clone(),
        }
    }

    /// Creates a `force_drop_guard`.
    ///
    /// If a this object is created and dropped, it will allow the inner value to drop when the
    /// `Parent` container is dropped regardless of any `Guard`s that exist.
    ///
    /// It remains safe to hold `Guard` after this API is called and used.
    pub(crate) fn force_drop_guard(&self) -> DropAll {
        DropAll(GuardArc::downgrade(&self.guard._value))
    }
}

impl<T> Deref for Parent<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        // SAFETY: Only one `Parent` may exist
        unsafe { &*UnsafeCell::get(self.value.as_ref()) }
    }
}

impl<T> DerefMut for Parent<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        // SAFETY: Only one `Parent` may exist
        unsafe { &mut *UnsafeCell::get(self.value.as_ref()) }
    }
}

#[cfg(test)]
mod test {
    use core::{
        assert_eq,
        ops::Drop,
        sync::atomic::{AtomicBool, AtomicUsize, Ordering},
    };
    use std::sync::Arc;

    use super::Parent;

    struct IsDropped {
        inner: Arc<AtomicBool>,
    }

    impl IsDropped {
        fn new() -> (Self, Arc<AtomicBool>) {
            let inner = Arc::new(AtomicBool::default());
            (
                IsDropped {
                    inner: inner.clone(),
                },
                inner,
            )
        }
    }

    impl Drop for IsDropped {
        fn drop(&mut self) {
            self.inner.store(true, Ordering::SeqCst);
        }
    }

    #[test]
    fn immediate_drop_drops() {
        let (tester, is_dropped) = IsDropped::new();
        let primary = Parent::new(tester);
        drop(primary);
        assert_eq!(is_dropped.load(Ordering::Relaxed), true);
    }

    #[test]
    fn children_keep_parent_alive() {
        let (tester, is_dropped) = IsDropped::new();
        let primary = Parent::new(tester);
        let guard_1 = primary.new_guard();
        let guard_2 = primary.new_guard();
        assert_eq!(is_dropped.load(Ordering::Relaxed), false);
        drop(guard_1);
        assert_eq!(is_dropped.load(Ordering::Relaxed), false);
        drop(primary);
        assert_eq!(is_dropped.load(Ordering::Relaxed), false);
        drop(guard_2);
        assert_eq!(is_dropped.load(Ordering::Relaxed), true);
    }

    #[test]
    fn drop_all_doesnt_drop_primary() {
        let (tester, is_dropped) = IsDropped::new();
        let primary = Parent::new(tester);
        let drop_all = primary.force_drop_guard();
        drop(drop_all);
        // the primary is still alive
        assert_eq!(is_dropped.load(Ordering::Relaxed), false);

        // now it is not
        drop(primary);
        assert_eq!(is_dropped.load(Ordering::Relaxed), true);
    }

    #[test]
    fn make_two_drop_alls() {
        let (tester, is_dropped) = IsDropped::new();
        let primary = Parent::new(tester);
        let drop_all_1 = primary.force_drop_guard();
        let drop_all_2 = primary.force_drop_guard();
        // the primary is still alive
        assert_eq!(is_dropped.load(Ordering::Relaxed), false);
        drop(drop_all_1);
        drop(drop_all_2);

        assert_eq!(is_dropped.load(Ordering::Relaxed), false);
        drop(primary);
        assert_eq!(is_dropped.load(Ordering::Relaxed), true);
    }

    #[test]
    fn drop_all_doesnt_keep_parent_alive() {
        let (tester, is_dropped) = IsDropped::new();
        let primary = Parent::new(tester);
        let drop_all = primary.force_drop_guard();

        assert_eq!(is_dropped.load(Ordering::Relaxed), false);

        // having the drop_all alive does not prevent primary from dropping
        drop(primary);
        assert_eq!(is_dropped.load(Ordering::Relaxed), true);

        drop(drop_all);
    }

    #[test]
    fn all_guards_can_be_dropped() {
        let (tester, is_dropped) = IsDropped::new();
        let sut = Parent::new(tester);
        // Make two guards
        let _guard_1 = sut.new_guard();
        let _guard_2 = sut.new_guard();

        // Everything is alive
        assert_eq!(is_dropped.load(Ordering::Relaxed), false);

        // Create a drop guard, that when dropped, will ignore any other open guards
        let force_drop_guard = sut.force_drop_guard();

        // Creating this doesn't drop the Parent
        assert_eq!(is_dropped.load(Ordering::Relaxed), false);

        // Dropping the primary doesn't cause a drop, we still have two guards alive
        drop(sut);
        assert_eq!(is_dropped.load(Ordering::Relaxed), false);

        // Dropping one guard doesn't cause a drop (there is still one guard left)
        drop(_guard_1);
        assert_eq!(is_dropped.load(Ordering::Relaxed), false);

        // now the force drop guard is dropped & sut is dropped
        drop(force_drop_guard);
        assert_eq!(is_dropped.load(Ordering::Relaxed), true);

        drop(_guard_2);
    }

    struct DropCounter {
        count: Arc<AtomicUsize>,
    }

    impl DropCounter {
        fn new() -> (Self, Arc<AtomicUsize>) {
            let count = Arc::new(AtomicUsize::new(0));
            (
                DropCounter {
                    count: count.clone(),
                },
                count,
            )
        }
    }

    impl Drop for DropCounter {
        fn drop(&mut self) {
            self.count.fetch_add(1, Ordering::SeqCst);
        }
    }

    /// The value must be released exactly once: not zero (a leak)
    /// and not more than once (a double drop, which given `Parent`'s
    /// `unsafe impl Send`/`Sync` and `UnsafeCell` would risk real
    /// undefined behavior, not just a logic bug).
    #[test]
    fn stress_concurrent_guard_and_drop_all_race_releases_value_exactly_once() {
        use std::sync::Barrier;

        const GUARDS: usize = 4;

        for _ in 0..1000 {
            let (tester, drop_count) = DropCounter::new();
            let primary = Parent::new(tester);

            let guards: Vec<_> = (0..GUARDS).map(|_| primary.new_guard()).collect();
            let drop_all = primary.force_drop_guard();

            // GUARDS threads dropping their guard + 1 thread dropping
            // `drop_all` + this thread dropping `primary` -- all racing
            // through the same barrier.
            let barrier = Arc::new(Barrier::new(GUARDS + 1 + 1));

            let mut handles: Vec<_> = guards
                .into_iter()
                .map(|guard| {
                    let barrier = barrier.clone();
                    std::thread::spawn(move || {
                        barrier.wait();
                        drop(guard);
                    })
                })
                .collect();

            handles.push({
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    drop(drop_all);
                })
            });

            barrier.wait();
            drop(primary);

            for h in handles {
                h.join().unwrap();
            }

            assert_eq!(
                drop_count.load(Ordering::SeqCst),
                1,
                "value must be dropped exactly once, regardless of how the guard/drop_all/primary drops interleaved"
            );
        }
    }
}

// Shuttle interleaving tests for `Guard`/`DropAll` races.
#[cfg(all(test, shuttle, feature = "_shuttle"))]
mod shuttle_tests {
    use core::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use super::Parent;
    use metrique_shuttle_test::shuttle_test;

    struct DropCounter {
        count: Arc<AtomicUsize>,
    }

    impl Drop for DropCounter {
        fn drop(&mut self) {
            self.count.fetch_add(1, Ordering::SeqCst);
        }
    }

    /// Two `DropAll`s dropped concurrently, racing each other (and
    /// `Parent`'s own drop) on the same slot. Unlike a `DropAll` racing
    /// ordinary `Guard`s (which races inside `Arc`'s own atomics, outside
    /// what Shuttle instruments), this race sits entirely between two
    /// `Mutex` operations Shuttle does control.
    #[shuttle_test(2_000, 3)]
    fn concurrent_drop_alls_race_releases_value_exactly_once() {
        let count = Arc::new(AtomicUsize::new(0));
        let tester = DropCounter {
            count: count.clone(),
        };
        let primary = Parent::new(tester);

        let drop_all_1 = primary.force_drop_guard();
        let drop_all_2 = primary.force_drop_guard();

        let h1 = shuttle::thread::spawn(move || drop(drop_all_1));
        let h2 = shuttle::thread::spawn(move || drop(drop_all_2));

        drop(primary);

        h1.join().unwrap();
        h2.join().unwrap();

        assert_eq!(
            count.load(Ordering::SeqCst),
            1,
            "value must be dropped exactly once, regardless of how the two DropAlls and primary interleave"
        );
    }

    /// Ordinary `Guard`s racing each other (and `Parent`'s drop), no
    /// `DropAll` -- the load-bearing race plain `Arc`/`Weak` couldn't expose.
    #[shuttle_test(2_000, 3)]
    fn concurrent_guards_race_releases_value_exactly_once() {
        const GUARDS: usize = 2;

        let count = Arc::new(AtomicUsize::new(0));
        let tester = DropCounter {
            count: count.clone(),
        };
        let primary = Parent::new(tester);

        let guards: Vec<_> = (0..GUARDS).map(|_| primary.new_guard()).collect();
        let handles: Vec<_> = guards
            .into_iter()
            .map(|guard| shuttle::thread::spawn(move || drop(guard)))
            .collect();

        drop(primary);

        for h in handles {
            h.join().unwrap();
        }

        assert_eq!(
            count.load(Ordering::SeqCst),
            1,
            "value must be dropped exactly once, regardless of how the guards and primary interleave"
        );
    }

    /// Ordinary `Guard`s racing a `DropAll` (and `Parent`'s drop) -- same
    /// shape as the real-thread stress test above, made exhaustive.
    #[shuttle_test(2_000, 3)]
    fn concurrent_guards_and_drop_all_race_releases_value_exactly_once() {
        const GUARDS: usize = 2;

        let count = Arc::new(AtomicUsize::new(0));
        let tester = DropCounter {
            count: count.clone(),
        };
        let primary = Parent::new(tester);

        let guards: Vec<_> = (0..GUARDS).map(|_| primary.new_guard()).collect();
        let drop_all = primary.force_drop_guard();

        let mut handles: Vec<_> = guards
            .into_iter()
            .map(|guard| shuttle::thread::spawn(move || drop(guard)))
            .collect();
        handles.push(shuttle::thread::spawn(move || drop(drop_all)));

        drop(primary);

        for h in handles {
            h.join().unwrap();
        }

        assert_eq!(
            count.load(Ordering::SeqCst),
            1,
            "value must be dropped exactly once, regardless of how the guards, drop_all, and primary interleave"
        );
    }

    /// Verifies release doesn't wait on a lingering `DropAll`.
    fn drop_all_lingering_does_not_delay_release() {
        let count = Arc::new(AtomicUsize::new(0));
        let tester = DropCounter {
            count: count.clone(),
        };
        let primary = Parent::new(tester);
        let drop_all = primary.force_drop_guard();

        drop(primary);
        assert_eq!(
            count.load(Ordering::SeqCst),
            1,
            "value must release once Parent (and all Guards) drop, even with a live DropAll"
        );

        drop(drop_all);
    }

    #[test]
    fn drop_all_lingering_does_not_delay_release_check() {
        shuttle::check(drop_all_lingering_does_not_delay_release);
    }

    /// `new_guard()` itself called concurrently from multiple threads
    /// `Parent: Sync` implies this must be safe; exercised here.
    #[shuttle_test(2_000, 3)]
    fn concurrent_guard_creation_and_drop_releases_value_exactly_once() {
        const GUARDS: usize = 2;

        let count = Arc::new(AtomicUsize::new(0));
        let tester = DropCounter {
            count: count.clone(),
        };
        let primary = Parent::new(tester);

        shuttle::thread::scope(|s| {
            for _ in 0..GUARDS {
                s.spawn(|| drop(primary.new_guard()));
            }
        });

        drop(primary);

        assert_eq!(
            count.load(Ordering::SeqCst),
            1,
            "value must be dropped exactly once, regardless of how concurrent guard creation and drop interleave"
        );
    }

    /// Guard creation, guard drops, `DropAll` creation, and
    /// `DropAll` drops, all racing each other (and `Parent`'s drop) at once
    #[shuttle_test(2_000, 3)]
    fn concurrent_guard_and_drop_all_creation_and_drop_releases_value_exactly_once() {
        const GUARDS: usize = 2;
        const DROP_ALLS: usize = 2;

        let count = Arc::new(AtomicUsize::new(0));
        let tester = DropCounter {
            count: count.clone(),
        };
        let primary = Parent::new(tester);

        shuttle::thread::scope(|s| {
            for _ in 0..GUARDS {
                s.spawn(|| drop(primary.new_guard()));
            }
            for _ in 0..DROP_ALLS {
                s.spawn(|| drop(primary.force_drop_guard()));
            }
        });

        drop(primary);

        assert_eq!(
            count.load(Ordering::SeqCst),
            1,
            "value must be dropped exactly once, regardless of how concurrent guard/DropAll creation and drop interleave"
        );
    }
}
