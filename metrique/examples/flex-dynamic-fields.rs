// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! This example demonstrates the use of `Flex` for dynamic metric field names.
//!
//! `Flex` is useful when you need to create metric fields with names that are only
//! known at runtime, such as user-provided tags, dynamic dimensions, or computed
//! field names based on configuration.

use std::collections::HashMap;
use std::time::SystemTime;

use metrique::{flex::Flex, unit::Count, unit_of_work::metrics};
use metrique_writer::{
    AttachGlobalEntrySinkExt, Entry, EntryIoStreamExt, FormatExt, GlobalEntrySink,
    sink::global_entry_sink,
};
use metrique_writer_format_emf::Emf;

global_entry_sink! { ServiceMetrics }

#[metrics(rename_all = "PascalCase")]
struct DynamicMetrics {
    #[metrics(timestamp)]
    timestamp: SystemTime,

    operation: &'static str,

    // Static field for comparison
    #[metrics(unit = Count)]
    static_counter: usize,

    // Dynamic field - the key name is determined at runtime
    #[metrics(flatten)]
    dynamic_field: Flex<usize>,

    // Multiple dynamic fields can be used
    #[metrics(flatten)]
    user_tag: Flex<String>,

    #[metrics(flatten)]
    computed_metric: Flex<f64>,
}

impl DynamicMetrics {
    fn init(operation: &'static str) -> DynamicMetricsGuard {
        Self {
            timestamp: SystemTime::now(),
            operation,
            static_counter: 0,
            // Create Flex fields with just keys - values will be set later based on runtime conditions
            dynamic_field: Flex::new("records_processed"),
            user_tag: Flex::new("environment"),
            computed_metric: Flex::new(format!("{}Duration", operation.to_lowercase())),
        }
        .append_on_drop(ServiceMetrics::sink())
    }
}

#[derive(Entry)]
#[entry(rename_all = "PascalCase")]
struct Globals {
    service: String,
    region: String,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let globals = Globals {
        service: "DynamicService".into(),
        region: "us-west-2".into(),
    };

    let _handle = ServiceMetrics::attach_to_stream(
        Emf::all_validations("DynamicMetrics".to_string(), vec![vec![]])
            .output_to_makewriter(std::io::stdout)
            .merge_globals(globals),
    );

    // Example 1: Basic dynamic field usage
    basic_dynamic_example().await;

    // Example 2: Configuration-driven dynamic fields
    config_driven_example().await;

    // Example 3: User-provided tags
    user_tags_example().await;

    // Example 4: A/B test metrics
    ab_test_example().await;

    // Example 5: Builder API
    builder_api_example().await;

    // Example 6: Optional fields
    optional_fields_example().await;

    // Example 7: Helper function usage
    let _error_metric = create_error_metric("timeout", 3);

    // Example 8: Create with unset values, then set later
    unset_then_set_example().await;
}

async fn basic_dynamic_example() {
    println!("=== Basic Dynamic Field Example ===");

    let mut metrics = DynamicMetrics::init("ProcessData");

    // Simulate processing some records and setting values based on results
    let records_processed = 42;
    let environment = "production";
    let processing_duration = 123.45;

    // Set values based on runtime conditions
    metrics.dynamic_field.set_value(records_processed);
    metrics.user_tag.set_value(environment.to_string());
    metrics.computed_metric.set_value(processing_duration);
    metrics.static_counter = 1;

    // The output will include:
    // - StaticCounter: 1
    // - records_processed: 42 (set at runtime)
    // - environment: "production" (set at runtime)
    // - processdataDuration: 123.45 (set at runtime)
}

async fn config_driven_example() {
    println!("=== Configuration-Driven Example ===");

    // Simulate configuration that determines metric field names
    let config = HashMap::from([
        ("metric_prefix".to_string(), "api_".to_string()),
        ("counter_suffix".to_string(), "_total".to_string()),
    ]);

    let field_name = format!(
        "{}requests{}",
        config.get("metric_prefix").unwrap(),
        config.get("counter_suffix").unwrap()
    );

    let mut metrics = DynamicMetrics::init("HandleRequest");

    // Update the dynamic field name based on configuration
    metrics.dynamic_field = Flex::new(field_name).with_value(156); // "api_requests_total"
    metrics.user_tag.set_value("v1.2.3".to_string());
    metrics.computed_metric.set_value(89.2);
    metrics.static_counter = 2;
}

async fn user_tags_example() {
    println!("=== User-Provided Tags Example ===");

    // Simulate user-provided tags that become metric fields
    let user_tags = vec![
        ("customer_tier", "premium"),
        ("feature_flag", "new_ui_enabled"),
    ];

    for (tag_key, tag_value) in user_tags {
        let mut metrics = DynamicMetrics::init("UserAction");

        // Set the user-provided tag as a dynamic field
        metrics.user_tag = Flex::new(tag_key).with_value(tag_value.to_string());
        metrics.dynamic_field.set_value(1); // action_count
        metrics.computed_metric.set_value(45.6);
        metrics.static_counter = 3;
    }
}

