//! Test that KeyedAggregator works with a non-Default MergeConfig

use assert2::check;
use metrique::unit_of_work::metrics;
use metrique_aggregation::aggregator::KeyedAggregator;
use metrique_aggregation::traits::{AggregateStrategy, FlushableSink, Key, Merge, MergeRef};
use metrique_writer::test_util::test_entry_sink;
use std::borrow::Cow;

/// A config that does NOT implement Default
pub struct ThresholdConfig {
    pub min_threshold: u64,
}

#[metrics]
pub struct Event {
    endpoint: String,
    value: u64,
}

/// Accumulated result: counts values above the threshold
#[derive(Default)]
#[metrics]
pub struct AggregatedEvent {
    count_above_threshold: u64,
    total: u64,
}

impl Merge for Event {
    type Merged = AggregatedEvent;
    type MergeConfig = ThresholdConfig;

    fn new_merged(conf: &Self::MergeConfig) -> Self::Merged {
        // Use the config to initialize—here we store the threshold as a sentinel in total
        // to prove the config was actually used
        AggregatedEvent {
            count_above_threshold: 0,
            total: conf.min_threshold, // pre-seed with threshold to prove config was used
        }
    }

    fn merge(accum: &mut Self::Merged, input: Self) {
        accum.total += input.value;
        if input.value > 0 {
            accum.count_above_threshold += 1;
        }
    }
}

impl MergeRef for Event {
    fn merge_ref(accum: &mut Self::Merged, input: &Self) {
        accum.total += input.value;
        if input.value > 0 {
            accum.count_above_threshold += 1;
        }
    }
}

#[derive(Clone, Hash, PartialEq, Eq)]
#[metrics]
pub struct EventKey<'a> {
    endpoint: Cow<'a, str>,
}

pub struct EventByEndpoint;

impl Key<Event> for EventByEndpoint {
    type Key<'a> = EventKey<'a>;

    fn from_source(source: &Event) -> Self::Key<'_> {
        EventKey {
            endpoint: Cow::Borrowed(&source.endpoint),
        }
    }

    fn static_key<'a>(key: &Self::Key<'a>) -> Self::Key<'static> {
        EventKey {
            endpoint: Cow::Owned(key.endpoint.clone().into_owned()),
        }
    }

    fn static_key_matches<'a>(owned: &Self::Key<'static>, borrowed: &Self::Key<'a>) -> bool {
        owned == borrowed
    }
}

impl AggregateStrategy for Event {
    type Source = Event;
    type Key = EventByEndpoint;
}

#[test]
fn test_keyed_aggregator_with_non_default_config() {
    let test_sink = test_entry_sink();
    let config = ThresholdConfig { min_threshold: 100 };
    let mut aggregator: KeyedAggregator<Event, _> =
        KeyedAggregator::new_with_config(test_sink.sink, config);

    use metrique_aggregation::traits::AggregateSink;
    aggregator.merge(Event {
        endpoint: "api1".to_string(),
        value: 5,
    });
    aggregator.merge(Event {
        endpoint: "api1".to_string(),
        value: 10,
    });
    aggregator.flush();

    let entries = test_sink.inspector.entries();
    check!(entries.len() == 1);

    // total should be min_threshold (100) + 5 + 10 = 115, proving the config was used
    check!(entries[0].metrics["total"] == 115u64);
}

#[test]
fn test_keyed_aggregator_with_non_default_config_merge_ref() {
    let test_sink = test_entry_sink();
    let config = ThresholdConfig { min_threshold: 100 };
    let mut aggregator: KeyedAggregator<Event, _> =
        KeyedAggregator::new_with_config(test_sink.sink, config);

    use metrique_aggregation::traits::AggregateSinkRef;
    aggregator.merge_ref(&Event {
        endpoint: "api1".to_string(),
        value: 5,
    });
    aggregator.merge_ref(&Event {
        endpoint: "api1".to_string(),
        value: 10,
    });
    aggregator.flush();

    let entries = test_sink.inspector.entries();
    check!(entries.len() == 1);

    // total should be min_threshold (100) + 5 + 10 = 115, proving the config was used
    // via the merge_ref path
    check!(entries[0].metrics["total"] == 115u64);
}
