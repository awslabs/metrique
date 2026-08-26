// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Verifies that `AppendAndCloseOnDrop` construction and drop are
//! zero-allocation in the common case (no flush guard, no handle).
//!
//! This test uses a global allocator counter, so all assertions must run
//! sequentially in a single test to avoid cross-contamination.

use metrique::unit_of_work::metrics;
use metrique::writer::sink::DevNullSink;
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

struct CountingAllocator;

static ALLOC_COUNT: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static GLOBAL: CountingAllocator = CountingAllocator;

#[metrics]
#[derive(Default)]
struct TestMetrics {
    value: u32,
    name: &'static str,
}

fn reset_count() -> usize {
    ALLOC_COUNT.swap(0, Ordering::SeqCst)
}

#[test]
fn allocation_counts() {
    // Warm up: first Mutex::lock() allocates on some platforms (e.g. macOS).
    // Run each path once so subsequent measurements are stable.
    drop(TestMetrics::default().append_on_drop(DevNullSink::new()));
    drop(
        TestMetrics::default()
            .append_on_drop(DevNullSink::new())
            .flush_guard(),
    );
    drop(
        TestMetrics::default()
            .append_on_drop(DevNullSink::new())
            .handle(),
    );

    // --- Common path: construct + mutate + drop = 0 allocations ---
    reset_count();
    let mut guard = TestMetrics {
        value: 42,
        name: "test",
    }
    .append_on_drop(DevNullSink::new());
    guard.value = 99;
    drop(guard);
    let allocs = reset_count();
    assert_eq!(
        allocs, 0,
        "common path: expected 0 allocations, got {allocs}"
    );

    // --- flush_guard path: bounded allocations ---
    reset_count();
    let guard = TestMetrics::default().append_on_drop(DevNullSink::new());
    let fg = guard.flush_guard();
    drop(guard);
    drop(fg);
    let allocs = reset_count();
    assert!(
        allocs >= 3,
        "flush_guard path: expected at least 3 allocations, got {allocs}"
    );
    assert!(
        allocs <= 5,
        "flush_guard path: expected at most 5 allocations, got {allocs}"
    );

    // --- handle path: exactly 1 allocation ---
    reset_count();
    let guard = TestMetrics::default().append_on_drop(DevNullSink::new());
    let h = guard.handle();
    drop(h);
    let allocs = reset_count();
    assert_eq!(
        allocs, 1,
        "handle path: expected 1 allocation, got {allocs}"
    );
}
