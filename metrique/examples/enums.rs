// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Enum examples showing both value(string) and entry enums.
//!
//! This example demonstrates:
//! - Value enums with `#[metrics(value(string))]`
//! - Entry enums with tuple and struct variants
//! - Using the test_metric API to inspect emitted metrics

use metrique::test_util::test_metric;
use metrique::unit_of_work::metrics;

// Value enum - represents request priority as a string
#[metrics(value(string))]
#[derive(Copy, Clone)]
enum Priority {
    Low,
    Medium,
    High,
}

#[metrics(subfield)]
struct ReadMetrics {
    bytes_read: usize,
    cache_hit: bool,
}

#[metrics(subfield)]
struct WriteMetrics {
    bytes_written: usize,
    fsync_required: bool,
}

// Entry enum - different fields per operation type
//
// You can optionally inject a "tag" field
// where the value is the variant name (here: "Operation")
// You can also sample on that field with tag(name="Operation", sample_group)
#[metrics(tag(name = "Operation"), subfield)]
enum OperationMetrics {
    Read(#[metrics(flatten)] ReadMetrics),
    // override name in tag field (or sample group)
    #[metrics(name = "Put")]
    Write(#[metrics(flatten)] WriteMetrics),
    Delete {
        key_count: usize,
    },
}

#[metrics(rename_all = "PascalCase")]
struct RequestMetrics {
    #[metrics(sample_group)]
    priority: Priority,

    #[metrics(flatten)]
    details: OperationMetrics,

    success: bool,
    request_id: String,
}

fn main() {
    // Get operation
    let get_entry = test_metric(RequestMetrics {
        priority: Priority::High,
        details: OperationMetrics::Read(ReadMetrics {
            bytes_read: 1024,
            cache_hit: true,
        }),
        success: true,
        request_id: "req-1".to_string(),
    });

    // Put operation
    let put_entry = test_metric(RequestMetrics {
        priority: Priority::Medium,
        details: OperationMetrics::Write(WriteMetrics {
            bytes_written: 2048,
            fsync_required: false,
        }),
        success: true,
        request_id: "req-2".to_string(),
    });

    // Delete operation (struct variant)
    let delete_entry = test_metric(RequestMetrics {
        priority: Priority::Low,
        details: OperationMetrics::Delete { key_count: 5 },
        success: true,
        request_id: "req-3".to_string(),
    });

    // Example output for Get operation:
    // Values: { "Priority": "High" }
    // Metrics: { "Success": 1, "BytesRead": 1024, "CacheHit": 1 }

    assert_eq!(get_entry.values["Priority"], "High");
    assert_eq!(get_entry.metrics["Success"].as_u64(), 1);
    assert_eq!(get_entry.metrics["BytesRead"].as_u64(), 1024);
    assert_eq!(get_entry.metrics["CacheHit"].as_u64(), 1);

    // Example output for Put operation:
    // Values: { "Priority": "Medium" }
    // Metrics: { "Success": 1, "BytesWritten": 2048, "FsyncRequired": 0 }

    assert_eq!(put_entry.values["Priority"], "Medium");
    assert_eq!(put_entry.metrics["Success"].as_u64(), 1);
    assert_eq!(put_entry.metrics["BytesWritten"].as_u64(), 2048);
    assert_eq!(put_entry.metrics["FsyncRequired"].as_u64(), 0);

    // Example output for Delete operation:
    // Values: { "Priority": "Low" }
    // Metrics: { "Success": 1, "KeyCount": 5 }

    assert_eq!(delete_entry.values["Priority"], "Low");
    assert_eq!(delete_entry.metrics["Success"].as_u64(), 1);
    assert_eq!(delete_entry.metrics["KeyCount"].as_u64(), 5);
}
