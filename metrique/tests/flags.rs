// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::time::{SystemTime, UNIX_EPOCH};

use metrique::emf::Emf;
use metrique::writer::{format::Format, value::ForceFlag};
use metrique::{CloseValue, RootEntry, unit_of_work::metrics};
use serde_json::Value;

// --- EMF: HighStorageResolution on individual field ---

#[metrics(
    rename_all = "PascalCase",
    emf::dimension_sets = [["Operation"]],
)]
struct HighResField {
    #[metrics(timestamp)]
    timestamp: SystemTime,
    operation: String,
    #[metrics(flags(metrique::emf::flags::HighStorageResolution))]
    event_count: u64,
    normal_count: u64,
}

#[test]
fn flags_high_res_on_individual_field() {
    let m = HighResField {
        timestamp: UNIX_EPOCH,
        operation: "test".into(),
        event_count: 42,
        normal_count: 7,
    };
    let closed = CloseValue::close(m);
    let entry = RootEntry::new(closed);

    let mut emf = Emf::all_validations("Test".to_string(), vec![vec!["Operation".to_string()]]);
    let mut output = vec![];
    emf.format(&entry, &mut output).unwrap();
    let json: Value = serde_json::from_slice(&output).unwrap();

    let metrics = json["_aws"]["CloudWatchMetrics"][0]["Metrics"]
        .as_array()
        .unwrap();

    let event_metric = metrics.iter().find(|m| m["Name"] == "EventCount").unwrap();
    assert_eq!(event_metric["StorageResolution"], 1);

    let normal_metric = metrics.iter().find(|m| m["Name"] == "NormalCount").unwrap();
    assert!(
        normal_metric.get("StorageResolution").is_none()
            || normal_metric["StorageResolution"] == 60
    );
}

// --- EMF: NoMetric flag ---

#[metrics(
    rename_all = "PascalCase",
    emf::dimension_sets = [["Operation"]],
)]
struct NoMetricField {
    #[metrics(timestamp)]
    timestamp: SystemTime,
    operation: String,
    #[metrics(flags(metrique::emf::flags::NoMetric))]
    debug_value: u64,
    normal_count: u64,
}

#[test]
fn flags_no_metric_excludes_from_metrics_array() {
    let m = NoMetricField {
        timestamp: UNIX_EPOCH,
        operation: "test".into(),
        debug_value: 99,
        normal_count: 5,
    };
    let closed = CloseValue::close(m);
    let entry = RootEntry::new(closed);

    let mut emf = Emf::all_validations("Test".to_string(), vec![vec!["Operation".to_string()]]);
    let mut output = vec![];
    emf.format(&entry, &mut output).unwrap();
    let json: Value = serde_json::from_slice(&output).unwrap();

    assert_eq!(json["DebugValue"], 99);
    let metrics = json["_aws"]["CloudWatchMetrics"][0]["Metrics"]
        .as_array()
        .unwrap();
    assert!(!metrics.iter().any(|m| m["Name"] == "DebugValue"));
    assert!(metrics.iter().any(|m| m["Name"] == "NormalCount"));
}

// --- EMF: Multiple flags on one field ---

#[metrics(
    rename_all = "PascalCase",
    emf::dimension_sets = [["Operation"]],
)]
struct MultiFlagField {
    #[metrics(timestamp)]
    timestamp: SystemTime,
    operation: String,
    #[metrics(flags(
        metrique::emf::flags::HighStorageResolution,
        metrique::emf::flags::NoMetric
    ))]
    debug_value: u64,
    normal_count: u64,
}

#[test]
fn flags_multiple_on_single_field() {
    let m = MultiFlagField {
        timestamp: UNIX_EPOCH,
        operation: "test".into(),
        debug_value: 99,
        normal_count: 5,
    };
    let closed = CloseValue::close(m);
    let entry = RootEntry::new(closed);

    let mut emf = Emf::all_validations("Test".to_string(), vec![vec!["Operation".to_string()]]);
    let mut output = vec![];
    emf.format(&entry, &mut output).unwrap();
    let json: Value = serde_json::from_slice(&output).unwrap();

    assert_eq!(json["DebugValue"], 99);
    let metrics = json["_aws"]["CloudWatchMetrics"][0]["Metrics"]
        .as_array()
        .unwrap();
    assert!(!metrics.iter().any(|m| m["Name"] == "DebugValue"));
}

// --- EMF: Enum variant with flags ---

