#![expect(unexpected_cfgs)]
// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Integration tests for entry descriptors and field tags.

use metrique::unit_of_work::metrics;
use metrique::writer::Entry;
use metrique_writer_core::value::{FlagConstructor, MetricFlags, MetricOptions};
use std::any::TypeId;
use std::time::SystemTime;

// Flag marker types for testing
#[derive(Debug)]
struct AuditExportOpt;
impl MetricOptions for AuditExportOpt {}
struct AuditExport;
impl FlagConstructor for AuditExport {
    fn construct() -> MetricFlags<'static> {
        MetricFlags::upcast(&AuditExportOpt)
    }
}

#[derive(Debug)]
struct Dial9EmitOpt;
impl MetricOptions for Dial9EmitOpt {}
struct Dial9Emit;
impl FlagConstructor for Dial9Emit {
    fn construct() -> MetricFlags<'static> {
        MetricFlags::upcast(&Dial9EmitOpt)
    }
}

#[metrics(rename_all = "PascalCase")]
struct BasicMetrics {
    request_id: String,
    count: u64,
}

#[test]
fn basic_descriptor_fields() {
    let m = BasicMetrics {
        request_id: String::new(),
        count: 0,
    };
    let closed = metrique::CloseValue::close(m);
    let entry = metrique::RootEntry::new(closed);
    let __descs = entry.descriptors().unwrap();
    let desc_ref = &__descs[0];
    let desc = desc_ref;

    assert_eq!(desc.name(), "BasicMetrics");
    assert_eq!(desc.fields_len(), 2);
    assert_eq!(
        desc.fields().collect::<Vec<_>>()[0].base_name(),
        "RequestId"
    );
    assert_eq!(desc.fields().collect::<Vec<_>>()[1].base_name(), "Count");
    assert!(desc.timestamp().is_none());
}

#[metrics(rename_all = "PascalCase")]
struct WithTimestamp {
    #[metrics(timestamp)]
    start: SystemTime,
    value: u64,
}

#[test]
fn descriptor_with_timestamp() {
    let m = WithTimestamp {
        start: SystemTime::UNIX_EPOCH,
        value: 42,
    };
    let closed = metrique::CloseValue::close(m);
    let entry = metrique::RootEntry::new(closed);
    let __descs = entry.descriptors().unwrap();
    let desc_ref = &__descs[0];
    let desc = desc_ref;

    assert_eq!(desc.name(), "WithTimestamp");
    // timestamp is excluded from fields()
    assert_eq!(desc.fields_len(), 1);
    assert_eq!(desc.fields().collect::<Vec<_>>()[0].base_name(), "Value");
    // but available via timestamp()
    let ts = desc.timestamp().unwrap();
    assert_eq!(ts.name(), "start");
}

#[metrics(rename_all = "PascalCase")]
struct WithUnit {
    #[metrics(unit = metrique::unit::Millisecond)]
    latency: std::time::Duration,
}

#[test]
fn descriptor_with_unit() {
    let m = WithUnit {
        latency: std::time::Duration::from_millis(100),
    };
    let closed = metrique::CloseValue::close(m);
    let entry = metrique::RootEntry::new(closed);
    let __descs = entry.descriptors().unwrap();
    let desc = &__descs[0];
    let field = &desc.fields().collect::<Vec<_>>()[0];

    assert_eq!(field.base_name(), "Latency");
    assert!(field.unit().is_some());
}

#[metrics(rename_all = "PascalCase", default_flags(AuditExport))]
struct TaggedMetrics {
    request_id: String,
    operation: &'static str,
    #[metrics(flags(skip(AuditExport)))]
    debug_blob: String,
}

#[test]
fn flag_resolution_default_and_skip() {
    let m = TaggedMetrics {
        request_id: String::new(),
        operation: "test",
        debug_blob: String::new(),
    };
    let closed = metrique::CloseValue::close(m);
    let entry = metrique::RootEntry::new(closed);
    let __descs = entry.descriptors().unwrap();
    let desc = &__descs[0];
    let fields: Vec<_> = desc.fields().collect();

    let audit_id = TypeId::of::<AuditExport>();

    // request_id: inherits default_flags(AuditExport) -> Present
    let request_id_flags = fields[0].flags().collect::<Vec<_>>();
    assert_eq!(request_id_flags.len(), 1);
    assert_eq!(request_id_flags[0].type_id(), audit_id);
    // present (in the list)

    // operation: inherits default_flags(AuditExport) -> Present
    let op_flags = fields[1].flags().collect::<Vec<_>>();
    assert_eq!(op_flags.len(), 1);
    assert_eq!(op_flags[0].type_id(), audit_id);
    // present (in the list)

    // debug_blob: flags(skip(AuditExport)) suppresses default -> not present
    let debug_flags = fields[2].flags().collect::<Vec<_>>();
    assert_eq!(debug_flags.len(), 0);
}

#[metrics(rename_all = "PascalCase")]
struct MultiTagMetrics {
    #[metrics(flags(AuditExport, Dial9Emit))]
    important: u64,
    #[metrics(flags(Dial9Emit))]
    trace_only: u64,
    untagged: u64,
}

#[test]
fn multiple_flags_on_field() {
    let m = MultiTagMetrics {
        important: 1,
        trace_only: 2,
        untagged: 3,
    };
    let closed = metrique::CloseValue::close(m);
    let entry = metrique::RootEntry::new(closed);
    let __descs = entry.descriptors().unwrap();
    let desc = &__descs[0];
    let fields: Vec<_> = desc.fields().collect();

    let audit_id = TypeId::of::<AuditExport>();
    let dial9_id = TypeId::of::<Dial9Emit>();

    // important: both tags present
    assert_eq!(fields[0].flags().count(), 2);
    assert!(fields[0].flags().any(|t| t.type_id() == audit_id));
    assert!(fields[0].flags().any(|t| t.type_id() == dial9_id));

    // trace_only: only Dial9Emit
    assert_eq!(fields[1].flags().count(), 1);
    assert_eq!(fields[1].flags().collect::<Vec<_>>()[0].type_id(), dial9_id);

    // untagged: no tags
    assert!(fields[2].flags().next().is_none());
}

#[metrics(rename_all = "PascalCase")]
struct IgnoredField {
    visible: u64,
    #[metrics(ignore)]
    _hidden: u64,
}

#[test]
fn ignored_fields_excluded_from_descriptor() {
    let m = IgnoredField {
        visible: 1,
        _hidden: 2,
    };
    let closed = metrique::CloseValue::close(m);
    let entry = metrique::RootEntry::new(closed);
    let __descs = entry.descriptors().unwrap();
    let desc = &__descs[0];

    assert_eq!(desc.fields_len(), 1);
    assert_eq!(desc.fields().collect::<Vec<_>>()[0].base_name(), "Visible");
}

#[test]
fn descriptor_id_stable_across_calls() {
    let m1 = BasicMetrics {
        request_id: String::new(),
        count: 0,
    };
    let m2 = BasicMetrics {
        request_id: String::new(),
        count: 99,
    };
    let c1 = metrique::CloseValue::close(m1);
    let c2 = metrique::CloseValue::close(m2);
    let e1 = metrique::RootEntry::new(c1);
    let e2 = metrique::RootEntry::new(c2);

    let id1 = e1.descriptors().unwrap()[0].id();
    let id2 = e2.descriptors().unwrap()[0].id();
    assert_eq!(id1, id2);
}

#[test]
fn boxentry_forwards_descriptor() {
    let m = BasicMetrics {
        request_id: String::new(),
        count: 0,
    };
    let closed = metrique::CloseValue::close(m);
    let entry = metrique::RootEntry::new(closed);
    let boxed = entry.boxed();

    let __descs = boxed.descriptors().unwrap();
    let desc = &__descs[0];
    assert_eq!(desc.name(), "BasicMetrics");
}

#[metrics(rename_all = "PascalCase")]
struct FieldNameOverride {
    #[metrics(name = "CustomName")]
    original: u64,
}

#[test]
fn field_name_override_in_descriptor() {
    let m = FieldNameOverride { original: 1 };
    let closed = metrique::CloseValue::close(m);
    let entry = metrique::RootEntry::new(closed);
    let __descs = entry.descriptors().unwrap();
    let desc = &__descs[0];

    assert_eq!(
        desc.fields().collect::<Vec<_>>()[0].base_name(),
        "CustomName"
    );
}

