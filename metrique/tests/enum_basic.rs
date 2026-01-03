// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use metrique::{unit_of_work::metrics, writer::test_util::test_metric};

// Basic tuple variant with flatten
#[metrics]
#[derive(Clone)]
struct NestedMetrics {
    value: u32,
}

#[metrics]
enum TupleVariantEnum {
    Variant(#[metrics(flatten)] NestedMetrics),
}

#[test]
fn test_tuple_variant_flatten() {
    let entry = test_metric(TupleVariantEnum::Variant(NestedMetrics { value: 42 }));

    assert_eq!(entry.metrics["value"].as_u64(), 42);
}

// Basic tuple variant with flatten_entry (flattens a type that implements Entry)
use metrique::writer::Entry;

#[derive(Entry)]
struct EntryMetrics {
    count: u32,
    name: String,
}

#[metrics]
enum TupleVariantFlattenEntry {
    Variant(#[metrics(flatten_entry, no_close)] EntryMetrics),
}

#[test]
fn test_tuple_variant_flatten_entry() {
    let entry = test_metric(TupleVariantFlattenEntry::Variant(EntryMetrics {
        count: 100,
        name: "test".to_string(),
    }));

    // flatten_entry writes the entry directly (calls Entry::write, not InflectableEntry::write)
    assert_eq!(entry.metrics["count"].as_u64(), 100);
    assert_eq!(entry.values["name"], "test");
}

// Basic struct variant
#[metrics]
enum StructVariantEnum {
    Variant { field1: u32, field2: bool },
}

#[test]
fn test_struct_variant_basic() {
    let entry = test_metric(StructVariantEnum::Variant {
        field1: 10,
        field2: true,
    });

    assert_eq!(entry.metrics["field1"].as_u64(), 10);
    assert_eq!(entry.metrics["field2"].as_u64(), 1);
}

// Mixed tuple and struct variants
#[metrics]
enum MixedEnum {
    Tuple(#[metrics(flatten)] NestedMetrics),
    Struct { x: u32, y: u32 },
}

#[test]
fn test_mixed_variants() {
    let entry1 = test_metric(MixedEnum::Tuple(NestedMetrics { value: 5 }));
    let entry2 = test_metric(MixedEnum::Struct { x: 1, y: 2 });

    assert_eq!(entry1.metrics["value"].as_u64(), 5);
    assert_eq!(entry2.metrics["x"].as_u64(), 1);
    assert_eq!(entry2.metrics["y"].as_u64(), 2);
}

// Enum with rename_all - both tuple and struct variants
#[metrics(rename_all = "PascalCase")]
enum RenamedEnum {
    TupleVariant(#[metrics(flatten)] NestedMetrics),
    StructVariant { field_name: u32 },
}

#[test]
fn test_enum_rename_all() {
    let entry1 = test_metric(RenamedEnum::TupleVariant(NestedMetrics { value: 100 }));
    let entry2 = test_metric(RenamedEnum::StructVariant { field_name: 200 });

    assert_eq!(entry1.metrics["Value"].as_u64(), 100);
    assert_eq!(entry2.metrics["FieldName"].as_u64(), 200);
}

// Enum with prefix - both tuple and struct variants
#[metrics(prefix = "api_")]
enum PrefixedEnum {
    TupleVariant(#[metrics(flatten)] NestedMetrics),
    StructVariant { counter: u32 },
}

#[test]
fn test_enum_prefix() {
    let entry1 = test_metric(PrefixedEnum::TupleVariant(NestedMetrics { value: 50 }));
    let entry2 = test_metric(PrefixedEnum::StructVariant { counter: 75 });

    assert_eq!(entry1.metrics["value"].as_u64(), 50);

    // Container prefix DOES apply to struct variant fields
    assert_eq!(entry2.metrics["api_counter"].as_u64(), 75);
}

// Tuple variant with field-level prefix
#[metrics]
#[derive(Clone)]
struct PrefixedNested {
    metric: u32,
}

#[metrics]
enum TuplePrefixEnum {
    WithPrefix(#[metrics(flatten, prefix = "nested_")] PrefixedNested),
    StructVariant { other: u32 },
}

#[test]
fn test_tuple_variant_field_prefix() {
    let entry1 = test_metric(TuplePrefixEnum::WithPrefix(PrefixedNested { metric: 25 }));
    let entry2 = test_metric(TuplePrefixEnum::StructVariant { other: 30 });

    assert_eq!(entry1.metrics["nested_metric"].as_u64(), 25);
    assert_eq!(entry2.metrics["other"].as_u64(), 30);
}

// Container prefix + struct variant fields (verify prefix applies)
#[metrics(prefix = "api_")]
enum ContainerPrefixStruct {
    Operation {
        request_count: u32,
        error_count: u32,
    },
}

#[test]
fn test_container_prefix_struct_variant() {
    let entry = test_metric(ContainerPrefixStruct::Operation {
        request_count: 100,
        error_count: 5,
    });

    // Container prefix applies to struct variant fields
    assert_eq!(entry.metrics["api_request_count"].as_u64(), 100);
    assert_eq!(entry.metrics["api_error_count"].as_u64(), 5);
}
