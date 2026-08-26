// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use metrique::{CloseValue, unit_of_work::metrics};

struct OwnedOnly;

impl CloseValue for OwnedOnly {
    type Closed = u64;

    fn close(self) -> Self::Closed {
        0
    }
}

#[metrics(subfield)]
struct StructSubfield {
    value: OwnedOnly,
}

#[metrics(subfield)]
enum EnumSubfield {
    Struct {
        value: OwnedOnly,
    },
}

fn main() {}
