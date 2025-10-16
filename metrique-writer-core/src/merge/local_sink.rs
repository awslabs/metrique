// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Lock-free entry sink for single-threaded aggregation.

use std::collections::HashMap;
use std::cell::RefCell;

use crate::EntrySink;
use super::{AggregatableEntry, AggregatedEntry, AggregateConfig};

/// A lock-free sink that aggregates entries in single-threaded contexts.
///
/// This sink uses RefCell instead of Mutex, making it more efficient for
/// single-threaded use cases but not Send/Sync.
pub struct LocalAggregatingEntrySink<E: AggregatableEntry, S> {
    downstream: S,
    state: RefCell<HashMap<E::Key, E::Aggregated>>,
    config: AggregateConfig,
}

impl<E: AggregatableEntry, S> LocalAggregatingEntrySink<E, S>
where
    S: EntrySink<E::Aggregated>,
{
    /// Create a new lock-free aggregating sink with default configuration.
    pub fn new(downstream: S) -> Self {
        Self::with_config(downstream, AggregateConfig::default())
    }

    /// Create a new lock-free aggregating sink with custom configuration.
    pub fn with_config(downstream: S, config: AggregateConfig) -> Self {
        Self {
            downstream,
            state: RefCell::new(HashMap::new()),
            config,
        }
    }

    /// Flush all accumulated entries to the downstream sink.
    pub fn flush_aggregated(&self) {
        let mut state = self.state.borrow_mut();
        for (_, aggregated) in state.drain() {
            self.downstream.append(aggregated);
        }
    }

    /// Get the number of unique keys currently being aggregated.
    pub fn pending_keys(&self) -> usize {
        self.state.borrow().len()
    }
}

impl<E: AggregatableEntry, S> EntrySink<E> for LocalAggregatingEntrySink<E, S>
where
    S: EntrySink<E::Aggregated>,
{
    fn append(&self, entry: E) {
        let mut state = self.state.borrow_mut();
        
        let key = entry.key();
        let aggregated = state
            .entry(key.clone())
            .or_insert_with(|| E::new_aggregated(key));

        aggregated.aggregate_into(&entry);

        // Check if we should flush
        if state.len() >= self.config.max_entries {
            // Need to drop the borrow before calling flush
            drop(state);
            self.flush_aggregated();
        }
    }

    fn flush_async(&self) -> crate::sink::FlushWait {
        self.flush_aggregated();
        self.downstream.flush_async()
    }
}

// Note: This sink is intentionally NOT Send/Sync since it uses RefCell
// This makes it perfect for single-threaded contexts where locks are unnecessary
