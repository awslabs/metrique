// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Entry sink that merges entries before forwarding to a downstream sink.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::EntrySink;
use super::{MergeableEntry, MergedEntry};

/// A sink that merges entries with the same sample group before forwarding.
///
/// Entries are accumulated in memory and periodically flushed to the downstream sink.
pub struct MergingEntrySink<E: MergeableEntry, S> {
    inner: Arc<MergingEntrySinkInner<E, S>>,
}

struct MergingEntrySinkInner<E: MergeableEntry, S> {
    downstream: S,
    state: Mutex<MergeState<E>>,
    config: MergeConfig,
}

struct MergeState<E: MergeableEntry> {
    /// Merged entries keyed by their Key type
    merged: HashMap<E::Key, E::Merged>,
}

/// Configuration for merging behavior.
#[derive(Debug, Clone)]
pub struct MergeConfig {
    /// Maximum number of entries to merge before flushing.
    pub max_entries: usize,
    /// Sample rate for emitting unmerged entries (0.0 = none, 1.0 = all).
    pub sample_rate: f64,
}

impl Default for MergeConfig {
    fn default() -> Self {
        Self {
            max_entries: 100,
            sample_rate: 0.01,
        }
    }
}

impl<E: MergeableEntry, S> MergingEntrySink<E, S>
where
    S: EntrySink<E::Merged>,
{
    /// Create a new merging sink with default configuration.
    pub fn new(downstream: S) -> Self {
        Self::with_config(downstream, MergeConfig::default())
    }

    /// Create a new merging sink with custom configuration.
    pub fn with_config(downstream: S, config: MergeConfig) -> Self {
        Self {
            inner: Arc::new(MergingEntrySinkInner {
                downstream,
                state: Mutex::new(MergeState {
                    merged: HashMap::new(),
                }),
                config,
            }),
        }
    }

    /// Flush all accumulated merged entries to the downstream sink.
    pub fn flush_merged(&self) {
        let mut state = self.inner.state.lock().unwrap();
        for (_, merged) in state.merged.drain() {
            self.inner.downstream.append(merged);
        }
    }
}

impl<E: MergeableEntry, S> EntrySink<E> for MergingEntrySink<E, S>
where
    S: EntrySink<E::Merged>,
{
    fn append(&self, entry: E) {
        let mut state = self.inner.state.lock().unwrap();
        
        // Extract key from entry
        let key = entry.key();

        // Get or create merged entry for this key
        let merged = state
            .merged
            .entry(key.clone())
            .or_insert_with(|| E::new_merged(key));

        merged.merge_into(&entry);

        // Check if we should flush
        if state.merged.len() >= self.inner.config.max_entries {
            drop(state);
            self.flush_merged();
        }
    }

    fn flush_async(&self) -> crate::sink::FlushWait {
        self.flush_merged();
        self.inner.downstream.flush_async()
    }
}
