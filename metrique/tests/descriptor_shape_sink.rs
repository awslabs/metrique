// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! E2E test: a descriptor-aware sink that uses shapes to select encoding strategies.
//!
//! Demonstrates the intended consumption pattern where a sink pre-registers a schema
//! from descriptor metadata (shapes, names, units, flags) and then uses the write path
//! to fill in values against that schema.

use metrique::timers::Timer;
use metrique::unit_of_work::metrics;
use metrique::writer::Entry;
use metrique_writer_core::Unit;
use metrique_writer_core::descriptor::{FieldShape, KnownShape, ShapeRef};
use metrique_writer_core::value::{FlagConstructor, MetricFlags, MetricOptions};
use std::any::TypeId;
use std::time::Duration;

// A flag marking fields for export to an external system.
#[derive(Debug)]
struct ExportOpt;
impl MetricOptions for ExportOpt {}
struct Export;
impl FlagConstructor for Export {
    fn construct() -> MetricFlags<'static> {
        MetricFlags::upcast(&ExportOpt)
    }
}

/// Simulates a sink's per-entry-type schema, built once from descriptors.
#[derive(Debug)]
struct FieldSchema {
    name: String,
    encoding: Encoding,
    unit: Option<Unit>,
    export: bool,
}

/// The encoding a sink would choose based on shape.
#[derive(Debug, PartialEq)]
enum Encoding {
    IntGauge,
    FloatGauge,
    Attribute,
    NullableInt,
    NullableFloat,
    NullableAttribute,
    IntList,
    Opaque,
}

fn encoding_for_shape(shape: FieldShape<'_>) -> Encoding {
    match shape {
        FieldShape::Known(KnownShape::Bool)
        | FieldShape::Known(KnownShape::U8)
        | FieldShape::Known(KnownShape::U16)
        | FieldShape::Known(KnownShape::U32)
        | FieldShape::Known(KnownShape::U64) => Encoding::IntGauge,
        FieldShape::Known(KnownShape::F32) | FieldShape::Known(KnownShape::F64) => {
            Encoding::FloatGauge
        }
        FieldShape::Known(KnownShape::String) => Encoding::Attribute,
        FieldShape::Optional(inner) => match inner.get() {
            FieldShape::Known(KnownShape::String) => Encoding::NullableAttribute,
            FieldShape::Known(KnownShape::F32) | FieldShape::Known(KnownShape::F64) => {
                Encoding::NullableFloat
            }
            FieldShape::Known(_) => Encoding::NullableInt,
            _ => Encoding::Opaque,
        },
        FieldShape::List(inner) => match inner.get() {
            FieldShape::Known(
                KnownShape::U8
                | KnownShape::U16
                | KnownShape::U32
                | KnownShape::U64
                | KnownShape::Bool,
            ) => Encoding::IntList,
            _ => Encoding::Opaque,
        },
        _ => Encoding::Opaque,
    }
}

/// Build a schema from entry descriptors, the way a real sink would on first encounter.
fn build_schema(entry: &impl Entry) -> Vec<FieldSchema> {
    let export_id = TypeId::of::<Export>();
    let descs = entry.descriptors().unwrap();
    let mut schema = Vec::new();

    for seg in descs.iter() {
        for field in seg.fields() {
            schema.push(FieldSchema {
                name: field.name_parts().collect(),
                encoding: encoding_for_shape(field.shape()),
                unit: field.unit(),
                export: field.flags().any(|f| f.type_id() == export_id),
            });
        }
    }
    schema
}

#[test]
fn shape_driven_schema_registration() {
    #[metrics(rename_all = "PascalCase", default_flags(Export))]
    struct RequestMetrics {
        operation: String,
        request_count: u64,
        latency: Timer,
        success: bool,
        opt_note: Option<String>,
        opt_retry_count: Option<u32>,
        batch_sizes: Vec<u64>,
        #[metrics(flags(skip(Export)))]
        internal_debug: f64,
    }

    let m = RequestMetrics {
        operation: "ListDucks".into(),
        request_count: 1,
        latency: Timer::default(),
        success: true,
        opt_note: Some("ok".into()),
        opt_retry_count: None,
        batch_sizes: vec![10, 20],
        internal_debug: 3.14,
    };

    let closed = metrique::CloseValue::close(m);
    let entry = metrique::RootEntry::new(closed);
    let schema = build_schema(&entry);

    // Verify schema built from shapes matches expected encodings
    assert_eq!(schema.len(), 8);

    assert_eq!(schema[0].name, "Operation");
    assert_eq!(schema[0].encoding, Encoding::Attribute);
    assert!(schema[0].export);

    assert_eq!(schema[1].name, "RequestCount");
    assert_eq!(schema[1].encoding, Encoding::IntGauge);
    assert!(schema[1].export);

    assert_eq!(schema[2].name, "Latency");
    assert_eq!(schema[2].encoding, Encoding::FloatGauge);
    assert_eq!(
        schema[2].unit,
        Some(Unit::Second(
            metrique_writer_core::unit::NegativeScale::Milli
        ))
    );
    assert!(schema[2].export);

    assert_eq!(schema[3].name, "Success");
    assert_eq!(schema[3].encoding, Encoding::IntGauge); // bool encodes as int
    assert!(schema[3].export);

    assert_eq!(schema[4].name, "OptNote");
    assert_eq!(schema[4].encoding, Encoding::NullableAttribute);
    assert!(schema[4].export);

    assert_eq!(schema[5].name, "OptRetryCount");
    assert_eq!(schema[5].encoding, Encoding::NullableInt);
    assert!(schema[5].export);

    assert_eq!(schema[6].name, "BatchSizes");
    assert_eq!(schema[6].encoding, Encoding::IntList);
    assert!(schema[6].export);

    assert_eq!(schema[7].name, "InternalDebug");
    assert_eq!(schema[7].encoding, Encoding::FloatGauge);
    assert!(!schema[7].export); // explicitly skipped
}

#[test]
fn schema_caching_by_descriptor_id() {
    #[metrics(rename_all = "PascalCase")]
    struct CacheableMetrics {
        count: u64,
        name: String,
    }

    // First entry
    let m1 = CacheableMetrics {
        count: 1,
        name: "a".into(),
    };
    let closed1 = metrique::CloseValue::close(m1);
    let entry1 = metrique::RootEntry::new(closed1);

    // Second entry of same type
    let m2 = CacheableMetrics {
        count: 2,
        name: "b".into(),
    };
    let closed2 = metrique::CloseValue::close(m2);
    let entry2 = metrique::RootEntry::new(closed2);

    // Same descriptor ID means schema can be cached and reused
    let descs1 = entry1.descriptors().unwrap();
    let descs2 = entry2.descriptors().unwrap();
    assert_eq!(descs1[0].id(), descs2[0].id());

    // Schema is identical for both
    let schema1 = build_schema(&entry1);
    let schema2 = build_schema(&entry2);
    assert_eq!(schema1.len(), schema2.len());
    for (a, b) in schema1.iter().zip(schema2.iter()) {
        assert_eq!(a.name, b.name);
        assert_eq!(a.encoding, b.encoding);
    }
}