#[metrics(prefix = "api_", rename_all = "PascalCase")]
struct PrefixedMetrics {
    latency: u64,
}

#[test]
fn prefix_applied_in_descriptor() {
    let m = PrefixedMetrics { latency: 100 };
    let closed = metrique::CloseValue::close(m);
    let entry = metrique::RootEntry::new(closed);
    let __descs = entry.descriptors().unwrap();
    let desc = &__descs[0];

    assert_eq!(
        desc.fields().collect::<Vec<_>>()[0].base_name(),
        "ApiLatency"
    );
}

#[metrics(rename_all = "PascalCase", subfield)]
struct SubMetrics {
    #[metrics(flags(AuditExport))]
    sub_value: u64,
    other: u64,
}

#[metrics(rename_all = "PascalCase")]
struct ParentWithFlatten {
    own_field: u64,
    #[metrics(flatten)]
    child: SubMetrics,
}

#[test]
fn flatten_child_descriptors_chained() {
    let m = ParentWithFlatten {
        own_field: 1,
        child: SubMetrics {
            sub_value: 2,
            other: 3,
        },
    };
    let closed = metrique::CloseValue::close(m);
    let entry = metrique::RootEntry::new(closed);

    let descriptors = entry.descriptors().unwrap();
    assert_eq!(descriptors.len(), 2, "parent + flattened child");

    // First descriptor: parent's own fields
    let parent_desc = &descriptors[0];
    assert_eq!(parent_desc.name(), "ParentWithFlatten");
    assert_eq!(parent_desc.fields_len(), 1);
    assert_eq!(
        parent_desc.fields().collect::<Vec<_>>()[0].base_name(),
        "OwnField"
    );

    // Second descriptor: child's fields
    let child_desc = &descriptors[1];
    assert_eq!(child_desc.name(), "SubMetrics");
    assert_eq!(child_desc.fields_len(), 2);
    assert_eq!(
        child_desc.fields().collect::<Vec<_>>()[0].base_name(),
        "SubValue"
    );
    assert_eq!(
        child_desc.fields().collect::<Vec<_>>()[1].base_name(),
        "Other"
    );

    // Child's field_tag is preserved
    let sub_value_flags = child_desc.fields().collect::<Vec<_>>()[0]
        .flags()
        .collect::<Vec<_>>();
    assert_eq!(sub_value_flags.len(), 1);
    assert_eq!(sub_value_flags[0].type_id(), TypeId::of::<AuditExport>());
}

#[metrics(rename_all = "PascalCase", subfield)]
struct TaggedSubMetrics {
    #[metrics(flags(Dial9Emit))]
    alpha: u64,
    #[metrics(flags(skip(Dial9Emit)))]
    beta: u64,
}

#[metrics(rename_all = "PascalCase")]
struct ParentWithTaggedFlatten {
    top: u64,
    #[metrics(flatten)]
    inner: TaggedSubMetrics,
}

#[test]
fn flatten_child_default_flags_resolved() {
    let m = ParentWithTaggedFlatten {
        top: 1,
        inner: TaggedSubMetrics { alpha: 2, beta: 3 },
    };
    let closed = metrique::CloseValue::close(m);
    let entry = metrique::RootEntry::new(closed);

    let descriptors = entry.descriptors().unwrap();
    assert_eq!(descriptors.len(), 2);

    let child_desc = &descriptors[1];
    let dial9_id = TypeId::of::<Dial9Emit>();

    // alpha inherits default_flags(Dial9Emit) -> Present
    let alpha_flags = child_desc.fields().collect::<Vec<_>>()[0]
        .flags()
        .collect::<Vec<_>>();
    assert_eq!(alpha_flags.len(), 1);
    assert_eq!(alpha_flags[0].type_id(), dial9_id);

    // beta has flags(skip(Dial9Emit)) -> suppressed, not present
    let beta_flags = child_desc.fields().collect::<Vec<_>>()[1]
        .flags()
        .collect::<Vec<_>>();
    assert_eq!(beta_flags.len(), 0);
}

// ─── Multilayer flatten tests ───────────────────────────────────────────────

#[metrics(subfield)]
struct GrandChild {
    #[metrics(flags(AuditExport))]
    deep_value: u64,
}

#[metrics(subfield, rename_all = "PascalCase")]
struct MiddleChild {
    middle_value: u64,
    #[metrics(flatten, prefix = "inner_")]
    grand: GrandChild,
}

#[metrics(rename_all = "PascalCase")]
struct NestedFlattenParent {
    top_value: u64,
    #[metrics(flatten, prefix = "mid_")]
    middle: MiddleChild,
}

#[test]
fn nested_flatten_prefix_stacking() {
    let m = NestedFlattenParent {
        top_value: 1,
        middle: MiddleChild {
            middle_value: 2,
            grand: GrandChild { deep_value: 3 },
        },
    };
    let closed = metrique::CloseValue::close(m);
    let entry = metrique::RootEntry::new(closed);

    let descriptors = entry.descriptors().unwrap();
    // Parent's own fields + middle's descriptor + grandchild's descriptor
    assert!(descriptors.len() >= 2);

    // Parent's own descriptor
    let parent_fields: Vec<_> = descriptors[0].fields().collect();
    assert_eq!(parent_fields[0].base_name(), "TopValue");

    // Middle child's descriptor (with parent's flatten prefix "Mid" applied)
    let middle_fields: Vec<_> = descriptors[1].fields().collect();
    let mid_parts: Vec<&str> = middle_fields[0].name_parts().collect();
    assert_eq!(mid_parts, vec!["Mid", "MiddleValue"]);

    // Grandchild's descriptor (with both prefixes: parent's "Mid" then middle's "Inner")
    assert!(descriptors.len() >= 3, "expected grandchild descriptor");
    let grand_fields: Vec<_> = descriptors[2].fields().collect();
    let grand_parts: Vec<&str> = grand_fields[0].name_parts().collect();
    // Outermost prefix first: Mid, then Inner, then field name
    // Style propagates from parent: GrandChild field rendered as PascalCase
    assert_eq!(grand_parts, vec!["Mid", "Inner", "DeepValue"]);
}

#[metrics(subfield)]
struct CfgChild {
    cfg_value: u64,
}

#[metrics(rename_all = "PascalCase")]
struct CfgFlattenParent {
    own: u64,
    #[cfg(test)]
    #[metrics(flatten)]
    child: CfgChild,
}

#[test]
fn cfg_gated_flatten_included_in_test() {
    let m = CfgFlattenParent {
        own: 1,
        child: CfgChild { cfg_value: 2 },
    };
    let closed = metrique::CloseValue::close(m);
    let entry = metrique::RootEntry::new(closed);

    let descriptors = entry.descriptors().unwrap();
    // In test cfg, child is included
    assert_eq!(descriptors.len(), 2);
    assert_eq!(descriptors[0].fields_len(), 1); // parent's own field
    assert_eq!(descriptors[1].fields_len(), 1); // child's field
}

#[metrics(subfield)]
struct NeverChild {
    never_value: u64,
}

#[metrics(rename_all = "PascalCase")]
struct CfgDisabledFlatten {
    own: u64,
    #[cfg(feature = "__metrique_nonexistent_feature")]
    #[metrics(flatten)]
    never: NeverChild,
}

#[test]
fn cfg_disabled_flatten_excluded() {
    let m = CfgDisabledFlatten { own: 1 };
    let closed = metrique::CloseValue::close(m);
    let entry = metrique::RootEntry::new(closed);

    let descriptors = entry.descriptors().unwrap();
    // Only parent's own descriptor, child is cfg-disabled
    assert_eq!(descriptors.len(), 1);
    assert_eq!(descriptors[0].fields_len(), 1);
}

#[metrics]
struct AllIgnored {
    #[metrics(ignore)]
    #[allow(dead_code)]
    _a: u64,
    #[metrics(ignore)]
    #[allow(dead_code)]
    _b: u64,
}

#[test]
fn all_ignored_fields_produces_empty_descriptor() {
    let m = AllIgnored { _a: 1, _b: 2 };
    let closed = metrique::CloseValue::close(m);
    let entry = metrique::RootEntry::new(closed);
    let descriptors = entry.descriptors().unwrap();
    assert_eq!(descriptors.len(), 1);
    assert_eq!(descriptors[0].fields_len(), 0);
}

