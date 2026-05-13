// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Integration tests for entry descriptors and field tags.

use metrique::unit_of_work::metrics;
use metrique::writer::Entry;
use metrique_writer_core::FieldTagState;
use std::any::TypeId;
use std::time::SystemTime;

// Tag marker types for testing
struct AuditExport;
struct Dial9Emit;

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
    let desc_ref = entry.descriptors().next().expect("should have descriptor");
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
    let desc_ref = entry.descriptors().next().unwrap();
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
    let desc = entry.descriptors().next().unwrap();
    let field = &desc.fields().collect::<Vec<_>>()[0];

    assert_eq!(field.base_name(), "Latency");
    assert!(field.unit().is_some());
}

#[metrics(rename_all = "PascalCase", default_field_tag(AuditExport))]
struct TaggedMetrics {
    request_id: String,
    operation: &'static str,
    #[metrics(field_tag(skip(AuditExport)))]
    debug_blob: String,
}

#[test]
fn tag_resolution_default_and_skip() {
    let m = TaggedMetrics {
        request_id: String::new(),
        operation: "test",
        debug_blob: String::new(),
    };
    let closed = metrique::CloseValue::close(m);
    let entry = metrique::RootEntry::new(closed);
    let desc = entry.descriptors().next().unwrap();
    let fields: Vec<_> = desc.fields().collect();

    let audit_id = TypeId::of::<AuditExport>();

    // request_id: inherits default_field_tag(AuditExport) -> Present
    let request_id_tags = fields[0].tags().collect::<Vec<_>>();
    assert_eq!(request_id_tags.len(), 1);
    assert_eq!(request_id_tags[0].tag_id(), audit_id);
    assert_eq!(request_id_tags[0].state(), FieldTagState::Present);

    // operation: inherits default_field_tag(AuditExport) -> Present
    let op_tags = fields[1].tags().collect::<Vec<_>>();
    assert_eq!(op_tags.len(), 1);
    assert_eq!(op_tags[0].tag_id(), audit_id);
    assert_eq!(op_tags[0].state(), FieldTagState::Present);

    // debug_blob: field_tag(skip(AuditExport)) overrides default -> Absent
    let debug_tags = fields[2].tags().collect::<Vec<_>>();
    assert_eq!(debug_tags.len(), 1);
    assert_eq!(debug_tags[0].tag_id(), audit_id);
    assert_eq!(debug_tags[0].state(), FieldTagState::Absent);
}

#[metrics(rename_all = "PascalCase")]
struct MultiTagMetrics {
    #[metrics(field_tag(AuditExport), field_tag(Dial9Emit))]
    important: u64,
    #[metrics(field_tag(Dial9Emit))]
    trace_only: u64,
    untagged: u64,
}

#[test]
fn multiple_tags_on_field() {
    let m = MultiTagMetrics {
        important: 1,
        trace_only: 2,
        untagged: 3,
    };
    let closed = metrique::CloseValue::close(m);
    let entry = metrique::RootEntry::new(closed);
    let desc = entry.descriptors().next().unwrap();
    let fields: Vec<_> = desc.fields().collect();

    let audit_id = TypeId::of::<AuditExport>();
    let dial9_id = TypeId::of::<Dial9Emit>();

    // important: both tags present
    assert_eq!(fields[0].tags().count(), 2);
    assert!(
        fields[0]
            .tags()
            .any(|t| t.tag_id() == audit_id && t.state() == FieldTagState::Present)
    );
    assert!(
        fields[0]
            .tags()
            .any(|t| t.tag_id() == dial9_id && t.state() == FieldTagState::Present)
    );

    // trace_only: only Dial9Emit
    assert_eq!(fields[1].tags().count(), 1);
    assert_eq!(fields[1].tags().collect::<Vec<_>>()[0].tag_id(), dial9_id);

    // untagged: no tags
    assert!(fields[2].tags().next().is_none());
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
    let desc = entry.descriptors().next().unwrap();

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

    let id1 = e1.descriptors().next().unwrap().id();
    let id2 = e2.descriptors().next().unwrap().id();
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

    let desc = boxed
        .descriptors()
        .next()
        .expect("BoxEntry should forward descriptor");
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
    let desc = entry.descriptors().next().unwrap();

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
    let desc = entry.descriptors().next().unwrap();

    assert_eq!(
        desc.fields().collect::<Vec<_>>()[0].base_name(),
        "ApiLatency"
    );
}

