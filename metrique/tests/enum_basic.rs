// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use metrique::{
    unit_of_work::metrics,
    writer::{sink::VecEntrySink, test_util},
};

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
    let vec_sink = VecEntrySink::new();

    TupleVariantEnum::Variant(NestedMetrics { value: 42 }).append_on_drop(vec_sink.clone());

    let entries = vec_sink.drain();
    assert_eq!(entries.len(), 1);
    let entry = test_util::to_test_entry(&entries[0]);

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
    let vec_sink = VecEntrySink::new();

    TupleVariantFlattenEntry::Variant(EntryMetrics {
        count: 100,
        name: "test".to_string(),
    })
    .append_on_drop(vec_sink.clone());

    let entries = vec_sink.drain();
    assert_eq!(entries.len(), 1);
    let entry = test_util::to_test_entry(&entries[0]);

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
    let vec_sink = VecEntrySink::new();

    StructVariantEnum::Variant {
        field1: 10,
        field2: true,
    }
    .append_on_drop(vec_sink.clone());

    let entries = vec_sink.drain();
    assert_eq!(entries.len(), 1);
    let entry = test_util::to_test_entry(&entries[0]);

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
    let vec_sink = VecEntrySink::new();

    MixedEnum::Tuple(NestedMetrics { value: 5 }).append_on_drop(vec_sink.clone());
    MixedEnum::Struct { x: 1, y: 2 }.append_on_drop(vec_sink.clone());

    let entries = vec_sink.drain();
    assert_eq!(entries.len(), 2);

    let entry1 = test_util::to_test_entry(&entries[0]);
    assert_eq!(entry1.metrics["value"].as_u64(), 5);

    let entry2 = test_util::to_test_entry(&entries[1]);
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
    let vec_sink = VecEntrySink::new();

    RenamedEnum::TupleVariant(NestedMetrics { value: 100 }).append_on_drop(vec_sink.clone());
    RenamedEnum::StructVariant { field_name: 200 }.append_on_drop(vec_sink.clone());

    let entries = vec_sink.drain();
    assert_eq!(entries.len(), 2);

    let entry1 = test_util::to_test_entry(&entries[0]);
    assert_eq!(entry1.metrics["Value"].as_u64(), 100);

    let entry2 = test_util::to_test_entry(&entries[1]);
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
    let vec_sink = VecEntrySink::new();

    PrefixedEnum::TupleVariant(NestedMetrics { value: 50 }).append_on_drop(vec_sink.clone());
    PrefixedEnum::StructVariant { counter: 75 }.append_on_drop(vec_sink.clone());

    let entries = vec_sink.drain();
    assert_eq!(entries.len(), 2);

    let entry1 = test_util::to_test_entry(&entries[0]);
    assert_eq!(entry1.metrics["value"].as_u64(), 50);

    // Container prefix DOES apply to struct variant fields
    let entry2 = test_util::to_test_entry(&entries[1]);
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
    let vec_sink = VecEntrySink::new();

    TuplePrefixEnum::WithPrefix(PrefixedNested { metric: 25 }).append_on_drop(vec_sink.clone());
    TuplePrefixEnum::StructVariant { other: 30 }.append_on_drop(vec_sink.clone());

    let entries = vec_sink.drain();
    assert_eq!(entries.len(), 2);

    let entry1 = test_util::to_test_entry(&entries[0]);
    assert_eq!(entry1.metrics["nested_metric"].as_u64(), 25);

    let entry2 = test_util::to_test_entry(&entries[1]);
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
    let vec_sink = VecEntrySink::new();

    ContainerPrefixStruct::Operation {
        request_count: 100,
        error_count: 5,
    }
    .append_on_drop(vec_sink.clone());

    let entries = vec_sink.drain();
    assert_eq!(entries.len(), 1);
    let entry = test_util::to_test_entry(&entries[0]);

    // Container prefix applies to struct variant fields
    assert_eq!(entry.metrics["api_request_count"].as_u64(), 100);
    assert_eq!(entry.metrics["api_error_count"].as_u64(), 5);
}
