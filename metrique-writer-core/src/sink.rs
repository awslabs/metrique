// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Contains the [`EntrySink`] trait, which provides sinks into which metric entries
//! can be written. Unlike [`EntryIoStream`], these can be asynchronous.
//!
//! [`EntryIoStream`]: crate::stream::EntryIoStream

use std::{
    fmt::Debug,
    ops::{Deref, DerefMut},
    pin::Pin,
    sync::Arc,
};

use crate::{Entry, entry::BoxEntry};

/// Stores entries in an in-memory buffer until they can be written to the destination.
///
/// Implementations of this trait normally manage a queueing policy, then pass the [`Entry`]
/// to an [`EntryIoStream`] (in `metrique-writer`, there is `FlushImmediately` with a trivial queueing
/// policy, and `BackgroundQueue` which flushes entries via a queue).
///
/// [`EntryIoStream`]: crate::stream::EntryIoStream
pub trait EntrySink<E: Entry> {
    /// Append the `entry` to the in-memory buffer. Unless this is explicitly a test sink, the `append()` call must
    /// never block and must never panic. Test sinks are encouraged to immediately panic on invalid entries. Production
    /// sinks should emit a `tracing` event when invalid entries are found.
    ///
    /// If the in-memory buffer is bounded and full, the oldest entries should be dropped. More recent entries are more
    /// valuable for monitoring service health.
    fn append(&self, entry: E);

    /// Request the sink to flush its contents to some sort of persistent storage. The returned
    /// `FlushWait` can be used to tell when the sink is flushed.
    ///
    /// In synchronous code, you can use `pollster::block_on` or `futures::executor::block_on` to
    /// wait for this future to complete.
    fn flush_async(&self) -> FlushWait;

    /// Wrap `entry` in a smart pointer that will automatically append it to this sink when dropped.
    ///
    /// This will help enforce that an entry is always appended even if it's used across branching business logic. Note
    /// that Rust can't guarantee that the entry is dropped (e.g. `forget(entry)`).
    ///
    /// # Example
    /// ```
    /// # use metrique_writer::{Entry, sink::VecEntrySink, EntrySink};
    /// #[derive(Entry, PartialEq, Debug)]
    /// struct MyEntry {
    ///     counter: u64,
    /// }
    ///
    /// let sink = VecEntrySink::default();
    /// {
    ///     let mut entry = sink.append_on_drop(MyEntry { counter: 0 });
    ///     // do some business logic
    ///     entry.counter += 1;
    /// }
    /// assert_eq!(sink.drain(), &[MyEntry { counter: 1}]);
    /// ```
    fn append_on_drop(&self, entry: E) -> AppendOnDrop<E, Self>
    where
        Self: Sized + Clone,
    {
        AppendOnDrop::new(entry, self.clone())
    }

    /// See [`EntrySink::append_on_drop()`].
    fn append_on_drop_default(&self) -> AppendOnDrop<E, Self>
    where
        Self: Sized + Clone,
        E: Default,
    {
        self.append_on_drop(E::default())
    }
}

/// Provides a more generic interface than [`EntrySink`] but may come at the cost of dynamic dispatch and heap
/// allocation to store the in-memory buffer.
pub trait AnyEntrySink {
    /// Generic version of [`EntrySink::append()`] with the same contract.
    fn append_any(&self, entry: impl Entry + Send + 'static);

    /// Request the sink to flush its contents and wait until they are flushed.
    ///
    /// In synchronous code, you can use `pollster::block_on` or `futures::executor::block_on` to
    /// wait for this future to complete.
    fn flush_async(&self) -> FlushWait;

    /// Returns a [`BoxEntrySink`] that is a type-erased version of this entry sink
    fn boxed(self) -> BoxEntrySink
    where
        Self: Sized + Send + Sync + 'static,
    {
        BoxEntrySink::new(self)
    }
}

impl<T: AnyEntrySink, E: Entry + Send + 'static> EntrySink<E> for T {
    fn flush_async(&self) -> FlushWait {
        AnyEntrySink::flush_async(self)
    }

    fn append(&self, entry: E) {
        self.append_any(entry)
    }
}

/// A type-erased [`EntrySink`], that can sink a [`BoxEntry`] (which can contain
/// an arbitrary [`Entry`]).
#[derive(Clone)]
pub struct BoxEntrySink(Arc<Box<dyn EntrySink<BoxEntry> + Send + Sync + 'static>>);

impl Debug for BoxEntrySink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("BoxEntrySink").finish()
    }
}

impl AnyEntrySink for BoxEntrySink {
    fn append_any(&self, entry: impl Entry + Send + 'static) {
        self.0.append(entry.boxed())
    }

    fn flush_async(&self) -> FlushWait {
        self.0.flush_async()
    }
}

impl BoxEntrySink {
    /// Create a new [BoxEntrySink]
    pub fn new(sink: impl EntrySink<BoxEntry> + Send + Sync + 'static) -> Self {
        Self(Arc::new(Box::new(sink)))
    }

