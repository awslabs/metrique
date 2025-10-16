// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Tests for flatten with prefix functionality

use std::time::{Duration, SystemTime};
use metrique::unit_of_work::metrics;
use metrique::writer::test_util;

#[metrics(subfield)]
struct CoreMetrics {
    processing_time: Duration,
    validation_time: Duration,
}

#[metrics]
struct PrefixedMetrics {
    #[metrics(timestamp)]
    timestamp: SystemTime,

    #[metrics(flatten, prefix = "api_")]
    core: CoreMetrics,

    request_count: usize,
}

#[test]
fn test_flatten_with_prefix() {
    let test_util::TestEntrySink { inspector, sink } = test_util::test_entry_sink();

    let metrics = PrefixedMetrics {
        timestamp: SystemTime::now(),
        core: CoreMetrics {
            processing_time: Duration::from_millis(100),
            validation_time: Duration::from_millis(25),
        },
        request_count: 5,
    }
    .append_on_drop(sink);

    drop(metrics);

    let entries = inspector.entries();
    assert_eq!(entries.len(), 1);

    let entry = &entries[0];

    // Check that the flattened fields have the prefix
    assert_eq!(entry.metrics["api_processing_time"].as_u64(), 100);
    assert_eq!(entry.metrics["api_validation_time"].as_u64(), 25);
    
    // Check that non-flattened fields don't have the prefix
    assert_eq!(entry.metrics["request_count"].as_u64(), 5);
}

#[metrics(subfield)]
struct NestedCore {
    cpu_time: Duration,
    memory_usage: usize,
}

#[metrics]
struct MultiPrefixMetrics {
    #[metrics(timestamp)]
    timestamp: SystemTime,

    #[metrics(flatten, prefix = "worker_")]
    worker_stats: NestedCore,

    #[metrics(flatten, prefix = "db_")]
    db_stats: NestedCore,

    total_operations: usize,
}

#[test]
fn test_multiple_prefixes() {
    let test_util::TestEntrySink { inspector, sink } = test_util::test_entry_sink();

    let metrics = MultiPrefixMetrics {
        timestamp: SystemTime::now(),
        worker_stats: NestedCore {
            cpu_time: Duration::from_millis(200),
            memory_usage: 1024,
        },
        db_stats: NestedCore {
            cpu_time: Duration::from_millis(50),
            memory_usage: 512,
        },
        total_operations: 10,
    }
    .append_on_drop(sink);

    drop(metrics);

    let entries = inspector.entries();
    let entry = &entries[0];

    // Check worker prefixed fields
    assert_eq!(entry.metrics["worker_cpu_time"].as_u64(), 200);
    assert_eq!(entry.metrics["worker_memory_usage"].as_u64(), 1024);

    // Check db prefixed fields
    assert_eq!(entry.metrics["db_cpu_time"].as_u64(), 50);
    assert_eq!(entry.metrics["db_memory_usage"].as_u64(), 512);

    // Check non-prefixed field
    assert_eq!(entry.metrics["total_operations"].as_u64(), 10);
}