#[metrics(rename_all = "PascalCase", subfield)]
struct SubMetrics {
    #[metrics(field_tag(AuditExport))]
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

    let descriptors: Vec<_> = entry.descriptors().collect();
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
    let sub_value_tags = child_desc.fields().collect::<Vec<_>>()[0]
        .tags()
        .collect::<Vec<_>>();
    assert_eq!(sub_value_tags.len(), 1);
    assert_eq!(sub_value_tags[0].tag_id(), TypeId::of::<AuditExport>());
    assert_eq!(sub_value_tags[0].state(), FieldTagState::Present);
}

#[metrics(rename_all = "PascalCase", subfield)]
struct TaggedSubMetrics {
    #[metrics(field_tag(Dial9Emit))]
    alpha: u64,
    #[metrics(field_tag(skip(Dial9Emit)))]
    beta: u64,
}

#[metrics(rename_all = "PascalCase")]
struct ParentWithTaggedFlatten {
    top: u64,
    #[metrics(flatten)]
    inner: TaggedSubMetrics,
}

#[test]
fn flatten_child_default_field_tag_resolved() {
    let m = ParentWithTaggedFlatten {
        top: 1,
        inner: TaggedSubMetrics { alpha: 2, beta: 3 },
    };
    let closed = metrique::CloseValue::close(m);
    let entry = metrique::RootEntry::new(closed);

    let descriptors: Vec<_> = entry.descriptors().collect();
    assert_eq!(descriptors.len(), 2);

    let child_desc = &descriptors[1];
    let dial9_id = TypeId::of::<Dial9Emit>();

    // alpha inherits default_field_tag(Dial9Emit) -> Present
    let alpha_tags = child_desc.fields().collect::<Vec<_>>()[0]
        .tags()
        .collect::<Vec<_>>();
    assert_eq!(alpha_tags.len(), 1);
    assert_eq!(alpha_tags[0].tag_id(), dial9_id);
    assert_eq!(alpha_tags[0].state(), FieldTagState::Present);

    // beta has field_tag(skip(Dial9Emit)) -> Absent
    let beta_tags = child_desc.fields().collect::<Vec<_>>()[1]
        .tags()
        .collect::<Vec<_>>();
    assert_eq!(beta_tags.len(), 1);
    assert_eq!(beta_tags[0].tag_id(), dial9_id);
    assert_eq!(beta_tags[0].state(), FieldTagState::Absent);
}

// ─── Multilayer flatten tests ───────────────────────────────────────────────

#[metrics(subfield)]
struct GrandChild {
    #[metrics(field_tag(AuditExport))]
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

    let descriptors: Vec<_> = entry.descriptors().collect();
    // Parent's own fields + middle's descriptor + grandchild's descriptor
    assert!(descriptors.len() >= 2);

    // Parent's own descriptor
    let parent_fields: Vec<_> = descriptors[0].fields().collect();
    assert_eq!(parent_fields[0].base_name(), "TopValue");

    // Middle child's descriptor (with parent's flatten prefix "Mid" applied)
    let middle_fields: Vec<_> = descriptors[1].fields().collect();
    let mid_parts: Vec<&str> = middle_fields[0].name_parts().collect();
    assert_eq!(mid_parts, vec!["Mid", "MiddleValue"]);
}

#[metrics(subfield, default_field_tag(Dial9Emit))]
struct TaggedChild {
    emitted: u64,
    #[metrics(field_tag(skip(Dial9Emit)))]
    skipped: u64,
}