    /// Returns a [`BoxEntrySink`] that silently discards all entries.
    ///
    /// Only available when the `test-util` feature is enabled.
    #[cfg(feature = "test-util")]
    pub fn noop() -> Self {
        struct NoopSink;
        impl EntrySink<BoxEntry> for NoopSink {
            fn append(&self, _entry: BoxEntry) {}
            fn flush_async(&self) -> FlushWait {
                FlushWait::ready()
            }
        }
        Self::new(NoopSink)
    }

    /// Returns a [`BoxEntrySink`] that defers sink resolution until entries are appended.
    ///
    /// The `factory` closure is called each time an entry is appended or flushed. If it
    /// returns `Some(sink)`, the entry is forwarded to that sink. If it returns `None`,
    /// the entry is silently discarded.
    pub fn lazy(factory: impl Fn() -> Option<BoxEntrySink> + Send + Sync + 'static) -> Self {
        Self::new(LazySink(Arc::new(factory)))
    }
}

struct LazySink(Arc<dyn Fn() -> Option<BoxEntrySink> + Send + Sync>);

impl EntrySink<BoxEntry> for LazySink {
    fn append(&self, entry: BoxEntry) {
        if let Some(sink) = (self.0)() {
            sink.0.append(entry);
        }
    }

    fn flush_async(&self) -> FlushWait {
        match (self.0)() {
            Some(sink) => sink.0.flush_async(),
            None => FlushWait::ready(),
        }
    }
}

/// This struct contains a future that can be used to wait for flushing to complete
#[must_use = "future does nothing unless polled"]
pub struct FlushWait(Pin<Box<dyn std::future::Future<Output = ()> + Send + Sync + 'static>>);

impl Future for FlushWait {
    type Output = ();

    fn poll(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        self.0.as_mut().poll(cx)
    }
}

impl FlushWait {
    /// Return a FlushWait that is ready immediately
    pub fn ready() -> Self {
        // VecEntrySink is synchronous, poll_fn is zero_sized unlike Ready<()>
        Self(Box::pin(std::future::poll_fn(|_| {
            std::task::Poll::Ready(())
        })))
    }

    /// Create a FlushWait that returns when a future is ready
    pub fn from_future(f: impl std::future::Future<Output = ()> + Send + Sync + 'static) -> Self {
        Self(Box::pin(f))
    }
}

/// Smart pointer that will append the wrapped entry to a sink when dropped.
#[derive(Debug, Clone)]
pub struct AppendOnDrop<E: Entry, Q: EntrySink<E>> {
    entry: Option<E>,
    sink: Q,
}

impl<E: Entry, Q: EntrySink<E>> AppendOnDrop<E, Q> {
    pub(crate) fn new(entry: E, sink: Q) -> Self {
        Self {
            entry: Some(entry),
            sink,
        }
    }
}

impl<E: Entry, Q: EntrySink<E>> Drop for AppendOnDrop<E, Q> {
    fn drop(&mut self) {
        if let Some(entry) = self.entry.take() {
            self.sink.append(entry)
        }
    }
}

impl<E: Entry, Q: EntrySink<E>> AppendOnDrop<E, Q> {
    /// Take and return the entry out of this [AppendOnDrop], without
    /// appending it to the sink
    pub fn into_entry(mut self) -> E {
        self.entry.take().unwrap()
    }

    /// Drop the entry, but don't append it to the sink.
    pub fn forget(mut self) {
        self.entry = None;
    }
}

impl<E: Entry, Q: EntrySink<E>> Deref for AppendOnDrop<E, Q> {
    type Target = E;

    fn deref(&self) -> &Self::Target {
        self.entry.as_ref().unwrap()
    }
}

impl<E: Entry, Q: EntrySink<E>> DerefMut for AppendOnDrop<E, Q> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.entry.as_mut().unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_stream::TestEntry;
    use std::sync::{Arc, Mutex};

    #[test]
    fn lazy_sink_forwards_when_resolved() {
        let appended = Arc::new(Mutex::new(false));
        let appended_clone = appended.clone();

        struct MarkerSink(Arc<Mutex<bool>>);
        impl EntrySink<BoxEntry> for MarkerSink {
            fn append(&self, _entry: BoxEntry) {
                *self.0.lock().unwrap() = true;
            }
            fn flush_async(&self) -> FlushWait {
                FlushWait::ready()
            }
        }

        let sink =
            BoxEntrySink::lazy(move || Some(BoxEntrySink::new(MarkerSink(appended_clone.clone()))));
        sink.append_any(TestEntry(1));
        assert!(*appended.lock().unwrap());
    }

    #[test]
    fn lazy_sink_discards_when_none() {
        let sink = BoxEntrySink::lazy(|| None);
        sink.append_any(TestEntry(1));
    }
}