async fn ab_test_example() {
    println!("=== A/B Test Metrics Example ===");

    // Different variants of an A/B test create different metric fields
    let variants = vec![
        ("variant_a_conversions", 45),
        ("variant_b_conversions", 52),
        ("control_conversions", 38),
    ];

    for (variant_field, conversion_count) in variants {
        let mut metrics = DynamicMetrics::init("ABTest");

        // Set the variant-specific field name and value
        metrics.dynamic_field = Flex::new(variant_field).with_value(conversion_count);
        metrics.user_tag.set_value("checkout_flow_v2".to_string());
        metrics.computed_metric.set_value(234.1);
        metrics.static_counter = 4;
    }
}

// Example of a helper function for creating commonly used dynamic metrics
fn create_error_metric(error_type: &str, error_count: usize) -> DynamicMetricsGuard {
    let mut metrics = DynamicMetrics::init("ErrorTracking");

    // Set error-specific field name and values
    let error_field = format!("{}_errors", error_type.to_lowercase());
    metrics.dynamic_field = Flex::new(error_field).with_value(error_count);
    metrics.user_tag.set_value("high".to_string()); // error_severity
    metrics.computed_metric.set_value(123.45);

    metrics
}

// Example showing how to use Flex builder API
async fn builder_api_example() {
    println!("=== Builder API Example ===");

    let mut metrics = DynamicMetrics::init("ConvenienceTest");

    // Using builder API for flexible construction - set values after creation
    metrics.dynamic_field = Flex::new("api_requests").with_value(150usize);
    metrics.user_tag = Flex::new("deployment").with_value("v2.1.0".to_string());
    metrics.computed_metric = Flex::new("response_time_ms").with_value(45.2f64);
    metrics.static_counter = 5;
}

// Example showing optional fields that may or may not be present
async fn optional_fields_example() {
    println!("=== Optional Fields Example ===");

    // Simulate a scenario where some fields might not be available
    let user_id: Option<String> = Some("user456".to_string());
    let session_id: Option<String> = None; // Not available

    let mut metrics = DynamicMetrics::init("OptionalFields");

    // Set values based on what's available
    metrics.dynamic_field.set_value(1usize); // page_views
    metrics.user_tag = Flex::new("user_id").with_optional_value(user_id);
    metrics.computed_metric =
        Flex::new("session_id").with_optional_value(session_id.map(|s| s.len() as f64));
    metrics.static_counter = 6;

    // Only user_id will appear in the output, session_id will be omitted
}

// Example showing the key benefit: create metrics with unset Flex fields, then set values later
async fn unset_then_set_example() {
    println!("=== Unset Then Set Example ===");

    // Create metrics struct with unset Flex fields - this is the key use case!
    let mut metrics = DynamicMetrics {
        timestamp: SystemTime::now(),
        operation: "ProcessRequest",
        static_counter: 0,
        dynamic_field: Flex::new("items_processed"), // No value initially
        user_tag: Flex::new("request_source"),       // No value initially
        computed_metric: Flex::new("processing_time"), // No value initially
    }
    .append_on_drop(ServiceMetrics::sink());

    // Simulate business logic that determines values at runtime
    let items_to_process = vec!["item1", "item2", "item3"];
    let request_came_from_api = true;

    // Process items and set count
    let processed_count = items_to_process.len();
    metrics.dynamic_field.set_value(processed_count);

    // Set source based on runtime condition
    if request_came_from_api {
        metrics.user_tag.set_value("api".to_string());
    } else {
        metrics.user_tag.set_value("batch".to_string());
    }

    // Simulate some processing time measurement
    let start_time = std::time::Instant::now();
    // ... do some work ...
    let processing_duration = start_time.elapsed();
    metrics
        .computed_metric
        .set_value(processing_duration.as_millis() as f64);

    metrics.static_counter = 7;

    // When metrics drops, all the dynamically set fields will be included
}

#[cfg(test)]
mod tests {
    use super::*;
    use metrique_writer::test_util;

    #[test]
    fn test_flex_dynamic_fields() {
        let test_util::TestEntrySink { inspector, sink } = test_util::test_entry_sink();

        let metrics = DynamicMetrics {
            timestamp: SystemTime::now(),
            operation: "TestOp",
            static_counter: 100,
            dynamic_field: Flex::new("custom_metric").with_value(42usize),
            user_tag: Flex::new("user_id").with_value("user123".to_string()),
            computed_metric: Flex::new("computed_value").with_value(3.14),
        }
        .append_on_drop(sink);

        drop(metrics);

        let entries = inspector.entries();
        assert_eq!(entries.len(), 1);

        let entry = &entries[0];

        // Check static fields
        assert_eq!(entry.values["Operation"], "TestOp");
        assert_eq!(entry.metrics["StaticCounter"].as_u64(), 100);

        // Check dynamic fields
        assert_eq!(entry.metrics["custom_metric"].as_u64(), 42);
        assert_eq!(entry.values["user_id"], "user123");
        assert_eq!(entry.metrics["computed_value"].as_f64(), 3.14);
    }
}
