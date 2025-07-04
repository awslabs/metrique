// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use metrique::unit_of_work::metrics;

#[metrics(rename_all = "snake_case", bad_root_attr, bad_root_attr_eq = "foo")]
struct MetricsWithInvalidUnit {
    operation: &'static str,

    #[metrics(name = "a", name = "b")]
    duplicate_name: usize,

    // This should fail because NonExistentUnit is not a valid unit
    #[metrics(unit = Seconds, unit = Minutes)]
    size: usize,

    #[metrics(flatten, name = "foo")]
    subfield: SubMetric,

    #[metrics(name = 5)]
    bad_name: usize,

    #[metrics(nme = "foo")]
    not_valid: usize,

    #[metrics(name = "")]
    bad_name_2: usize,

    #[metrics(name = "a b")]
    bad_name_3: usize,
}

#[metrics(rename_all = "snake_case")]
struct SubMetric {
    a: usize,
}

fn main() {}
