//! Background worker thread sink for aggregation

use std::{
    marker::PhantomData,
    sync::Arc,
    sync::mpsc::RecvTimeoutError,
    time::{Duration, Instant},
};
use tokio::sync::oneshot;

use crate::traits::{AggregateSink, FlushableSink, RootSink};

// Cfg-gated concurrency primitives (std vs. shuttle). Gated on both
// `cfg(shuttle)` and `feature = "_shuttle"` so it also reaches
// builds of this crate that don't have `_shuttle` enabled (e.g. as a
// dev-dependency with different requested features) and therefore don't
// have the optional `shuttle` crate linked at all.
//
// `RecvTimeoutError` needs no swap -- shuttle re-exports std's type
// unchanged. Its `recv_timeout` never actually times out though,
// that's a known gap documented in Shuttle itself:
// https://github.com/awslabs/shuttle/blob/c8a46d3965048df3207ec920dae066bc9c4d9d89/shuttle-std/src/sync/mpsc.rs#L433
#[cfg(all(shuttle, feature = "_shuttle"))]
use shuttle::{
    sync::mpsc::{Sender, channel},
    thread,
};
#[cfg(not(all(shuttle, feature = "_shuttle")))]
use std::{
    sync::mpsc::{Sender, channel},
    thread,
};

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

// Shuttle interleaving tests for `WorkerSink`, covering merge correctness
// and clean exit on channel disconnect.
//
// Since `recv_timeout` never actually times out under shuttle (see the
// primitives import comment above), these tests don't exercise the
// periodic-flush path.
#[cfg(all(test, shuttle, feature = "_shuttle"))]
mod shuttle_tests {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    // `futures::executor::block_on`'s real thread park/unpark on wake
    // is invisible to shuttle and can deadlock the exploration. Shuttle's own
    // `block_on` polls and yields to its scheduler on `Pending` instead of
    // really blocking, using shuttle's own waker under the hood.
    use shuttle::future::block_on;

    use super::*;

    struct CollectingSink {
        merged: Arc<Mutex<Vec<u64>>>,
        flushes: Arc<AtomicUsize>,
    }

    impl AggregateSink<u64> for CollectingSink {
        fn merge(&mut self, entry: u64) {
            self.merged.lock().unwrap().push(entry);
        }
    }

    impl FlushableSink for CollectingSink {
        fn flush(&mut self) {
            self.flushes.fetch_add(1, Ordering::SeqCst);
        }
    }

    /// `flush_interval` is irrelevant under shuttle (see the module-level
    /// comment: `recv_timeout` never actually times out), so any value
    /// works; 60s just documents "this is not what's under test."
    fn flush_interval() -> Duration {
        Duration::from_secs(60)
    }

    /// Entries sent concurrently from several cloned handles are all merged
    /// by the time `flush()`'s returned future resolves, for every
    /// interleaving shuttle explores.
    fn concurrent_sends_all_merged_before_flush_returns() {
        const THREADS: u64 = 3;
        const PER_THREAD: u64 = 3;

        let merged: Arc<Mutex<Vec<u64>>> = Arc::default();
        let flushes = Arc::new(AtomicUsize::new(0));
        let sink = WorkerSink::<u64, _>::new(
            CollectingSink {
                merged: merged.clone(),
                flushes: flushes.clone(),
            },
            flush_interval(),
        );

        let senders: Vec<_> = (0..THREADS)
            .map(|t| {
                let sink = sink.clone();
                thread::spawn(move || {
                    for i in 0..PER_THREAD {
                        sink.send(t * PER_THREAD + i);
                    }
                })
            })
            .collect();
        for t in senders {
            t.join().unwrap();
        }

        block_on(sink.flush());

        let mut values = merged.lock().unwrap().clone();
        values.sort();
        assert_eq!(values, (0..(THREADS * PER_THREAD)).collect::<Vec<_>>());

        let handle = Arc::clone(&sink._handle);
        drop(sink);
        Arc::into_inner(handle)
            .expect("sole handle ref after dropping the only WorkerSink")
            .join()
            .expect("worker thread panicked");
    }

