//! Background worker thread sink for aggregation

use std::{
    marker::PhantomData,
    sync::Arc,
    sync::mpsc::{RecvTimeoutError, Sender, channel},
    thread,
    time::{Duration, Instant},
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
            let mut last_flush = Instant::now();
            loop {
                let time_until_flush = flush_interval.saturating_sub(last_flush.elapsed());
                match receiver.recv_timeout(time_until_flush) {
                    Ok(QueueMessage::Entry(entry)) => {
                        inner.merge(entry);
                        if last_flush.elapsed() >= flush_interval {
                            inner.flush();
                            last_flush = Instant::now();
                        }
                    }
                    Ok(QueueMessage::Flush(sender)) => {
                        inner.flush();
                        last_flush = Instant::now();
                        let _ = sender.send(());
                    }
                    Err(RecvTimeoutError::Timeout) => {
                        inner.flush();
                        last_flush = Instant::now();
                    }
                    Err(RecvTimeoutError::Disconnected) => {
                        inner.flush();
                        return;
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct CountingSink {
        flushes: Arc<AtomicUsize>,
    }

    impl AggregateSink<()> for CountingSink {
        fn merge(&mut self, _entry: ()) {}
    }

    impl FlushableSink for CountingSink {
        fn flush(&mut self) {
            self.flushes.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[test]
    fn worker_flushes_and_exits_when_all_senders_dropped() {
        let flushes = Arc::new(AtomicUsize::new(0));
        let sink = WorkerSink::<(), _>::new(
            CountingSink {
                flushes: flushes.clone(),
            },
            Duration::from_secs(60),
        );
        let handle = Arc::clone(&sink._handle);
        drop(sink);

        let handle = Arc::into_inner(handle).expect("test holds the only handle ref");
        let (tx, rx) = std::sync::mpsc::channel();
        thread::spawn(move || {
            handle.join().expect("worker thread panicked");
            let _ = tx.send(());
        });
        rx.recv_timeout(Duration::from_secs(5))
            .expect("worker thread did not exit within 5s of disconnect");

        assert_eq!(
            flushes.load(Ordering::SeqCst),
            1,
            "worker should flush exactly once before exiting on disconnect",
        );
    }
}
