// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

#![deny(deprecated)]
use metrique::unit_of_work::metrics;

#[metrics]
#[derive(Default)]
pub struct Metrics {
    pub public_field: usize,
}

fn main() {
    let entry: Option<MetricsEntry> = None;
    println!("{:?}", entry.map(|f| f.public_field));
}
