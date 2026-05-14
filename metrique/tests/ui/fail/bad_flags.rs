// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use metrique::unit_of_work::metrics;

struct MyTag;

// Conflicting flags: same flag both present and skipped on one field
#[metrics]
struct ConflictingFlags {
    #[metrics(flags(MyTag, skip(MyTag)))]
    field: u64,
}

// skip(...) not allowed in default_flags
#[metrics(default_flags(skip(MyTag)))]
struct SkipInDefaultFlags {
    field: u64,
}

// flags(...) not allowed on flatten fields
#[metrics(subfield)]
struct Child {
    value: u64,
}

#[metrics]
struct FlagsOnFlatten {
    #[metrics(flatten, flags(MyTag))]
    child: Child,
}

fn main() {}

// cfg on any field inside enum variant is not supported
#[metrics]
enum CfgFieldEnum {
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
enum CfgTupleEnum {
    V(
        #[cfg(test)]
        #[metrics(flatten)]
        TupleChild,
    ),
}
