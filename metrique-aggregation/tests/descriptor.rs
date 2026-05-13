// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use metrique::unit_of_work::metrics;
use metrique::writer::Entry;
use metrique_aggregation::aggregator::KeyedAggregator;
use metrique_aggregation::traits::{AggregateSink, FlushableSink};
use metrique_aggregation::{aggregate, value::Sum};
use metrique_writer::sink::VecEntrySink;
use metrique_writer_core::FieldTagState;
use std::any::TypeId;

struct Export;

#[aggregate]
#[metrics(rename_all = "PascalCase", default_field_tag(Export))]
struct RequestMetrics {
    #[aggregate(key)]
    #[metrics(field_tag(skip(Export)))]
    operation: &'static str,
    #[aggregate(strategy = Sum)]
    count: u64,
}

#[test]
fn aggregation_result_yields_two_descriptors() {
    let sink = VecEntrySink::default();
    let mut aggregator: KeyedAggregator<RequestMetrics, _> = KeyedAggregator::new(sink.clone());

    let m = RequestMetrics {
        operation: "GetItem",
        count: 1,
    };
    aggregator.merge(metrique::CloseValue::close(m));
    aggregator.flush();

    let entries = sink.drain();
    assert_eq!(entries.len(), 1);

    let descriptors: Vec<_> = entries[0].descriptors().collect();
    assert_eq!(
        descriptors.len(),
        2,
        "should yield key + aggregated descriptors"
    );

    // First descriptor: key fields
    let key_desc = &descriptors[0];
    assert_eq!(key_desc.fields_len(), 1);
    assert_eq!(
        key_desc.fields().collect::<Vec<_>>()[0].base_name(),
        "Operation"
    );
    let key_tags = key_desc.fields().collect::<Vec<_>>()[0].tags();
    assert_eq!(key_tags.len(), 1);
    assert_eq!(key_tags[0].tag_id(), TypeId::of::<Export>());
    assert_eq!(key_tags[0].state(), FieldTagState::Absent);

    // Second descriptor: aggregated fields
    let agg_desc = &descriptors[1];
    assert_eq!(agg_desc.fields_len(), 1);
    assert_eq!(
        agg_desc.fields().collect::<Vec<_>>()[0].base_name(),
        "Count"
    );
    let agg_tags = agg_desc.fields().collect::<Vec<_>>()[0].tags();
    assert_eq!(agg_tags.len(), 1);
    assert_eq!(agg_tags[0].tag_id(), TypeId::of::<Export>());
    assert_eq!(agg_tags[0].state(), FieldTagState::Present);
}

#[test]
fn key_struct_inherits_parent_rename_all_and_default_field_tag() {
    let sink = VecEntrySink::default();
    let mut aggregator: KeyedAggregator<RequestMetrics, _> = KeyedAggregator::new(sink.clone());

    aggregator.merge(metrique::CloseValue::close(RequestMetrics {
        operation: "PutItem",
        count: 5,
    }));
    aggregator.flush();

    let entries = sink.drain();
    let key_desc = entries[0].descriptors().next().expect("key descriptor");

    // rename_all = "PascalCase" propagated to key struct
    assert_eq!(
        key_desc.fields().collect::<Vec<_>>()[0].base_name(),
        "Operation"
    );

    // default_field_tag(Export) propagated, then field_tag(skip(Export)) applied
    let tag = &key_desc.fields().collect::<Vec<_>>()[0]
        .tags()
        .collect::<Vec<_>>()[0];
    assert_eq!(tag.tag_id(), TypeId::of::<Export>());
    assert_eq!(tag.state(), FieldTagState::Absent);
}
