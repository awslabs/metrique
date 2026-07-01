// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! E2E test demonstrating a descriptor-aware sink that uses flags to filter fields.

use metrique::unit_of_work::metrics;
use metrique::writer::Entry;
use metrique_writer_core::value::{FlagConstructor, MetricFlags, MetricOptions};
use std::any::TypeId;

// Flag marker types
#[derive(Debug)]
struct AuditExportOpt;
impl MetricOptions for AuditExportOpt {}
struct AuditExport;
impl FlagConstructor for AuditExport {
    fn construct() -> MetricFlags<'static> {
        MetricFlags::upcast(&AuditExportOpt)
    }
}

/// Demonstrates how a real sink would use descriptors to decide what to emit.
/// The sink inspects each field's flags and only "exports" fields marked with AuditExport.
#[test]
fn descriptor_aware_sink_filters_by_flag() {
    #[metrics(subfield)]
    struct LibMetrics {
        public_latency: u64,
        internal_counter: u64,
    }

    #[metrics(rename_all = "PascalCase", default_flags(AuditExport))]
    struct AuditedMetrics {
        request_count: u64,
        #[metrics(flags(skip(AuditExport)))]
        debug_info: u64,
    }

    #[metrics(rename_all = "PascalCase")]
    struct AppMetrics {
        #[metrics(flatten, default_flags(AuditExport))]
        lib: LibMetrics,
        #[metrics(flatten)]
        audited: AuditedMetrics,
        #[metrics(flags(AuditExport))]
        top_level: u64,
        untagged: u64,
    }

    let m = AppMetrics {
        lib: LibMetrics {
            public_latency: 100,
            internal_counter: 42,
        },
        audited: AuditedMetrics {
            request_count: 7,
            debug_info: 999,
        },
        top_level: 1,
        untagged: 2,
    };

    let closed = metrique::CloseValue::close(m);
    let entry = metrique::RootEntry::new(closed);
    let descs = entry.descriptors().unwrap();

    // Simulate a sink that only exports fields with AuditExport
    let audit_type_id = TypeId::of::<AuditExport>();
    let mut auditable_fields: Vec<String> = Vec::new();

    for seg in descs.iter() {
        for field in seg.fields() {
            if field.flags().any(|f| f.type_id() == audit_type_id) {
                let name: String = field.name_parts().collect();
                auditable_fields.push(name);
            }
        }
    }

    auditable_fields.sort();

    // Expected auditable fields:
    // - lib.public_latency and lib.internal_counter (from flatten-site default_flags)
    // - audited.request_count (from struct-level default_flags, NOT debug_info which is skipped)
    // - top_level (explicit field-level flag)
    // NOT: untagged (no flag), debug_info (explicitly skipped)
    assert_eq!(
        auditable_fields,
        vec![
            "InternalCounter",
            "PublicLatency",
            "RequestCount",
            "TopLevel"
        ],
        "only AuditExport-flagged fields should be auditable"
    );
}
