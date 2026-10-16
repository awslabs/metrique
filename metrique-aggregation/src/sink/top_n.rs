use std::{
    cmp::{Ordering, Reverse},
    collections::BinaryHeap,
};

use crate::traits::{AggregateSink, FlushableSink};

/// Retains the `N` entries with the highest scores until the sink is flushed.
///
/// A higher score means an entry is considered worse. On each flush, the retained entries are
/// merged into the inner sink and the selection is reset. The order in which retained entries are
/// merged into the inner sink is unspecified.
///
/// This sink is useful with [`crate::sink::TeeSink`]: one branch can aggregate every entry while a
/// `TopNSink` on the other branch preserves a bounded number of raw events for debugging. A
/// [`crate::sink::WorkerSink`] provides periodic flushes, making each period a separate selection
/// window.
///
/// Selection among entries with equal scores is unspecified. A limit of zero is allowed and
/// retains no entries.
///
/// # Example
///
/// ```no_run
/// use metrique::{ServiceMetrics, unit_of_work::metrics, writer::GlobalEntrySink};
/// use metrique_aggregation::{
///     aggregate,
///     aggregator::KeyedAggregator,
///     histogram::Histogram,
///     sink::{TopNSink, TeeSink, WorkerSink, non_aggregate},
/// };
/// use std::time::Duration;
///
/// #[aggregate(ref)]
/// #[metrics]
/// struct Request {
///     #[aggregate(key)]
///     operation: String,
///     #[aggregate(ignore)]
///     request_id: String,
///     #[aggregate(strategy = Histogram<Duration>)]
///     latency: Duration,
/// }
///
/// impl RequestEntry {
///     fn latency(&self) -> Duration {
///         #[allow(deprecated)]
///         self.latency
///     }
/// }
///
/// let aggregated = KeyedAggregator::<Request>::new(ServiceMetrics::sink());
/// let worst_requests = TopNSink::new(
///     100,
///     RequestEntry::latency,
///     non_aggregate(ServiceMetrics::sink()),
/// );
/// let sink = WorkerSink::new(
///     TeeSink::new(aggregated, worst_requests),
///     Duration::from_secs(60),
/// );
///
/// Request {
///     operation: "GetItem".to_string(),
///     request_id: "request-123".to_string(),
///     latency: Duration::from_millis(250),
/// }
/// .close_and_merge(sink.clone());
/// ```
pub struct TopNSink<T, Sink, ScoreFn, Score> {
    limit: usize,
    score: ScoreFn,
    selected: BinaryHeap<Reverse<ScoredEntry<T, Score>>>,
    sink: Sink,
}

impl<T, Sink, ScoreFn, Score> TopNSink<T, Sink, ScoreFn, Score>
where
    ScoreFn: FnMut(&T) -> Score,
    Score: Ord,
{
    /// Creates a sink that retains the `limit` entries with the highest scores per flush.
    pub fn new(limit: usize, score: ScoreFn, sink: Sink) -> Self {
        Self {
            limit,
            score,
            selected: BinaryHeap::new(),
            sink,
        }
    }
}

impl<T, Sink, ScoreFn, Score> AggregateSink<T> for TopNSink<T, Sink, ScoreFn, Score>
where
    ScoreFn: FnMut(&T) -> Score,
    Score: Ord,
{
    fn merge(&mut self, entry: T) {
        if self.limit == 0 {
            return;
        }

        let scored = ScoredEntry {
            score: (self.score)(&entry),
            entry,
        };

        if self.selected.len() < self.limit {
            self.selected.push(Reverse(scored));
        } else if self.selected.peek().is_some_and(|lowest| scored > lowest.0) {
            self.selected.pop();
            self.selected.push(Reverse(scored));
        }
    }
}

impl<T, Sink, ScoreFn, Score> FlushableSink for TopNSink<T, Sink, ScoreFn, Score>
where
    Sink: AggregateSink<T> + FlushableSink,
    ScoreFn: FnMut(&T) -> Score,
    Score: Ord,
{
    fn flush(&mut self) {
        while let Some(Reverse(scored)) = self.selected.pop() {
            self.sink.merge(scored.entry);
        }
        self.sink.flush();
    }
}

struct ScoredEntry<T, Score> {
    score: Score,
    entry: T,
}

impl<T, Score: Ord> Ord for ScoredEntry<T, Score> {
    fn cmp(&self, other: &Self) -> Ordering {
        self.score.cmp(&other.score)
    }
}

impl<T, Score: Ord> PartialOrd for ScoredEntry<T, Score> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<T, Score: Ord> PartialEq for ScoredEntry<T, Score> {
    fn eq(&self, other: &Self) -> bool {
        self.score == other.score
    }
}

impl<T, Score: Ord> Eq for ScoredEntry<T, Score> {}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, PartialEq, Eq)]
    struct Request {
        id: u64,
        latency: u64,
    }

    #[derive(Default)]
    struct CollectingSink {
        entries: Vec<Request>,
        flushes: usize,
    }

    impl AggregateSink<Request> for CollectingSink {
        fn merge(&mut self, entry: Request) {
            self.entries.push(entry);
        }
    }

    impl FlushableSink for CollectingSink {
        fn flush(&mut self) {
            self.flushes = self.flushes.saturating_add(1);
        }
    }

    #[test]
    fn retains_highest_scores_and_resets_after_flush() {
        let mut sink = TopNSink::new(
            2,
            |request: &Request| request.latency,
            CollectingSink::default(),
        );

        for (id, latency) in [(1, 10), (2, 50), (3, 30), (4, 100)] {
            sink.merge(Request { id, latency });
        }
        sink.flush();

        let mut first_window = sink
            .sink
            .entries
            .iter()
            .map(|request| request.id)
            .collect::<Vec<_>>();
        first_window.sort_unstable();
        assert_eq!(first_window, [2, 4]);

        sink.merge(Request { id: 5, latency: 5 });
        sink.flush();

        assert_eq!(sink.sink.entries.last().unwrap().id, 5);
        assert_eq!(sink.sink.flushes, 2);
    }

    #[test]
    fn equal_scores_are_supported_without_specifying_which_entries_are_retained() {
        let mut sink = TopNSink::new(
            2,
            |request: &Request| request.latency,
            CollectingSink::default(),
        );

        for id in 1..=3 {
            sink.merge(Request { id, latency: 10 });
        }
        sink.flush();

        assert_eq!(sink.sink.entries.len(), 2);
        assert!(
            sink.sink
                .entries
                .iter()
                .all(|request| request.latency == 10)
        );
    }

    #[test]
    fn never_buffers_more_than_the_limit() {
        let mut sink = TopNSink::new(
            3,
            |request: &Request| request.latency,
            CollectingSink::default(),
        );

        for id in 0..1_000 {
            sink.merge(Request { id, latency: id });
            assert!(sink.selected.len() <= 3);
        }
    }

    #[test]
    fn zero_limit_retains_nothing_but_still_flushes_inner_sink() {
        let mut sink = TopNSink::new(
            0,
            |request: &Request| request.latency,
            CollectingSink::default(),
        );

        sink.merge(Request { id: 1, latency: 10 });
        sink.flush();

        assert!(sink.sink.entries.is_empty());
        assert_eq!(sink.sink.flushes, 1);
    }
}
