// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Observer traits for capturing lifecycle events from the sinks in this crate.
//!
//! These traits let callers receive the same events that the built-in
//! `metrics.rs` integration captures, without depending on `metrics.rs`. Each
//! observer has a single method that receives an event enum, so an
//! implementation matches on the events it cares about and ignores the rest. A
//! blanket impl makes any closure of the right shape an observer. New events may
//! be added in the future as additional enum variants; the enums are
//! `#[non_exhaustive]`, so existing implementations will continue to compile.

/// A lifecycle event emitted by a [`BackgroundQueue`].
///
/// Delivered to a [`BackgroundQueueObserver`] alongside the queue's configured
/// metric name. The queue constructs these events; an observer only ever matches
/// on them.
///
/// [`BackgroundQueue`]: super::BackgroundQueue
#[cfg(feature = "background-queue")]
#[non_exhaustive]
pub enum BackgroundQueueEvent {
    /// An entry was dropped because the queue was full.
    #[non_exhaustive]
    QueueOverflow,
    /// Entries were successfully written to the underlying stream since the
    /// previous flush.
    #[non_exhaustive]
    MetricsEmitted {
        /// Number of entries written since the previous flush.
        count: u64,
    },
    /// IO errors occurred while writing to the underlying stream since the
    /// previous flush.
    #[non_exhaustive]
    IoErrors {
        /// Number of IO errors since the previous flush.
        count: u64,
    },
    /// Validation errors occurred while writing to the underlying stream since
    /// the previous flush.
    #[non_exhaustive]
    ValidationErrors {
        /// Number of validation errors since the previous flush.
        count: u64,
    },
    /// A flush cycle has completed.
    #[non_exhaustive]
    FlushComplete {
        /// Percentage of the cycle the background thread spent parked waiting
        /// for work.
        idle_percent: u32,
        /// Queue length sampled at the end of the cycle.
        queue_len: u32,
    },
}

/// Receives lifecycle events from a [`BackgroundQueue`].
///
/// Plug an implementation into [`BackgroundQueueBuilder::observer`] to capture
/// queue overflow events, per-flush counts of emitted entries and errors, and
/// per-flush idle/length samples. Each event is delivered with the queue's
/// configured metric name as `queue`.
///
/// Any `Fn(&str, BackgroundQueueEvent)` is an observer via the blanket impl, so
/// a closure works wherever a `BackgroundQueueObserver` is expected.
///
/// [`BackgroundQueue`]: super::BackgroundQueue
/// [`BackgroundQueueBuilder::observer`]: super::BackgroundQueueBuilder::observer
#[cfg(feature = "background-queue")]
pub trait BackgroundQueueObserver: Send + Sync {
    /// Handle a single [`BackgroundQueueEvent`] for the queue named `queue`.
    fn on_event(&self, queue: &str, event: BackgroundQueueEvent);
}

#[cfg(feature = "background-queue")]
impl<F: Fn(&str, BackgroundQueueEvent) + Send + Sync> BackgroundQueueObserver for F {
    fn on_event(&self, queue: &str, event: BackgroundQueueEvent) {
        self(queue, event)
    }
}

/// A lifecycle event emitted by a [`FlushImmediately`] sink.
///
/// Delivered to a [`FlushImmediatelyObserver`] alongside the sink's configured
/// metric name. The sink constructs these events; an observer only ever matches
/// on them.
///
/// [`FlushImmediately`]: super::FlushImmediately
#[non_exhaustive]
pub enum FlushImmediatelyEvent {
    /// IO errors occurred while writing or flushing the underlying stream.
    #[non_exhaustive]
    IoErrors {
        /// Number of IO errors.
        count: u64,
    },
    /// Validation errors occurred while writing to the underlying stream.
    #[non_exhaustive]
    ValidationErrors {
        /// Number of validation errors.
        count: u64,
    },
    /// A flush has completed.
    #[non_exhaustive]
    FlushComplete {
        /// Time spent flushing the underlying stream.
        duration: std::time::Duration,
    },
}

/// Receives lifecycle events from a [`FlushImmediately`] sink.
///
/// Plug an implementation into [`FlushImmediatelyBuilder::observer`] to capture
/// per-flush timing and write/flush errors. Each event is delivered with the
/// sink's configured metric name as `sink`.
///
/// Any `Fn(&str, FlushImmediatelyEvent)` is an observer via the blanket impl, so
/// a closure works wherever a `FlushImmediatelyObserver` is expected.
///
/// [`FlushImmediately`]: super::FlushImmediately
/// [`FlushImmediatelyBuilder::observer`]: super::FlushImmediatelyBuilder::observer
pub trait FlushImmediatelyObserver: Send + Sync {
    /// Handle a single [`FlushImmediatelyEvent`] for the sink named `sink`.
    fn on_event(&self, sink: &str, event: FlushImmediatelyEvent);
}

impl<F: Fn(&str, FlushImmediatelyEvent) + Send + Sync> FlushImmediatelyObserver for F {
    fn on_event(&self, sink: &str, event: FlushImmediatelyEvent) {
        self(sink, event)
    }
}
