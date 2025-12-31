//! Keyed aggregation sink with background thread

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

use metrique_writer::BoxEntrySink;

use crate::aggregate::AggregateEntry;

/// [`KeyedAggregationSink`] uses a HashMap to aggregate a set of keys
///
/// It is fronted by a channel, and serviced by a dedicated background thread.
///
/// It emits aggregated entry to a secondary sink, `Sink`. The interval and conditions for aggregation
/// are configurable.
pub struct KeyedAggregationSink<T: AggregateEntry, Sink = BoxEntrySink> {
    sender: Sender<T::Source>,
    _handle: Arc<thread::JoinHandle<()>>,
    _phantom: PhantomData<Sink>,
}

impl<T, Sink> KeyedAggregationSink<T, Sink>
where
    T: AggregateEntry + Send + 'static,
    T::Source: Send,
    T::Aggregated: metrique_writer::Entry + Send,
    Sink: metrique_writer::EntrySink<T::Aggregated> + Send + 'static,
{
    /// Create a new keyed aggregation sink with a flush interval
    pub fn new(sink: Sink, flush_interval: Duration) -> Self {
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
                            sink.append(aggregated);
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
        let _ = self.sender.send(entry);
    }
}
