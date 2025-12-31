//! Keyed aggregation sink with background thread

use metrique_core::{CloseValue, InflectableEntry};
use metrique_writer::{Entry, EntryWriter};
use metrique_writer_core::entry::SampleGroupElement;
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

/// Helper that "Roots" an inflectable entry (temporary copy from metrique, needs to move to core)
pub struct RootEntry<M: InflectableEntry> {
    metric: M,
}

type RootMetric<E> = RootEntry<<E as CloseValue>::Closed>;

impl<M: InflectableEntry> RootEntry<M> {
    /// create a new [`RootEntry`]
    pub fn new(metric: M) -> Self {
        Self { metric }
    }
}

impl<M: InflectableEntry> Entry for RootEntry<M> {
    fn write<'a>(&'a self, w: &mut impl EntryWriter<'a>) {
        self.metric.write(w);
    }

    fn sample_group(&self) -> impl Iterator<Item = SampleGroupElement> {
        self.metric.sample_group()
    }
}

use metrique_writer::BoxEntrySink;

use crate::{sink::AggregateSink, traits::AggregateEntry};

/// [`KeyedAggregationSink`] uses a HashMap to aggregate a set of keys
///
/// It is fronted by a channel, and serviced by a dedicated background thread.
///
/// It emits aggregated entry to a secondary sink, `Sink`. The interval and conditions for aggregation
/// are configurable.
#[derive(Clone)]
pub struct KeyedAggregationSink<T: AggregateEntry, Sink = BoxEntrySink> {
    sender: Sender<T::Source>,
    _handle: Arc<thread::JoinHandle<()>>,
    _phantom: PhantomData<Sink>,
}

impl<T: AggregateEntry> AggregateSink<T> for KeyedAggregationSink<T> {
    fn merge(&self, entry: T::Source) {
        let _ = self.sender.send(entry);
    }
}

impl<T, Sink> KeyedAggregationSink<T, Sink>
where
    T: AggregateEntry + 'static,
    T::Aggregated: metrique_core::CloseEntry,
    Sink: metrique_writer::EntrySink<RootMetric<T::Aggregated>> + Send + 'static,
{
    /// Create a new keyed aggregation sink with a flush interval
    pub fn new(sink: Sink, flush_interval: Duration) -> KeyedAggregationSink<T, Sink> {
        let (sender, receiver) = channel();
        let mut storage = HashMap::new();

        let handle = thread::spawn(move || {
            loop {
                match receiver.recv_timeout(flush_interval) {
                    Ok(entry) => {
                        let key = T::key(&entry);
                        let aggregated = storage
                            .entry(key)
                            .or_insert_with_key(|k| T::new_aggregated(k));
                        T::merge_entry(aggregated, entry);
                    }
                    Err(_) => {
                        for (_, aggregated) in storage.drain() {
                            sink.append(RootEntry::new(metrique_core::CloseValue::close(
                                aggregated,
                            )));
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

    /// Send a raw entry to be aggregated
    pub fn send_raw(&self, entry: T::Source) {
        let _ = self.sender.send(entry);
    }

    /// Send an entry to be aggregated
    pub fn send(&self, entry: T)
    where
        T: CloseValue<Closed = T::Source>,
    {
        self.send_raw(entry.close());
    }
}
