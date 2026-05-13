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
    assert_eq!(ts.base_name(), "start");
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
    let fields = desc.fields();

    let audit_id = TypeId::of::<AuditExport>();

    // request_id: inherits default_field_tag(AuditExport) -> Present
    let request_id_tags = fields[0].tags();
    assert_eq!(request_id_tags.len(), 1);
    assert_eq!(request_id_tags[0].tag_id(), audit_id);
    assert_eq!(request_id_tags[0].state(), FieldTagState::Present);

    // operation: inherits default_field_tag(AuditExport) -> Present
    let op_tags = fields[1].tags();
    assert_eq!(op_tags.len(), 1);
    assert_eq!(op_tags[0].tag_id(), audit_id);
    assert_eq!(op_tags[0].state(), FieldTagState::Present);

    // debug_blob: field_tag(skip(AuditExport)) overrides default -> Absent
    let debug_tags = fields[2].tags();
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
    let fields = desc.fields();

    let audit_id = TypeId::of::<AuditExport>();
    let dial9_id = TypeId::of::<Dial9Emit>();

    // important: both tags present
    assert_eq!(fields[0].tags().count(), 2);
    assert!(
        fields[0]
            .tags()
            .iter()
            .any(|t| t.tag_id() == audit_id && t.state() == FieldTagState::Present)
    );
    assert!(
        fields[0]
            .tags()
            .iter()
            .any(|t| t.tag_id() == dial9_id && t.state() == FieldTagState::Present)
    );

    // trace_only: only Dial9Emit
    assert_eq!(fields[1].tags().count(), 1);
    assert_eq!(fields[1].tags().collect::<Vec<_>>()[0].tag_id(), dial9_id);

    // untagged: no tags
    assert!(fields[2].tags().is_empty());
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
    let sub_value_tags = child_desc.fields().collect::<Vec<_>>()[0].tags();
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
    let alpha_tags = child_desc.fields().collect::<Vec<_>>()[0].tags();
    assert_eq!(alpha_tags.len(), 1);
    assert_eq!(alpha_tags[0].tag_id(), dial9_id);
    assert_eq!(alpha_tags[0].state(), FieldTagState::Present);

    // beta has field_tag(skip(Dial9Emit)) -> Absent
    let beta_tags = child_desc.fields().collect::<Vec<_>>()[1].tags();
    assert_eq!(beta_tags.len(), 1);
    assert_eq!(beta_tags[0].tag_id(), dial9_id);
    assert_eq!(beta_tags[0].state(), FieldTagState::Absent);
}
