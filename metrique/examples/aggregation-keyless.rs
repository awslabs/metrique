// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Example showing keyless aggregation - all entries merge together regardless of dimensions.

use metrique::emf::Emf;
use metrique::writer::{
    AttachGlobalEntrySinkExt, Entry, EntrySink, EntryWriter, FormatExt, GlobalEntrySink,
    merge::{AggregatableEntry, AggregatedEntry},
    sink::global_entry_sink,
};

global_entry_sink! { ServiceMetrics }

/// A simple counter metric without keys - all entries merge together.
#[derive(Debug, Clone)]
struct TotalRequests {
    count: u64,
}

/// Aggregated version accumulates all requests.
#[derive(Debug)]
struct AggregatedTotalRequests {
    count: u64,
    entry_count: usize,
}

impl Entry for TotalRequests {
    fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
        writer.value("RequestCount", &self.count);
    }
}

impl Entry for AggregatedTotalRequests {
    fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
        writer.value("RequestCount", &self.count);
        writer.value("AggregatedEntryCount", &(self.entry_count as u64));
    }
}

impl AggregatableEntry for TotalRequests {
    type Key = ();  // No key - all entries merge together
    type Aggregated = AggregatedTotalRequests;

    fn new_aggregated(_key: Self::Key) -> Self::Aggregated {
        AggregatedTotalRequests {
            count: 0,
            entry_count: 0,
        }
    }

    fn key(&self) -> Self::Key {
        ()  // Always return unit
    }
}

impl AggregatedEntry for AggregatedTotalRequests {
    type Key = ();
    type Source = TotalRequests;

    fn aggregate_into(&mut self, entry: &Self::Source) {
        self.count += entry.count;
        self.entry_count += 1;
    }

    fn count(&self) -> usize {
        self.entry_count
    }
}

fn main() {
    let _handle = ServiceMetrics::attach_to_stream(
        Emf::builder("KeylessExample".to_string(), vec![vec![]])
            .build()
            .output_to_makewriter(|| std::io::stdout().lock()),
    );

    // Create multiple entries - they'll all merge together
    let entries = vec![
        TotalRequests { count: 10 },
        TotalRequests { count: 25 },
        TotalRequests { count: 15 },
    ];

    let mut merged = TotalRequests::new_aggregated(());
    for entry in &entries {
        merged.aggregate_into(entry);
    }

    println!("Aggregated {} entries", merged.count());
    println!("Total count: {}", merged.count);

    ServiceMetrics::sink().append(merged);
}
