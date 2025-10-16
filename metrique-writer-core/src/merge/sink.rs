// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Entry sink that aggregates entries before forwarding to a downstream sink.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::EntrySink;
use super::{AggregatableEntry, AggregatedEntry};

/// A sink that aggregates entries with the same sample group before forwarding.
///
/// Entries are accumulated in memory and periodically flushed to the downstream sink.
pub struct AggregatingEntrySink<E: AggregatableEntry, S> {
    inner: Arc<AggregatingEntrySinkInner<E, S>>,
}

struct AggregatingEntrySinkInner<E: AggregatableEntry, S> {
    downstream: S,
    state: Mutex<AggregateState<E>>,
    config: AggregateConfig,
}

struct AggregateState<E: AggregatableEntry> {
    /// Aggregated entries keyed by their Key type
    aggregated: HashMap<E::Key, E::Aggregated>,
}

/// Configuration for aggregation behavior.
#[derive(Debug, Clone)]
pub struct AggregateConfig {
    /// Maximum number of entries to merge before flushing.
    pub max_entries: usize,
    /// Sample rate for emitting unmerged entries (0.0 = none, 1.0 = all).
    pub sample_rate: f64,
}

impl Default for AggregateConfig {
    fn default() -> Self {
        Self {
            max_entries: 100,
            sample_rate: 0.01,
        }
    }
}

impl<E: AggregatableEntry, S> AggregatingEntrySink<E, S>
where
    S: EntrySink<E::Aggregated>,
{
    /// Create a new aggregating sink with default configuration.
    pub fn new(downstream: S) -> Self {
        Self::with_config(downstream, AggregateConfig::default())
    }

    /// Create a new aggregating sink with custom configuration.
    pub fn with_config(downstream: S, config: AggregateConfig) -> Self {
        Self {
            inner: Arc::new(AggregatingEntrySinkInner {
                downstream,
                state: Mutex::new(AggregateState {
                    aggregated: HashMap::new(),
                }),
                config,
            }),
        }
    }

    /// Flush all accumulated merged entries to the downstream sink.
    pub fn flush_aggregated(&self) {
        let mut state = self.inner.state.lock().unwrap();
        for (_, merged) in state.aggregated.drain() {
            self.inner.downstream.append(merged);
        }
    }
}

impl<E: AggregatableEntry, S> EntrySink<E> for AggregatingEntrySink<E, S>
where
    S: EntrySink<E::Aggregated>,
{
    fn append(&self, entry: E) {
        let mut state = self.inner.state.lock().unwrap();
        
        // Extract key from entry
        let key = entry.key();

        // Get or create merged entry for this key
        let merged = state
            .aggregated
            .entry(key.clone())
            .or_insert_with(|| E::new_aggregated(key));

        merged.aggregate_into(&entry);

        // Check if we should flush
        if state.aggregated.len() >= self.inner.config.max_entries {
            drop(state);
            self.flush_aggregated();
        }
    }

    fn flush_async(&self) -> crate::sink::FlushWait {
        self.flush_aggregated();
        self.inner.downstream.flush_async()
    }
}