#[metrics(subfield)]
pub struct EnumPrefixChild {
    child_val: u64,
}

#[metrics(rename_all = "PascalCase")]
enum EnumWithFlatten {
    Simple {
        count: u64,
    },
    WithChild {
        count: u64,
        #[metrics(flatten)]
        child: EnumPrefixChild,
    },
}

#[test]
fn enum_variant_with_flatten_chains_child_descriptor() {
    let m = EnumWithFlatten::WithChild {
        count: 1,
        child: EnumPrefixChild { child_val: 2 },
    };
    let closed = metrique::CloseValue::close(m);
    let entry = metrique::RootEntry::new(closed);

    let descriptors = entry.descriptors().unwrap();
    // Base descriptor (union of non-flatten fields) + child's descriptor
    assert!(
        descriptors.len() >= 2,
        "expected base + child, got {}",
        descriptors.len()
    );

    // Child's descriptor has its field
    let child_fields: Vec<_> = descriptors[1].fields().collect();
    assert_eq!(child_fields[0].base_name(), "ChildVal");
}

#[test]
fn enum_variant_without_flatten_yields_one_descriptor() {
    let m = EnumWithFlatten::Simple { count: 1 };
    let closed = metrique::CloseValue::close(m);
    let entry = metrique::RootEntry::new(closed);

    let descriptors = entry.descriptors().unwrap();
    // Only the base descriptor, no flatten children
    assert_eq!(descriptors.len(), 1);
}

#[test]
fn enum_variants_have_different_descriptor_ids() {
    let simple = EnumWithFlatten::Simple { count: 1 };
    let with_child = EnumWithFlatten::WithChild {
        count: 1,
        child: EnumPrefixChild { child_val: 2 },
    };

    let closed_simple = metrique::CloseValue::close(simple);
    let closed_child = metrique::CloseValue::close(with_child);
    let entry_simple = metrique::RootEntry::new(closed_simple);
    let entry_child = metrique::RootEntry::new(closed_child);

    let descs_simple = entry_simple.descriptors().unwrap();
    let descs_child = entry_child.descriptors().unwrap();

    // Different variants produce different base descriptor ids
    // (each variant has its own static with only its fields)
    assert_ne!(descs_simple[0].id(), descs_child[0].id());

    // Each variant's descriptor name includes the variant
    assert!(descs_simple[0].name().contains("Simple"));
    assert!(descs_child[0].name().contains("WithChild"));
}

#[metrics(subfield)]
pub struct OrderChildA {
    a_val: u64,
}
#[metrics(subfield)]
pub struct OrderChildB {
    b_val: u64,
}
#[metrics(subfield)]
pub struct OrderChildC {
    c_val: u64,
}

#[metrics(rename_all = "PascalCase")]
struct CfgOrderParent {
    #[metrics(flatten)]
    first: OrderChildA,
    #[cfg(test)]
    #[metrics(flatten)]
    middle: OrderChildB,
    #[metrics(flatten)]
    last: OrderChildC,
}

#[test]
fn cfg_flatten_ordering_preserved() {
    let m = CfgOrderParent {
        first: OrderChildA { a_val: 1 },
        middle: OrderChildB { b_val: 2 },
        last: OrderChildC { c_val: 3 },
    };
    let closed = metrique::CloseValue::close(m);
    let entry = metrique::RootEntry::new(closed);
    let descriptors = entry.descriptors().unwrap();
    assert_eq!(descriptors.len(), 4);
    let d1: Vec<_> = descriptors[1].fields().collect();
    let d2: Vec<_> = descriptors[2].fields().collect();
    let d3: Vec<_> = descriptors[3].fields().collect();
    assert_eq!(d1[0].base_name(), "AVal");
    assert_eq!(d2[0].base_name(), "BVal");
    assert_eq!(d3[0].base_name(), "CVal");
}

#[metrics(rename_all = "PascalCase")]
enum EnumFieldOrder {
    Multi {
        alpha: u64,
        beta: u64,
        gamma: u64,
        #[metrics(flatten)]
        child: OrderChildA,
        delta: u64,
    },
}

#[test]
fn enum_variant_field_order_matches_declaration() {
    let m = EnumFieldOrder::Multi {
        alpha: 1,
        beta: 2,
        gamma: 3,
        child: OrderChildA { a_val: 4 },
        delta: 5,
    };
    let closed = metrique::CloseValue::close(m);
    let entry = metrique::RootEntry::new(closed);
    let descriptors = entry.descriptors().unwrap();

    // Own fields split into contiguous runs at the flatten boundary,
    // in write order: [alpha, beta, gamma], [child], [delta]
    assert_eq!(descriptors.len(), 3);
    let base_fields: Vec<_> = descriptors[0].fields().collect();
    assert_eq!(base_fields.len(), 3);
    assert_eq!(base_fields[0].base_name(), "Alpha");
    assert_eq!(base_fields[1].base_name(), "Beta");
    assert_eq!(base_fields[2].base_name(), "Gamma");

    // Flatten child at its declaration position
    let child_fields: Vec<_> = descriptors[1].fields().collect();
    assert_eq!(child_fields[0].base_name(), "AVal");

    // Trailing own-field run after the flatten
    let trailing_fields: Vec<_> = descriptors[2].fields().collect();
    assert_eq!(trailing_fields.len(), 1);
    assert_eq!(trailing_fields[0].base_name(), "Delta");
}

#[metrics(subfield)]
pub struct TupleCfgChild {
    tc_val: u64,
}

#[test]
fn descriptors_forward_through_option_and_box() {
    // Use BasicMetrics which has a known descriptor
    let m = BasicMetrics {
        request_id: String::new(),
        count: 0,
    };
    let closed = metrique::CloseValue::close(m);
    let entry = metrique::RootEntry::new(closed);
    let base_descs = entry.descriptors().unwrap();
    assert!(!base_descs.is_empty());

    // Option<T> forwards when Some
    let opt = Some(metrique::CloseValue::close(BasicMetrics {
        request_id: String::new(),
        count: 0,
    }));
    let opt_entry = metrique::RootEntry::new(opt);
    let opt_descs = opt_entry.descriptors().unwrap();
    assert_eq!(opt_descs.len(), base_descs.len());
    assert_eq!(opt_descs[0].name(), base_descs[0].name());

    // Option<T> returns empty when None
    let none: Option<<BasicMetrics as metrique::CloseValue>::Closed> = None;
    let none_entry = metrique::RootEntry::new(none);
    let none_descs = none_entry.descriptors().unwrap();
    assert_eq!(none_descs.len(), 0);
}

#[metrics(subfield, rename_all = "snake_case")]
struct SnakeCaseChild {
    my_field: u64,
}

#[metrics(rename_all = "PascalCase")]
struct PascalParentSnakeChild {
    own_field: u64,
    #[metrics(flatten)]
    child: SnakeCaseChild,
}

#[test]
fn style_propagation_child_preferred_over_parent() {
    let m = PascalParentSnakeChild {
        own_field: 1,
        child: SnakeCaseChild { my_field: 2 },
    };
    let closed = metrique::CloseValue::close(m);
    let entry = metrique::RootEntry::new(closed);
    let descs = entry.descriptors().unwrap();

    // Parent's own field uses PascalCase
    assert_eq!(descs[0].fields().next().unwrap().base_name(), "OwnField");

    // Child's field uses snake_case (child's rename_all wins over parent's PascalCase)
    let child_fields: Vec<_> = descs[1].fields().collect();
    assert_eq!(child_fields[0].base_name(), "my_field");
}

#[metrics(subfield)]
enum StyleEnum {
    Variant { my_field: u64 },
}

#[metrics(rename_all = "PascalCase")]
struct ParentWithEnumPrefixChild {
    #[metrics(flatten)]
    child: StyleEnum,
}

#[test]
fn enum_style_propagation_from_parent() {
    let m = ParentWithEnumPrefixChild {
        child: StyleEnum::Variant { my_field: 42 },
    };
    let closed = metrique::CloseValue::close(m);
    let entry = metrique::RootEntry::new(closed);
    let descs = entry.descriptors().unwrap();

    // The enum child has no rename_all, so it inherits PascalCase from parent
    let child_fields: Vec<_> = descs[1].fields().collect();
    assert_eq!(child_fields[0].base_name(), "MyField");
}

#[metrics(subfield, rename_all = "snake_case")]
enum SnakeCaseEnum {
    Variant { my_field_name: u64 },
}

