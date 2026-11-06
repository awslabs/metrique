// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Collection operations preserve the semantics of inserting each owned entry.

use metrique::{CloseValue, unit_of_work::metrics, writer::test_util::test_metric};
use metrique_aggregation::aggregate;
use metrique_aggregation::aggregator::Aggregate;
use metrique_aggregation::traits::{AggregateStrategy, Merge};
use metrique_aggregation::value::{KeepLast, NoKey, Sum};
use rstest::rstest;

#[aggregate]
#[metrics]
struct Reading {
    #[aggregate(strategy = Sum)]
    total: u64,
    #[aggregate(strategy = KeepLast)]
    label: String,
}

fn readings() -> [Reading; 3] {
    [
        Reading {
            total: 2,
            label: "first".into(),
        },
        Reading {
            total: 3,
            label: "second".into(),
        },
        Reading {
            total: 5,
            label: "last".into(),
        },
    ]
}

/// Non-Clone entries can be collected from arrays, vectors and boxed slices (#386).
#[rstest]
#[case::array(readings())]
#[case::vec(Vec::from(readings()))]
#[case::boxed_slice(Box::<[Reading]>::from(readings()))]
fn collect_owned_entries(#[case] entries: impl IntoIterator<Item = Reading>) {
    let aggregate: Aggregate<Reading> = entries.into_iter().collect();
    let entry = test_metric(aggregate);
    assert_eq!(entry.metrics["total"].as_u64(), 10);
    assert_eq!(entry.values["label"], "last");
}

/// Collecting no entries produces the usual default aggregate.
#[test]
fn collect_empty_iterator() {
    let aggregate: Aggregate<Reading> = std::iter::empty().collect();
    let entry = test_metric(aggregate);
    assert_eq!(entry.metrics["total"].as_u64(), 0);
    assert!(!entry.values.contains_key("label"));
}

/// Extending an existing aggregate adds to its state and preserves input order.
#[test]
fn extend_existing_aggregate() {
    let mut aggregate = Aggregate::<Reading>::default();
    aggregate.insert(Reading {
        total: 7,
        label: "seed".into(),
    });
    aggregate.extend(readings());
    aggregate.extend(std::iter::empty::<Reading>());
    let entry = test_metric(aggregate);
    assert_eq!(entry.metrics["total"].as_u64(), 17);
    assert_eq!(entry.values["label"], "last");
}

#[metrics]
struct NonDefaultState {
    total: u64,
}

struct NonDefaultReading(u64);

impl CloseValue for NonDefaultReading {
    type Closed = Self;
    fn close(self) -> Self {
        self
    }
}

impl Merge for NonDefaultReading {
    type Merged = NonDefaultState;
    type MergeConfig = ();
    fn new_merged(_: &()) -> Self::Merged {
        NonDefaultState { total: 0 }
    }
    fn merge(accum: &mut Self::Merged, input: Self) {
        accum.total += input.0;
    }
}

impl AggregateStrategy for NonDefaultReading {
    type Source = Self;
    type Key = NoKey;
}

/// Extend does not require Default when the caller provides the initial state.
#[test]
fn extend_non_default_accumulator() {
    let mut aggregate = Aggregate::<NonDefaultReading>::new(NonDefaultState { total: 10 });
    aggregate.extend([NonDefaultReading(2), NonDefaultReading(3)]);
    assert_eq!(test_metric(aggregate).metrics["total"].as_u64(), 15);
}
