// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Tools for libraries (and applications) to manage metrics
//!
//! Both as a library vendor (or an application author with a complex application), it is
//! often helpful to combine an return type of some kind with metrics that record information about the event.
//!
//! Making this convenient is the purpose of [`Instrumented`]

use std::ops::DerefMut;

/// Combine a value (`T`) with a metric, (`U`)
///
/// `Instrumented` is typically created with [`instrument`](Instrumented::instrument) or
/// [`instrument_async`](Instrumented::instrument_async). In both cases, you provide the initial/default value for the metrics
/// struct and a closure. The closure will then be passed a mutable reference. This simplifies the task of ensuring that all code
/// branches all return metrics.
pub struct Instrumented<T, U> {
    value: T,
    metrics: U,
}

/// Convenience alias for `Instrumented<Result<T, E>, Metrics>`
pub type Result<T, E, Metrics> = Instrumented<std::result::Result<T, E>, Metrics>;

impl<T, U> Instrumented<T, U> {
    /// Discard the metrics and return the inner value type
    pub fn discard_metrics(self) -> T {
        self.value
    }

    /// Split the `Instrument` into a tuple containing the `(Value, Metrics)` pair.
    pub fn into_parts(self) -> (T, U) {
        (self.value, self.metrics)
    }

    /// Write the metrics from this instrument into a parent metric and return the value
    pub fn split_metrics_to(self, mut target: impl DerefMut<Target = Option<U>>) -> T {
        *target = Some(self.metrics);
        self.value
    }

    /// Construct `Instrumented` directly from parts
    ///
    /// Callers may prefer the ergonomics of [`Instrumented::instrument`], however,
    /// this method exists as a fallback option when that method is not practical.
    pub fn from_parts(value: T, metrics: U) -> Self {
        Self { value, metrics }
    }

    /// Instrument a synchronous function
    pub fn instrument(mut metrics: U, f: impl FnOnce(&mut U) -> T) -> Self {
        let value = f(&mut metrics);
        Self { value, metrics }
    }

    /// Instrument an asynchronous function
    pub async fn instrument_async(mut metrics: U, f: impl AsyncFnOnce(&mut U) -> T) -> Self {
        let value = f(&mut metrics).await;
        Self { value, metrics }
    }

    /// Mutate the metrics after completion
    ///
    /// This enables doing things like incrementing an error code when failure happens.
    ///
    /// See [`Instrumented::on_error`] and [`Instrumented::on_success`].
    pub fn finalize_metrics(mut self, f: impl FnOnce(&T, &mut U)) -> Self {
        f(&self.value, &mut self.metrics);
        self
    }
}

impl<T, E, U> Instrumented<std::result::Result<T, E>, U> {
    /// Provide a callback to mutate metrics on error
    ///
    /// # Examples
    /// ```rust
    /// use metrique::unit_of_work::metrics;
    /// use metrique::instrument::Instrumented;
    ///
    /// #[metrics]
    /// #[derive(Default)]
    /// struct EventMetrics {
    ///   error: bool
    /// }
    ///
    /// fn handle_event(event: String) -> Instrumented<Result<usize, anyhow::Error>, EventMetrics> {
    ///     Instrumented::instrument(EventMetrics::default(), |metrics| {
    ///         let event: usize = event.parse()?;
    ///         Ok(event)
    ///     }).on_error(|_e, metrics|metrics.error = true)
    /// }
    /// ```
    pub fn on_error(self, f: impl FnOnce(&E, &mut U)) -> Self {
        self.finalize_metrics(|res, metrics| {
            if let Err(e) = res {
                f(e, metrics);
            }
        })
    }

    /// Provide a callback to mutate metrics on success
    ///
    /// # Examples
    /// ```rust
    /// use metrique::unit_of_work::metrics;
    /// use metrique::instrument::Instrumented;
    ///
    /// #[metrics]
    /// #[derive(Default)]
    /// struct EventMetrics {
    ///   error: bool,
    ///   success: bool
    /// }
    ///
    /// fn handle_event(event: String) -> Instrumented<Result<usize, anyhow::Error>, EventMetrics> {
    ///     Instrumented::instrument(EventMetrics::default(), |metrics| {
    ///         let event: usize = event.parse()?;
    ///         Ok(event)
    ///     }).on_success(|_v, metrics|metrics.success = true)
    /// }
    /// ```
    pub fn on_success(self, f: impl FnOnce(&T, &mut U)) -> Self {
        self.finalize_metrics(|res, metrics| {
            if let Ok(v) = res {
                f(v, metrics);
            }
        })
    }
}

impl<T, Entry, Sink> Instrumented<T, crate::AppendAndCloseOnDrop<Entry, Sink>>
where
    Entry: crate::CloseEntry,
    Sink: crate::EntrySink<crate::RootMetric<Entry>>,
{
    /// Emit the metrics and return the value
    ///
    /// This is equivalent to calling `into_parts` and dropping the metrics,
    /// but makes the intent explicit that the metrics should be emitted.
    ///
    /// # Examples
    /// ```rust
    /// use metrique::unit_of_work::metrics;
    /// use metrique::instrument::Instrumented;
    /// use metrique::ServiceMetrics;
    /// use metrique_writer_core::global::GlobalEntrySink;
    ///
    /// #[metrics]
    /// #[derive(Default)]
    /// struct EventMetrics {
    ///     success: bool,
    ///     error: bool,
    /// }
    ///
    /// fn process_event(input: &str) -> Result<usize, &'static str> {
    ///     let metrics = EventMetrics::default().append_on_drop(ServiceMetrics::sink());
    ///     Instrumented::instrument(metrics, |_m| {
    ///         if input.is_empty() {
    ///             Err("empty input")
    ///         } else {
    ///             Ok(input.len())
    ///         }
    ///     })
    ///     .on_success(|_val, m| m.success = true)
    ///     .on_error(|_err, m| m.error = true)
    ///     .emit()
    /// }
    /// ```
    pub fn emit(self) -> T {
        let (value, metrics) = self.into_parts();
        drop(metrics); // Explicitly drop to trigger emission
        value
    }
}
