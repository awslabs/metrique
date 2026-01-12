//! Background worker thread sink for aggregation

use std::{
    marker::PhantomData,
    sync::Arc,
    sync::mpsc::{Sender, channel},
    thread,
    time::Duration,
};
use tokio::sync::oneshot;

use crate::traits::{AggregateSink, FlushableSink, RootSink};

enum QueueMessage<T> {
    Entry(T),
    Flush(oneshot::Sender<()>),
}

/// Wraps any AggregateSink with a channel and background thread
pub struct WorkerSink<T, Inner> {
    sender: Sender<QueueMessage<T>>,
    _handle: Arc<thread::JoinHandle<()>>,
    _phantom: PhantomData<Inner>,
}

impl<T, Inner> Clone for WorkerSink<T, Inner> {
    fn clone(&self) -> Self {
        Self {
            sender: self.sender.clone(),
            _handle: self._handle.clone(),
            _phantom: PhantomData,
        }
    }
}

impl<T, Inner> WorkerSink<T, Inner>
where
    T: Send + 'static,
    Inner: AggregateSink<T> + FlushableSink + Send + 'static,
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

impl<T, Inner> RootSink<T> for WorkerSink<T, Inner>
where
    T: Send + 'static,
    Inner: AggregateSink<T> + FlushableSink + Send + 'static,
{
    fn merge(&self, entry: T) {
        self.send(entry);
    }
}
