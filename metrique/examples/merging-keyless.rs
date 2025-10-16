// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Example showing keyless merging - all entries merge together regardless of dimensions.

use metrique::emf::Emf;
use metrique::writer::{
    AttachGlobalEntrySinkExt, Entry, EntrySink, EntryWriter, FormatExt, GlobalEntrySink,
    merge::{MergeableEntry, MergedEntry},
    sink::global_entry_sink,
};

global_entry_sink! { ServiceMetrics }

/// A simple counter metric without keys - all entries merge together.
#[derive(Debug, Clone)]
struct TotalRequests {
    count: u64,
}

/// Merged version accumulates all requests.
#[derive(Debug)]
struct MergedTotalRequests {
    count: u64,
    entry_count: usize,
}

impl Entry for TotalRequests {
    fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
        writer.value("RequestCount", &self.count);
    }
}

impl Entry for MergedTotalRequests {
    fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
        writer.value("RequestCount", &self.count);
        writer.value("MergedEntryCount", &(self.entry_count as u64));
    }
}

impl MergeableEntry for TotalRequests {
    type Key = ();  // No key - all entries merge together
    type Merged = MergedTotalRequests;

    fn new_merged(_key: Self::Key) -> Self::Merged {
        MergedTotalRequests {
            count: 0,
            entry_count: 0,
        }
    }

    fn key(&self) -> Self::Key {
        ()  // Always return unit
    }
}

impl MergedEntry for MergedTotalRequests {
    type Key = ();
    type Source = TotalRequests;

    fn merge_into(&mut self, entry: &Self::Source) {
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

    let mut merged = TotalRequests::new_merged(());
    for entry in &entries {
        merged.merge_into(entry);
    }

    println!("Merged {} entries", merged.count());
    println!("Total count: {}", merged.count);

    ServiceMetrics::sink().append(merged);
}
