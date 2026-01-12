//! Keyed aggregation sink with background thread

use metrique_core::CloseValue;
use std::{
    marker::PhantomData,
    sync::Arc,
    sync::mpsc::{Sender, channel},
    thread,
    time::Duration,
};
use tokio::sync::oneshot;

use metrique_writer::BoxEntrySink;

use crate::traits::{AggregateStrategy, AggregateTy, FlushableSink, Key, KeyTy, Merge, MergeRef};

/// The Entry type you have when merging entries
pub type AggregatedEntry<T> = crate::traits::AggregationResult<
    <<<T as AggregateStrategy>::Key as Key<<T as AggregateStrategy>::Source>>::Key<'static> as CloseValue>::Closed,
    <<<T as AggregateStrategy>::Source as Merge>::Merged as CloseValue>::Closed,
>;

/// Keyed aggregator that uses a HashMap to aggregate entries by key
///
/// This is the core aggregation logic without any threading or channel concerns.
pub struct KeyedAggregator<T: AggregateStrategy, Sink = BoxEntrySink> {
    storage: hashbrown::HashMap<KeyTy<'static, T>, AggregateTy<T>>,
    sink: Sink,
    _phantom: PhantomData<T>,
}

impl<T, Sink> KeyedAggregator<T, Sink>
where
    T: AggregateStrategy,
    <T::Source as Merge>::MergeConfig: Default,
    Sink: metrique_writer::EntrySink<AggregatedEntry<T>>,
{
    /// Create a new keyed aggregator
    pub fn new(sink: Sink) -> Self {
        Self {
            storage: Default::default(),
            sink,
            _phantom: PhantomData,
        }
    }
}

use hashbrown::hash_map::RawEntryMut;
use std::hash::BuildHasher;

impl<T, Sink> KeyedAggregator<T, Sink>
where
    T: AggregateStrategy,
    <T::Source as Merge>::MergeConfig: Default,
    Sink: metrique_writer::EntrySink<AggregatedEntry<T>>,
{
    fn get_or_create_accum<'a>(
        storage: &'a mut hashbrown::HashMap<KeyTy<'static, T>, AggregateTy<T>>,
        entry: &T::Source,
    ) -> &'a mut AggregateTy<T> {
        let borrowed_key = T::Key::from_source(entry);
        let hash = storage.hasher().hash_one(&borrowed_key);

        match storage
            .raw_entry_mut()
            .from_hash(hash, |k| T::Key::static_key_matches(k, &borrowed_key))
        {
            RawEntryMut::Occupied(occupied) => occupied.into_mut(),
            RawEntryMut::Vacant(vacant) => {
                let static_key = T::Key::static_key(&borrowed_key);
                let new_value = T::Source::new_merged(&Default::default());
                vacant.insert_hashed_nocheck(hash, static_key, new_value).1
            }
        }
    }
}

impl<T, Sink> crate::traits::AggregateSink<T::Source> for KeyedAggregator<T, Sink>
where
    T: AggregateStrategy,
    <T::Source as Merge>::MergeConfig: Default,
    Sink: metrique_writer::EntrySink<AggregatedEntry<T>>,
{
    fn merge(&mut self, entry: T::Source) {
        let accum = Self::get_or_create_accum(&mut self.storage, &entry);
        T::Source::merge(accum, entry);
    }
}

impl<T, Sink> crate::traits::AggregateSinkRef<T::Source> for KeyedAggregator<T, Sink>
where
    T: AggregateStrategy,
    T::Source: MergeRef,
    <T::Source as Merge>::MergeConfig: Default,
    Sink: metrique_writer::EntrySink<AggregatedEntry<T>>,
{
    fn merge_ref(&mut self, entry: &T::Source) {
        let accum = Self::get_or_create_accum(&mut self.storage, entry);
        T::Source::merge_ref(accum, entry);
    }
}

impl<T, Sink> FlushableSink for KeyedAggregator<T, Sink>
where
    T: AggregateStrategy,
    Sink: metrique_writer::EntrySink<AggregatedEntry<T>>,
{
    fn flush(&mut self) {
        for (key, aggregated) in self.storage.drain() {
            let merged = crate::traits::AggregationResult {
                key: key.close(),
                aggregated: aggregated.close(),
            };
            self.sink.append(merged);
        }
    }
}

enum QueueMessage<T> {
    Entry(T),
    Flush(oneshot::Sender<()>),
}

/// Wraps any AggregateSink with a channel and background thread
pub struct WorkerAggregator<T, Inner> {
    sender: Sender<QueueMessage<T>>,
    _handle: Arc<thread::JoinHandle<()>>,
    _phantom: PhantomData<Inner>,
}

impl<T, Inner> Clone for WorkerAggregator<T, Inner> {
    fn clone(&self) -> Self {
        Self {
            sender: self.sender.clone(),
            _handle: self._handle.clone(),
            _phantom: PhantomData,
        }
    }
}

impl<T, Inner> WorkerAggregator<T, Inner>
where
    T: Send + 'static,
    Inner: crate::traits::AggregateSink<T> + FlushableSink + Send + 'static,
{
    /// Create a new background thread sink
    pub fn new(mut inner: Inner, flush_interval: Duration) -> Self {
        let (sender, receiver) = channel();

        let handle = thread::spawn(move || {
            loop {
                match receiver.recv_timeout(flush_interval) {
                    Ok(QueueMessage::Entry(entry)) => {
                        inner.merge(entry);
                    }
                    Ok(QueueMessage::Flush(sender)) => {
                        inner.flush();
                        let _ = sender.send(());
                    }
                    Err(_) => {
                        inner.flush();
                    }
                }
            }
        });

        Self {
            sender,
            _handle: Arc::new(handle),
            _phantom: PhantomData,
        }
    }

    /// Send an entry to be aggregated
    pub fn send(&self, entry: T) {
        let _ = self.sender.send(QueueMessage::Entry(entry));
    }

    /// Flush all pending entries
    pub async fn flush(&self) {
        let (tx, rx) = oneshot::channel();
        let _ = self.sender.send(QueueMessage::Flush(tx));
        rx.await.unwrap()
    }
}

impl<T, Inner> crate::traits::RootSink<T> for WorkerAggregator<T, Inner>
where
    T: Send + 'static,
    Inner: crate::traits::AggregateSink<T> + FlushableSink + Send + 'static,
{
    fn merge(&self, entry: T) {
        self.send(entry);
    }
}