    #[test]
    fn concurrent_sends_all_merged_before_flush_returns_pct() {
        shuttle::check_pct(concurrent_sends_all_merged_before_flush_returns, 2_000, 3);
    }

    #[test]
    fn concurrent_sends_all_merged_before_flush_returns_determinism() {
        shuttle::check_uncontrolled_nondeterminism(
            concurrent_sends_all_merged_before_flush_returns,
            2_000,
        );
    }

    /// The historical bug, reproduced directly: several cloned handles send
    /// an entry and drop concurrently. The background thread must still
    /// exit (this used to hang forever -- see the module doc comment),
    /// flush exactly once at shutdown, and lose no entries, no matter how
    /// the drops and the final disconnect interleave.
    fn concurrent_drops_exit_cleanly_and_flush_once() {
        const CLONES: u64 = 2;

        let merged: Arc<Mutex<Vec<u64>>> = Arc::default();
        let flushes = Arc::new(AtomicUsize::new(0));
        let sink = WorkerSink::<u64, _>::new(
            CollectingSink {
                merged: merged.clone(),
                flushes: flushes.clone(),
            },
            flush_interval(),
        );
        let handle = Arc::clone(&sink._handle);

        let droppers: Vec<_> = (0..CLONES)
            .map(|i| {
                let sink = sink.clone();
                thread::spawn(move || {
                    sink.send(i);
                    drop(sink);
                })
            })
            .collect();

        drop(sink);
        for d in droppers {
            d.join().unwrap();
        }

        Arc::into_inner(handle)
            .expect("all WorkerSink clones dropped, sole handle ref remains")
            .join()
            .expect("worker thread panicked");

        let mut values = merged.lock().unwrap().clone();
        values.sort();
        assert_eq!(values, (0..CLONES).collect::<Vec<_>>());
        assert_eq!(flushes.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn concurrent_drops_exit_cleanly_and_flush_once_pct() {
        shuttle::check_pct(concurrent_drops_exit_cleanly_and_flush_once, 2_000, 3);
    }

    #[test]
    fn concurrent_drops_exit_cleanly_and_flush_once_determinism() {
        shuttle::check_uncontrolled_nondeterminism(
            concurrent_drops_exit_cleanly_and_flush_once,
            2_000,
        );
    }

    /// The shared mpsc channel preserves each sender's own order, so this thread's
    /// entries must be merged by the time its own `flush()` resolves.
    fn flush_resolves_after_own_prior_sends() {
        let merged: Arc<Mutex<Vec<u64>>> = Arc::default();
        let flushes = Arc::new(AtomicUsize::new(0));
        let sink = WorkerSink::<u64, _>::new(
            CollectingSink {
                merged: merged.clone(),
                flushes: flushes.clone(),
            },
            flush_interval(),
        );

        let other = {
            let sink = sink.clone();
            shuttle::thread::spawn(move || {
                for i in 100..102 {
                    sink.send(i);
                }
            })
        };

        for i in 0..2 {
            sink.send(i);
        }
        block_on(sink.flush());

        let values = merged.lock().unwrap().clone();
        for i in 0..2 {
            assert!(
                values.contains(&i),
                "flush() resolved without observing entry {i} sent before it on the same thread"
            );
        }

        other.join().unwrap();
        let handle = Arc::clone(&sink._handle);
        drop(sink);
        Arc::into_inner(handle)
            .expect("sole handle ref after dropping the only WorkerSink")
            .join()
            .expect("worker thread panicked");
    }

    #[test]
    fn flush_resolves_after_own_prior_sends_pct() {
        shuttle::check_pct(flush_resolves_after_own_prior_sends, 2_000, 3);
    }

    #[test]
    fn flush_resolves_after_own_prior_sends_determinism() {
        shuttle::check_uncontrolled_nondeterminism(flush_resolves_after_own_prior_sends, 2_000);
    }
}
