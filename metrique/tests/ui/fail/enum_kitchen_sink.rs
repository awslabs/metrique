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

// Entry enum with unit variants should error
#[metrics]
enum EntryEnumWithUnitVariants {
    Success,
    Failure,
}

fn main() {}