#[metrics(rename_all = "PascalCase")]
struct ParentWithSnakeEnum {
    #[metrics(flatten)]
    child: SnakeCaseEnum,
}

#[test]
fn enum_child_rename_all_takes_precedence() {
    let m = ParentWithSnakeEnum {
        child: SnakeCaseEnum::Variant { my_field_name: 1 },
    };
    let closed = metrique::CloseValue::close(m);
    let entry = metrique::RootEntry::new(closed);
    let descs = entry.descriptors().unwrap();

    // Enum child has rename_all = "snake_case", so it wins over parent's PascalCase
    let child_fields: Vec<_> = descs[1].fields().collect();
    assert_eq!(child_fields[0].base_name(), "my_field_name");
}

#[metrics(rename_all = "PascalCase")]
enum EnumCfgVariant {
    Always {
        val: u64,
    },
    #[cfg(test)]
    TestOnly {
        val: u64,
    },
    Never {
        val: u64,
    },
}

#[test]
fn enum_cfg_on_whole_variant() {
    // Always variant
    let m = EnumCfgVariant::Always { val: 1 };
    let closed = metrique::CloseValue::close(m);
    let entry = metrique::RootEntry::new(closed);
    let descs = entry.descriptors().unwrap();
    assert_eq!(descs[0].name(), "EnumCfgVariant::Always");

    // TestOnly variant (cfg(test) is active)
    let m2 = EnumCfgVariant::TestOnly { val: 2 };
    let closed2 = metrique::CloseValue::close(m2);
    let entry2 = metrique::RootEntry::new(closed2);
    let descs2 = entry2.descriptors().unwrap();
    assert_eq!(descs2[0].name(), "EnumCfgVariant::TestOnly");
}

#[test]
fn descriptor_forwarding_through_wrappers() {
    // Box<T> forwards via BoxEntry
    let m = BasicMetrics {
        request_id: String::new(),
        count: 0,
    };
    let closed = metrique::CloseValue::close(m);
    let root = metrique::RootEntry::new(closed);
    let boxed = metrique::writer::BoxEntry::new(root);
    let descs = boxed.descriptors().unwrap();
    assert_eq!(descs[0].name(), "BasicMetrics");

    // &T forwards
    let m2 = BasicMetrics {
        request_id: String::new(),
        count: 0,
    };
    let closed2 = metrique::CloseValue::close(m2);
    let root2 = metrique::RootEntry::new(closed2);
    let descs = (&root2).descriptors().unwrap();
    assert_eq!(descs[0].name(), "BasicMetrics");

    // Arc<T> forwards (via BoxEntry which uses Arc internally)
    // Already tested via BoxEntry above

    // Option<T> forwards when Some, Unavailable-like when None
    let m3 = BasicMetrics {
        request_id: String::new(),
        count: 0,
    };
    let closed3 = metrique::CloseValue::close(m3);
    let opt = Some(metrique::RootEntry::new(closed3));
    let descs = opt.as_ref().unwrap().descriptors().unwrap();
    assert_eq!(descs[0].name(), "BasicMetrics");
}

// ============================================================
// Deep nesting stress test: 4 levels, conflicting styles, prefixes
// ============================================================

// Level 4 (deepest): snake_case, no prefix
#[metrics(subfield, rename_all = "snake_case")]
struct DeepLeaf {
    leaf_metric: u64,
    #[metrics(flags(AuditExport))]
    tagged_leaf: u64,
}

// Level 3: preserve (no rename), flattens DeepLeaf with prefix
#[metrics(subfield, rename_all = "preserve")]
struct Level3 {
    level_three_value: u64,
    #[metrics(flatten, prefix = "deep_")]
    leaf: DeepLeaf,
}

// Level 2: PascalCase, flattens Level3 with prefix
#[metrics(subfield, rename_all = "PascalCase")]
struct Level2 {
    level_two_value: u64,
    #[metrics(flatten, prefix = "l3_")]
    child: Level3,
}

// Level 1 (root): PascalCase, flattens Level2 with prefix
#[metrics(rename_all = "PascalCase")]
struct DeepNestRoot {
    root_value: u64,
    #[metrics(flatten, prefix = "l2_")]
    nested: Level2,
}

#[test]
fn deep_nesting_style_and_prefix_stress() {
    let m = DeepNestRoot {
        root_value: 1,
        nested: Level2 {
            level_two_value: 2,
            child: Level3 {
                level_three_value: 3,
                leaf: DeepLeaf {
                    leaf_metric: 4,
                    tagged_leaf: 5,
                },
            },
        },
    };

    // ---- Write path verification ----
    // Style rules: child's own rename_all wins for its fields.
    // Prefixes are inflected by the PARENT's style.
    let closed = metrique::CloseValue::close(m);
    let written = metrique::writer::test_util::to_test_entry(metrique::RootEntry::new(closed));

    // Root field: PascalCase (root's own style)
    assert!(
        written.metrics.contains_key("RootValue"),
        "root field should be PascalCase, got keys: {:?}",
        written.metrics.keys().collect::<Vec<_>>()
    );

    // Level2 field: Level2 has rename_all="PascalCase", prefix "l2_" inflected
    // by root's PascalCase -> "L2". Field "level_two_value" -> "LevelTwoValue" (Level2's own PascalCase)
    assert!(
        written.metrics.contains_key("L2LevelTwoValue"),
        "level2 field should be L2LevelTwoValue, got keys: {:?}",
        written.metrics.keys().collect::<Vec<_>>()
    );

    // Level3 field: Level3 has rename_all="preserve" which means "inherit parent style"
    // So parent's PascalCase applies. Prefix "l3_" -> "L3". Field "level_three_value" -> "LevelThreeValue"
    assert!(
        written.metrics.contains_key("L2L3LevelThreeValue"),
        "level3 field should be L2L3LevelThreeValue (preserve=inherit parent), got keys: {:?}",
        written.metrics.keys().collect::<Vec<_>>()
    );

    // DeepLeaf field: DeepLeaf has rename_all="snake_case", prefix "deep_" inflected
    // by root's PascalCase -> "Deep". Field "leaf_metric" -> "leaf_metric" (snake_case)
    assert!(
        written.metrics.contains_key("L2L3Deepleaf_metric"),
        "leaf field should be L2L3Deepleaf_metric, got keys: {:?}",
        written.metrics.keys().collect::<Vec<_>>()
    );

    assert!(
        written.metrics.contains_key("L2L3Deeptagged_leaf"),
        "tagged leaf field should be L2L3Deeptagged_leaf, got keys: {:?}",
        written.metrics.keys().collect::<Vec<_>>()
    );

    // ---- Descriptor verification: must match write path exactly ----
    let m2 = DeepNestRoot {
        root_value: 1,
        nested: Level2 {
            level_two_value: 2,
            child: Level3 {
                level_three_value: 3,
                leaf: DeepLeaf {
                    leaf_metric: 4,
                    tagged_leaf: 5,
                },
            },
        },
    };
    let closed2 = metrique::CloseValue::close(m2);
    let entry = metrique::RootEntry::new(closed2);
    let descriptors = entry.descriptors().unwrap();

    // Should have 4 segments: root, level2, level3, leaf
    assert_eq!(
        descriptors.len(),
        4,
        "expected 4 descriptor segments (root + 3 flatten levels)"
    );

    // Segment 0: root's own field (PascalCase)
    let root_fields: Vec<_> = descriptors[0].fields().collect();
    assert_eq!(root_fields.len(), 1);
    assert_eq!(root_fields[0].base_name(), "RootValue");

    // Segment 1: Level2 (prefix "L2", field in Level2's PascalCase)
    let l2_fields: Vec<_> = descriptors[1].fields().collect();
    assert_eq!(l2_fields.len(), 1);
    let l2_parts: Vec<&str> = l2_fields[0].name_parts().collect();
    assert_eq!(l2_parts, vec!["L2", "LevelTwoValue"]);

    // Segment 2: Level3 (prefixes "L2" + "L3", field in parent's PascalCase since preserve=inherit)
    let l3_fields: Vec<_> = descriptors[2].fields().collect();
    assert_eq!(l3_fields.len(), 1);
    let l3_parts: Vec<&str> = l3_fields[0].name_parts().collect();
    assert_eq!(l3_parts, vec!["L2", "L3", "LevelThreeValue"]);

    // Segment 3: DeepLeaf (prefixes "L2" + "L3" + "Deep", fields in DeepLeaf's snake_case)
    let leaf_fields: Vec<_> = descriptors[3].fields().collect();
    assert_eq!(leaf_fields.len(), 2);
    let leaf_parts: Vec<&str> = leaf_fields[0].name_parts().collect();
    assert_eq!(leaf_parts, vec!["L2", "L3", "Deep", "leaf_metric"]);

    let tagged_parts: Vec<&str> = leaf_fields[1].name_parts().collect();
    assert_eq!(tagged_parts, vec!["L2", "L3", "Deep", "tagged_leaf"]);

    // Verify flag survived the nesting
    assert!(
        leaf_fields[1].flags().any(|f| f.is::<AuditExport>()),
        "flag should propagate through nesting"
    );
}