#[metrics(rename_all = "PascalCase", default_field_tag(AuditExport))]
struct TagPropagationParent {
    own_field: u64,
    #[metrics(flatten)]
    child: TaggedChild,
}

#[test]
fn flatten_tag_propagation_with_parent_default() {
    let m = TagPropagationParent {
        own_field: 1,
        child: TaggedChild {
            emitted: 2,
            skipped: 3,
        },
    };
    let closed = metrique::CloseValue::close(m);
    let entry = metrique::RootEntry::new(closed);

    let descriptors: Vec<_> = entry.descriptors().collect();
    assert_eq!(descriptors.len(), 2);

    // Parent's own field has AuditExport (from default_field_tag)
    let parent_fields: Vec<_> = descriptors[0].fields().collect();
    let parent_tags: Vec<_> = parent_fields[0].tags().collect();
    assert!(
        parent_tags
            .iter()
            .any(|t| t.tag_id() == TypeId::of::<AuditExport>()
                && t.state() == FieldTagState::Present)
    );

    // Child's "emitted" field: has Dial9Emit (child's own default), plus AuditExport from parent default propagation
    let child_fields: Vec<_> = descriptors[1].fields().collect();
    let emitted_tags: Vec<_> = child_fields[0].tags().collect();
    // Child's own Dial9Emit is present (baked)
    assert!(emitted_tags.iter().any(|t| t.tag_id() == TypeId::of::<Dial9Emit>() && t.state() == FieldTagState::Present));
    // Parent's AuditExport propagates as default (fills gap)
    assert!(
        emitted_tags
            .iter()
            .any(|t| t.tag_id() == TypeId::of::<AuditExport>()
                && t.state() == FieldTagState::Present)
    );

    // Child's "skipped" field: has skip(Dial9Emit) (baked), plus AuditExport from parent default
    let skipped_tags: Vec<_> = child_fields[1].tags().collect();
    assert!(
        skipped_tags
            .iter()
            .any(|t| t.tag_id() == TypeId::of::<Dial9Emit>() && t.state() == FieldTagState::Absent)
    );
    assert!(
        skipped_tags
            .iter()
            .any(|t| t.tag_id() == TypeId::of::<AuditExport>()
                && t.state() == FieldTagState::Present)
    );
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

    let descriptors: Vec<_> = entry.descriptors().collect();
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

    let descriptors: Vec<_> = entry.descriptors().collect();
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
    let descriptors: Vec<_> = entry.descriptors().collect();
    assert_eq!(descriptors.len(), 1);
    assert_eq!(descriptors[0].fields_len(), 0);
}

#[metrics(subfield)]
struct EnumChild {
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
        child: EnumChild,
    },
}

#[test]
fn enum_variant_with_flatten_chains_child_descriptor() {
    use metrique::writer::Entry;

    let m = EnumWithFlatten::WithChild {
        count: 1,
        child: EnumChild { child_val: 2 },
    };
    let closed = metrique::CloseValue::close(m);
    let entry = metrique::RootEntry::new(closed);

    let descriptors: Vec<_> = entry.descriptors().collect();
    // Base descriptor (union of non-flatten fields) + child's descriptor
    assert!(
        descriptors.len() >= 2,
        "expected base + child, got {}",
        descriptors.len()
    );

    // Child's descriptor has its field
    let child_fields: Vec<_> = descriptors[1].fields().collect();
    assert_eq!(child_fields[0].base_name(), "child_val");
}

#[test]
fn enum_variant_without_flatten_yields_one_descriptor() {
    use metrique::writer::Entry;

    let m = EnumWithFlatten::Simple { count: 1 };
    let closed = metrique::CloseValue::close(m);
    let entry = metrique::RootEntry::new(closed);

    let descriptors: Vec<_> = entry.descriptors().collect();
    // Only the base descriptor, no flatten children
    assert_eq!(descriptors.len(), 1);
}

