// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use metrique::{
    CloseValue, RootEntry,
    unit_of_work::metrics,
    writer::test_util::{test_metric, to_test_entry},
};
use metrique_aggregation::{
    aggregate,
    aggregator::Aggregate,
    traits::{AggregateSinkRef, AggregateValue},
    value::{KeepFirst, KeepLast, MergeOptions},
};

#[test]
fn empty_accumulator_does_not_require_default_value() {
    struct NoDefault;
    let mut accumulator: <KeepFirst as AggregateValue<NoDefault>>::Aggregated = Default::default();
    assert!(accumulator.is_none());
    KeepFirst::insert(&mut accumulator, NoDefault);
    assert!(accumulator.is_some());
}

#[test]
fn first_owned_value_wins_without_clone() {
    struct NotClone(String);
    let mut accumulator = None;
    KeepFirst::insert(&mut accumulator, NotClone("first".into()));
    KeepFirst::insert(&mut accumulator, NotClone("second".into()));
    assert_eq!(accumulator.unwrap().0, "first");
}

#[test]
fn initial_none_is_still_the_first_value() {
    let mut accumulator = None;
    KeepFirst::insert(&mut accumulator, None::<u64>);
    KeepFirst::insert(&mut accumulator, Some(7));
    assert_eq!(accumulator, Some(None));
}

#[test]
fn merge_options_skips_none_before_the_first_value() {
    let mut accumulator = None;
    MergeOptions::<KeepFirst>::insert(&mut accumulator, None::<u64>);
    MergeOptions::<KeepFirst>::insert(&mut accumulator, Some(7));
    MergeOptions::<KeepFirst>::insert(&mut accumulator, Some(9));
    assert_eq!(accumulator, Some(7));
}

#[aggregate]
#[metrics]
struct Reading {
    #[aggregate(strategy = KeepFirst)]
    first: String,
    #[aggregate(strategy = KeepLast)]
    last: String,
}

#[test]
fn derived_aggregate_keeps_first_and_last_values() {
    let mut aggregate = Aggregate::<Reading>::default();
    for value in ["first", "second"] {
        aggregate.insert(Reading {
            first: value.into(),
            last: value.into(),
        });
    }
    let entry = test_metric(aggregate);
    assert_eq!(entry.values["first"], "first");
    assert_eq!(entry.values["last"], "second");
}

#[aggregate(ref)]
#[metrics]
struct BorrowedReading {
    #[aggregate(strategy = KeepFirst, clone)]
    value: String,
}

#[test]
fn borrowed_aggregation_keeps_the_source_entry() {
    let first = BorrowedReading {
        value: "first".into(),
    }
    .close();
    let second = BorrowedReading {
        value: "second".into(),
    }
    .close();
    let mut aggregate = Aggregate::<BorrowedReading>::default();
    aggregate.merge_ref(&first);
    aggregate.merge_ref(&second);
    assert_eq!(test_metric(aggregate).values["value"], "first");
    assert_eq!(
        to_test_entry(RootEntry::new(first)).values["value"],
        "first"
    );
}
