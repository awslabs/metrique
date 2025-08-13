// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use metrique::writer::Entry;
use metrique::{CloseValue, RootEntry};
use metrique_macro::metrics;

#[metrics(rename_all = "PascalCase")]
#[derive(Default, Clone)]
struct Metrics {
    #[metrics(flatten)]
    f: Nested,
}

#[metrics(rename_all = "PascalCase")]
#[derive(Default, Clone)]
struct Nested {
    a: usize,
}

#[test]
fn sample_group_correctly_handled() {
    let metric = Metrics::default();
    let entry = metric.close();
    assert_eq!(RootEntry::new(entry).sample_group().count(), 0);
}
