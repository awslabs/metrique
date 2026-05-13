// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use metrique::unit_of_work::metrics;

struct MyTag;

// Conflicting flags: same tag both present and skipped on one field
#[metrics]
struct ConflictingFlags {
    #[metrics(flags(MyTag), flags(skip(MyTag)))]
    field: u64,
}

// Conflicting default_flags: same tag both present and skipped at struct level
#[metrics(default_flags(MyTag), default_flags(skip(MyTag)))]
struct ConflictingDefaultTag {
    field: u64,
}

fn main() {}
