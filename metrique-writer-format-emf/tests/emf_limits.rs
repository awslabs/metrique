// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Comprehensive tests for EMF (Embedded Metric Format) limits validation.
//!
//! This module tests the behavior of the metrique library when EMF limits are approached
//! or exceeded. In the future, this behavior will be changed to truncation, however, for the moment
//! this test serves to document the current behavior.

use metrique_writer::{
    Entry, EntryWriter, MetricFlags, Observation, Unit, Value, ValueWriter, format::Format,
};
use metrique_writer_format_emf::{Emf, EntryDimensions};
use serde::Deserialize;
use std::{borrow::Cow, collections::HashMap, time::SystemTime};

/// Represents the structure of an EMF (Embedded Metric Format) output
#[derive(Debug, Deserialize)]
struct EmfOutput {
    #[serde(rename = "_aws")]
    aws: AwsMetadata,
    #[serde(flatten)]
    #[allow(dead_code)]
    fields: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct AwsMetadata {
    #[serde(rename = "CloudWatchMetrics")]
    cloudwatch_metrics: Vec<CloudWatchMetrics>,
    #[serde(rename = "Timestamp")]
    #[allow(dead_code)]
    timestamp: u64,
}

#[derive(Debug, Deserialize)]
struct CloudWatchMetrics {
    #[serde(rename = "Namespace")]
    #[allow(dead_code)]
    namespace: String,
    #[serde(rename = "Dimensions")]
    #[allow(dead_code)]
    dimensions: Vec<Vec<String>>,
    #[serde(rename = "Metrics")]
    metrics: Vec<MetricDefinition>,
}

#[derive(Debug, Deserialize)]
struct MetricDefinition {
    #[serde(rename = "Name")]
    #[allow(dead_code)]
    name: String,
    #[serde(rename = "Unit")]
    #[allow(dead_code)]
    unit: Option<String>,
}

impl EmfOutput {
    /// Count the total number of metrics across all CloudWatch metric groups
    fn count_total_metrics(&self) -> usize {
        self.aws
            .cloudwatch_metrics
            .iter()
            .map(|cw| cw.metrics.len())
            .sum()
    }

    /// Get the maximum number of metrics in any single CloudWatch directive
    /// This is the relevant limit for EMF (100 metrics per directive)
    fn max_metrics_per_directive(&self) -> usize {
        self.aws
            .cloudwatch_metrics
            .iter()
            .map(|cw| cw.metrics.len())
            .max()
            .unwrap_or(0)
    }

    /// Count the number of CloudWatch metric directives
    #[allow(dead_code)]
    fn count_directives(&self) -> usize {
        self.aws.cloudwatch_metrics.len()
    }

    /// Get the maximum number of dimensions in any single DimensionSet
    /// This is the relevant EMF limit (30 dimensions per DimensionSet)
    /// Each DimensionSet (inner array) can have at most 30 dimension references
    /// Example: [["AZ", "Region"], ["Service"]] -> max is 2 (from first DimensionSet)
    fn max_dimensions_per_dimension_set(&self) -> usize {
        self.aws
            .cloudwatch_metrics
            .iter()
            .flat_map(|cw_metrics| &cw_metrics.dimensions)
            .map(|dimension_set| dimension_set.len())
            .max()
            .unwrap_or(0)
    }

    /// Get the number of values for a specific metric
    /// This is relevant for the EMF limit (100 values per metric)
    /// Looks for a metric with "Values" array and returns the count
    fn count_values_for_metric(&self, metric_name: &str) -> usize {
        self.fields[metric_name]["Values"]
            .as_array()
            .expect("values should be an array")
            .len()
    }
}

/// Helper struct to generate test entries with a controlled number of metrics
#[derive(Debug)]
struct MetricCountTestEntry {
    metric_count: usize,
    timestamp: SystemTime,
}

impl MetricCountTestEntry {
    fn new(metric_count: usize) -> Self {
        Self {
            metric_count,
            timestamp: SystemTime::UNIX_EPOCH,
        }
    }
}

impl Entry for MetricCountTestEntry {
    fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
        writer.timestamp(self.timestamp);

        // Generate the specified number of metrics
        for i in 0..self.metric_count {
            let metric_name = format!("Metric{}", i);
            let metric_value = (i as u64) + 1; // Avoid zero values
            writer.value(metric_name, &metric_value);
        }
    }
}

/// Helper struct to generate test entries with a controlled number of dimensions
#[derive(Debug)]
struct DimensionCountTestEntry {
    dimension_count: usize,
    timestamp: SystemTime,
    entry_dimensions: Option<EntryDimensions>,
}

