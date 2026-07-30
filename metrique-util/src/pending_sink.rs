// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Deferred sink attachment with bounded entry buffering.
//!
//! See [`new()`] for details.

use std::sync::Arc;

use metrique_writer_core::{
    entry::BoxEntry,
    sink::{BoxEntrySink, EntrySink, FlushWait},
};

// Cfg-gated concurrency primitives (std/crossbeam vs. shuttle). Gated on both
// `cfg(shuttle)` and `feature = "_shuttle"`, not `cfg(shuttle)` alone: `--cfg
// shuttle` is set process-wide via RUSTFLAGS, so it also reaches builds of
// this crate (e.g. as a dev-dependency with different requested features)
// that don't have `_shuttle` enabled and therefore don't have the optional
// `shuttle` crate linked at all.
//
// Real hardware atomics and `crossbeam_queue::ArrayQueue` are invisible to
// shuttle's scheduler (no yield points), so without this swap a shuttle test
// here wouldn't actually explore any interleavings of the buffer/resolve
// protocol below. Shuttle also always treats atomics as `SeqCst` regardless
// of the ordering used, so it can find scheduling/logic bugs in this
// protocol but not pure Acquire/Release memory-ordering bugs.
#[cfg(all(shuttle, feature = "_shuttle"))]
use shuttle::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
#[cfg(not(all(shuttle, feature = "_shuttle")))]
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

#[cfg(not(all(shuttle, feature = "_shuttle")))]
use crossbeam_queue::ArrayQueue;
#[cfg(all(shuttle, feature = "_shuttle"))]
use metrique_writer_core::shuttle_test_support::ArrayQueue;

use once_slot::OnceSlot;

struct Inner {
    buffer: ArrayQueue<BoxEntry>,
    sink: OnceSlot<BoxEntrySink>,
    cancelled: AtomicBool,
    /// Number of append() calls currently in the buffering path (between the
    /// sink.get() check and the force_push completion). resolve() waits for
    /// this to reach zero before draining, preventing stranded entries.
    buffering: AtomicUsize,
}

/// A slot that can be set at most once, then read back via [`with`](Self::with).
///
/// Under `cfg(not(shuttle))` this is a zero-cost wrapper around
/// [`std::sync::OnceLock`] -- `with` just calls `f` on `self.0.get()`
/// directly, no lock, no clone, matching `OnceLock`'s own performance.
///
/// Under `cfg(shuttle)` it's backed by a `shuttle::sync::Mutex` so the
/// scheduler can see `get`/`set` as real interleaving points. It clones the
/// value out and releases the lock *before* calling `f` -- not while holding
/// it -- so that `f` (which in practice calls into the wrapped
/// `BoxEntrySink`, arbitrary user code) is never run with this lock held.
/// Holding it across `f` would artificially serialize concurrent
/// post-resolution appends against each other through a lock that doesn't
/// exist in production, which isn't part of what this module's protocol
/// needs to guarantee (that's the real sink's own thread-safety contract).
/// This is why the shuttle side needs `T: Clone` and the non-shuttle side
/// doesn't.
mod once_slot {
    #[cfg(not(all(shuttle, feature = "_shuttle")))]
    pub(super) struct OnceSlot<T>(std::sync::OnceLock<T>);
    #[cfg(not(all(shuttle, feature = "_shuttle")))]
    impl<T> OnceSlot<T> {
        pub(super) fn new() -> Self {
            Self(std::sync::OnceLock::new())
        }
        pub(super) fn with<R>(&self, f: impl FnOnce(Option<&T>) -> R) -> R {
            f(self.0.get())
        }
        pub(super) fn set(&self, value: T) -> Result<(), T> {
            self.0.set(value)
        }
    }

    #[cfg(all(shuttle, feature = "_shuttle"))]
    pub(super) struct OnceSlot<T>(shuttle::sync::Mutex<Option<T>>);
    // `T: Clone` only under shuttle.
    #[cfg(all(shuttle, feature = "_shuttle"))]
    impl<T: Clone> OnceSlot<T> {
        pub(super) fn new() -> Self {
            Self(shuttle::sync::Mutex::new(None))
        }
        pub(super) fn with<R>(&self, f: impl FnOnce(Option<&T>) -> R) -> R {
            let value = self.0.lock().unwrap().clone();
            f(value.as_ref())
        }
        pub(super) fn set(&self, value: T) -> Result<(), T> {
            let mut guard = self.0.lock().unwrap();
            if guard.is_some() {
                Err(value)
            } else {
                *guard = Some(value);
                Ok(())
            }
        }
    }
}

