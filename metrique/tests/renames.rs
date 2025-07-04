// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use metrique::{CloseValue, RootEntry, unit_of_work::metrics};
use metrique_writer::test_util;

#[metrics(rename_all = "kebab-case")]
struct Metrics {
    foo_bar: usize,
    #[metrics(name = "correct_correct")]
    overriden: &'static str,

    #[metrics(flatten)]
    nested: PrefixedMetrics,

    #[metrics(flatten)]
    also_nested: SubMetrics,
}

#[metrics(rename_all = "snake_case", prefix = "prefix_")]
struct PrefixedMetrics {
    a: usize,

    // local renames take precence, this field is just called `name`
    #[metrics(name = "name")]
    local_rename: &'static str,
}

#[metrics]
struct SubMetrics {
    sub_field_a: usize,
}

#[test]
fn metrics_renames_work() {
    let metrics = Metrics {
        foo_bar: 10,
        overriden: "abcd",
        nested: PrefixedMetrics {
            a: 100,
            local_rename: "abcd",
        },
        also_nested: SubMetrics { sub_field_a: 4 },
    };
    let entry = test_util::to_test_entry(RootEntry::new(metrics.close()));
    assert_eq!(entry.metrics["foo-bar"], 10);
    // the rename rule doesn't apply to explicit renames
    assert_eq!(entry.values["correct_correct"], "abcd");

    // the submetric has explicit snake casing, it doesn't get transitively renamed
    assert_eq!(entry.metrics["prefix_a"].as_u64(), 100);

    // a prefix doesn't apply when name is set
    assert_eq!(entry.values["name"], "abcd");

    // For sub metrics which _don't_ set a rename, they get rename transitively from the pattern
    assert_eq!(entry.metrics["sub-field-a"], 4);
}