impl DimensionCountTestEntry {
    fn new(dimension_count: usize) -> Self {
        // Create EMF dimensions if requested
        let entry_dimensions = if dimension_count > 0 {
            // Create dimension names dynamically
            let dimension_names: Vec<String> = (0..dimension_count)
                .map(|i| format!("Dimension{}", i))
                .collect();

            // Create a single dimension set containing all the dimension names
            let dimension_set: Vec<Cow<'static, str>> = dimension_names
                .into_iter()
                .map(|name| Cow::Owned(name))
                .collect();

            Some(EntryDimensions::new(Cow::Owned(vec![Cow::Owned(
                dimension_set,
            )])))
        } else {
            None
        };

        Self {
            dimension_count,
            timestamp: SystemTime::UNIX_EPOCH,
            entry_dimensions,
        }
    }
}

impl Entry for DimensionCountTestEntry {
    fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
        writer.timestamp(self.timestamp);

        // Add a single metric with the specified number of dimensions
        writer.value("TestMetric", &42u64);

        // Generate the specified number of string dimensions
        for i in 0..self.dimension_count {
            let dimension_name = format!("Dimension{}", i);
            let dimension_value = format!("Value{}", i);
            writer.value(dimension_name, &dimension_value.as_str());
        }

        // Configure EMF dimensions if they were created
        if let Some(ref entry_dimensions) = self.entry_dimensions {
            writer.config(entry_dimensions);
        }
    }
}

/// Helper struct to generate test entries with metrics containing many values (numeric arrays)
#[derive(Debug)]
struct ValuesPerMetricTestEntry {
    values_count: usize,
    timestamp: SystemTime,
}

impl ValuesPerMetricTestEntry {
    fn new(values_count: usize) -> Self {
        Self {
            values_count,
            timestamp: SystemTime::UNIX_EPOCH,
        }
    }
}

/// Custom value type that generates multiple observations for testing values per metric limit
#[derive(Debug)]
struct MultiValueMetric {
    values: Vec<u64>,
}

impl MultiValueMetric {
    fn new(count: usize) -> Self {
        let values = (1..=count).map(|i| i as u64).collect();
        Self { values }
    }
}

impl Value for MultiValueMetric {
    fn write(&self, writer: impl ValueWriter) {
        let observations: Vec<Observation> = self
            .values
            .iter()
            .map(|&v| Observation::Unsigned(v))
            .collect();

        writer.metric(observations, Unit::None, [], MetricFlags::empty());
    }
}

impl Entry for ValuesPerMetricTestEntry {
    fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
        writer.timestamp(self.timestamp);

        // Create a metric with the specified number of values
        let multi_value = MultiValueMetric::new(self.values_count);
        writer.value("MultiValueMetric", &multi_value);
    }
}

/// Helper function to format an entry and return the parsed EMF output
fn format_entry_to_emf(entry: &impl Entry) -> EmfOutput {
    let mut output = Vec::new();
    let mut formatter = Emf::all_validations("TestNamespace".into(), vec![vec![]]);
    formatter.format(entry, &mut output).unwrap();
    serde_json::from_slice(&output).unwrap()
}

#[test]
fn test_basic_output() {
    // Test that our helper functions can generate entries correctly
    let entry = MetricCountTestEntry::new(5);
    let emf_output = format_entry_to_emf(&entry);

    // Test metric counting
    assert_eq!(
        emf_output.count_total_metrics(),
        5,
        "Should count 5 metrics total"
    );
    assert_eq!(
        emf_output.max_metrics_per_directive(),
        5,
        "Should have max 5 metrics per directive"
    );

    assert_eq!(emf_output.count_directives(), 1, "should have 1 directive");
}

#[test]
fn test_values_per_metric_helper() {
    let entry = ValuesPerMetricTestEntry::new(5);
    let emf_output = format_entry_to_emf(&entry);

    // The metric should be present with 5 values
    assert_eq!(
        emf_output.count_total_metrics(),
        1,
        "Should have 1 metric with multiple values"
    );
    assert_eq!(
        emf_output.count_values_for_metric("MultiValueMetric"),
        5,
        "Should have exactly 5 values in the MultiValueMetric"
    );

    assert_eq!(
        emf_output.fields["MultiValueMetric"]["Counts"]
            .as_array()
            .unwrap()
            .len(),
        5
    );
    assert_eq!(
        emf_output.fields["MultiValueMetric"]["Values"]
            .as_array()
            .unwrap()
            .len(),
        5
    );
}

#[test]
fn test_boundary_metrics_99() {
    let entry = MetricCountTestEntry::new(99);
    let emf_output = format_entry_to_emf(&entry);

    assert_eq!(
        emf_output.count_total_metrics(),
        99,
        "Should have exactly 99 metrics total"
    );
    assert_eq!(
        emf_output.max_metrics_per_directive(),
        99,
        "Should have max 99 metrics per directive"
    );
}

