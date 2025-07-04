// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use metrique::unit_of_work::metrics;
use metrique::Slot;

#[metrics(rename_all = "snake_case")]
#[derive(Default)]
struct ChildMetrics {
    child_count: usize,
}

#[metrics(rename_all = "PascalCase")]
struct ParentMetrics {
    operation: &'static str,

    // This should fail because nested metrics fields need the #[metrics(flatten)] attribute
    // Missing the flatten attribute
    downstream_operation: Slot<ChildMetrics>,
}

fn main() {
    let _metrics = ParentMetrics {
        operation: "Test",
        downstream_operation: Default::default(),
    };
}