// Test that parent's style propagates to flattened children
// (child's own rename_all is irrelevant; parent always wins for flatten)
#[metrics(subfield, rename_all = "snake_case")]
struct ChildOwnStyle {
    my_field: u64,
}

#[metrics(rename_all = "PascalCase")]
struct ParentConflictingStyle {
    parent_field: u64,
    #[metrics(flatten, prefix = "sub_")]
    child: ChildOwnStyle,
}

#[test]
fn conflicting_styles_parent_wins_for_flatten() {
    let m = ParentConflictingStyle {
        parent_field: 1,
        child: ChildOwnStyle { my_field: 2 },
    };

    // Write path: parent's style propagates to child
    let closed = metrique::CloseValue::close(m);
    let written = metrique::writer::test_util::to_test_entry(metrique::RootEntry::new(closed));

    // Parent's own field: PascalCase
    assert!(
        written.metrics.contains_key("ParentField"),
        "parent field should be PascalCase, got keys: {:?}",
        written.metrics.keys().collect::<Vec<_>>()
    );

    // Child's field: child's own rename_all="snake_case" wins for field names.
    // prefix "sub_" inflected by parent's PascalCase -> "Sub"
    // field "my_field" stays snake_case (child's own style)
    assert!(
        written.metrics.contains_key("Submy_field"),
        "child field should be Submy_field (prefix=PascalCase, field=snake_case), got keys: {:?}",
        written.metrics.keys().collect::<Vec<_>>()
    );

    // Descriptor should match
    let m2 = ParentConflictingStyle {
        parent_field: 1,
        child: ChildOwnStyle { my_field: 2 },
    };
    let closed2 = metrique::CloseValue::close(m2);
    let entry = metrique::RootEntry::new(closed2);
    let descriptors = entry.descriptors().unwrap();

    assert_eq!(descriptors.len(), 2);

    // Parent segment
    let parent_fields: Vec<_> = descriptors[0].fields().collect();
    assert_eq!(parent_fields[0].base_name(), "ParentField");

    // Child segment: prefix in parent's PascalCase, field in child's snake_case
    let child_fields: Vec<_> = descriptors[1].fields().collect();
    let parts: Vec<&str> = child_fields[0].name_parts().collect();
    assert_eq!(parts, vec!["Sub", "my_field"]);
}

// ============================================================
// Enum variant with prefixed flatten
// ============================================================

#[metrics(subfield, rename_all = "snake_case")]
pub struct EnumVariantPfxChild {
    child_val: u64,
}

#[metrics(rename_all = "PascalCase")]
enum EnumWithPrefix {
    WithChild {
        own_field: u64,
        #[metrics(flatten, prefix = "nested_")]
        child: EnumVariantPfxChild,
    },
}

#[test]
fn enum_variant_flatten_prefix_in_descriptor() {
    let m = EnumWithPrefix::WithChild {
        own_field: 1,
        child: EnumVariantPfxChild { child_val: 2 },
    };

    // Write path verification
    let closed = metrique::CloseValue::close(m);
    let written = metrique::writer::test_util::to_test_entry(metrique::RootEntry::new(closed));

    // own_field in PascalCase
    assert!(
        written.metrics.contains_key("OwnField"),
        "got keys: {:?}",
        written.metrics.keys().collect::<Vec<_>>()
    );

    // child_val: prefix "nested_" inflected as "Nested" (PascalCase), field stays snake_case
    assert!(
        written.metrics.contains_key("Nestedchild_val"),
        "expected Nestedchild_val, got keys: {:?}",
        written.metrics.keys().collect::<Vec<_>>()
    );

    // Descriptor verification
    let m2 = EnumWithPrefix::WithChild {
        own_field: 1,
        child: EnumVariantPfxChild { child_val: 2 },
    };
    let closed2 = metrique::CloseValue::close(m2);
    let entry = metrique::RootEntry::new(closed2);
    let descriptors = entry.descriptors().unwrap();

    // 2 segments: variant's own fields + child
    assert_eq!(descriptors.len(), 2);

    let own_fields: Vec<_> = descriptors[0].fields().collect();
    assert_eq!(own_fields[0].base_name(), "OwnField");

    let child_fields: Vec<_> = descriptors[1].fields().collect();
    let parts: Vec<&str> = child_fields[0].name_parts().collect();
    assert_eq!(parts, vec!["Nested", "child_val"]);
}

#[test]
fn field_flag_construct_returns_usable_metric_flags() {
    // Use TaggedMetrics which has AuditExport flag on fields via default_flags
    let m = TaggedMetrics {
        request_id: String::new(),
        operation: "op",
        debug_blob: String::new(),
    };
    let closed = metrique::CloseValue::close(m);
    let entry = metrique::RootEntry::new(closed);
    let descriptors = entry.descriptors().unwrap();
    let fields: Vec<_> = descriptors[0].fields().collect();

    // request_id has AuditExport flag (from default_flags)
    let flag = fields[0]
        .flags()
        .next()
        .expect("request_id should have a flag");
    assert!(flag.is::<AuditExport>());

    // construct() returns a MetricFlags that can be downcast to the original options type
    let metric_flags = flag.construct();
    let opts = metric_flags.downcast::<AuditExportOpt>();
    assert!(
        opts.is_some(),
        "construct() should return downcastable MetricFlags"
    );
}

#[test]
fn field_flag_construct_carries_data_beyond_identity() {
    use metrique_writer_core::value::MetricFlags;

    // Define a flag whose MetricOptions carries actual data (not just identity)
    #[derive(Debug)]
    struct RetentionOpts {
        days: u32,
    }
    impl MetricOptions for RetentionOpts {}

    static RETENTION_30: RetentionOpts = RetentionOpts { days: 30 };

    struct Retention30;
    impl FlagConstructor for Retention30 {
        fn construct() -> MetricFlags<'static> {
            MetricFlags::upcast(&RETENTION_30)
        }
    }

    // Build a FieldFlag from this data-carrying constructor
    let flag = metrique_writer_core::FieldFlag::new::<Retention30>();
    assert!(flag.is::<Retention30>());

    // construct() gives access to the actual data, not just identity
    let metric_flags = flag.construct();
    let opts = metric_flags.downcast::<RetentionOpts>().unwrap();
    assert_eq!(opts.days, 30);

    // A sink reading descriptors can use this to make format decisions
    // without the write path (Phase 2 readiness)
}

#[test]
fn metric_flags_supports_non_static_lifetime() {
    use metrique_writer_core::value::MetricFlags;

    // Demonstrate MetricFlags<'a> works with a stack-local MetricOptions value
    #[derive(Debug)]
    struct LocalOpts(u32);
    impl MetricOptions for LocalOpts {}

    let local = LocalOpts(42);
    // MetricFlags borrows local (non-'static)
    let flags: MetricFlags<'_> = MetricFlags::upcast(&local);

    // Can downcast back to the local type
    let recovered = flags.downcast::<LocalOpts>().unwrap();
    assert_eq!(recovered.0, 42);

    // This proves MetricFlags is not inherently tied to 'static.
    // FieldFlag::construct() returns MetricFlags<'static> because FlagConstructor
    // types reference static data, but the MetricFlags type itself is flexible.
}

// --- Flatten-site default_flags tests ---

#[metrics(subfield)]
struct InnerMetrics {
    latency: u64,
    count: u64,
}

#[metrics(subfield, default_flags(Dial9Emit))]
struct InnerWithOwnDefaults {
    normal_field: u64,
    #[metrics(flags(skip(Dial9Emit)))]
    excluded_field: u64,
}

