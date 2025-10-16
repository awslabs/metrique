// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Example showing manual implementation of MergeableEntry for in-memory aggregation.

use metrique::emf::Emf;
use metrique::writer::{
    AttachGlobalEntrySinkExt, Entry, EntrySink, EntryWriter, FormatExt, GlobalEntrySink,
    merge::{MergeableEntry, MergedEntry},
    sink::global_entry_sink,
};

global_entry_sink! { ServiceMetrics }

/// Key for grouping requests that can be merged together.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct RequestKey {
    operation: &'static str,
    status_code: u16,
}

/// A simple request metric that can be merged.
#[derive(Debug, Clone)]
struct RequestMetrics {
    operation: &'static str,
    status_code: u16,
    request_count: u64,
    total_latency_ms: u64,
}

/// The merged version accumulates multiple RequestMetrics.
#[derive(Debug)]
struct MergedRequestMetrics {
    key: RequestKey,
    request_count: u64,
    total_latency_ms: u64,
    entry_count: usize,
}

impl Entry for RequestMetrics {
    fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
        writer.value("Operation", &self.operation);
        writer.value("StatusCode", &self.status_code);
        writer.value("RequestCount", &self.request_count);
        writer.value("TotalLatencyMs", &self.total_latency_ms);
    }

    fn sample_group(&self) -> impl Iterator<Item = (std::borrow::Cow<'static, str>, std::borrow::Cow<'static, str>)> {
        [
            ("Operation".into(), self.operation.into()),
            ("StatusCode".into(), self.status_code.to_string().into()),
        ]
        .into_iter()
    }
}

impl Entry for MergedRequestMetrics {
    fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
        writer.value("Operation", &self.key.operation);
        writer.value("StatusCode", &self.key.status_code);
        writer.value("RequestCount", &self.request_count);
        writer.value("TotalLatencyMs", &self.total_latency_ms);
        writer.value("AverageLatencyMs", &(self.total_latency_ms / self.request_count.max(1)));
        writer.value("MergedEntryCount", &(self.entry_count as u64));
    }

    fn sample_group(&self) -> impl Iterator<Item = (std::borrow::Cow<'static, str>, std::borrow::Cow<'static, str>)> {
        [
            ("Operation".into(), self.key.operation.into()),
            ("StatusCode".into(), self.key.status_code.to_string().into()),
        ]
        .into_iter()
    }
}

impl MergeableEntry for RequestMetrics {
    type Key = RequestKey;
    type Merged = MergedRequestMetrics;

    fn new_merged(key: Self::Key) -> Self::Merged {
        MergedRequestMetrics {
            key,
            request_count: 0,
            total_latency_ms: 0,
            entry_count: 0,
        }
    }

    fn key(&self) -> Self::Key {
        RequestKey {
            operation: self.operation,
            status_code: self.status_code,
        }
    }
}

impl MergedEntry for MergedRequestMetrics {
    type Key = RequestKey;
    type Source = RequestMetrics;

    fn merge_into(&mut self, entry: &Self::Source) {
        // Key is already set during construction, just merge metrics
        self.request_count += entry.request_count;
        self.total_latency_ms += entry.total_latency_ms;
        self.entry_count += 1;
    }

    fn count(&self) -> usize {
        self.entry_count
    }
}

fn main() {
    // Initialize metrics sink
    let _handle = ServiceMetrics::attach_to_stream(
        Emf::builder("MergingExample".to_string(), vec![vec!["Operation".to_string()]])
            .build()
            .output_to_makewriter(|| std::io::stdout().lock()),
    );

    // Create some sample metrics
    let metrics = vec![
        RequestMetrics {
            operation: "GetItem",
            status_code: 200,
            request_count: 1,
            total_latency_ms: 50,
        },
        RequestMetrics {
            operation: "GetItem",
            status_code: 200,
            request_count: 1,
            total_latency_ms: 75,
        },
        RequestMetrics {
            operation: "GetItem",
            status_code: 500,
            request_count: 1,
            total_latency_ms: 200,
        },
    ];

    // Manually merge entries with same sample group
    let key = RequestKey {
        operation: "GetItem",
        status_code: 200,
    };
    let mut merged = RequestMetrics::new_merged(key);
    for metric in metrics.iter().take(2) {
        merged.merge_into(metric);
    }

    let count = merged.count();
    let total_requests = merged.request_count;
    let avg_latency = merged.total_latency_ms / merged.request_count;

    println!("Merged {} entries", count);
    println!("Total requests: {}", total_requests);
    println!("Average latency: {}ms", avg_latency);

    // Emit the merged entry
    ServiceMetrics::sink().append(merged);
}
