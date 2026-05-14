// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use metrique::unit_of_work::metrics;

// cfg on any field inside a struct enum variant is not supported
#[metrics]
enum CfgStructVariant {
    V {
        #[cfg(test)]
        field: u64,
    },
}

// cfg on tuple variant field also not supported
#[metrics(subfield)]
struct TupleChild {
    val: u64,
}

#[metrics]
enum CfgTupleVariant {
    V(
        #[cfg(test)]
        #[metrics(flatten)]
        TupleChild,
    ),
}

fn main() {}
