// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use core::sync::atomic::{AtomicBool, AtomicU8, AtomicU16, AtomicU32, AtomicU64, AtomicUsize};

use crate::CloseValueRef;

/// A thin wrapper around `AtomicU64` that implements [`CloseValue`](crate::CloseValue).
///
/// This is provided for convenience to avoid the need to specify an ordering. However,
/// all other atomics also implement [`CloseValueRef`] and can be used directly.
#[derive(Default, Debug)]
pub struct Counter(pub AtomicU64);
impl Counter {
    /// Create a new [`Counter`], initialized a specific value
    pub fn new(starting_count: u64) -> Self {
        Self(AtomicU64::new(starting_count))
    }

    /// Add 1 to this counter
    pub fn increment(&self) {
        self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    /// Increase the value of this counter by `i`
    pub fn add(&self, i: u64) {
        self.0.fetch_add(i, std::sync::atomic::Ordering::Relaxed);
    }

    /// Set this counter to `i`, discarding the previous value
    pub fn set(&self, i: u64) {
        self.0.store(i, std::sync::atomic::Ordering::SeqCst);
    }
}

impl CloseValueRef for Counter {
    type Closed = u64;

    fn close_ref(&self) -> Self::Closed {
        self.0.close_ref()
    }
}

macro_rules! close_value_atomic {
    (atomic: $atomic: ty, inner: $inner: ty) => {
        impl $crate::CloseValueRef for $atomic {
            type Closed = $inner;

            fn close_ref(&self) -> Self::Closed {
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
