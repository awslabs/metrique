// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use metrique::{test_util::test_metric, unit_of_work::metrics};

#[metrics]
#[derive(Clone)]
pub struct NestedMetrics {
    value: u32,
}

// Child rename_all + parent prefix
#[metrics(rename_all = "PascalCase")]
#[derive(Clone)]
pub struct ChildRenamed {
    read_data: u32,
}

#[metrics]
enum ChildRenameParentPrefix {
    Variant(#[metrics(flatten, prefix = "op_")] ChildRenamed),
}

#[test]
fn test_child_rename_parent_prefix() {
    let entry = test_metric(ChildRenameParentPrefix::Variant(ChildRenamed {
        read_data: 42,
    }));

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
    let entry = test_metric(ParentRenameAndPrefix::ReadData(NestedMetrics {
        value: 100,
    }));

    // Parent rename_all applies to flattened fields
    assert_eq!(entry.metrics["Value"].as_u64(), 100);
}

// Competing rename_all (child wins)
#[metrics(rename_all = "kebab-case")]
#[derive(Clone)]
pub struct ChildKebabCase {
    read_data: u32,
}

#[metrics(rename_all = "PascalCase")]
enum ParentPascalCase {
    Variant(#[metrics(flatten)] ChildKebabCase),
}

#[test]
fn test_competing_rename_all() {
    let entry = test_metric(ParentPascalCase::Variant(ChildKebabCase { read_data: 50 }));

    // Child rename wins (read_data -> read-data), parent rename ignored
    assert_eq!(entry.metrics["read-data"].as_u64(), 50);
}

// Exact prefix tests
// Child has exact_prefix, parent has rename_all - exact prefix preserved
#[metrics(exact_prefix = "op-")]
#[derive(Clone)]
pub struct ExactPrefixChild {
    value: u32,
}

#[metrics(rename_all = "PascalCase")]
enum ExactPrefixEnum {
    Variant(#[metrics(flatten)] ExactPrefixChild),
}

#[test]
fn test_exact_prefix() {
    let entry = test_metric(ExactPrefixEnum::Variant(ExactPrefixChild { value: 75 }));

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
pub struct MiddleLevel {
    #[metrics(flatten, prefix = "inner_")]
    inner: InnerLevel,
}

#[metrics]
enum TriplePrefixEnum {
    Variant(#[metrics(flatten, prefix = "field_")] MiddleLevel),
}

#[test]
fn test_triple_prefix_combination() {
    let entry = test_metric(TriplePrefixEnum::Variant(MiddleLevel {
        inner: InnerLevel { data: 42 },
    }));

    // All three prefixes combine: field_ + inner_ + data
    assert_eq!(entry.metrics["field_inner_data"].as_u64(), 42);
}

// Subfield enum flattened into parent struct with field-level prefix
// Uses TimestampOnClose to test subfield_owned (only implements CloseValue for owned, not &T)
#[metrics(subfield_owned)]
pub struct SubfieldNested {
    timestamp: metrique::timers::TimestampOnClose,
}

#[metrics(subfield_owned)]
pub enum SubfieldStatus {
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
    let entry1 = test_metric(ParentWithFieldPrefix {
        status: SubfieldStatus::TupleVariant(SubfieldNested {
            timestamp: Default::default(),
        }),
        direct_field: 200,
    });

    let entry2 = test_metric(ParentWithFieldPrefix {
        status: SubfieldStatus::StructVariant {
            timestamp: Default::default(),
        },
        direct_field: 400,
    });

    // Field-level prefix applies to flattened subfield enum fields
    // TimestampOnClose only implements CloseValue for owned, testing subfield_owned works
    // TimestampOnClose closes to TimestampValue which is a string property
    assert!(entry1.values.contains_key("status_timestamp"));
    assert_eq!(entry1.metrics["direct_field"].as_u64(), 200);

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
    let entry1 = test_metric(ParentWithContainerPrefix::Operation {
        status: SubfieldStatus::TupleVariant(SubfieldNested {
            timestamp: Default::default(),
        }),
        direct_field: 200,
    });

    let entry2 = test_metric(ParentWithContainerPrefix::Operation {
        status: SubfieldStatus::StructVariant {
            timestamp: Default::default(),
        },
        direct_field: 400,
    });

    // Container-level prefix does NOT apply to flattened subfield enum (child controls naming)
    // TimestampOnClose closes to TimestampValue which is a string property
    assert!(entry1.values.contains_key("timestamp"));
    assert_eq!(entry1.metrics["api_direct_field"].as_u64(), 200);

    assert!(entry2.values.contains_key("timestamp"));
    assert_eq!(entry2.metrics["api_direct_field"].as_u64(), 400);
}

// Tests both struct and tuple variants with types that only implement CloseValue for owned
#[metrics(subfield_owned)]
pub struct TimestampWrapper {
    timestamp: metrique::timers::TimestampOnClose,
}

#[metrics(subfield_owned)]
pub struct StringWrapper {
    value: String,
}

#[metrics(subfield_owned)]
pub enum InnerStatus {
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
    let entry1 = test_metric(OuterOperation::Process {
        status: InnerStatus::Active {
            timestamp: Default::default(),
        },
    });

    let entry2 = test_metric(OuterOperation::Process {
        status: InnerStatus::Pending(TimestampWrapper {
            timestamp: Default::default(),
        }),
    });

    let entry3 = test_metric(OuterOperation::Execute(StringWrapper {
        value: "test".to_string(),
    }));

    // Verify TimestampOnClose fields emitted (as string properties)
    assert!(entry1.values.contains_key("timestamp"));
    assert!(entry2.values.contains_key("timestamp"));

    // Verify String field emitted (as string property)
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
pub struct MiddleNested {
    #[metrics(flatten, prefix = "mid_")]
    deep: DeepNested,
    middle_value: u32,
}

#[metrics]
pub enum StructVariantNested {
    Variant {
        #[metrics(flatten, prefix = "outer_")]
        middle: MiddleNested,
        outer_value: u32,
    },
}

#[test]
fn test_struct_variant_nested_flatten() {
    let entry = test_metric(StructVariantNested::Variant {
        middle: MiddleNested {
            deep: DeepNested { inner_value: 10 },
            middle_value: 20,
        },
        outer_value: 30,
    });

    // Prefixes combine: outer_ + mid_ + inner_value
    assert_eq!(entry.metrics["outer_mid_inner_value"].as_u64(), 10);
    assert_eq!(entry.metrics["outer_middle_value"].as_u64(), 20);
    assert_eq!(entry.metrics["outer_value"].as_u64(), 30);
}