#[test]
fn enum_variants_have_different_descriptor_ids() {
    use metrique::writer::Entry;

    let simple = EnumWithFlatten::Simple { count: 1 };
    let with_child = EnumWithFlatten::WithChild {
        count: 1,
        child: EnumChild { child_val: 2 },
    };

    let closed_simple = metrique::CloseValue::close(simple);
    let closed_child = metrique::CloseValue::close(with_child);
    let entry_simple = metrique::RootEntry::new(closed_simple);
    let entry_child = metrique::RootEntry::new(closed_child);

    let descs_simple: Vec<_> = entry_simple.descriptors().collect();
    let descs_child: Vec<_> = entry_child.descriptors().collect();

    // Different variants produce different base descriptor ids
    // (each variant has its own static with only its fields)
    assert_ne!(descs_simple[0].id(), descs_child[0].id());

    // Each variant's descriptor name includes the variant
    assert!(descs_simple[0].name().contains("Simple"));
    assert!(descs_child[0].name().contains("WithChild"));
}

#[metrics(subfield)]
struct OrderChildA {
    a_val: u64,
}
#[metrics(subfield)]
struct OrderChildB {
    b_val: u64,
}
#[metrics(subfield)]
struct OrderChildC {
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
    let descriptors: Vec<_> = entry.descriptors().collect();
    assert_eq!(descriptors.len(), 4);
    let d1: Vec<_> = descriptors[1].fields().collect();
    let d2: Vec<_> = descriptors[2].fields().collect();
    let d3: Vec<_> = descriptors[3].fields().collect();
    assert_eq!(d1[0].base_name(), "a_val");
    assert_eq!(d2[0].base_name(), "b_val");
    assert_eq!(d3[0].base_name(), "c_val");
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
    use metrique::writer::Entry;

    let m = EnumFieldOrder::Multi {
        alpha: 1,
        beta: 2,
        gamma: 3,
        child: OrderChildA { a_val: 4 },
        delta: 5,
    };
    let closed = metrique::CloseValue::close(m);
    let entry = metrique::RootEntry::new(closed);
    let descriptors: Vec<_> = entry.descriptors().collect();

    // Base descriptor has non-flatten fields in declaration order
    let base_fields: Vec<_> = descriptors[0].fields().collect();
    assert_eq!(base_fields[0].base_name(), "Alpha");
    assert_eq!(base_fields[1].base_name(), "Beta");
    assert_eq!(base_fields[2].base_name(), "Gamma");
    assert_eq!(base_fields[3].base_name(), "Delta");

    // Flatten child comes after base
    assert_eq!(descriptors.len(), 2);
    let child_fields: Vec<_> = descriptors[1].fields().collect();
    assert_eq!(child_fields[0].base_name(), "a_val");
}

#[metrics(rename_all = "PascalCase")]
enum EnumCfgFlatten {
    WithCfg {
        #[metrics(flatten)]
        first: OrderChildA,
        #[cfg(test)]
        #[metrics(flatten)]
        middle: OrderChildB,
        #[metrics(flatten)]
        last: OrderChildC,
    },
}

#[test]
fn enum_cfg_flatten_ordering_preserved() {
    use metrique::writer::Entry;

    let m = EnumCfgFlatten::WithCfg {
        first: OrderChildA { a_val: 1 },
        middle: OrderChildB { b_val: 2 },
        last: OrderChildC { c_val: 3 },
    };
    let closed = metrique::CloseValue::close(m);
    let entry = metrique::RootEntry::new(closed);
    let descriptors: Vec<_> = entry.descriptors().collect();
    // base (0 fields) + first + middle + last
    assert_eq!(descriptors.len(), 4);
    let d1: Vec<_> = descriptors[1].fields().collect();
    let d2: Vec<_> = descriptors[2].fields().collect();
    let d3: Vec<_> = descriptors[3].fields().collect();
    assert_eq!(d1[0].base_name(), "a_val");
    assert_eq!(d2[0].base_name(), "b_val");
    assert_eq!(d3[0].base_name(), "c_val");
}
