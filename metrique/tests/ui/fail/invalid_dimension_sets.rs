// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use metrique::unit_of_work::metrics;

#[metrics(
    emf::dimension_sets = ["a", "b", "c"],
    rename_all = "snake_case"
)]
#[derive(Default, Clone)]
struct Nested {
    a: usize,
}

fn main() {}