struct PendingSink(Arc<Inner>);

/// Forwards `entry` to `slot`'s sink if resolved, handing it back (unused)
/// otherwise -- `BoxEntry` can only be moved once, so this is the shape a
/// "maybe consume it" check needs when the check itself goes through a
/// `with`-style closure API rather than returning a plain reference.
fn forward_or_return(slot: &OnceSlot<BoxEntrySink>, entry: BoxEntry) -> Option<BoxEntry> {
    slot.with(|sink| match sink {
        Some(sink) => {
            sink.append(entry);
            None
        }
        None => Some(entry),
    })
}

impl EntrySink<BoxEntry> for PendingSink {
    fn append(&self, entry: BoxEntry) {
        let Some(entry) = forward_or_return(&self.0.sink, entry) else {
            return;
        };
        if self.0.cancelled.load(Ordering::Acquire) {
            return;
        }
        self.0.buffering.fetch_add(1, Ordering::AcqRel);
        // Re-check after incrementing: if the sink was set between our
        // first check and the increment, forward directly instead.
        let entry = forward_or_return(&self.0.sink, entry);
        if let Some(entry) = entry {
            // force_push before decrementing: resolve()'s spin-wait treats
            // buffering == 0 as a promise that nothing can still write to the
            // buffer, so the push must be visible before the count drops.
            self.0.buffer.force_push(entry);
        }
        self.0.buffering.fetch_sub(1, Ordering::AcqRel);
    }

    fn flush_async(&self) -> FlushWait {
        self.0.sink.with(|sink| match sink {
            Some(sink) => EntrySink::<BoxEntry>::flush_async(sink),
            None => FlushWait::ready(),
        })
    }
}

/// Handle for resolving a pending sink created by [`new()`].
///
/// Call [`resolve`](PendingSinkResolver::resolve) to drain buffered entries into the
/// real sink and switch to direct forwarding. If this handle is dropped without
/// calling `resolve`, the pending sink becomes a no-op and buffered entries are
/// discarded.
#[must_use = "if dropped without calling resolve(), buffered entries will be discarded"]
pub struct PendingSinkResolver(Option<Arc<Inner>>);

impl PendingSinkResolver {
    /// Drain all buffered entries into `sink` and switch to direct forwarding.
    ///
    /// After this call, new entries appended to the associated [`BoxEntrySink`] will
    /// go directly to `sink`. This method consumes the resolver.
    pub fn resolve(mut self, sink: BoxEntrySink) {
        if let Some(inner) = self.0.take() {
            // Set the sink so new appends go directly to it.
            let Ok(()) = inner.sink.set(sink) else {
                return;
            };
            // Wait for any in-flight buffering appenders to finish pushing.
            while inner.buffering.load(Ordering::Acquire) != 0 {
                // A bare spin_loop() hint gives shuttle's scheduler no reason to
                // consider switching threads fairly, which can pathologically
                // starve this loop within a single exploration (real OS
                // preemption doesn't have that failure mode). Yield explicitly
                // under shuttle; std::hint::spin_loop() covers the real path.
                #[cfg(all(shuttle, feature = "_shuttle"))]
                shuttle::thread::yield_now();
                #[cfg(not(all(shuttle, feature = "_shuttle")))]
                std::hint::spin_loop();
            }
            // Drain all buffered entries. Fetched once, not per-entry: nothing
            // else can still be racing this (buffering == 0, and the sink was
            // already set for good above), so there's no reason to pay a
            // separate `with` (lock, under shuttle) per entry.
            inner.sink.with(|sink| {
                let sink = sink.unwrap();
                while let Some(entry) = inner.buffer.pop() {
                    sink.append(entry);
                }
            });
        }
    }
}

impl Drop for PendingSinkResolver {
    fn drop(&mut self) {
        if let Some(inner) = self.0.take() {
            inner.cancelled.store(true, Ordering::Release);
        }
    }
}

