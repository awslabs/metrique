// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Integration tests for LocalFormat output with real Histogram types.
//!
//! These tests verify that:
//! 1. HistogramClosed sets the Distribution flag so formatters always show percentiles
//! 2. LocalFormat output is stable across all three styles

use metrique::RootEntry;
use metrique::local::{LocalFormat, OutputStyle};
use metrique::unit_of_work::metrics;
use metrique_aggregation::histogram::{ExponentialAggregationStrategy, Histogram};
use metrique_core::CloseValue;
use metrique_writer::format::Format;
use metrique_writer::unit::Millisecond;
use std::time::Duration;

#[metrics(rename_all = "PascalCase")]
struct RequestMetrics {
    operation: &'static str,
    #[metrics(unit = Millisecond)]
    latency: Histogram<Duration, ExponentialAggregationStrategy>,
    request_count: u64,
}

/// Format a metrics struct through LocalFormat, stripping the timestamp line for determinism.
fn format_entry(style: OutputStyle, metrics: RequestMetrics) -> String {
    let closed = metrics.close();
    let entry = RootEntry::new(closed);
    let mut buf = Vec::new();
    LocalFormat::new(style).format(&entry, &mut buf).unwrap();
    let output = String::from_utf8(buf).unwrap();
    // Strip timestamp lines for deterministic snapshots
    output
        .lines()
        .filter(|line| !line.contains("timestamp"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn sample_metrics() -> RequestMetrics {
    let mut m = RequestMetrics {
        operation: "GetUser",
        latency: Histogram::default(),
        request_count: 42,
    };
    m.latency.add_value(Duration::from_millis(5));
    m.latency.add_value(Duration::from_millis(12));
    m.latency.add_value(Duration::from_millis(25));
    m.latency.add_value(Duration::from_millis(100));
    m
}

/// A histogram with a single observation must still render as a distribution
/// (with percentiles), not as a single inline value. This proves HistogramClosed
/// sets the Distribution flag.
#[test]
fn histogram_single_value_shows_as_distribution() {
    let mut m = RequestMetrics {
        operation: "PutItem",
        latency: Histogram::default(),
        request_count: 1,
    };
    m.latency.add_value(Duration::from_millis(42));

    let output = format_entry(OutputStyle::json(), m);
    let parsed: serde_json::Value = serde_json::from_str(&output).unwrap();

    // With the Distribution flag, even a single observation should produce
    // a percentile object, not a bare number.
    let latency = &parsed["Latency"];
    assert!(
        latency.is_object(),
        "single-observation histogram should render as object with percentiles, got: {latency}"
    );
    assert!(latency["count"].is_number());
    assert!(latency["min"].is_number());
    assert!(latency["max"].is_number());
}

#[test]
fn snapshot_pretty() {
    let output = format_entry(OutputStyle::pretty(), sample_metrics());
    insta::assert_snapshot!(output);
}

#[test]
fn snapshot_json() {
    let output = format_entry(OutputStyle::json(), sample_metrics());
    insta::assert_snapshot!(output);
}

#[test]
fn snapshot_compact_json() {
    let output = format_entry(OutputStyle::compact_json(), sample_metrics());
    insta::assert_snapshot!(output);
}

#[test]
fn snapshot_markdown() {
    let output = format_entry(OutputStyle::markdown_table(), sample_metrics());
    insta::assert_snapshot!(output);
}
