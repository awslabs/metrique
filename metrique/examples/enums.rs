// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Enum examples showing both value(string) and entry enums.

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
pub struct ReadMetrics {
    bytes_read: usize,
    cache_hit: bool,
}

#[metrics(subfield)]
pub struct WriteMetrics {
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

// Simulate a storage layer that returns operation-specific metrics
struct StorageLayer;

impl StorageLayer {
    fn read(&self, key: &str) -> (Vec<u8>, ReadMetrics) {
        let cache_hit = key.len() % 2 == 0;
        let data = vec![0u8; 1024];

        let metrics = ReadMetrics {
            bytes_read: data.len(),
            cache_hit,
        };

        (data, metrics)
    }

    fn write(&self, data: &[u8]) -> WriteMetrics {
        WriteMetrics {
            bytes_written: data.len(),
            fsync_required: data.len() > 4096,
        }
    }

    fn delete(&self, keys: &[String]) -> usize {
        keys.len()
    }
}

// Application layer that wraps storage operations with metrics
fn handle_read_request(storage: &StorageLayer, key: &str, priority: Priority) -> RequestMetrics {
    let (data, read_metrics) = storage.read(key);

    RequestMetrics {
        priority,
        details: OperationMetrics::Read(read_metrics),
        success: !data.is_empty(),
        request_id: format!("read-{}", key),
    }
}

fn handle_write_request(
    storage: &StorageLayer,
    data: Vec<u8>,
    priority: Priority,
) -> RequestMetrics {
    let write_metrics = storage.write(&data);

    RequestMetrics {
        priority,
        details: OperationMetrics::Write(write_metrics),
        success: true,
        request_id: format!("write-{}", data.len()),
    }
}

fn handle_delete_request(
    storage: &StorageLayer,
    keys: Vec<String>,
    priority: Priority,
) -> RequestMetrics {
    let key_count = storage.delete(&keys);

    RequestMetrics {
        priority,
        details: OperationMetrics::Delete { key_count },
        success: key_count > 0,
        request_id: format!("delete-{}", key_count),
    }
}

fn main() {
    let storage = StorageLayer;

    // Read operation - metrics constructed by storage layer
    let read_metrics = handle_read_request(&storage, "user:123", Priority::High);
    let read_entry = test_metric(read_metrics);

    // Write operation - metrics constructed by storage layer
    let write_metrics = handle_write_request(&storage, vec![0u8; 2048], Priority::Medium);
    let write_entry = test_metric(write_metrics);

    // Delete operation - metrics constructed inline
    let delete_metrics = handle_delete_request(
        &storage,
        vec!["key1".to_string(), "key2".to_string()],
        Priority::Low,
    );
    let delete_entry = test_metric(delete_metrics);

    // Example output for Read operation:
    // Values: { "Priority": "High", "Operation": "Read" }
    // Metrics: { "Success": 1, "BytesRead": 1024, "CacheHit": 1 }

    assert_eq!(read_entry.values["Priority"], "High");
    assert_eq!(read_entry.values["Operation"], "Read");
    assert_eq!(read_entry.metrics["Success"], 1);
    assert_eq!(read_entry.metrics["BytesRead"], 1024);

    // Example output for Write operation:
    // Values: { "Priority": "Medium", "Operation": "Put" }
    // Metrics: { "Success": 1, "BytesWritten": 2048, "FsyncRequired": 0 }

    assert_eq!(write_entry.values["Priority"], "Medium");
    assert_eq!(write_entry.values["Operation"], "Put"); // Note: renamed via #[metrics(name = "Put")]
    assert_eq!(write_entry.metrics["Success"], 1);
    assert_eq!(write_entry.metrics["BytesWritten"], 2048);

    // Example output for Delete operation:
    // Values: { "Priority": "Low", "Operation": "Delete" }
    // Metrics: { "Success": 1, "KeyCount": 2 }

    assert_eq!(delete_entry.values["Priority"], "Low");
    assert_eq!(delete_entry.values["Operation"], "Delete");
    assert_eq!(delete_entry.metrics["Success"], 1);
    assert_eq!(delete_entry.metrics["KeyCount"], 2);
}