/// Creates a `(BoxEntrySink, PendingSinkResolver)` pair for deferred sink attachment.
///
/// The returned sink can be used immediately. While the resolver has not yet been
/// called, entries are buffered in a bounded ring buffer of the given `capacity`.
/// When the buffer is full, the oldest entry is dropped (consistent with
/// `BackgroundQueue` backpressure behavior).
///
/// Call [`PendingSinkResolver::resolve`] to drain the buffer into the real sink and
/// switch to direct forwarding. If the resolver is dropped without calling `resolve`,
/// buffered entries are discarded and the sink becomes a no-op.
///
/// The hot path (`append` after resolution) is a single atomic load with no locking.
///
/// # Panics
///
/// Panics if `capacity` is 0.
///
/// # Example
///
/// ```
/// use metrique_util::pending_sink;
///
/// let (sink, resolver) = pending_sink::new(1024);
///
/// // Entries are buffered while the real sink initializes
/// // sink.append_any(my_entry);
///
/// // Later, once the real sink is ready:
/// // resolver.resolve(real_sink);
/// // Buffered entries are drained and future entries go directly to real_sink.
///
/// // Or, if initialization fails, just drop the resolver:
/// drop(resolver);
/// // The sink becomes a no-op; buffered entries are discarded.
/// ```
pub fn new(capacity: usize) -> (BoxEntrySink, PendingSinkResolver) {
    assert!(capacity > 0, "pending sink capacity must be greater than 0");
    let inner = Arc::new(Inner {
        buffer: ArrayQueue::new(capacity),
        sink: OnceSlot::new(),
        cancelled: AtomicBool::new(false),
        buffering: AtomicUsize::new(0),
    });
    let sink = BoxEntrySink::new(PendingSink(Arc::clone(&inner)));
    let resolver = PendingSinkResolver(Some(inner));
    (sink, resolver)
}

#[cfg(test)]
mod tests {
    use super::*;
    use metrique_writer_core::sink::AnyEntrySink;
    use std::sync::{Arc, Mutex};

    // A simple Entry for testing
    struct TestEntry(u64);
    impl metrique_writer_core::Entry for TestEntry {
        fn write<'a>(&'a self, writer: &mut impl metrique_writer_core::EntryWriter<'a>) {
            writer.value("value", &self.0);
        }
    }

    struct CollectorSink {
        appended: Arc<Mutex<Vec<u64>>>,
        flushes: Arc<Mutex<u64>>,
    }

    impl EntrySink<BoxEntry> for CollectorSink {
        fn append(&self, _entry: BoxEntry) {
            self.appended.lock().unwrap().push(1);
        }
        fn flush_async(&self) -> FlushWait {
            *self.flushes.lock().unwrap() += 1;
            FlushWait::ready()
        }
    }

    fn collector() -> (BoxEntrySink, Arc<Mutex<Vec<u64>>>, Arc<Mutex<u64>>) {
        let appended = Arc::new(Mutex::new(Vec::new()));
        let flushes = Arc::new(Mutex::new(0u64));
        let sink = BoxEntrySink::new(CollectorSink {
            appended: Arc::clone(&appended),
            flushes: Arc::clone(&flushes),
        });
        (sink, appended, flushes)
    }

    #[test]
    fn resolve_drains_buffered_entries() {
        let (sink, resolver) = new(16);

        sink.append_any(TestEntry(1));
        sink.append_any(TestEntry(2));
        sink.append_any(TestEntry(3));

        let (real_sink, appended, _) = collector();
        resolver.resolve(real_sink);

        assert_eq!(appended.lock().unwrap().len(), 3);
    }

    #[test]
    fn entries_forward_after_resolve() {
        let (sink, resolver) = new(16);
        let (real_sink, appended, _) = collector();
        resolver.resolve(real_sink);

        sink.append_any(TestEntry(1));
        sink.append_any(TestEntry(2));

        assert_eq!(appended.lock().unwrap().len(), 2);
    }

    #[test]
    fn flush_forwards_after_resolve() {
        let (sink, resolver) = new(16);
        let (real_sink, _, flushes) = collector();
        resolver.resolve(real_sink);

        let _ = AnyEntrySink::flush_async(&sink);
        assert_eq!(*flushes.lock().unwrap(), 1);
    }

    #[test]
    fn flush_is_noop_while_pending() {
        let (sink, _resolver) = new(16);
        // Should not panic; returns ready immediately
        let _ = AnyEntrySink::flush_async(&sink);
    }

    #[test]
    fn drop_oldest_when_buffer_full() {
        let (sink, resolver) = new(2);

        sink.append_any(TestEntry(1));
        sink.append_any(TestEntry(2));
        sink.append_any(TestEntry(3)); // evicts TestEntry(1)

        let (real_sink, appended, _) = collector();
        resolver.resolve(real_sink);

        // Only 2 entries survive (the buffer capacity)
        assert_eq!(appended.lock().unwrap().len(), 2);
    }

    #[test]
    fn drop_resolver_cancels_and_discards_buffer() {
        let (sink, resolver) = new(16);

        sink.append_any(TestEntry(1));
        sink.append_any(TestEntry(2));

        drop(resolver);

        // After cancellation, new entries are silently discarded
        sink.append_any(TestEntry(3));
        let _ = AnyEntrySink::flush_async(&sink);
    }

    #[test]
    fn resolve_drains_then_forwards_new_entries() {
        let (sink, resolver) = new(16);

        sink.append_any(TestEntry(1));
        sink.append_any(TestEntry(2));

        let (real_sink, appended, _) = collector();
        resolver.resolve(real_sink);

        sink.append_any(TestEntry(3));
        assert_eq!(appended.lock().unwrap().len(), 3);
    }

    #[test]
    fn pending_sink_is_clone_safe() {
        let (sink, resolver) = new(16);
        let sink2 = sink.clone();

        sink.append_any(TestEntry(1));
        sink2.append_any(TestEntry(2));

        let (real_sink, appended, _) = collector();
        resolver.resolve(real_sink);

        assert_eq!(appended.lock().unwrap().len(), 2);

        sink.append_any(TestEntry(3));
        sink2.append_any(TestEntry(4));
        assert_eq!(appended.lock().unwrap().len(), 4);
    }

    #[test]
    #[should_panic(expected = "pending sink capacity must be greater than 0")]
    fn zero_capacity_panics() {
        let _ = new(0);
    }

    #[test]
    fn stress_no_entry_loss_on_concurrent_resolve() {
        use std::sync::Barrier;

        for _ in 0..1000 {
            let n = 100;
            let threads = 4;
            let expected = n * threads;

            let (sink, resolver) = new(expected + 1024);
            let barrier = Arc::new(Barrier::new(threads + 1));

            let (real_sink, appended, _) = collector();

            let handles: Vec<_> = (0..threads)
                .map(|_| {
                    let sink = sink.clone();
                    let barrier = barrier.clone();
                    std::thread::spawn(move || {
                        barrier.wait();
                        for i in 0..n {
                            sink.append_any(TestEntry(i as u64));
                        }
                    })
                })
                .collect();

            barrier.wait();
            resolver.resolve(real_sink);

            for h in handles {
                h.join().unwrap();
            }

            let got = appended.lock().unwrap().len();
            assert_eq!(got, expected, "lost {} entries", expected - got);
        }
    }
}

