// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use metrique::{CloseValue, unit_of_work::metrics};
use metrique_writer::test_util::to_test_entry;

#[metrics(rename_all = "PascalCase")]
#[derive(Default)]
struct Root {
    #[metrics(flatten)]
    nested_a: NestedA,
    #[metrics(flatten)]
    nested_b: Option<NestedB>,
}

#[metrics]
#[derive(Default)]
struct NestedA {
    this_is_a_field: usize,
}

#[metrics(prefix = "prefix_")]
#[derive(Default)]
struct NestedB {
    this_is_b_field: usize,
}

#[test]
fn renames_applied_transitively() {
    let mut metric = Root::default();
    metric.nested_b = Some(NestedB::default());
    let entry = metric.close();
    let entry = to_test_entry(metrique::RootEntry::new(entry));
    let mut keys = entry.metrics.keys().collect::<Vec<_>>();
    keys.sort();
    assert_eq!(keys, vec!["PrefixThisIsBField", "ThisIsAField"]);
}
