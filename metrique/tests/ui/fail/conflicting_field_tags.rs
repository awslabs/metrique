// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use metrique::unit_of_work::metrics;

struct MyTag;

// Conflicting field_tag: same tag both present and skipped on one field
#[metrics]
struct ConflictingFieldTag {
    #[metrics(field_tag(MyTag), field_tag(skip(MyTag)))]
    field: u64,
}

// Conflicting default_field_tag: same tag both present and skipped at struct level
#[metrics(default_field_tag(MyTag), default_field_tag(skip(MyTag)))]
struct ConflictingDefaultTag {
    field: u64,
}

fn main() {}
