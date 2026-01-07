// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use metrique::{
    test_util::test_metric,
    unit_of_work::metrics,
    writer::{unit, value::ToString},
};
use metrique_writer_core::SampleGroup;

#[metrics]
#[derive(Clone)]
pub struct NestedMetrics {
    value: u32,
}

// Variant-level name override
#[metrics]
enum VariantNameOverride {
    #[metrics(name = "custom_name")]
    DefaultName(#[metrics(flatten)] NestedMetrics),
}

#[test]
fn test_variant_name_override() {
    let variant = VariantNameOverride::DefaultName(NestedMetrics { value: 99 });

    // Test From<&Entry> for &'static str
    let name: &'static str = (&variant).into();
    assert_eq!(name, "custom_name");

    // Test SampleGroup::as_sample_group
    assert_eq!(variant.as_sample_group(), "custom_name");

    // Verify metrics still emitted correctly
    let entry = test_metric(variant);
    assert_eq!(entry.metrics["value"].as_u64(), 99);
}

// Variant name + container rename_all (name overrides rename_all)
#[metrics(rename_all = "kebab-case")]
enum VariantNameWithRenameAll {
    #[metrics(name = "custom_variant")]
    DefaultVariant(#[metrics(flatten)] NestedMetrics),
}

#[test]
fn test_variant_name_with_rename_all() {
    let variant = VariantNameWithRenameAll::DefaultVariant(NestedMetrics { value: 88 });

    // Custom name overrides rename_all for variant name
    let name: &'static str = (&variant).into();
    assert_eq!(name, "custom_variant");
    assert_eq!(variant.as_sample_group(), "custom_variant");

    // Container rename_all still applies to flattened fields
    let entry = test_metric(variant);
    assert_eq!(entry.metrics["value"].as_u64(), 88);
}

// Variant name + container prefix (name is NOT prefixed)
#[metrics(prefix = "api_")]
enum VariantNameWithPrefix {
    #[metrics(name = "operation")]
    DefaultOp(#[metrics(flatten)] NestedMetrics),
}

#[test]
fn test_variant_name_with_prefix() {
    let variant = VariantNameWithPrefix::DefaultOp(NestedMetrics { value: 77 });

    // Variant name is NOT prefixed
    let name: &'static str = (&variant).into();
    assert_eq!(name, "operation");
    assert_eq!(variant.as_sample_group(), "operation");

    let entry = test_metric(variant);
    assert_eq!(entry.metrics["value"].as_u64(), 77);
}

// Struct variant field with custom name
#[metrics]
enum StructFieldName {
    Variant {
        #[metrics(name = "custom_field")]
        default_field: u32,
    },
}

#[test]
fn test_struct_field_name() {
    let entry = test_metric(StructFieldName::Variant { default_field: 123 });

    // Custom field name used
    assert_eq!(entry.metrics["custom_field"].as_u64(), 123);
}

// Struct variant field with unit attribute
#[metrics]
enum StructFieldUnit {
    Variant {
        #[metrics(unit = unit::Millisecond)]
        latency: u64,
    },
}

#[test]
fn test_struct_field_unit() {
    let entry = test_metric(StructFieldUnit::Variant { latency: 150 });

    // Unit attribute applied
    assert_eq!(entry.metrics["latency"].as_u64(), 150);
    assert_eq!(
        entry.metrics["latency"].unit,
        unit::Unit::Second(unit::NegativeScale::Milli)
    );
}

// Struct variant field with format attribute
#[metrics]
enum StructFieldFormat {
    Variant {
        #[metrics(format = ToString)]
        name: u32,
    },
}

#[test]
fn test_struct_field_format() {
    let entry = test_metric(StructFieldFormat::Variant { name: 42 });

    // Format attribute applied (ToString converts to string)
    assert_eq!(entry.values["name"], "42");
}

// Test From<&Entry> and SampleGroup for tuple variants
#[metrics]
enum TupleStringConversion {
    #[metrics(name = "custom_op")]
    Operation(#[metrics(flatten)] NestedMetrics),
    DefaultName(#[metrics(flatten)] NestedMetrics),
}

#[test]
fn test_tuple_variant_string_conversion() {
    let op = TupleStringConversion::Operation(NestedMetrics { value: 1 });
    let default = TupleStringConversion::DefaultName(NestedMetrics { value: 2 });

    // Test From<&Entry> for &'static str
    let op_str: &'static str = (&op).into();
    let default_str: &'static str = (&default).into();

    assert_eq!(op_str, "custom_op");
    assert_eq!(default_str, "DefaultName");

    // Test SampleGroup::as_sample_group
    assert_eq!(op.as_sample_group(), "custom_op");
    assert_eq!(default.as_sample_group(), "DefaultName");
}

// Test From<&Entry> and SampleGroup for struct variants with rename_all
#[metrics(rename_all = "kebab-case")]
enum StructStringConversion {
    #[metrics(name = "custom_variant")]
    DefaultVariant {
        field: u32,
    },
    AnotherVariant {
        field: u32,
    },
}

#[test]
fn test_struct_variant_string_conversion() {
    let custom = StructStringConversion::DefaultVariant { field: 1 };
    let another = StructStringConversion::AnotherVariant { field: 2 };

    // Test From<&Entry> for &'static str
    let custom_str: &'static str = (&custom).into();
    let another_str: &'static str = (&another).into();

    // Custom name overrides rename_all, but default variant uses rename_all
    assert_eq!(custom_str, "custom_variant");
    assert_eq!(another_str, "another-variant");

    // Test SampleGroup::as_sample_group
    assert_eq!(custom.as_sample_group(), "custom_variant");
    assert_eq!(another.as_sample_group(), "another-variant");
}

// Test multi-field tuple variant with flatten and ignore
#[metrics]
enum MultiFieldTuple {
    Variant(
        #[metrics(flatten, prefix = "a_")] NestedMetrics,
        #[metrics(ignore)] u32,
        #[metrics(flatten, prefix = "b_")] NestedMetrics,
        #[metrics(ignore)] String,
    ),
}

#[test]
fn test_multi_field_tuple_variant() {
    let entry = test_metric(MultiFieldTuple::Variant(
        NestedMetrics { value: 10 },
        999,
        NestedMetrics { value: 20 },
        "ignored".to_string(),
    ));

    // Two flatten fields with different prefixes
    assert_eq!(entry.metrics["a_value"].as_u64(), 10);
    assert_eq!(entry.metrics["b_value"].as_u64(), 20);

    // Ignored fields not present
    assert!(!entry.metrics.contains_key("999"));
    assert!(!entry.values.contains_key("ignored"));
}
