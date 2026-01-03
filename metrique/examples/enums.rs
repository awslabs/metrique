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

// Value enum - represents operation type as a string
#[metrics(value(string))]
#[derive(Copy, Clone)]
enum OperationType {
    Get,
    Put,
    Delete,
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
#[metrics]
enum OperationMetrics {
    Read(#[metrics(flatten)] ReadMetrics),
    Write(#[metrics(flatten)] WriteMetrics),
    Delete { key_count: usize },
}

#[metrics(rename_all = "PascalCase")]
struct RequestMetrics {
    #[metrics(sample_group)]
    operation: OperationType,

    #[metrics(flatten)]
    details: OperationMetrics,

    success: bool,
}

fn main() {
    // Get operation
    let get_entry = test_metric(RequestMetrics {
        operation: OperationType::Get,
        details: OperationMetrics::Read(ReadMetrics {
            bytes_read: 1024,
            cache_hit: true,
        }),
        success: true,
    });

    // Put operation
    let put_entry = test_metric(RequestMetrics {
        operation: OperationType::Put,
        details: OperationMetrics::Write(WriteMetrics {
            bytes_written: 2048,
            fsync_required: false,
        }),
        success: true,
    });

    // Delete operation (struct variant)
    let delete_entry = test_metric(RequestMetrics {
        operation: OperationType::Delete,
        details: OperationMetrics::Delete { key_count: 5 },
        success: true,
    });

    // Example output for Get operation:
    // Values: { "Operation": "Get" }
    // Metrics: { "Success": 1, "BytesRead": 1024, "CacheHit": 1 }

    assert_eq!(get_entry.values["Operation"], "Get");
    assert_eq!(get_entry.metrics["Success"].as_u64(), 1);
    assert_eq!(get_entry.metrics["BytesRead"].as_u64(), 1024);
    assert_eq!(get_entry.metrics["CacheHit"].as_u64(), 1);

    // Example output for Put operation:
    // Values: { "Operation": "Put" }
    // Metrics: { "Success": 1, "BytesWritten": 2048, "FsyncRequired": 0 }

    assert_eq!(put_entry.values["Operation"], "Put");
    assert_eq!(put_entry.metrics["Success"].as_u64(), 1);
    assert_eq!(put_entry.metrics["BytesWritten"].as_u64(), 2048);
    assert_eq!(put_entry.metrics["FsyncRequired"].as_u64(), 0);

    // Example output for Delete operation:
    // Values: { "Operation": "Delete" }
    // Metrics: { "Success": 1, "KeyCount": 5 }

    assert_eq!(delete_entry.values["Operation"], "Delete");
    assert_eq!(delete_entry.metrics["Success"].as_u64(), 1);
    assert_eq!(delete_entry.metrics["KeyCount"].as_u64(), 5);

    println!("✓ All assertions passed - enum metrics work correctly!");
}
