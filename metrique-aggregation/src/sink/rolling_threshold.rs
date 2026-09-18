use std::marker::PhantomData;

use histogram::{Config, Histogram};

use crate::traits::{AggregateSink, FlushableSink};

/// Approximately selects the highest-scoring entries using the previous flush window's cutoff.
///
/// Unlike [`super::TopNSink`], this sink does not retain full entries. It builds a fixed-size
/// histogram of scores during each flush window and immediately forwards entries whose scores are
/// at or above the cutoff learned from the previous non-empty window.
///
/// This trades exact selection for fixed memory use:
///
/// - The first window trains the cutoff and forwards no entries.
/// - The number forwarded may differ from `target_per_window` because histogram buckets are
///   approximate and multiple entries may share the cutoff score.
/// - A distribution shift is reflected one window late and can temporarily forward too many or
///   too few entries.
/// - Empty windows retain the last learned cutoff.
///
/// Higher scores are considered worse. A target of zero is allowed and forwards no entries.
pub struct RollingThresholdSink<T, Sink, ScoreFn> {
    target_per_window: u64,
    threshold: Option<u64>,
    observed: u64,
    scores: Histogram,
    score: ScoreFn,
    sink: Sink,
    entry: PhantomData<fn(T)>,
}

impl<T, Sink, ScoreFn> RollingThresholdSink<T, Sink, ScoreFn>
where
    ScoreFn: FnMut(&T) -> u64,
{
    /// Creates a sink targeting approximately `target_per_window` entries per flush window.
    pub fn new(target_per_window: u64, score: ScoreFn, sink: Sink) -> Self {
        let config = Config::new(4, 64).expect("known-good histogram configuration");
        Self {
            target_per_window,
            threshold: None,
            observed: 0,
            scores: Histogram::with_config(&config),
            score,
            sink,
            entry: PhantomData,
        }
    }

    /// Returns the cutoff learned from the previous non-empty flush window.
    pub fn threshold(&self) -> Option<u64> {
        self.threshold
    }

    fn next_threshold(&self) -> Option<u64> {
        if self.observed == 0 || self.target_per_window == 0 {
            return None;
        }
        if self.observed <= self.target_per_window {
            return Some(0);
        }

        let first_tail_rank = self.observed - self.target_per_window + 1;
        let percentile = 100.0 * first_tail_rank as f64 / self.observed as f64;
        self.scores
            .percentile(percentile)
            .expect("the calculated percentile is in range")
            // Favor false positives over excluding entries that may belong to the estimated tail.
            .map(|bucket| bucket.start())
    }
}

impl<T, Sink, ScoreFn> AggregateSink<T> for RollingThresholdSink<T, Sink, ScoreFn>
where
    Sink: AggregateSink<T>,
    ScoreFn: FnMut(&T) -> u64,
{
    fn merge(&mut self, entry: T) {
        if self.target_per_window == 0 {
            return;
        }

        let score = (self.score)(&entry);
        self.scores
            .increment(score)
            .expect("the histogram accepts every u64 score");
        self.observed = self.observed.saturating_add(1);

        if self.threshold.is_some_and(|threshold| score >= threshold) {
            self.sink.merge(entry);
        }
    }
}

impl<T, Sink, ScoreFn> FlushableSink for RollingThresholdSink<T, Sink, ScoreFn>
where
    Sink: AggregateSink<T> + FlushableSink,
    ScoreFn: FnMut(&T) -> u64,
{
    fn flush(&mut self) {
        if self.observed > 0 {
            self.threshold = self.next_threshold();
            self.observed = 0;
            self.scores.as_mut_slice().fill(0);
        }
        self.sink.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    struct Request {
        latency_ms: u64,
    }

    #[derive(Default)]
    struct CollectingSink {
        requests: Vec<Request>,
        flushes: usize,
    }

    impl AggregateSink<Request> for CollectingSink {
        fn merge(&mut self, request: Request) {
            self.requests.push(request);
        }
    }

    impl FlushableSink for CollectingSink {
        fn flush(&mut self) {
            self.flushes = self.flushes.saturating_add(1);
        }
    }

    fn test_sink(
        target: u64,
    ) -> RollingThresholdSink<Request, CollectingSink, impl FnMut(&Request) -> u64> {
        RollingThresholdSink::new(
            target,
            |request: &Request| request.latency_ms,
            CollectingSink::default(),
        )
    }

    fn add_window(
        sink: &mut RollingThresholdSink<Request, CollectingSink, impl FnMut(&Request) -> u64>,
        scores: impl IntoIterator<Item = u64>,
    ) {
        for latency_ms in scores {
            sink.merge(Request { latency_ms });
        }
        sink.flush();
    }

    #[test]
    fn learns_from_one_window_and_selects_the_next() {
        let mut sink = test_sink(10);

        add_window(&mut sink, 1..=100);
        assert!(sink.sink.requests.is_empty());

        add_window(&mut sink, 1..=100);

        assert!((10..=16).contains(&sink.sink.requests.len()));
        assert!(
            sink.sink
                .requests
                .iter()
                .all(|request| request.latency_ms >= 80)
        );
        assert_eq!(sink.sink.flushes, 2);
    }

    #[test]
    fn reacts_to_a_distribution_shift_one_window_late() {
        let mut sink = test_sink(10);

        add_window(&mut sink, 1..=100);
        add_window(&mut sink, 1_000..=1_099);
        let selected_during_shift = sink.sink.requests.len();
        add_window(&mut sink, 1_000..=1_099);
        let selected_after_relearning = sink.sink.requests.len() - selected_during_shift;

        assert_eq!(selected_during_shift, 100);
        assert!((10..=16).contains(&selected_after_relearning));
    }

    #[test]
    fn empty_windows_keep_the_last_threshold() {
        let mut sink = test_sink(10);

        add_window(&mut sink, 1..=100);
        let threshold = sink.threshold();
        sink.flush();

        assert_eq!(sink.threshold(), threshold);
    }

    #[test]
    fn target_larger_than_window_selects_every_entry_next_time() {
        let mut sink = test_sink(100);

        add_window(&mut sink, 1..=10);
        add_window(&mut sink, 1..=10);

        assert_eq!(sink.sink.requests.len(), 10);
    }

    #[test]
    fn zero_target_never_selects_entries() {
        let mut sink = test_sink(0);

        add_window(&mut sink, 1..=100);
        add_window(&mut sink, 1..=100);

        assert!(sink.sink.requests.is_empty());
        assert_eq!(sink.threshold(), None);
    }
}
