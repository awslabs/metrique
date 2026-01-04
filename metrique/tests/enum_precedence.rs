// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use metrique::{
    unit_of_work::metrics,
    writer::{sink::VecEntrySink, test_util},
};

#[metrics]
#[derive(Clone)]
struct NestedMetrics {
    value: u32,
}

// Child rename_all + parent prefix
#[metrics(rename_all = "PascalCase")]
#[derive(Clone)]
struct ChildRenamed {
    read_data: u32,
}

#[metrics]
enum ChildRenameParentPrefix {
    Variant(#[metrics(flatten, prefix = "op_")] ChildRenamed),
}

#[test]
fn test_child_rename_parent_prefix() {
    let vec_sink = VecEntrySink::new();

    ChildRenameParentPrefix::Variant(ChildRenamed { read_data: 42 })
        .append_on_drop(vec_sink.clone());

    let entries = vec_sink.drain();
    assert_eq!(entries.len(), 1);
    let entry = test_util::to_test_entry(&entries[0]);

    // Child rename applies first (read_data -> ReadData), then parent prefix added (op_ReadData)
    assert_eq!(entry.metrics["op_ReadData"].as_u64(), 42);
}

// Parent rename_all + parent prefix (no child rename)
// When both prefix and rename_all are at container level, rename_all applies to combined prefix+name
#[metrics(prefix = "op_", rename_all = "PascalCase")]
enum ParentRenameAndPrefix {
    ReadData(#[metrics(flatten)] NestedMetrics),
}

#[test]
fn test_parent_rename_and_prefix() {
    let vec_sink = VecEntrySink::new();

    ParentRenameAndPrefix::ReadData(NestedMetrics { value: 100 }).append_on_drop(vec_sink.clone());

    let entries = vec_sink.drain();
    assert_eq!(entries.len(), 1);
    let entry = test_util::to_test_entry(&entries[0]);

    // Parent rename_all applies to flattened fields
    assert_eq!(entry.metrics["Value"].as_u64(), 100);
}

// Competing rename_all (child wins)
#[metrics(rename_all = "kebab-case")]
#[derive(Clone)]
struct ChildKebabCase {
    read_data: u32,
}

#[metrics(rename_all = "PascalCase")]
enum ParentPascalCase {
    Variant(#[metrics(flatten)] ChildKebabCase),
}

#[test]
fn test_competing_rename_all() {
    let vec_sink = VecEntrySink::new();

    ParentPascalCase::Variant(ChildKebabCase { read_data: 50 }).append_on_drop(vec_sink.clone());

    let entries = vec_sink.drain();
    assert_eq!(entries.len(), 1);
    let entry = test_util::to_test_entry(&entries[0]);

    // Child rename wins (read_data -> read-data), parent rename ignored
    assert_eq!(entry.metrics["read-data"].as_u64(), 50);
}

// Exact prefix tests
// Child has exact_prefix, parent has rename_all - exact prefix preserved
#[metrics(exact_prefix = "op-")]
#[derive(Clone)]
struct ExactPrefixChild {
    value: u32,
}

#[metrics(rename_all = "PascalCase")]
enum ExactPrefixEnum {
    Variant(#[metrics(flatten)] ExactPrefixChild),
}

#[test]
fn test_exact_prefix() {
    let vec_sink = VecEntrySink::new();

    ExactPrefixEnum::Variant(ExactPrefixChild { value: 75 }).append_on_drop(vec_sink.clone());

    let entries = vec_sink.drain();
    assert_eq!(entries.len(), 1);
    let entry = test_util::to_test_entry(&entries[0]);

    // Child exact_prefix preserved, parent rename_all applies
    assert_eq!(entry.metrics["op-Value"].as_u64(), 75);
}

// Triple prefix combination: field prefix + nested prefix + deeper nested prefix
#[metrics]
#[derive(Clone)]
struct InnerLevel {
    data: u32,
}

#[metrics]
#[derive(Clone)]
struct MiddleLevel {
    #[metrics(flatten, prefix = "inner_")]
    inner: InnerLevel,
}

#[metrics]
enum TriplePrefixEnum {
    Variant(#[metrics(flatten, prefix = "field_")] MiddleLevel),
}

#[test]
fn test_triple_prefix_combination() {
    let vec_sink = VecEntrySink::new();

    TriplePrefixEnum::Variant(MiddleLevel {
        inner: InnerLevel { data: 42 },
    })
    .append_on_drop(vec_sink.clone());

    let entries = vec_sink.drain();
    assert_eq!(entries.len(), 1);
    let entry = test_util::to_test_entry(&entries[0]);

    // All three prefixes combine: field_ + inner_ + data
    assert_eq!(entry.metrics["field_inner_data"].as_u64(), 42);
}

// Subfield enum flattened into parent struct with field-level prefix
// Uses TimestampOnClose to test subfield_owned (only implements CloseValue for owned, not &T)
#[metrics(subfield_owned)]
struct SubfieldNested {
    timestamp: metrique::timers::TimestampOnClose,
}

#[metrics(subfield_owned)]
enum SubfieldStatus {
    TupleVariant(#[metrics(flatten)] SubfieldNested),
    StructVariant {
        timestamp: metrique::timers::TimestampOnClose,
    },
}

#[metrics]
struct ParentWithFieldPrefix {
    #[metrics(flatten, prefix = "status_")]
    status: SubfieldStatus,
    direct_field: u32,
}

#[test]
fn test_subfield_enum_parent_field_prefix() {
    let vec_sink = VecEntrySink::new();

    ParentWithFieldPrefix {
        status: SubfieldStatus::TupleVariant(SubfieldNested {
            timestamp: Default::default(),
        }),
        direct_field: 200,
    }
    .append_on_drop(vec_sink.clone());

    ParentWithFieldPrefix {
        status: SubfieldStatus::StructVariant {
            timestamp: Default::default(),
        },
        direct_field: 400,
    }
    .append_on_drop(vec_sink.clone());

    let entries = vec_sink.drain();
    assert_eq!(entries.len(), 2);

    // Field-level prefix applies to flattened subfield enum fields
    // TimestampOnClose only implements CloseValue for owned, testing subfield_owned works
    let entry1 = test_util::to_test_entry(&entries[0]);
    // TimestampOnClose closes to TimestampValue which is a string property
    assert!(entry1.values.contains_key("status_timestamp"));
    assert_eq!(entry1.metrics["direct_field"].as_u64(), 200);

    let entry2 = test_util::to_test_entry(&entries[1]);
    assert!(entry2.values.contains_key("status_timestamp"));
    assert_eq!(entry2.metrics["direct_field"].as_u64(), 400);
}

// Subfield enum flattened into parent enum with container-level prefix
#[metrics(prefix = "api_")]
enum ParentWithContainerPrefix {
    Operation {
        #[metrics(flatten)]
        status: SubfieldStatus,
        direct_field: u32,
    },
}

#[test]
fn test_subfield_enum_parent_container_prefix() {
    let vec_sink = VecEntrySink::new();

    ParentWithContainerPrefix::Operation {
        status: SubfieldStatus::TupleVariant(SubfieldNested {
            timestamp: Default::default(),
        }),
        direct_field: 200,
    }
    .append_on_drop(vec_sink.clone());

    ParentWithContainerPrefix::Operation {
        status: SubfieldStatus::StructVariant {
            timestamp: Default::default(),
        },
        direct_field: 400,
    }
    .append_on_drop(vec_sink.clone());

    let entries = vec_sink.drain();
    assert_eq!(entries.len(), 2);

    // Container-level prefix does NOT apply to flattened subfield enum (child controls naming)
    let entry1 = test_util::to_test_entry(&entries[0]);
    // TimestampOnClose closes to TimestampValue which is a string property
    assert!(entry1.values.contains_key("timestamp"));
    assert_eq!(entry1.metrics["api_direct_field"].as_u64(), 200);

    let entry2 = test_util::to_test_entry(&entries[1]);
    assert!(entry2.values.contains_key("timestamp"));
    assert_eq!(entry2.metrics["api_direct_field"].as_u64(), 400);
}

// Tests both struct and tuple variants with types that only implement CloseValue for owned
#[metrics(subfield_owned)]
struct TimestampWrapper {
    timestamp: metrique::timers::TimestampOnClose,
}

#[metrics(subfield_owned)]
struct StringWrapper {
    value: String,
}

#[metrics(subfield_owned)]
enum InnerStatus {
    Active {
        timestamp: metrique::timers::TimestampOnClose,
    },
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

#[test]
fn test_enum_enum_subfield_owned() {
    let vec_sink = VecEntrySink::new();

    OuterOperation::Process {
        status: InnerStatus::Active {
            timestamp: Default::default(),
        },
    }
    .append_on_drop(vec_sink.clone());

    OuterOperation::Process {
        status: InnerStatus::Pending(TimestampWrapper {
            timestamp: Default::default(),
        }),
    }
    .append_on_drop(vec_sink.clone());

    OuterOperation::Execute(StringWrapper {
        value: "test".to_string(),
    })
    .append_on_drop(vec_sink.clone());

    let entries = vec_sink.drain();
    assert_eq!(entries.len(), 3);

    // Verify TimestampOnClose fields emitted (as string properties)
    let entry1 = test_util::to_test_entry(&entries[0]);
    assert!(entry1.values.contains_key("timestamp"));

    let entry2 = test_util::to_test_entry(&entries[1]);
    assert!(entry2.values.contains_key("timestamp"));

    // Verify String field emitted (as string property)
    let entry3 = test_util::to_test_entry(&entries[2]);
    assert_eq!(entry3.values["value"], "test");
}

// Struct variant field with nested flatten
#[metrics]
#[derive(Clone)]
struct DeepNested {
    inner_value: u32,
}

#[metrics]
#[derive(Clone)]
struct MiddleNested {
    #[metrics(flatten, prefix = "mid_")]
    deep: DeepNested,
    middle_value: u32,
}

#[metrics]
enum StructVariantNested {
    Variant {
        #[metrics(flatten, prefix = "outer_")]
        middle: MiddleNested,
        outer_value: u32,
    },
}

#[test]
fn test_struct_variant_nested_flatten() {
    let vec_sink = VecEntrySink::new();

    StructVariantNested::Variant {
        middle: MiddleNested {
            deep: DeepNested { inner_value: 10 },
            middle_value: 20,
        },
        outer_value: 30,
    }
    .append_on_drop(vec_sink.clone());

    let entries = vec_sink.drain();
    assert_eq!(entries.len(), 1);
    let entry = test_util::to_test_entry(&entries[0]);

    // Prefixes combine: outer_ + mid_ + inner_value
    assert_eq!(entry.metrics["outer_mid_inner_value"].as_u64(), 10);
    assert_eq!(entry.metrics["outer_middle_value"].as_u64(), 20);
    assert_eq!(entry.metrics["outer_value"].as_u64(), 30);
}
