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
use std::sync::{Arc, Mutex, Weak};
/// [`Parent`] owner
///
/// The [`Parent`] provides exclusive mutable access to its inner value.
///
/// You can delay the primary value being dropped by calling [`new_guard`](Parent::new_guard).
/// As long as guards exist, the value backed by primary will not be dropped.
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
// They exist to make `DropAll` possible. We want to have a single switch to make all of the existing guards
// release their handle on `Parent` to allow the inner value to Drop.
//
// 1. Mutex: This is inside an Arc. We need to be able to take the function out of the
// 2. Option: Allow taking ownership of the:
// 3. Box<dyn Fn Once()...>: Function pointer (which inside of it holds a reference to the main `Arc`)
//    Why the Function pointer? It allows us to erase the generic in the guard.
type GuardInner = Mutex<Option<Box<dyn FnOnce() + Send + Sync>>>;

/// Any guards that remain alive will prevent the `value` within `Parent` from being dropped
pub(crate) struct Guard {
    _value: Arc<GuardInner>,
}

/// If a `DropAll` is created, dropping the `DropAll` will effectively ignore the existence of all `Guards`.
///
/// Dropping a `DropAll` will cause `value` to drop if and only if the `Parent` has been dropped already.
/// Keeping a `DropAll` alive will NOT prevent the `Parent` from being dropped, if it and all standard guards have
/// already been dropped.
pub(crate) struct DropAll(Weak<GuardInner>);
impl Drop for DropAll {
    fn drop(&mut self) {
        if let Some(guard) = self.0.upgrade() {
            if let Some(f) = guard.lock().unwrap().take() {
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
            _value: Arc::new(Mutex::new(Some(Box::new(|| drop(guard_value))))),
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
        DropAll(Arc::downgrade(&self.guard._value))
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
        sync::atomic::{AtomicBool, Ordering},
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
}
