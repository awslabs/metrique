//! Keyed aggregation sink with background thread

use metrique_core::CloseValue;
use std::{
    collections::HashMap,
    marker::PhantomData,
    sync::{
        Arc,
        mpsc::{Sender, channel},
    },
    thread,
    time::Duration,
};
use tokio::sync::oneshot;

use metrique_writer::BoxEntrySink;

use crate::traits::{AggregateStrategy, AggregateTy, Key, KeyTy, Merge};

enum QueueMessage<T> {
    Entry(T),
    Flush(oneshot::Sender<()>),
}

/// [`KeyedAggregationSink`] uses a HashMap to aggregate a set of keys
///
/// It is fronted by a channel, and serviced by a dedicated background thread.
///
/// It emits aggregated entry to a secondary sink, `Sink`. The interval and conditions for aggregation
/// are configurable.
#[derive(Clone)]
pub struct KeyedAggregationSink<T: AggregateStrategy, Sink = BoxEntrySink> {
    sender: Sender<QueueMessage<T::Source>>,
    _handle: Arc<thread::JoinHandle<()>>,
    _phantom: PhantomData<Sink>,
}

/// The Entry type you have when merging entries
pub type AggregatedEntry<T> = crate::traits::AggregationResult<
    <<<T as AggregateStrategy>::Key as Key<<T as AggregateStrategy>::Source>>::Key<'static> as CloseValue>::Closed,
    <<<T as AggregateStrategy>::Source as Merge>::Merged as CloseValue>::Closed,
>;

impl<T, Sink> KeyedAggregationSink<T, Sink>
where
    T: AggregateStrategy,
    T::Source: Send,
    <T::Source as Merge>::Merged: Send,
    <T::Source as Merge>::MergeConfig: Default,
    Sink: metrique_writer::EntrySink<AggregatedEntry<T>> + Send + 'static,
{
    /// Create a new keyed aggregation sink with a flush interval
    pub fn new(sink: Sink, flush_interval: Duration) -> KeyedAggregationSink<T, Sink> {
        let (sender, receiver) = channel();
        let mut storage: HashMap<KeyTy<'static, T>, AggregateTy<T>> = HashMap::new();

        let handle = thread::spawn(move || {
            loop {
                match receiver.recv_timeout(flush_interval) {
                    Ok(QueueMessage::Entry(entry)) => {
                        // TODO: optimize this with hashbrown to avoid needing to always create a static key
                        let key = T::Key::static_key(&T::Key::from_source(&entry));
                        let accum = storage
                            .entry(key)
                            .or_insert_with(|| T::Source::new_merged(&Default::default()));
                        T::Source::merge(accum, entry);
                    }
                    Ok(QueueMessage::Flush(sender)) => {
                        for (key, aggregated) in storage.drain() {
                            let merged = crate::traits::AggregationResult {
                                key: key.close(),
                                aggregated: aggregated.close(),
                            };
                            sink.append(merged);
                        }
                        let _ = sender.send(());
                    }
                    Err(_) => {
                        for (key, aggregated) in storage.drain() {
                            let merged = crate::traits::AggregationResult {
                                key: key.close(),
                                aggregated: aggregated.close(),
                            };
                            sink.append(merged);
                        }
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
    pub fn send(&self, entry: T::Source) {
        let _ = self.sender.send(QueueMessage::Entry(entry));
    }

    /// Flush all pending entries
    ///
    /// Returns when all entries sent before this call have been processed
    pub async fn flush(&self) {
        let (tx, rx) = oneshot::channel();
        let _ = self.sender.send(QueueMessage::Flush(tx));
        rx.await.unwrap()
    }
}
