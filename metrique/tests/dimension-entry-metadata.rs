// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashSet;

use metrique::{CloseValue, RootEntry, unit_of_work::metrics};
use metrique_writer::{Entry, entry::WithGlobalDimensions, value::WithDimension};

#[metrics(rename_all = "PascalCase")]
struct RequestMetrics {
    #[metrics(sample_group)]
    operation: &'static str,
    count: u64,
}

fn metrics() -> RequestMetrics {
    RequestMetrics {
        operation: "CountDucks",
        count: 1,
    }
}

fn assert_sample_group(entry: &impl Entry) {
    let group = entry.sample_group().collect::<Vec<_>>();
    assert_eq!(group, vec![("Operation".into(), "CountDucks".into())]);
}

#[test]
fn dimensions_before_rooting_preserve_sample_group() {
    let entry = RootEntry::new(WithDimension::new(metrics(), "Zone", "a").close());
    assert_sample_group(&entry);
}

#[test]
fn dimensions_before_rooting_preserve_descriptors() {
    let original = RootEntry::new(metrics().close());
    let wrapped = RootEntry::new(WithDimension::new(metrics(), "Zone", "a").close());
    let original_descriptors = original.descriptors().unwrap();
    let wrapped_descriptors = wrapped.descriptors().unwrap();

    assert_eq!(wrapped_descriptors.len(), original_descriptors.len());
    assert_eq!(wrapped_descriptors[0].id(), original_descriptors[0].id());
    assert_eq!(
        wrapped_descriptors[0]
            .fields()
            .map(|field| field.base_name().to_owned())
            .collect::<Vec<_>>(),
        ["Operation", "Count"],
    );
}

#[test]
fn dimensions_after_rooting_preserve_sample_group() {
    let entry = WithDimension::new(RootEntry::new(metrics().close()), "Zone", "a");
    assert_sample_group(&entry);
}

#[test]
fn global_dimensions_preserve_sample_group() {
    let entry = WithGlobalDimensions::<_, 1>::new_with_global_dimensions(
        RootEntry::new(metrics().close()),
        [("Zone", "a")],
        HashSet::new(),
    );
    assert_sample_group(&entry);
}
