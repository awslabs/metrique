// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Integration tests for the Flex type

use metrique::{Flex, unit_of_work::metrics};
use metrique_writer::test_util;
use std::time::SystemTime;

#[metrics]
struct TestMetrics {
    #[metrics(timestamp)]
    timestamp: SystemTime,

    #[metrics(flatten)]
    dynamic_field: Flex<usize>,
}

#[metrics]
struct MultiFieldMetrics {
    #[metrics(timestamp)]
    timestamp: SystemTime,

    #[metrics(flatten)]
    set_field: Flex<usize>,

    #[metrics(flatten)]
    unset_field: Flex<String>,
}

#[test]
fn test_flex_dynamic_field_name() {
    let test_util::TestEntrySink { inspector, sink } = test_util::test_entry_sink();

    let metrics = TestMetrics {
        timestamp: SystemTime::now(),
        dynamic_field: Flex::new("runtime_field_name").with_value(42),
    }
    .append_on_drop(sink);

    drop(metrics);

    let entries = inspector.entries();
    let entry = &entries[0];

    // Field appears with the runtime-determined name
    assert_eq!(entry.metrics["runtime_field_name"].as_u64(), 42);
}

#[test]
fn test_flex_set_value_after_creation() {
    let test_util::TestEntrySink { inspector, sink } = test_util::test_entry_sink();

    let mut metrics = TestMetrics {
        timestamp: SystemTime::now(),
        dynamic_field: Flex::new("my_field"), // No value initially
    }
    .append_on_drop(sink);

    // Set value later
    metrics.dynamic_field.set_value(100);

    drop(metrics);

    let entries = inspector.entries();
    let entry = &entries[0];

    assert_eq!(entry.metrics["my_field"].as_u64(), 100);
}

#[test]
fn test_flex_unset_field_omitted() {
    let test_util::TestEntrySink { inspector, sink } = test_util::test_entry_sink();

    let metrics = MultiFieldMetrics {
        timestamp: SystemTime::now(),
        set_field: Flex::new("present_field").with_value(42),
        unset_field: Flex::new("missing_field"), // No value set
    }
    .append_on_drop(sink);

    drop(metrics);

    let entries = inspector.entries();
    let entry = &entries[0];

    // Set field should appear
    assert_eq!(entry.metrics["present_field"].as_u64(), 42);

    // Unset field should not appear in output
    assert!(!entry.metrics.contains_key("missing_field"));
    assert!(!entry.values.contains_key("missing_field"));
}
#[metrics]
struct TimestampMetrics {
    #[metrics(timestamp)]
    timestamp: SystemTime,

    #[metrics(flatten)]
    close_time: Flex<metrique::timers::TimestampOnClose>,
}

#[test]
fn test_flex_close_value_called() {
    use metrique::timers::TimestampOnClose;
    use metrique_timesource::{TimeSource, fakes::StaticTimeSource, set_time_source};
    use std::time::{Duration, UNIX_EPOCH};

    let test_util::TestEntrySink { inspector, sink } = test_util::test_entry_sink();

    // Set up a mock time source at a specific time
    let mock_time = UNIX_EPOCH + Duration::from_secs(1234567890);
    let time_source = TimeSource::custom(StaticTimeSource::at_time(mock_time));
    let _guard = set_time_source(time_source);

    let mut metrics = TimestampMetrics {
        timestamp: SystemTime::now(),
        close_time: Flex::new("close_timestamp"), // No value initially
    }
    .append_on_drop(sink);

    // Set a TimestampOnClose value - this should record the time when close() is called, not now
    metrics.close_time.set_value(TimestampOnClose::default());

    // When metrics is dropped, close() should be called on the TimestampOnClose
    drop(metrics);

    let entries = inspector.entries();
    let entry = &entries[0];

    // The field should appear with a timestamp value (proving close() was called)
    assert!(entry.values.contains_key("close_timestamp"));
    // The value should be the exact timestamp we mocked (in milliseconds since epoch)
    let timestamp_value = &entry.values["close_timestamp"];
    let expected_millis = mock_time.duration_since(UNIX_EPOCH).unwrap().as_millis();
    // TimestampValue formats as a float, so we expect ".0" at the end
    let expected_str = format!("{}.0", expected_millis);
    assert_eq!(timestamp_value, &expected_str);
}
