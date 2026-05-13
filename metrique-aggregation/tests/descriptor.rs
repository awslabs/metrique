// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use metrique::unit_of_work::metrics;
use metrique::writer::Entry;
use metrique_aggregation::aggregator::KeyedAggregator;
use metrique_aggregation::traits::{AggregateSink, FlushableSink};
use metrique_aggregation::{aggregate, value::Sum};
use metrique_writer::sink::VecEntrySink;
use metrique_writer_core::value::{FlagConstructor, MetricFlags, MetricOptions};
use std::any::TypeId;

#[derive(Debug)]
struct ExportOpt;
impl MetricOptions for ExportOpt {}
struct Export;
impl FlagConstructor for Export {
    fn construct() -> MetricFlags<'static> {
        MetricFlags::upcast(&ExportOpt)
    }
}

#[aggregate]
#[metrics(rename_all = "PascalCase", default_flags(Export))]
struct RequestMetrics {
    #[aggregate(key)]
    #[metrics(flags(skip(Export)))]
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
    let key_flags: Vec<_> = key_desc.fields().collect::<Vec<_>>()[0]
        .flags()
        .collect::<Vec<_>>();
    assert_eq!(key_flags.len(), 0);

    // Second descriptor: aggregated fields
    let agg_desc = &descriptors[1];
    assert_eq!(agg_desc.fields_len(), 1);
    assert_eq!(
        agg_desc.fields().collect::<Vec<_>>()[0].base_name(),
        "Count"
    );
    let agg_flags = agg_desc.fields().collect::<Vec<_>>()[0]
        .flags()
        .collect::<Vec<_>>();
    assert_eq!(agg_flags.len(), 1);
    assert!(agg_flags[0].is::<Export>());
}

#[test]
fn key_struct_inherits_parent_rename_all_and_default_flags() {
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

    // default_flags(Export) propagated, then flags(skip(Export)) applied
    // key field has skip(Export), so no flags
    let key_flags2: Vec<_> = key_desc.fields().collect::<Vec<_>>()[0].flags().collect();
    assert_eq!(key_flags2.len(), 0);
}