#[metrics(
    rename_all = "PascalCase",
    emf::dimension_sets = [["Op"]],
)]
enum EnumFlags {
    Fast {
        #[metrics(timestamp)]
        ts: SystemTime,
        op: String,
        #[metrics(flags(metrique::emf::flags::HighStorageResolution))]
        latency: u64,
    },
    Slow {
        #[metrics(timestamp)]
        ts: SystemTime,
        op: String,
        latency: u64,
    },
}

#[test]
fn flags_enum_variant() {
    let fast = EnumFlags::Fast {
        ts: UNIX_EPOCH,
        op: "read".into(),
        latency: 5,
    };
    let closed = CloseValue::close(fast);
    let entry = RootEntry::new(closed);

    let mut emf = Emf::all_validations("Test".to_string(), vec![vec!["Op".to_string()]]);
    let mut output = vec![];
    emf.format(&entry, &mut output).unwrap();
    let json: Value = serde_json::from_slice(&output).unwrap();

    let metrics = json["_aws"]["CloudWatchMetrics"][0]["Metrics"]
        .as_array()
        .unwrap();
    let latency_metric = metrics.iter().find(|m| m["Name"] == "Latency").unwrap();
    assert_eq!(latency_metric["StorageResolution"], 1);

    // Slow variant: no flag on latency
    let slow = EnumFlags::Slow {
        ts: UNIX_EPOCH,
        op: "write".into(),
        latency: 100,
    };
    let closed = CloseValue::close(slow);
    let entry = RootEntry::new(closed);

    let mut output2 = vec![];
    emf.format(&entry, &mut output2).unwrap();
    let json2: Value = serde_json::from_slice(&output2).unwrap();

    let metrics2 = json2["_aws"]["CloudWatchMetrics"][0]["Metrics"]
        .as_array()
        .unwrap();
    let latency_metric2 = metrics2.iter().find(|m| m["Name"] == "Latency").unwrap();
    assert!(
        latency_metric2.get("StorageResolution").is_none()
            || latency_metric2["StorageResolution"] == 60
    );
}

// --- Generic FlagConstructor (not EMF-specific) ---

#[derive(Debug)]
struct CustomFlagOpt;
impl metrique::writer::value::MetricOptions for CustomFlagOpt {}

struct CustomFlagCtor;
impl metrique::writer::value::FlagConstructor for CustomFlagCtor {
    fn construct() -> metrique::writer::MetricFlags<'static> {
        metrique::writer::MetricFlags::upcast(&CustomFlagOpt)
    }
}

#[metrics(rename_all = "PascalCase")]
struct CustomFlagMetrics {
    #[metrics(timestamp)]
    timestamp: SystemTime,
    #[metrics(flags(CustomFlagCtor))]
    flagged_value: u64,
    normal_value: u64,
}

#[test]
fn flags_generic_flag_constructor() {
    let m = CustomFlagMetrics {
        timestamp: UNIX_EPOCH,
        flagged_value: 123,
        normal_value: 456,
    };
    let closed = CloseValue::close(m);
    let entry = RootEntry::new(closed);

    // Unrecognized flags are ignored by EMF, but the write path still works
    let mut emf = Emf::all_validations("Test".to_string(), vec![vec![]]);
    let mut output = vec![];
    emf.format(&entry, &mut output).unwrap();
    let json: Value = serde_json::from_slice(&output).unwrap();

    assert_eq!(json["FlaggedValue"], 123);
    assert_eq!(json["NormalValue"], 456);
}

// --- ForceFlag<T, Ctor> type wrapper still works (backwards compat) ---

#[metrics(
    rename_all = "PascalCase",
    emf::dimension_sets = [["Operation"]],
)]
struct DirectForceFlagUsage {
    #[metrics(timestamp)]
    timestamp: SystemTime,
    operation: String,
    high_res: ForceFlag<u64, metrique::emf::HighStorageResolutionCtor>,
}

#[test]
fn force_flag_type_wrapper_still_works() {
    let m = DirectForceFlagUsage {
        timestamp: UNIX_EPOCH,
        operation: "test".into(),
        high_res: 42.into(),
    };
    let closed = CloseValue::close(m);
    let entry = RootEntry::new(closed);

    let mut emf = Emf::all_validations("Test".to_string(), vec![vec!["Operation".to_string()]]);
    let mut output = vec![];
    emf.format(&entry, &mut output).unwrap();
    let json: Value = serde_json::from_slice(&output).unwrap();

    let metrics = json["_aws"]["CloudWatchMetrics"][0]["Metrics"]
        .as_array()
        .unwrap();
    let hr_metric = metrics.iter().find(|m| m["Name"] == "HighRes").unwrap();
    assert_eq!(hr_metric["StorageResolution"], 1);
}
