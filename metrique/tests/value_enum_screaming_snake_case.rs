// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Tests that `rename_all = "SCREAMING_SNAKE_CASE"` works on value(string) enums
//! and that the enum can be used as a field inside a metrics struct.

use metrique::writer::test_util;
use metrique::{CloseValue, RootEntry, unit_of_work::metrics};

#[metrics(value(string), rename_all = "SCREAMING_SNAKE_CASE")]
#[derive(Debug, Clone, Copy)]
pub enum Operation {
    ReadData,
    WriteData,
    #[metrics(name = "CUSTOM_OP")]
    CustomOperation,
}

#[metrics]
struct RequestMetrics {
    operation: Operation,
    latency: u64,
}

#[test]
fn value_enum_screaming_snake_case_renames_correctly() {
    // Multi-word variants get SCREAMING_SNAKE_CASE
    assert_eq!(<&str>::from(&Operation::ReadData), "READ_DATA");
    assert_eq!(<&str>::from(&Operation::WriteData), "WRITE_DATA");
    // Explicit name override takes precedence
    assert_eq!(<&str>::from(&Operation::CustomOperation), "CUSTOM_OP");
}

#[test]
fn value_enum_screaming_snake_case_works_in_struct() {
    let metrics = RequestMetrics {
        operation: Operation::ReadData,
        latency: 42,
    };
    let entry = test_util::to_test_entry(RootEntry::new(metrics.close()));

    assert_eq!(entry.values["operation"], "READ_DATA");
    assert_eq!(entry.metrics["latency"], 42);
}

#[test]
fn value_enum_screaming_snake_case_with_override_in_struct() {
    let metrics = RequestMetrics {
        operation: Operation::CustomOperation,
        latency: 100,
    };
    let entry = test_util::to_test_entry(RootEntry::new(metrics.close()));

    assert_eq!(entry.values["operation"], "CUSTOM_OP");
    assert_eq!(entry.metrics["latency"], 100);
}
