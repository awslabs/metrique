// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Contains various utilities for working with [EntrySink]

use std::sync::{Arc, Mutex};

use crate::Entry;

#[cfg(feature = "background_queue")]
mod background;
mod immediate_flush;
mod metrics;

#[cfg(feature = "background_queue")]
pub use background::{BACKGROUND_QUEUE_METRICS, describe_sink_metrics};
#[cfg(feature = "background_queue")]
pub use background::{BackgroundQueue, BackgroundQueueBuilder, BackgroundQueueJoinHandle};
pub use immediate_flush::{
    AnyFlushImmediately, FlushImmediately, FlushImmediatelyBuilder,
    describe_immediate_flush_metrics,
};
pub use metrique_writer_core::sink::{AnyEntrySink, AppendOnDrop, FlushWait};
use metrique_writer_core::{BoxEntrySink, EntryIoStream, EntrySink};
pub use metrique_writer_core::{
    global::AttachGlobalEntrySink, global::AttachHandle, global_entry_sink,
};

/// Extension trait for `AttachGlobalEntrySink`, containing functions that use
/// types that are not present in [`metrique_writer_core`].
pub trait AttachGlobalEntrySinkExt: AttachGlobalEntrySink {
    /// Attach the given output stream to a default [`BackgroundQueue`] and then to this
    /// global queue reference.
    ///
    /// # Panics
    /// Panics if a queue is already attached.
    fn attach_to_stream(output: impl EntryIoStream + Send + 'static) -> AttachHandle {
        Self::attach(BackgroundQueue::new(output))
    }
}

impl<Q: AttachGlobalEntrySink + ?Sized> AttachGlobalEntrySinkExt for Q {}

/// In-memory sink backed by a [`Vec`] designed for testing.
///
/// Cloning will provide another reference to the same underlying sink.
///
/// # Example
/// ```
/// # use metrique_writer::{Entry, EntrySink, sink::VecEntrySink};
/// #[derive(Entry, PartialEq, Debug)]
/// struct MyEntry { counter: u64 }
///
/// let sink = VecEntrySink::default();
/// sink.append(MyEntry { counter: 21 });
/// sink.append(MyEntry { counter: 42 });
/// assert_eq!(sink.drain(), &[MyEntry { counter: 21 }, MyEntry { counter: 42 }]);
/// ```
#[derive(Debug)]
pub struct VecEntrySink<E>(Arc<Mutex<Vec<E>>>);

impl<E> Default for VecEntrySink<E> {
    fn default() -> Self {
        Self(Default::default())
    }
}

impl<E> Clone for VecEntrySink<E> {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

impl<E: Entry> EntrySink<E> for VecEntrySink<E> {
    fn append(&self, entry: E) {
        self.0.lock().unwrap().push(entry);
    }

    fn flush_async(&self) -> FlushWait {
        FlushWait::ready()
    }
}

impl<E> VecEntrySink<E> {
    /// Create a new, empty [VecEntrySink]
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a new [`VecEntrySink`] using initial capacity for entries, to avoid
    /// unnecessary reallocations.
    ///
    /// The between this function and [`VecEntrySink::new`] is purely performance,
    /// in both cases, the [`VecEntrySink`] will resize itself if needed to hold
    /// a number of entries limited only by available memory.
    pub fn with_capacity(capacity: usize) -> Self {
        Self(Arc::new(Mutex::new(Vec::with_capacity(capacity))))
    }

    /// Drains all currently appended entries from the sink and returns them as an owned [`Vec`].
    ///
    /// The sink can still be used afterwards.
    pub fn drain(&self) -> Vec<E> {
        let mut entries = self.0.lock().unwrap();
        let empty = Vec::with_capacity(entries.capacity());
        std::mem::replace(&mut entries, empty)
    }

    /// Returns true if this [`VecEntrySink`] contains an entry which evaluates the predicate to true.
    pub fn contains_entry<F>(&self, predicate: F) -> bool
    where
        F: FnMut(&E) -> bool,
    {
        let entries = self.0.lock().unwrap();
        entries.iter().any(predicate)
    }
}

/// An [EntrySink] that drops all entries.
///
/// Useful for testing, or when you want to ignore entries.
#[derive(Copy, Clone, Default)]
#[non_exhaustive]
pub struct DevNullSink;

impl DevNullSink {
    /// Return a new [`DevNullSink`]
    pub const fn new() -> Self {
        DevNullSink
    }

    /// Return a new [`DevNullSink`] as a [`BoxEntrySink`]
    pub fn boxed() -> BoxEntrySink {
        Self::new().boxed()
    }
}

impl AnyEntrySink for DevNullSink {
    fn append_any(&self, _entry: impl Entry + Send + 'static) {}

    fn flush_async(&self) -> FlushWait {
        FlushWait::ready()
    }
}

#[cfg(test)]
mod tests {
    use std::time::SystemTime;

    use super::*;
    struct TestEntry {
        timestamp: SystemTime,
        counter: u32,
        status: String,
    }

    impl Entry for TestEntry {
        fn write<'a>(&'a self, writer: &mut impl crate::EntryWriter<'a>) {
            writer.timestamp(self.timestamp);
            writer.value("counter", &self.counter);
            writer.value("status", &self.status);
        }
    }

    #[test]
    fn vec_entry_sink_create_update_drain() {
        let sink = VecEntrySink::<TestEntry>::new();

        sink.append(TestEntry {
            timestamp: SystemTime::now(),
            counter: 1,
            status: "OK".into(),
        });
        sink.append(TestEntry {
            timestamp: SystemTime::now(),
            counter: 2,
            status: "OK".into(),
        });
        sink.append(TestEntry {
            timestamp: SystemTime::now(),
            counter: 0,
            status: "ERR".into(),
        });

        assert!(sink.contains_entry(|e| e.status == "ERR"));
        assert!(sink.contains_entry(|e| e.status == "OK" && e.counter != 0));

        let entries = sink
            .drain()
            .into_iter()
            .map(|e| (e.status, e.counter))
            .collect::<Vec<_>>();

        assert_eq!(
            entries,
            &[("OK".into(), 1), ("OK".into(), 2), ("ERR".into(), 0),]
        );

        assert!(!sink.contains_entry(|e| e.status == "ERR"));
        assert!(!sink.contains_entry(|e| e.status == "OK" && e.counter != 0));
        assert!(!sink.contains_entry(|_| true));
    }

    #[test]
    fn test_null_entry_sink() {
        let sink = DevNullSink::new();
        sink.append(TestEntry {
            timestamp: SystemTime::now(),
            counter: 1,
            status: "OK".into(),
        });
        futures::executor::block_on(EntrySink::<TestEntry>::flush_async(&sink));
    }
}
