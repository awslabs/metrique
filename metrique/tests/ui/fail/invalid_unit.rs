// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use metrique::unit_of_work::metrics;

#[metrics(rename_all = "snake_case")]
struct MetricsWithInvalidUnit {
    operation: &'static str,

    // This should fail because NonExistentUnit is not a valid unit
    #[metrics(unit = NonExistentUnit)]
    size: usize,
}

fn main() {
    let _metrics = MetricsWithInvalidUnit {
        operation: "Test",
        size: 100,
    };
}
