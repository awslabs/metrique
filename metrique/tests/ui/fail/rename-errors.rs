// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use metrique::unit_of_work::metrics;

#[metrics(rename_all = "PascalCase")]
struct Metrics {
    #[metrics(flatten, name = "rename")]
    nested: Nested,
}

#[metrics(rename_all = "PascalCase")]
struct Nested {
    a_b: usize,
}

fn main() {}
