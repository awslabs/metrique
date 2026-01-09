//! Keyed aggregation sink with background thread

use metrique_core::CloseValue;
use std::{
    collections::HashMap,
    marker::PhantomData,
    sync::{
        Arc, Mutex,
        mpsc::{Sender, channel},
    },
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
    storage: Mutex<HashMap<KeyTy<'static, T>, AggregateTy<T>>>,
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
            storage: Mutex::new(HashMap::new()),
            sink,
            _phantom: PhantomData,
        }
    }
}

impl<T, Sink> crate::traits::AggregateSink<T::Source> for KeyedAggregator<T, Sink>
where
    T: AggregateStrategy,
    <T::Source as Merge>::MergeConfig: Default,
    Sink: metrique_writer::EntrySink<AggregatedEntry<T>>,
{
    fn add(&self, entry: T::Source) {
        let mut storage = self.storage.lock().unwrap();
        let key = T::Key::static_key(&T::Key::from_source(&entry));
        let accum = storage
            .entry(key)
            .or_insert_with(|| T::Source::new_merged(&Default::default()));
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
    fn add_ref(&self, entry: &T::Source) {
        let mut storage = self.storage.lock().unwrap();
        let key = T::Key::static_key(&T::Key::from_source(entry));
        let accum = storage
            .entry(key)
            .or_insert_with(|| T::Source::new_merged(&Default::default()));
        T::Source::merge_ref(accum, entry);
    }
}

impl<T, Sink> FlushableSink for KeyedAggregator<T, Sink>
where
    T: AggregateStrategy,
    Sink: metrique_writer::EntrySink<AggregatedEntry<T>>,
{
    fn flush(&self) {
        let mut storage = self.storage.lock().unwrap();
        for (key, aggregated) in storage.drain() {
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
pub struct BackgroundThreadSink<T, Inner> {
    sender: Sender<QueueMessage<T>>,
    _handle: Arc<thread::JoinHandle<()>>,
    _phantom: PhantomData<Inner>,
}

impl<T, Inner> Clone for BackgroundThreadSink<T, Inner> {
    fn clone(&self) -> Self {
        Self {
            sender: self.sender.clone(),
            _handle: self._handle.clone(),
            _phantom: PhantomData,
        }
    }
}

impl<T, Inner> BackgroundThreadSink<T, Inner>
where
    T: Send + 'static,
    Inner: crate::traits::AggregateSink<T> + FlushableSink + Send + 'static,
{
    /// Create a new background thread sink
    pub fn new(inner: Inner, flush_interval: Duration) -> Self {
        let (sender, receiver) = channel();

        let handle = thread::spawn(move || {
            loop {
                match receiver.recv_timeout(flush_interval) {
                    Ok(QueueMessage::Entry(entry)) => {
                        inner.add(entry);
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

impl<T, Inner, Strat> crate::sink::AggregateSink<Strat> for BackgroundThreadSink<T, Inner>
where
    T: Send + 'static,
    Strat: AggregateStrategy<Source = T>,
    Inner: crate::traits::AggregateSink<T> + FlushableSink + Send + 'static,
{
    fn merge(&self, entry: T) {
        self.send(entry);
    }
}

/// [`KeyedAggregationSink`] uses a HashMap to aggregate a set of keys
///
/// It is fronted by a channel, and serviced by a dedicated background thread.
///
/// It emits aggregated entry to a secondary sink, `Sink`. The interval and conditions for aggregation
/// are configurable.
pub struct KeyedAggregationSink<T: AggregateStrategy, Sink = BoxEntrySink> {
    inner: BackgroundThreadSink<T::Source, KeyedAggregator<T, Sink>>,
}

impl<T: AggregateStrategy, Sink> Clone for KeyedAggregationSink<T, Sink> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<T, Sink> KeyedAggregationSink<T, Sink>
where
    T: AggregateStrategy + Send,
    T::Source: Send,
    <T::Source as Merge>::Merged: Send,
    <T::Source as Merge>::MergeConfig: Default,
    Sink: metrique_writer::EntrySink<AggregatedEntry<T>> + Send + 'static,
{
    /// Create a new keyed aggregation sink with a flush interval
    pub fn new(sink: Sink, flush_interval: Duration) -> Self {
        let aggregator = KeyedAggregator::<T, Sink>::new(sink);
        Self {
            inner: BackgroundThreadSink::new(aggregator, flush_interval),
        }
    }

    /// Send an entry to be aggregated
    pub fn send(&self, entry: T::Source) {
        self.inner.send(entry);
    }

    /// Flush all pending entries
    pub async fn flush(&self) {
        self.inner.flush().await;
    }
}

impl<T, Sink> crate::sink::AggregateSink<T> for KeyedAggregationSink<T, Sink>
where
    T: AggregateStrategy + Send,
    T::Source: Send,
    <T::Source as Merge>::Merged: Send,
    <T::Source as Merge>::MergeConfig: Default,
    Sink: metrique_writer::EntrySink<AggregatedEntry<T>> + Send + 'static,
{
    fn merge(&self, entry: T::Source) {
        self.send(entry);
    }
}
