// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use metrique::unit_of_work::metrics;

#[metrics(rename_all = "snake_case")]
struct RootMetrics {
    child: ChildMetrics,
}

struct ChildMetrics {
    // this is not metrics
}

fn main() {}
