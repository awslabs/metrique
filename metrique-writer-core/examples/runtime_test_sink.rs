// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Example demonstrating the runtime-scoped test sink guard.
//!
//! This shows how to use `set_test_sink_on_runtime` which creates a Send + Sync
//! guard that works across threads within a tokio runtime.

use metrique_writer::GlobalEntrySink;
use metrique_writer::sink::global_entry_sink;
use metrique_writer::test_util::{TestEntrySink, test_entry_sink};
use metrique_writer_core::entry::EmptyEntry;

global_entry_sink! { TestMetrics }

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() {
    let TestEntrySink { inspector, sink } = test_entry_sink();

    // This guard is Send + Sync and works across all threads in the runtime
    let _guard = TestMetrics::set_test_sink_on_runtime(sink);

    // Spawn tasks on different threads
    let handles: Vec<_> = (0..4)
        .map(|_| {
            tokio::spawn(async move {
                TestMetrics::append(EmptyEntry);
            })
        })
        .collect();

    for handle in handles {
        handle.await.unwrap();
    }

    let entries = inspector.entries();
    println!("Captured {} entries from multiple threads", entries.len());
    assert_eq!(entries.len(), 4);
}