#[test]
fn test_boundary_metrics_100() {
    let entry = MetricCountTestEntry::new(100);
    let emf_output = format_entry_to_emf(&entry);

    assert_eq!(
        emf_output.count_total_metrics(),
        100,
        "Should have exactly 100 metrics total"
    );
    assert_eq!(
        emf_output.max_metrics_per_directive(),
        100,
        "Should have max 100 metrics per directive"
    );
}

#[test]
fn test_boundary_metrics_101() {
    let entry = MetricCountTestEntry::new(101);
    let emf_output = format_entry_to_emf(&entry);

    // Document current behavior - this will likely be > 100 until truncation is implemented
    let total_metrics = emf_output.count_total_metrics();
    let max_per_directive = emf_output.max_metrics_per_directive();
    println!(
        "Current behavior with 101 metrics: {} total metrics, {} max per directive",
        total_metrics, max_per_directive
    );
    // Current behavior: no truncation implemented, so 101 metrics are all in one directive
    assert_eq!(
        max_per_directive, 101,
        "Current behavior: no truncation, 101 metrics in single directive"
    );
}

#[test]
fn test_boundary_dimensions_29() {
    let entry = DimensionCountTestEntry::new(29);
    let emf_output = format_entry_to_emf(&entry);

    assert_eq!(
        emf_output.max_dimensions_per_dimension_set(),
        29,
        "Should have exactly 29 dimensions in the dimension set"
    );
}

#[test]
fn test_boundary_dimensions_30() {
    let entry = DimensionCountTestEntry::new(30);
    let emf_output = format_entry_to_emf(&entry);

    assert_eq!(
        emf_output.max_dimensions_per_dimension_set(),
        30,
        "Should have exactly 30 dimensions in the dimension set"
    );
}

#[test]
fn test_boundary_dimensions_31() {
    let entry = DimensionCountTestEntry::new(31);
    let emf_output = format_entry_to_emf(&entry);

    let dimension_count = emf_output.max_dimensions_per_dimension_set();
    println!(
        "Current behavior with 31 dimensions: {} EMF dimensions per dimension set",
        dimension_count
    );
    // Current behavior: no truncation implemented, so 31 dimensions are all in one dimension set
    assert_eq!(
        dimension_count, 31,
        "Current behavior: no truncation, 31 dimensions in single dimension set"
    );
}

#[test]
fn test_boundary_values_per_metric_99() {
    let entry = ValuesPerMetricTestEntry::new(99);
    let emf_output = format_entry_to_emf(&entry);

    assert_eq!(
        emf_output.count_values_for_metric("MultiValueMetric"),
        99,
        "Should have exactly 99 values in the MultiValueMetric"
    );
}

#[test]
fn test_boundary_values_per_metric_100() {
    let entry = ValuesPerMetricTestEntry::new(100);
    let emf_output = format_entry_to_emf(&entry);

    assert_eq!(
        emf_output.count_values_for_metric("MultiValueMetric"),
        100,
        "Should have exactly 100 values in the MultiValueMetric"
    );
}

#[test]
fn test_boundary_values_per_metric_101() {
    let entry = ValuesPerMetricTestEntry::new(101);
    let emf_output = format_entry_to_emf(&entry);

    // Current behavior: no truncation implemented, so 101 values are preserved
    assert_eq!(
        emf_output.count_values_for_metric("MultiValueMetric"),
        101,
        "Current behavior: no truncation, 101 values preserved in MultiValueMetric"
    );
}

#[test]
fn test_json_validity_with_large_entries() {
    // Test that even large entries can be formatted successfully
    let large_entry = MetricCountTestEntry::new(150);
    let _emf_output = format_entry_to_emf(&large_entry);

    // Should always produce valid EMF output regardless of truncation
}

#[test]
fn test_mixed_limits_entry() {
    // Create an entry that potentially exceeds multiple limits
    struct MixedLimitsEntry;

    impl Entry for MixedLimitsEntry {
        fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
            writer.timestamp(SystemTime::UNIX_EPOCH);

            // Add many metrics
            for i in 0..120 {
                writer.value(format!("Metric{}", i), &(i as u64));
            }

            // Add many dimensions
            for i in 0..35 {
                writer.value(format!("Dimension{}", i), &format!("Value{}", i).as_str());
            }
        }
    }

    let entry = MixedLimitsEntry;
    let emf_output = format_entry_to_emf(&entry);

    let total_metrics = emf_output.count_total_metrics();
    let max_per_directive = emf_output.max_metrics_per_directive();
    let dimensions = emf_output.max_dimensions_per_dimension_set();

    println!(
        "Mixed limits entry: {} total metrics, {} max per directive, {} dimensions",
        total_metrics, max_per_directive, dimensions
    );
}
