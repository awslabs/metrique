// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use metrique::unit_of_work::metrics;

// value(string) enum with tuple variant should error
#[metrics(value(string))]
enum ValueEnumWithTupleVariant {
    Variant(usize),
}

// value(string) enum with struct variant should error
#[metrics(value(string))]
enum ValueEnumWithStructVariant {
    Variant { field: usize },
}

// value(string) enum variant with unit attribute should error
#[metrics(value(string))]
enum ValueEnumVariantWithUnit {
    #[metrics(unit = metrique::writer::unit::Millisecond)]
    Variant,
}

// value(string) enum variant with timestamp attribute should error
#[metrics(value(string))]
enum ValueEnumVariantWithTimestamp {
    #[metrics(timestamp)]
    Variant,
}

// value(string) enum variant with flatten attribute should error
#[metrics(value(string))]
enum ValueEnumVariantWithFlatten {
    #[metrics(flatten)]
    Variant,
}

// value(string) enum variant with sample_group attribute should error
#[metrics(value(string))]
enum ValueEnumVariantWithSampleGroup {
    #[metrics(sample_group)]
    Variant,
}

// entry enum with tuple variant without flatten should error
#[metrics]
enum EntryEnumTupleWithoutFlatten {
    Variant(u32),
}

// entry enum with tuple variant with unit should error
#[metrics]
enum EntryEnumTupleWithUnit {
    #[metrics(unit = metrique::writer::unit::Millisecond)]
    Variant(u32),
}

// entry enum variant with invalid attribute should error
#[metrics]
enum EntryEnumVariantWithInvalidAttr {
    #[metrics(timestamp)]
    Variant,
}

// Entry enum tuple field with unit (no flatten) should error
#[metrics]
enum EntryEnumTupleFieldWithUnit {
    Variant(#[metrics(unit = metrique::writer::unit::Millisecond)] u32),
}

// Entry enum tuple field with timestamp (no flatten) should error
#[metrics]
enum EntryEnumTupleFieldWithTimestamp {
    Variant(#[metrics(timestamp)] metrique::Timestamp),
}

// Entry enum tuple field with incompatible attributes should error
#[metrics]
enum EntryEnumTupleIncompatibleAttrs {
    Variant(#[metrics(flatten, unit = metrique::writer::unit::Millisecond)] u32),
}

// Nested enum/struct scenarios with fields that don't implement CloseValue by ref:
// 1. Root entry enum -> struct variant -> flatten subfield enum -> struct/tuple variants -> TimestampOnClose
// 2. Root entry enum -> tuple variant -> flatten subfield struct -> String
// All use #[metrics(subfield)] which requires CloseValue for &T, causing errors.
// Fix: use #[metrics(subfield_owned)] instead.
#[metrics(subfield)]
struct TimestampWrapper {
    timestamp: metrique::timers::TimestampOnClose,
}

#[metrics(subfield)]
struct StringWrapper {
    value: String,
}

#[metrics(subfield)]
enum InnerStatus {
    Active { timestamp: metrique::timers::TimestampOnClose },
    Pending(#[metrics(flatten)] TimestampWrapper),
}

#[metrics]
enum OuterOperation {
    Process { 
        #[metrics(flatten)]
        status: InnerStatus,
    },
    Execute(#[metrics(flatten)] StringWrapper),
}

// Entry enum with unit variants should error
#[metrics]
enum EntryEnumWithUnitVariants {
    Success,
    Failure,
}

fn main() {}