#[test]
fn flatten_only_parent_emits_empty_own_segment() {
    #[metrics(rename_all = "PascalCase")]
    struct Parent {
        #[metrics(flatten)]
        inner: InnerMetrics,
    }

    let m = Parent {
        inner: InnerMetrics {
            latency: 1,
            count: 2,
        },
    };
    let closed = metrique::CloseValue::close(m);
    let entry = metrique::RootEntry::new(closed);
    let descs = entry.descriptors().unwrap();

    assert_eq!(descs.len(), 2);
    assert_eq!(descs[0].name(), "Parent");
    assert_eq!(descs[0].fields_len(), 0);
    assert_eq!(descs[1].name(), "InnerMetrics");
    assert_eq!(descs[1].fields_len(), 2);
}

#[test]
fn flatten_site_default_flags_propagates_to_child() {
    #[metrics(rename_all = "PascalCase")]
    struct Parent {
        own_field: u64,
        #[metrics(flatten, default_flags(AuditExport))]
        inner: InnerMetrics,
    }

    let m = Parent {
        own_field: 1,
        inner: InnerMetrics {
            latency: 10,
            count: 5,
        },
    };
    let closed = metrique::CloseValue::close(m);
    let entry = metrique::RootEntry::new(closed);
    let descs = entry.descriptors().unwrap();

    assert_eq!(descs.len(), 2);

    // Parent's own field has no flags
    let parent_fields: Vec<_> = descs[0].fields().collect();
    assert_eq!(parent_fields[0].flags().count(), 0);

    // Child's fields get AuditExport from flatten-site default_flags
    for f in descs[1].fields() {
        assert!(
            f.flags()
                .any(|fl| fl.type_id() == TypeId::of::<AuditExport>()),
            "child field '{}' missing AuditExport from flatten-site default_flags",
            f.base_name()
        );
    }
}

#[test]
fn flatten_site_default_flags_respects_child_field_skip() {
    #[metrics(rename_all = "PascalCase")]
    struct Parent {
        #[metrics(flatten, default_flags(Dial9Emit))]
        inner: InnerWithOwnDefaults,
    }

    let m = Parent {
        inner: InnerWithOwnDefaults {
            normal_field: 1,
            excluded_field: 2,
        },
    };
    let closed = metrique::CloseValue::close(m);
    let entry = metrique::RootEntry::new(closed);
    let descs = entry.descriptors().unwrap();

    let child = &descs[1];
    let fields: Vec<_> = child.fields().collect();

    // normal_field has Dial9Emit (from child's own default, not re-added)
    assert!(
        fields[0]
            .flags()
            .any(|fl| fl.type_id() == TypeId::of::<Dial9Emit>()),
        "normal_field should have Dial9Emit"
    );

    // excluded_field skipped Dial9Emit at field level: flatten-site must NOT re-add it
    assert!(
        !fields[1]
            .flags()
            .any(|fl| fl.type_id() == TypeId::of::<Dial9Emit>()),
        "excluded_field skip must take precedence over flatten-site default_flags"
    );
}

#[test]
fn flatten_site_default_flags_combines_with_prefix() {
    #[metrics(rename_all = "PascalCase")]
    struct Parent {
        #[metrics(flatten, prefix = "Api", default_flags(AuditExport))]
        inner: InnerMetrics,
    }

    let m = Parent {
        inner: InnerMetrics {
            latency: 10,
            count: 5,
        },
    };
    let closed = metrique::CloseValue::close(m);
    let entry = metrique::RootEntry::new(closed);
    let descs = entry.descriptors().unwrap();

    let child = &descs[1];
    let latency = child.fields().next().unwrap();

    // Prefix applied
    let name: String = latency.name_parts().collect();
    assert_eq!(name, "ApiLatency");

    // Flag applied
    assert!(
        latency
            .flags()
            .any(|fl| fl.type_id() == TypeId::of::<AuditExport>())
    );
}

// ==================== Shape resolution tests ====================

mod shape_tests {
    use metrique::unit_of_work::metrics;
    use metrique::writer::Entry;
    use metrique_writer_core::descriptor::{FieldShape, KnownShape, ShapeRef};
    use std::borrow::Cow;
    use std::time::Duration;

    /// Helper: assert the shape of the Nth field in the first descriptor segment.
    fn assert_field_shape(entry: &impl Entry, idx: usize, expected: FieldShape<'static>) {
        let descs = entry.descriptors().unwrap();
        let shape = descs[0].fields().nth(idx).unwrap().shape();
        assert_eq!(shape, expected, "field index {idx}");
    }

    // -- Scalars --

    #[metrics]
    #[derive(Default)]
    struct ScalarShapes {
        a_bool: bool,
        a_u8: u8,
        a_u16: u16,
        a_u32: u32,
        a_u64: u64,
        a_usize: usize,
        a_f32: f32,
        a_f64: f64,
        a_string: String,
    }

    #[test]
    fn scalar_shapes() {
        let m = ScalarShapes::default();
        let closed = metrique::CloseValue::close(m);
        let entry = metrique::RootEntry::new(closed);

        assert_field_shape(&entry, 0, FieldShape::Known(KnownShape::Bool));
        assert_field_shape(&entry, 1, FieldShape::Known(KnownShape::U8));
        assert_field_shape(&entry, 2, FieldShape::Known(KnownShape::U16));
        assert_field_shape(&entry, 3, FieldShape::Known(KnownShape::U32));
        assert_field_shape(&entry, 4, FieldShape::Known(KnownShape::U64));
        assert_field_shape(&entry, 5, FieldShape::Known(KnownShape::U64)); // usize -> U64
        assert_field_shape(&entry, 6, FieldShape::Known(KnownShape::F32));
        assert_field_shape(&entry, 7, FieldShape::Known(KnownShape::F64));
        assert_field_shape(&entry, 8, FieldShape::Known(KnownShape::String));
    }

    // -- Duration and Timer (close through CloseValue) --

    #[metrics]
    #[derive(Default)]
    struct DurationShapes {
        dur: Duration,
        timer: metrique::timers::Timer,
    }

    #[test]
    fn duration_and_timer_shapes() {
        let m = DurationShapes::default();
        let closed = metrique::CloseValue::close(m);
        let entry = metrique::RootEntry::new(closed);

        let descs = entry.descriptors().unwrap();
        let fields: Vec<_> = descs[0].fields().collect();

        // Duration writes as f64 (millis)
        assert_field_shape(&entry, 0, FieldShape::Known(KnownShape::F64));
        // Timer closes to Duration, same shape
        assert_field_shape(&entry, 1, FieldShape::Known(KnownShape::F64));

        // Both resolve Millisecond unit from the type (no explicit #[metrics(unit = ...)] needed)
        assert_eq!(
            fields[0].unit(),
            Some(metrique_writer_core::Unit::Second(
                metrique_writer_core::unit::NegativeScale::Milli
            ))
        );
        assert_eq!(
            fields[1].unit(),
            Some(metrique_writer_core::Unit::Second(
                metrique_writer_core::unit::NegativeScale::Milli
            ))
        );
    }

    // -- Optional --

    #[metrics]
    #[derive(Default)]
    struct OptionalShapes {
        opt_u64: Option<u64>,
        opt_string: Option<String>,
        opt_duration: Option<Duration>,
    }

    #[test]
    fn optional_shapes() {
        let m = OptionalShapes::default();
        let closed = metrique::CloseValue::close(m);
        let entry = metrique::RootEntry::new(closed);

        static U64_SHAPE: FieldShape = FieldShape::Known(KnownShape::U64);
        static STRING_SHAPE: FieldShape = FieldShape::Known(KnownShape::String);
        static F64_SHAPE: FieldShape = FieldShape::Known(KnownShape::F64);
        assert_field_shape(&entry, 0, FieldShape::Optional(ShapeRef::new(&U64_SHAPE)));
        assert_field_shape(
            &entry,
            1,
            FieldShape::Optional(ShapeRef::new(&STRING_SHAPE)),
        );
        assert_field_shape(&entry, 2, FieldShape::Optional(ShapeRef::new(&F64_SHAPE)));
    }

    // -- Vec / List --

    #[metrics]
    #[derive(Default)]
    struct ListShapes {
        vec_u64: Vec<u64>,
        vec_string: Vec<String>,
    }