// Shuttle interleaving test for the buffer/resolve protocol above (a
// hand-rolled quiescence scheme: an in-flight counter plus a double-checked
// sink slot, guarding against `resolve()` draining while an `append()` is
// still mid-flight). `stress_no_entry_loss_on_concurrent_resolve` in `mod
// tests` above already probes this with 1000 real-thread iterations; this
// explores interleavings directly instead of sampling them.
#[cfg(all(test, shuttle, feature = "_shuttle"))]
mod shuttle_tests {
    use std::sync::Mutex;

    use metrique_writer_core::sink::AnyEntrySink;

    use super::*;

    struct TestEntry(u64);
    impl metrique_writer_core::Entry for TestEntry {
        fn write<'a>(&'a self, writer: &mut impl metrique_writer_core::EntryWriter<'a>) {
            writer.value("value", &self.0);
        }
    }

    struct CountingSink {
        count: Arc<Mutex<u64>>,
    }

    impl EntrySink<BoxEntry> for CountingSink {
        fn append(&self, _entry: BoxEntry) {
            *self.count.lock().unwrap() += 1;
        }
        fn flush_async(&self) -> FlushWait {
            FlushWait::ready()
        }
    }

    /// Entries appended concurrently with `resolve()` must never be lost, no
    /// matter which side of the race each append lands on (buffered then
    /// drained, or forwarded directly).
    fn concurrent_appends_racing_resolve_lose_nothing() {
        const THREADS: u64 = 2;
        const PER_THREAD: u64 = 2;
        const TOTAL: u64 = THREADS * PER_THREAD;

        let (sink, resolver) = new((TOTAL as usize) + 16);
        let count = Arc::new(Mutex::new(0u64));

        let appenders: Vec<_> = (0..THREADS)
            .map(|t| {
                let sink = sink.clone();
                shuttle::thread::spawn(move || {
                    for i in 0..PER_THREAD {
                        sink.append_any(TestEntry(t * PER_THREAD + i));
                    }
                })
            })
            .collect();

        resolver.resolve(BoxEntrySink::new(CountingSink {
            count: count.clone(),
        }));

        for a in appenders {
            a.join().unwrap();
        }

        assert_eq!(*count.lock().unwrap(), TOTAL);
    }

    #[test]
    fn concurrent_appends_racing_resolve_lose_nothing_pct() {
        shuttle::check_pct(concurrent_appends_racing_resolve_lose_nothing, 2_000, 3);
    }

    #[test]
    fn concurrent_appends_racing_resolve_lose_nothing_determinism() {
        shuttle::check_uncontrolled_nondeterminism(
            concurrent_appends_racing_resolve_lose_nothing,
            2_000,
        );
    }

    /// The resolver can be dropped without ever calling `resolve()`
    /// (documented behavior: cancels the sink, discarding buffered entries)
    /// concurrently with in-flight `append()` calls. Unlike the resolve()
    /// race above, there's no "must not lose entries" guarantee here --
    /// discarding is the documented outcome -- so the invariant this checks
    /// is narrower but still real: this race must never panic or hang, for
    /// every interleaving shuttle explores, regardless of whether each
    /// append's `cancelled` check lands before or after the drop takes
    /// effect.
    fn concurrent_append_racing_resolver_drop() {
        const THREADS: u64 = 2;
        const PER_THREAD: u64 = 2;

        let (sink, resolver) = new(64);

        let appenders: Vec<_> = (0..THREADS)
            .map(|t| {
                let sink = sink.clone();
                shuttle::thread::spawn(move || {
                    for i in 0..PER_THREAD {
                        sink.append_any(TestEntry(t * PER_THREAD + i));
                    }
                })
            })
            .collect();

        // Cancels the sink; may race with any of the appends above.
        drop(resolver);

        for a in appenders {
            a.join().unwrap();
        }
        // Reaching here without panicking or hanging is the test.
    }

    #[test]
    fn concurrent_append_racing_resolver_drop_pct() {
        shuttle::check_pct(concurrent_append_racing_resolver_drop, 2_000, 3);
    }

    #[test]
    fn concurrent_append_racing_resolver_drop_determinism() {
        shuttle::check_uncontrolled_nondeterminism(concurrent_append_racing_resolver_drop, 2_000);
    }

    /// `flush_async()` can be called concurrently with `resolve()` itself,
    /// not just after it like the test above does with `append()`. Before
    /// resolution it's documented to be a no-op (see the real-thread test
    /// `flush_is_noop_while_pending`); this checks that calling it
    /// concurrently with `resolve()` never panics or hangs whichever side of
    /// the race it lands on, and that doing so doesn't interfere with the
    /// core no-entry-loss guarantee entries appended concurrently still
    /// depend on.
    fn concurrent_flush_racing_resolve_lose_nothing() {
        const THREADS: u64 = 2;
        const PER_THREAD: u64 = 2;
        const TOTAL: u64 = THREADS * PER_THREAD;

        let (sink, resolver) = new((TOTAL as usize) + 16);
        let count = Arc::new(Mutex::new(0u64));

        let appenders: Vec<_> = (0..THREADS)
            .map(|t| {
                let sink = sink.clone();
                shuttle::thread::spawn(move || {
                    for i in 0..PER_THREAD {
                        sink.append_any(TestEntry(t * PER_THREAD + i));
                    }
                })
            })
            .collect();

        let flusher = {
            let sink = sink.clone();
            shuttle::thread::spawn(move || {
                // Not polled to completion -- constructing it without panicking
                // is what this test checks; see the comment above.
                let _flush = AnyEntrySink::flush_async(&sink);
            })
        };

        resolver.resolve(BoxEntrySink::new(CountingSink {
            count: count.clone(),
        }));

        for a in appenders {
            a.join().unwrap();
        }
        flusher.join().unwrap();

        assert_eq!(*count.lock().unwrap(), TOTAL);
    }

    #[test]
    fn concurrent_flush_racing_resolve_lose_nothing_pct() {
        shuttle::check_pct(concurrent_flush_racing_resolve_lose_nothing, 2_000, 3);
    }

    #[test]
    fn concurrent_flush_racing_resolve_lose_nothing_determinism() {
        shuttle::check_uncontrolled_nondeterminism(
            concurrent_flush_racing_resolve_lose_nothing,
            2_000,
        );
    }
}
