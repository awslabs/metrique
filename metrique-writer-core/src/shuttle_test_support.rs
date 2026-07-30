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