    #[test]
    fn list_shapes() {
        let m = ListShapes::default();
        let closed = metrique::CloseValue::close(m);
        let entry = metrique::RootEntry::new(closed);

        static U64_SHAPE: FieldShape = FieldShape::Known(KnownShape::U64);
        static STRING_SHAPE: FieldShape = FieldShape::Known(KnownShape::String);
        assert_field_shape(&entry, 0, FieldShape::List(ShapeRef::new(&U64_SHAPE)));
        assert_field_shape(&entry, 1, FieldShape::List(ShapeRef::new(&STRING_SHAPE)));
    }

    // -- Nested composition: Option<Vec<T>>, Vec<Option<T>> --

    #[metrics]
    #[derive(Default)]
    struct NestedShapes {
        opt_vec: Option<Vec<u64>>,
        vec_opt: Vec<Option<String>>,
    }

    #[test]
    fn nested_composition_shapes() {
        let m = NestedShapes::default();
        let closed = metrique::CloseValue::close(m);
        let entry = metrique::RootEntry::new(closed);

        // Option<Vec<u64>> -> Optional(List(Known(U64)))
        static INNER_U64: FieldShape = FieldShape::Known(KnownShape::U64);
        static LIST_U64: FieldShape = FieldShape::List(ShapeRef::new(&INNER_U64));
        assert_field_shape(&entry, 0, FieldShape::Optional(ShapeRef::new(&LIST_U64)));
        // Vec<Option<String>> -> List(Optional(Known(String)))
        static INNER_STRING: FieldShape = FieldShape::Known(KnownShape::String);
        static OPT_STRING: FieldShape = FieldShape::Optional(ShapeRef::new(&INNER_STRING));
        assert_field_shape(&entry, 1, FieldShape::List(ShapeRef::new(&OPT_STRING)));
    }

    // -- Lifetimed types (Cow) --

    #[metrics]
    struct LifetimedShapes<'a> {
        cow_str: Cow<'a, str>,
        opt_cow: Option<Cow<'a, str>>,
    }

    #[test]
    fn lifetimed_shapes() {
        let m = LifetimedShapes {
            cow_str: Cow::Borrowed("hello"),
            opt_cow: None,
        };
        let closed = metrique::CloseValue::close(m);
        let entry = metrique::RootEntry::new(closed);

        // Cow<'a, str> closes to Cow<'a, str> which is Value with SHAPE = str::SHAPE = String
        assert_field_shape(&entry, 0, FieldShape::Known(KnownShape::String));
        // Option<Cow<'a, str>> -> Optional(Known(String))
        static INNER_STR: FieldShape = FieldShape::Known(KnownShape::String);
        assert_field_shape(&entry, 1, FieldShape::Optional(ShapeRef::new(&INNER_STR)));
    }

    // -- Custom CloseValue resolves through closed type --

    struct CustomGauge(f64);
    impl Default for CustomGauge {
        fn default() -> Self {
            Self(0.0)
        }
    }
    impl metrique::CloseValue for CustomGauge {
        type Closed = f64;
        fn close(self) -> f64 {
            self.0
        }
    }
    impl metrique::CloseValue for &'_ CustomGauge {
        type Closed = f64;
        fn close(self) -> f64 {
            self.0
        }
    }

    #[metrics]
    #[derive(Default)]
    struct CustomCloseShapes {
        gauge: CustomGauge,
    }

    #[test]
    fn custom_close_value_resolves_shape() {
        let m = CustomCloseShapes::default();
        let closed = metrique::CloseValue::close(m);
        let entry = metrique::RootEntry::new(closed);

        // CustomGauge closes to f64, which has SHAPE = Known(F64)
        assert_field_shape(&entry, 0, FieldShape::Known(KnownShape::F64));
    }

    // -- no_close field: Value::SHAPE used directly --

    #[derive(Clone, Default)]
    struct OpaqueNoClose;
    impl metrique::writer::Value for OpaqueNoClose {
        fn write(&self, writer: impl metrique::writer::ValueWriter) {
            writer.string("opaque");
        }
    }

    #[metrics]
    #[derive(Default)]
    struct NoCloseShapes {
        #[metrics(no_close)]
        opaque: OpaqueNoClose,
        normal: u64,
    }

    #[test]
    fn no_close_field_uses_value_shape_directly() {
        let m = NoCloseShapes::default();
        let closed = metrique::CloseValue::close(m);
        let entry = metrique::RootEntry::new(closed);

        // OpaqueNoClose doesn't override SHAPE, gets default Opaque
        assert_field_shape(&entry, 0, FieldShape::Opaque);
        // Normal field still resolves
        assert_field_shape(&entry, 1, FieldShape::Known(KnownShape::U64));
    }

    // -- Custom type with SHAPE override --

    #[derive(Clone, Default)]
    struct TypedNoClose;
    impl metrique::writer::Value for TypedNoClose {
        const SHAPE: FieldShape<'static> = FieldShape::Known(KnownShape::String);
        fn write(&self, writer: impl metrique::writer::ValueWriter) {
            writer.string("typed");
        }
    }

    #[metrics]
    #[derive(Default)]
    struct CustomShapeOverride {
        #[metrics(no_close)]
        typed: TypedNoClose,
    }

    #[test]
    fn custom_value_shape_override() {
        let m = CustomShapeOverride::default();
        let closed = metrique::CloseValue::close(m);
        let entry = metrique::RootEntry::new(closed);

        // TypedNoClose overrides SHAPE to Known(String)
        assert_field_shape(&entry, 0, FieldShape::Known(KnownShape::String));
    }

    // -- Wrapper passthrough (via CloseValue chain) --

    #[metrics]
    #[derive(Default)]
    struct WrapperShapes {
        arc_str: std::sync::Arc<String>,
    }

    #[test]
    fn wrapper_shapes_passthrough() {
        let m = WrapperShapes {
            arc_str: std::sync::Arc::new("hello".to_string()),
        };
        let closed = metrique::CloseValue::close(m);
        let entry = metrique::RootEntry::new(closed);

        // Arc<String> closes to Arc<String>, Value impl forwards to String shape
        assert_field_shape(&entry, 0, FieldShape::Known(KnownShape::String));
    }

    // -- #[metrics(value)] struct and enum shape forwarding --

    #[metrics(value)]
    struct Percent(u8);

    #[metrics(value(string))]
    enum Operation {
        CountDucks,
        FeedDucks,
    }

    #[metrics]
    struct ValueShapes {
        pct: Percent,
        op: Operation,
    }

    #[test]
    fn metrics_value_forwards_shape() {
        let m = ValueShapes {
            pct: Percent(50),
            op: Operation::CountDucks,
        };
        let closed = metrique::CloseValue::close(m);
        let entry = metrique::RootEntry::new(closed);

        // Percent(u8) wraps u8 -> Known(U8)
        assert_field_shape(&entry, 0, FieldShape::Known(KnownShape::U8));
        // Operation is a string-value enum -> Known(String)
        assert_field_shape(&entry, 1, FieldShape::Known(KnownShape::String));
    }

    // -- Formatted fields: shape falls back to Opaque (formatter controls wire representation) --

    struct AsString;
    impl metrique::writer::value::ValueFormatter<bool> for AsString {
        const SHAPE: metrique_writer_core::descriptor::FieldShape<'static> =
            metrique_writer_core::descriptor::FieldShape::Known(
                metrique_writer_core::descriptor::KnownShape::String,
            );
        fn format_value(writer: impl metrique::writer::ValueWriter, value: &bool) {
            writer.string(if *value { "true" } else { "false" });
        }
    }

    #[metrics]
    #[derive(Default)]
    struct FormattedShapes {
        #[metrics(format = AsString)]
        formatted_bool: bool,
    }

    #[test]
    fn formatted_field_shape_is_opaque() {
        let m = FormattedShapes::default();
        let closed = metrique::CloseValue::close(m);
        let entry = metrique::RootEntry::new(closed);

        // Formatted fields fall back to Opaque because the raw type may not impl Value.
        // ValueFormatter::SHAPE is available for sinks that want to query it separately.
        assert_field_shape(&entry, 0, FieldShape::Opaque);
    }
}

// ─── Write order vs descriptor segment order ────────────────────────────────
//
// The `Entry::write` order contract (docs/entry-descriptors.md) requires that
// walking `descriptors()` segments in sequence yields fields in exactly the
// order `Entry::write` emits `EntryWriter::value` callbacks.

/// Captures the order of `EntryWriter::value` callbacks.
#[derive(Default)]
struct ValueOrderWriter {
    names: Vec<String>,
}

impl<'a> metrique::writer::EntryWriter<'a> for ValueOrderWriter {
    fn timestamp(&mut self, _timestamp: SystemTime) {}

    fn value(
        &mut self,
        name: impl Into<std::borrow::Cow<'a, str>>,
        _value: &(impl metrique_writer_core::Value + ?Sized),
    ) {
        self.names.push(name.into().into_owned());
    }

    fn config(&mut self, _config: &'a dyn metrique_writer_core::EntryConfig) {}
}

/// Field names in the order `Entry::write` emits them.
fn write_order(entry: &impl Entry) -> Vec<String> {
    let mut writer = ValueOrderWriter::default();
    entry.write(&mut writer);
    writer.names
}

/// Field names in the order the descriptor segments list them.
fn descriptor_order(entry: &impl Entry) -> Vec<String> {
    entry
        .descriptors()
        .unwrap()
        .iter()
        .flat_map(|seg| {
            seg.fields()
                .map(|f| f.name_parts().collect::<String>())
                .collect::<Vec<_>>()
        })
        .collect()
}

#[metrics(subfield, rename_all = "PascalCase")]
pub struct OrderChild {
    child_value: u64,
}

#[metrics(rename_all = "PascalCase")]
struct FlattenFirstParent {
    #[metrics(flatten)]
    child: OrderChild,
    own_field: u64,
}

#[test]
fn flatten_first_descriptor_order_matches_write_order() {
    let m = FlattenFirstParent {
        child: OrderChild { child_value: 1 },
        own_field: 2,
    };
    let closed = metrique::CloseValue::close(m);
    let entry = metrique::RootEntry::new(closed);

    assert_eq!(write_order(&entry), vec!["ChildValue", "OwnField"]);
    assert_eq!(descriptor_order(&entry), write_order(&entry));
}

#[metrics(subfield, rename_all = "PascalCase")]
struct SecondOrderChild {
    other_value: u64,
}

#[metrics(rename_all = "PascalCase")]
struct InterleavedParent {
    alpha: u64,
    #[metrics(flatten, prefix = "first_")]
    first: OrderChild,
    beta: u64,
    #[metrics(flatten, prefix = "second_")]
    second: SecondOrderChild,
    gamma: u64,
}

#[test]
fn interleaved_flatten_descriptor_order_matches_write_order() {
    let m = InterleavedParent {
        alpha: 1,
        first: OrderChild { child_value: 2 },
        beta: 3,
        second: SecondOrderChild { other_value: 4 },
        gamma: 5,
    };
    let closed = metrique::CloseValue::close(m);
    let entry = metrique::RootEntry::new(closed);

    assert_eq!(
        write_order(&entry),
        vec![
            "Alpha",
            "FirstChildValue",
            "Beta",
            "SecondOtherValue",
            "Gamma"
        ]
    );

    assert_eq!(descriptor_order(&entry), write_order(&entry));
}

#[metrics(rename_all = "PascalCase", tag(name = "operation"))]
enum OrderEnum {
    FlattenFirst {
        #[metrics(flatten)]
        child: OrderChild,
        own_field: u64,
    },
}

#[test]
fn enum_flatten_first_descriptor_order_matches_write_order() {
    let m = OrderEnum::FlattenFirst {
        child: OrderChild { child_value: 1 },
        own_field: 2,
    };
    let closed = metrique::CloseValue::close(m);
    let entry = metrique::RootEntry::new(closed);

    // Tag writes first, then fields in declaration order.
    assert_eq!(
        write_order(&entry),
        vec!["Operation", "ChildValue", "OwnField"]
    );

    assert_eq!(descriptor_order(&entry), write_order(&entry));
}

#[test]
fn flatten_first_leading_parent_segment_is_empty() {
    let m = FlattenFirstParent {
        child: OrderChild { child_value: 1 },
        own_field: 2,
    };
    let closed = metrique::CloseValue::close(m);
    let entry = metrique::RootEntry::new(closed);
    let descriptors = entry.descriptors().unwrap();

    // The first segment always belongs to the parent so its canonical name
    // stays discoverable; it consumes zero values.
    assert_eq!(descriptors.len(), 3);
    assert_eq!(descriptors[0].name(), "FlattenFirstParent");
    assert_eq!(descriptors[0].fields_len(), 0);
    assert_eq!(descriptors[1].name(), "OrderChild");
    assert_eq!(descriptors[2].name(), "FlattenFirstParent");
    assert_eq!(descriptors[2].fields_len(), 1);
}

#[metrics(rename_all = "PascalCase")]
struct TimestampAfterFlatten {
    #[metrics(flatten)]
    child: OrderChild,
    #[metrics(timestamp)]
    start: SystemTime,
    own_field: u64,
}

#[test]
fn timestamp_after_flatten_lives_on_first_segment() {
    let m = TimestampAfterFlatten {
        child: OrderChild { child_value: 1 },
        start: SystemTime::UNIX_EPOCH,
        own_field: 2,
    };
    let closed = metrique::CloseValue::close(m);
    let entry = metrique::RootEntry::new(closed);
    let descriptors = entry.descriptors().unwrap();

    // The timestamp has no position in the value stream, so it lives on the
    // first segment regardless of where the field is declared.
    assert_eq!(descriptors[0].name(), "TimestampAfterFlatten");
    assert_eq!(descriptors[0].timestamp().unwrap().name(), "start");
    assert!(descriptors[2].timestamp().is_none());

    assert_eq!(descriptor_order(&entry), write_order(&entry));
}

#[metrics(rename_all = "PascalCase")]
struct CfgSplitParent {
    before: u64,
    #[cfg(feature = "__metrique_nonexistent_feature")]
    #[metrics(flatten)]
    never: OrderChild,
    after: u64,
}

#[test]
fn cfg_disabled_flatten_still_splits_own_field_runs() {
    let m = CfgSplitParent {
        before: 1,
        after: 2,
    };
    let closed = metrique::CloseValue::close(m);
    let entry = metrique::RootEntry::new(closed);
    let descriptors = entry.descriptors().unwrap();

    // Runs split at the flatten site even though it is compiled out; two
    // adjacent parent segments still satisfy the order contract.
    assert_eq!(descriptors.len(), 2);
    assert_eq!(
        descriptors[0].fields().next().unwrap().base_name(),
        "Before"
    );
    assert_eq!(descriptors[1].fields().next().unwrap().base_name(), "After");

    assert_eq!(descriptor_order(&entry), write_order(&entry));
}

#[metrics(subfield, rename_all = "PascalCase")]
pub struct FlaggedOrderChild {
    flagged_value: u64,
}

#[metrics(rename_all = "PascalCase")]
enum EnumFlattenFlags {
    Variant {
        #[metrics(flatten, default_flags(AuditExport))]
        child: FlaggedOrderChild,
    },
}

#[test]
fn enum_flatten_default_flags_applied_to_child_descriptor() {
    let m = EnumFlattenFlags::Variant {
        child: FlaggedOrderChild { flagged_value: 1 },
    };
    let closed = metrique::CloseValue::close(m);
    let entry = metrique::RootEntry::new(closed);
    let descriptors = entry.descriptors().unwrap();

    let child_fields: Vec<_> = descriptors[1].fields().collect();
    let flags: Vec<_> = child_fields[0].flags().collect();
    assert_eq!(flags.len(), 1);
    assert_eq!(flags[0].type_id(), TypeId::of::<AuditExport>());
}

#[metrics(rename_all = "PascalCase")]
struct CfgPlainFieldParent {
    before: u64,
    #[cfg(feature = "__metrique_nonexistent_feature")]
    never_written: u64,
    #[cfg(test)]
    test_only: u64,
    after: u64,
}

#[test]
fn cfg_disabled_plain_field_excluded_from_descriptor() {
    let m = CfgPlainFieldParent {
        before: 1,
        test_only: 2,
        after: 3,
    };
    let closed = metrique::CloseValue::close(m);
    let entry = metrique::RootEntry::new(closed);

    assert_eq!(write_order(&entry), vec!["Before", "TestOnly", "After"]);

    // FIXME: inverted assertion documenting the current bug. The descriptor
    // lists cfg-disabled plain fields even though write() never emits them.
    // Flip to assert_eq! with the fix.
    assert_ne!(descriptor_order(&entry), write_order(&entry));
}
