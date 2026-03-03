// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! A human-readable metrics format for local development and debugging.
//!
//! This module provides [`LocalFormat`], a [`Format`] implementation designed for local
//! development rather than production metric backends. It produces readable output to
//! stdout or any `io::Write` destination.
//!
//! # Output Styles
//!
//! - [`OutputStyle::Pretty`]: YAML-esque key-value pairs with smart unit display
//! - [`OutputStyle::Json`]: JSON objects with unit annotations
//! - [`OutputStyle::MarkdownTable`]: Markdown table for pasting into docs/issues
//!
//! # Histogram Analysis
//!
//! When a metric contains multiple observations (e.g. from aggregation), the format
//! automatically computes percentiles (min, p50, p99, p99.9, max).
//!
//! # How This Format Works (Guide to Implementing a Custom Format)
//!
//! Metrique uses a visitor/double-dispatch pattern to decouple metric data from formatting:
//!
//! 1. The [`Format::format`] method receives an [`Entry`] and an `io::Write` output.
//! 2. The format creates an [`EntryWriter`] — a collector that the entry will write into.
//! 3. It calls `entry.write(&mut writer)`, which triggers the entry to call back into
//!    the writer's methods: `timestamp()`, `value(name, value)`, and `config()`.
//! 4. For each `value()` call, the writer creates a [`ValueWriter`] — a one-shot handler
//!    for a single metric value. The value then calls one of three methods on it:
//!    - `string()` for string properties (dimensions, operation names)
//!    - `metric()` for numeric observations with units and dimensions
//!    - `error()` for validation errors
//! 5. After the entry has written all its fields, the format serializes the collected
//!    data to the output.
//!
//! This two-level visitor pattern (EntryWriter → ValueWriter) allows formats to handle
//! both string properties and numeric metrics uniformly, while giving each value type
//! control over how it presents itself.
//!
//! # Example
//!
//! ```no_run
//! use metrique::local::{LocalFormat, OutputStyle};
//! use metrique::writer::format::FormatExt;
//! use metrique::writer::{AttachGlobalEntrySinkExt, GlobalEntrySink};
//! use metrique::ServiceMetrics;
//!
//! let _handle = ServiceMetrics::attach_to_stream(
//!     LocalFormat::new(OutputStyle::Pretty)
//!         .output_to_makewriter(|| std::io::stderr().lock()),
//! );
//! ```

use std::{borrow::Cow, io, time::SystemTime};

use metrique_writer_core::{
    Distribution, Entry, EntryWriter, MetricFlags, Observation, Unit, ValidationError, Value,
    ValueWriter, format::Format, stream::IoStreamError,
};

/// The output style for [`LocalFormat`].
#[derive(Debug, Clone, Copy, Default)]
#[non_exhaustive]
pub enum OutputStyle {
    /// YAML-esque key-value pairs with smart unit display.
    /// Time values are shown in human-friendly units (e.g. "42.3ms" instead of "0.0423").
    #[default]
    Pretty,
    /// JSON objects. Units are included as a `_unit` suffix field.
    /// Pretty-printed by default; use [`LocalFormat::compact_json`] for single-line output.
    #[non_exhaustive]
    Json {
        /// If true, output compact single-line JSON. Defaults to false (pretty-printed).
        compact: bool,
    },
    /// Markdown table suitable for pasting into GitHub issues or docs.
    MarkdownTable,
}

impl OutputStyle {
    /// Pretty-printed key-value pairs.
    pub fn pretty() -> Self {
        Self::Pretty
    }

    /// Pretty-printed JSON output.
    pub fn json() -> Self {
        Self::Json { compact: false }
    }

    /// Compact single-line JSON output.
    pub fn compact_json() -> Self {
        Self::Json { compact: true }
    }

    /// Markdown table output.
    pub fn markdown_table() -> Self {
        Self::MarkdownTable
    }
}

/// A named percentile to compute from histogram data.
#[derive(Debug, Clone)]
pub struct Percentile {
    label: String,
    fraction: f64,
}

impl Percentile {
    /// Create a percentile from a fraction in `[0.0, 1.0]`.
    ///
    /// The display label is generated automatically: 0.0 → "min", 1.0 → "max",
    /// 0.5 → "p50", 0.99 → "p99", 0.999 → "p99.9", etc.
    pub fn new(fraction: f64) -> Self {
        let fraction = fraction.clamp(0.0, 1.0);
        let label = if fraction == 0.0 {
            "min".to_owned()
        } else if fraction == 1.0 {
            "max".to_owned()
        } else {
            let pct = fraction * 100.0;
            if pct == pct.floor() {
                format!("p{}", pct as u64)
            } else {
                format!("p{pct}")
            }
        };
        Self { label, fraction }
    }
}

/// Default percentiles: min, p50, p99, p99.9, max
fn default_percentiles() -> Vec<Percentile> {
    vec![
        Percentile::new(0.0),
        Percentile::new(0.5),
        Percentile::new(0.99),
        Percentile::new(0.999),
        Percentile::new(1.0),
    ]
}

/// A human-readable metrics format for local development.
///
/// See the [module-level docs](self) for details.
#[derive(Debug, Clone)]
pub struct LocalFormat {
    style: OutputStyle,
    percentiles: Vec<Percentile>,
}

impl LocalFormat {
    /// Create a new `LocalFormat` with the given output style.
    pub fn new(style: OutputStyle) -> Self {
        Self {
            style,
            percentiles: default_percentiles(),
        }
    }

    /// Create a `LocalFormat` with pretty-printed JSON output.
    pub fn json() -> Self {
        Self::new(OutputStyle::json())
    }

    /// Create a `LocalFormat` with compact (single-line) JSON output.
    pub fn compact_json() -> Self {
        Self::new(OutputStyle::compact_json())
    }

    /// Override which percentiles are computed for histogram data.
    ///
    /// ```
    /// # use metrique::local::{LocalFormat, Percentile};
    /// let format = LocalFormat::json()
    ///     .percentiles(vec![Percentile::new(0.0), Percentile::new(0.5), Percentile::new(1.0)]);
    /// ```
    pub fn percentiles(mut self, percentiles: Vec<Percentile>) -> Self {
        self.percentiles = percentiles;
        self
    }
}

// ── Format implementation ──────────────────────────────────────────────
//
// `Format` is the core trait that all metrique formatters implement. It receives an
// opaque `Entry` and must serialize it to an `io::Write` output. Since entries are
// opaque, we use the visitor pattern: create an `EntryWriter` that collects data,
// let the entry write into it, then serialize the collected data.

impl Format for LocalFormat {
    fn format(
        &mut self,
        entry: &impl Entry,
        output: &mut impl io::Write,
    ) -> Result<(), IoStreamError> {
        // Step 1: Create our collector. This implements `EntryWriter` so the entry
        // can write its fields into it.
        let mut collector = Collector::default();

        // Step 2: Let the entry write all its fields into our collector.
        // This triggers callbacks to `collector.timestamp()`, `collector.value()`, etc.
        entry.write(&mut collector);

        // Step 3: Serialize the collected data in the chosen style.
        match self.style {
            OutputStyle::Pretty => write_pretty(output, &collector, &self.percentiles)?,
            OutputStyle::Json { compact } => {
                write_json(output, &collector, &self.percentiles, compact)?
            }
            OutputStyle::MarkdownTable => {
                write_markdown_table(output, &collector, &self.percentiles)?;
            }
        }

        Ok(())
    }
}

// ── Data collection ────────────────────────────────────────────────────
//
// The collector implements `EntryWriter` to gather all fields from an entry.
// For each field, it creates a `FieldValueWriter` (implementing `ValueWriter`)
// that captures whether the field is a string property or a numeric metric.

/// Collected representation of a single entry's data.
#[derive(Default)]
struct Collector {
    timestamp: Option<SystemTime>,
    fields: Vec<Field>,
}

/// A single field collected from an entry.
struct Field {
    name: String,
    data: FieldData,
}

/// The data for a field — either a string property or a numeric metric.
enum FieldData {
    String(String),
    Metric {
        observations: Vec<WeightedObservation>,
        unit: Unit,
        dimensions: Vec<(String, String)>,
        is_distribution: bool,
    },
}

/// An observation with an associated weight for percentile computation.
#[derive(Debug, Clone, Copy)]
struct WeightedObservation {
    value: f64,
    weight: u64,
}

// `EntryWriter` is the first level of the visitor pattern. The format provides this
// to the entry, and the entry calls `timestamp()`, `value()`, and `config()` on it.
impl<'a> EntryWriter<'a> for Collector {
    fn timestamp(&mut self, timestamp: SystemTime) {
        self.timestamp = Some(timestamp);
    }

    // For each field, we create a `FieldValueWriter` that will capture the value.
    // The entry calls `value.write(writer)` which dispatches to either `string()`,
    // `metric()`, or `error()` on our writer.
    fn value(&mut self, name: impl Into<Cow<'a, str>>, value: &(impl Value + ?Sized)) {
        let name = name.into().into_owned();
        let mut data = None;
        value.write(FieldValueWriter(&mut data));
        if let Some(data) = data {
            self.fields.push(Field { name, data });
        }
        // If data is None, the value wrote nothing (e.g. Option::None) — skip it.
    }

    fn config(&mut self, _config: &'a dyn metrique_writer_core::EntryConfig) {
        // LocalFormat ignores format-specific config (like EMF dimension sets).
    }
}

/// `ValueWriter` is the second level of the visitor pattern. It's created per-field
/// and consumed when the value writes to it. The `Sized` bound on `self` means each
/// ValueWriter is used exactly once.
struct FieldValueWriter<'a>(&'a mut Option<FieldData>);

impl ValueWriter for FieldValueWriter<'_> {
    fn string(self, value: &str) {
        *self.0 = Some(FieldData::String(value.to_owned()));
    }

    fn metric<'a>(
        self,
        distribution: impl IntoIterator<Item = Observation>,
        unit: Unit,
        dimensions: impl IntoIterator<Item = (&'a str, &'a str)>,
        _flags: MetricFlags<'_>,
    ) {
        let is_distribution = _flags.downcast::<Distribution>().is_some();
        let mut observations = Vec::new();
        for obs in distribution {
            match obs {
                Observation::Unsigned(v) => observations.push(WeightedObservation {
                    value: v as f64,
                    weight: 1,
                }),
                Observation::Floating(v) => observations.push(WeightedObservation {
                    value: v,
                    weight: 1,
                }),
                Observation::Repeated { total, occurrences } => {
                    if occurrences > 0 {
                        // Repeated observations represent `occurrences` samples that sum
                        // to `total`. We store the average with the full weight so
                        // percentile computation accounts for the count without
                        // allocating one entry per occurrence.
                        observations.push(WeightedObservation {
                            value: total / occurrences as f64,
                            weight: occurrences,
                        });
                    }
                }
                // Observation is #[non_exhaustive]; unknown variants are intentionally
                // skipped — this is a best-effort debug format, not a lossless serializer.
                _ => {}
            }
        }
        let dimensions = dimensions
            .into_iter()
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
            .collect();
        *self.0 = Some(FieldData::Metric {
            observations,
            unit,
            dimensions,
            is_distribution,
        });
    }

    fn error(self, error: ValidationError) {
        *self.0 = Some(FieldData::String(format!("ERROR: {error}")));
    }
}

// ── Percentile computation ─────────────────────────────────────────────

fn compute_percentiles<'a>(
    observations: &[WeightedObservation],
    percentiles: &'a [Percentile],
) -> Vec<(&'a str, f64)> {
    if observations.is_empty() {
        return Vec::new();
    }
    let mut sorted: Vec<WeightedObservation> = observations.to_vec();
    sorted.sort_by(|a, b| {
        a.value
            .partial_cmp(&b.value)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let total_weight: u64 = sorted.iter().map(|o| o.weight).sum();
    percentiles
        .iter()
        .map(|p| {
            let target = if p.fraction <= 0.0 {
                0
            } else if p.fraction >= 1.0 {
                total_weight
            } else {
                (total_weight as f64 * p.fraction).ceil() as u64
            };
            let mut cumulative = 0u64;
            let mut value = sorted[0].value;
            for obs in &sorted {
                cumulative += obs.weight;
                value = obs.value;
                if cumulative >= target {
                    break;
                }
            }
            (p.label.as_str(), value)
        })
        .collect()
}

/// Total number of observations (accounting for weights).
fn total_count(observations: &[WeightedObservation]) -> u64 {
    observations.iter().map(|o| o.weight).sum()
}

// ── Smart unit display ─────────────────────────────────────────────────
//
// In Pretty mode, we display values in the most readable unit. For example,
// a metric in Microseconds with value 1_500_000 is shown as "1.5s" rather than
// "1500000μs".

/// Format a value with smart unit scaling for Pretty mode.
fn format_pretty_value(value: f64, unit: Unit) -> String {
    match unit {
        Unit::Second(scale) => {
            // Convert to seconds: value is in the scaled unit (e.g. milliseconds),
            // so divide by the reduction factor to get seconds.
            let seconds = value / scale.reduction_factor() as f64;
            format_duration_smart(seconds)
        }
        Unit::Byte(scale) => {
            // Convert to bytes: value is in the scaled unit (e.g. megabytes),
            // so multiply by the expansion factor to get bytes.
            let bytes = value * scale.expansion_factor() as f64;
            format_bytes_smart(bytes)
        }
        Unit::Percent => format!("{value:.1}%"),
        Unit::Count => {
            if value == value.floor() {
                format!("{}", value as i64)
            } else {
                format!("{value:.2}")
            }
        }
        Unit::None => {
            if value == value.floor() && value.abs() < i64::MAX as f64 {
                format!("{}", value as i64)
            } else {
                format!("{value:.3}")
            }
        }
        _ => format!("{value:.3} {unit}"),
    }
}

fn format_duration_smart(seconds: f64) -> String {
    if seconds == 0.0 {
        return "0s".to_owned();
    }
    let abs = seconds.abs();
    if abs >= 1.0 {
        format!("{seconds:.3}s")
    } else if abs >= 0.001 {
        format!("{:.3}ms", seconds * 1_000.0)
    } else {
        format!("{:.3}μs", seconds * 1_000_000.0)
    }
}

fn format_bytes_smart(bytes: f64) -> String {
    if bytes == 0.0 {
        return "0B".to_owned();
    }
    let abs = bytes.abs();
    if abs >= 1_000_000_000.0 {
        format!("{:.2}GB", bytes / 1_000_000_000.0)
    } else if abs >= 1_000_000.0 {
        format!("{:.2}MB", bytes / 1_000_000.0)
    } else if abs >= 1_000.0 {
        format!("{:.2}KB", bytes / 1_000.0)
    } else {
        format!("{bytes:.0}B")
    }
}

/// Format a value for JSON mode — keep the raw number, unit goes in a separate field.
fn format_json_value(value: f64) -> serde_json::Value {
    if value == value.floor() && value.abs() < i64::MAX as f64 {
        serde_json::Value::Number(serde_json::Number::from(value as i64))
    } else {
        serde_json::Number::from_f64(value)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null)
    }
}

// ── Pretty output ──────────────────────────────────────────────────────

/// Format a SystemTime as an ISO 8601 string using jiff.
fn format_timestamp(ts: SystemTime) -> String {
    jiff::Timestamp::try_from(ts)
        .map(|t| t.to_string())
        .unwrap_or_else(|_| format!("{ts:?}"))
}

fn write_pretty(
    output: &mut impl io::Write,
    collector: &Collector,
    percentiles: &[Percentile],
) -> io::Result<()> {
    writeln!(output, "---")?;
    if let Some(ts) = collector.timestamp {
        writeln!(output, "  timestamp: {}", format_timestamp(ts))?;
    }
    for field in &collector.fields {
        match &field.data {
            FieldData::String(s) => {
                writeln!(output, "  {}: {s}", field.name)?;
            }
            FieldData::Metric {
                observations,
                unit,
                dimensions,
                is_distribution,
            } => {
                if !dimensions.is_empty() {
                    let dims: Vec<String> =
                        dimensions.iter().map(|(k, v)| format!("{k}={v}")).collect();
                    writeln!(output, "  {} [{}]:", field.name, dims.join(", "))?;
                }
                let show_histogram = *is_distribution || total_count(observations) > 1;
                if !show_histogram {
                    // Single value — show inline
                    let val = observations.first().map(|o| o.value).unwrap_or(0.0);
                    if dimensions.is_empty() {
                        writeln!(
                            output,
                            "  {}: {}",
                            field.name,
                            format_pretty_value(val, *unit)
                        )?;
                    } else {
                        writeln!(output, "    {}", format_pretty_value(val, *unit))?;
                    }
                } else {
                    // Multiple observations — show percentiles
                    let count = total_count(observations);
                    if dimensions.is_empty() {
                        writeln!(output, "  {} ({count} samples):", field.name)?;
                    } else {
                        writeln!(output, "    ({count} samples):")?;
                    }
                    for (label, val) in compute_percentiles(observations, percentiles) {
                        writeln!(output, "    {label}: {}", format_pretty_value(val, *unit))?;
                    }
                }
            }
        }
    }
    Ok(())
}

// ── JSON output ────────────────────────────────────────────────────────

fn write_json(
    output: &mut impl io::Write,
    collector: &Collector,
    percentiles: &[Percentile],
    compact: bool,
) -> io::Result<()> {
    let mut map = serde_json::Map::new();

    if let Some(ts) = collector.timestamp {
        if let Ok(dur) = ts.duration_since(SystemTime::UNIX_EPOCH) {
            map.insert("timestamp".to_owned(), format_json_value(dur.as_secs_f64()));
            map.insert(
                "timestamp_iso".to_owned(),
                serde_json::Value::String(format_timestamp(ts)),
            );
        }
    }

    for field in &collector.fields {
        match &field.data {
            FieldData::String(s) => {
                map.insert(field.name.clone(), serde_json::Value::String(s.clone()));
            }
            FieldData::Metric {
                observations,
                unit,
                dimensions,
                is_distribution,
            } => {
                let show_histogram = *is_distribution || total_count(observations) > 1;
                if !show_histogram {
                    let val = observations.first().map(|o| o.value).unwrap_or(0.0);
                    map.insert(field.name.clone(), format_json_value(val));
                } else {
                    let pcts = compute_percentiles(observations, percentiles);
                    let count = total_count(observations);
                    let mut pct_map = serde_json::Map::new();
                    pct_map.insert("count".to_owned(), serde_json::Value::Number(count.into()));
                    for (label, val) in pcts {
                        pct_map.insert(label.to_owned(), format_json_value(val));
                    }
                    map.insert(field.name.clone(), serde_json::Value::Object(pct_map));
                }
                // Unit annotation: include as a sibling field when not None
                if *unit != Unit::None {
                    map.insert(
                        format!("{}_unit", field.name),
                        serde_json::Value::String(unit.to_string()),
                    );
                }
                if !dimensions.is_empty() {
                    let dim_map: serde_json::Map<String, serde_json::Value> = dimensions
                        .iter()
                        .map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone())))
                        .collect();
                    map.insert(
                        format!("{}_dimensions", field.name),
                        serde_json::Value::Object(dim_map),
                    );
                }
            }
        }
    }

    let obj = serde_json::Value::Object(map);
    let json = if compact {
        serde_json::to_string(&obj)
    } else {
        serde_json::to_string_pretty(&obj)
    }
    .map_err(io::Error::other)?;
    writeln!(output, "{json}")?;
    Ok(())
}

// ── Markdown table output ──────────────────────────────────────────────

fn write_markdown_table(
    output: &mut impl io::Write,
    collector: &Collector,
    percentiles: &[Percentile],
) -> io::Result<()> {
    // Collect rows: (name, value_string)
    let mut rows: Vec<(String, String)> = Vec::new();

    if let Some(ts) = collector.timestamp {
        rows.push(("timestamp".to_owned(), format_timestamp(ts)));
    }

    for field in &collector.fields {
        match &field.data {
            FieldData::String(s) => {
                rows.push((field.name.clone(), s.clone()));
            }
            FieldData::Metric {
                observations,
                unit,
                dimensions,
                is_distribution,
            } => {
                for (k, v) in dimensions {
                    rows.push((format!("{}.{k}", field.name), v.clone()));
                }
                let show_histogram = *is_distribution || total_count(observations) > 1;
                if !show_histogram {
                    let val = observations.first().map(|o| o.value).unwrap_or(0.0);
                    rows.push((field.name.clone(), format_pretty_value(val, *unit)));
                } else {
                    for (label, val) in compute_percentiles(observations, percentiles) {
                        rows.push((
                            format!("{}.{label}", field.name),
                            format_pretty_value(val, *unit),
                        ));
                    }
                    rows.push((
                        format!("{}.count", field.name),
                        total_count(observations).to_string(),
                    ));
                }
            }
        }
    }

    // Compute column widths
    let name_width = rows.iter().map(|(n, _)| n.len()).max().unwrap_or(4).max(4);
    let value_width = rows.iter().map(|(_, v)| v.len()).max().unwrap_or(5).max(5);

    writeln!(
        output,
        "| {:<name_width$} | {:<value_width$} |",
        "Name", "Value"
    )?;
    writeln!(output, "| {:-<name_width$} | {:-<value_width$} |", "", "")?;
    for (name, value) in &rows {
        writeln!(
            output,
            "| {:<name_width$} | {:<value_width$} |",
            name, value
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use metrique_writer_core::unit::{self, NegativeScale, UnitTag};

    #[test]
    fn test_pretty_single_value() {
        let mut format = LocalFormat::new(OutputStyle::Pretty);
        let entry = SimpleEntry {
            name: "GetUser",
            latency_ms: 42.5,
        };
        let mut buf = Vec::new();
        format.format(&entry, &mut buf).unwrap();
        let output = String::from_utf8(buf).unwrap();
        assert!(output.contains("operation: GetUser"));
        assert!(output.contains("latency: 42.500ms"));
    }

    #[test]
    fn test_json_output() {
        let mut format = LocalFormat::json();
        let entry = SimpleEntry {
            name: "GetUser",
            latency_ms: 42.5,
        };
        let mut buf = Vec::new();
        format.format(&entry, &mut buf).unwrap();
        let output = String::from_utf8(buf).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(output.trim()).unwrap();
        assert_eq!(parsed["operation"], "GetUser");
        assert_eq!(parsed["latency_unit"], "Milliseconds");
    }

    #[test]
    fn test_markdown_table() {
        let mut format = LocalFormat::new(OutputStyle::MarkdownTable);
        let entry = SimpleEntry {
            name: "GetUser",
            latency_ms: 42.5,
        };
        let mut buf = Vec::new();
        format.format(&entry, &mut buf).unwrap();
        let output = String::from_utf8(buf).unwrap();
        assert!(output.contains("| Name"));
        assert!(output.contains("operation"));
    }

    #[test]
    fn test_histogram_percentiles() {
        let mut format = LocalFormat::new(OutputStyle::Pretty);
        let entry = HistogramEntry {
            values: vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0],
        };
        let mut buf = Vec::new();
        format.format(&entry, &mut buf).unwrap();
        let output = String::from_utf8(buf).unwrap();
        assert!(output.contains("10 samples"));
        assert!(output.contains("min:"));
        assert!(output.contains("max:"));
        assert!(output.contains("p50:"));
        assert!(output.contains("p99:"));
    }

    #[test]
    fn test_smart_duration_display() {
        assert_eq!(format_duration_smart(0.0), "0s");
        assert_eq!(format_duration_smart(1.5), "1.500s");
        assert_eq!(format_duration_smart(0.042), "42.000ms");
        assert_eq!(format_duration_smart(0.000_042), "42.000μs");
    }

    #[test]
    fn test_smart_bytes_display() {
        assert_eq!(format_bytes_smart(0.0), "0B");
        assert_eq!(format_bytes_smart(512.0), "512B");
        assert_eq!(format_bytes_smart(1_500.0), "1.50KB");
        assert_eq!(format_bytes_smart(2_500_000.0), "2.50MB");
        assert_eq!(format_bytes_smart(3_000_000_000.0), "3.00GB");
    }

    #[test]
    fn test_percentile_computation() {
        let data: Vec<WeightedObservation> = (1..=100)
            .map(|i| WeightedObservation {
                value: i as f64,
                weight: 1,
            })
            .collect();
        let percentiles = default_percentiles();
        let pcts = compute_percentiles(&data, &percentiles);
        assert_eq!(pcts[0], ("min", 1.0));
        assert_eq!(pcts[4], ("max", 100.0));
        // p50 should be around 50
        assert!((pcts[1].1 - 50.0).abs() <= 1.0);
    }

    // ── Test helpers: manual Entry impls ───────────────────────────────

    struct SimpleEntry {
        name: &'static str,
        latency_ms: f64,
    }

    impl Entry for SimpleEntry {
        fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
            writer.value("operation", self.name);
            writer.value("latency", &MillisValue(self.latency_ms));
        }
    }

    struct MillisValue(f64);
    impl Value for MillisValue {
        fn write(&self, writer: impl ValueWriter) {
            writer.metric(
                [Observation::Floating(self.0)],
                Unit::Second(NegativeScale::Milli),
                [],
                MetricFlags::empty(),
            );
        }
    }

    struct HistogramEntry {
        values: Vec<f64>,
    }

    impl Entry for HistogramEntry {
        fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
            writer.value("latency", &HistogramValue(&self.values));
        }
    }

    struct HistogramValue<'a>(&'a [f64]);
    impl Value for HistogramValue<'_> {
        fn write(&self, writer: impl ValueWriter) {
            writer.metric(
                self.0.iter().copied().map(Observation::Floating),
                unit::None::UNIT,
                [],
                MetricFlags::empty(),
            );
        }
    }

    /// A single Repeated observation with the Distribution flag — this is what
    /// HistogramClosed emits when all samples collapse into one bucket.
    #[test]
    fn distribution_flag_forces_histogram_display() {
        let mut format = LocalFormat::new(OutputStyle::Pretty);
        let entry = SingleRepeatedDistribution;
        let mut buf = Vec::new();
        format.format(&entry, &mut buf).unwrap();
        let output = String::from_utf8(buf).unwrap();
        // Even though there's only one observation entry, the Distribution flag
        // should cause it to render as a histogram with percentiles.
        assert!(output.contains("1000 samples"), "output was: {output}");
        assert!(output.contains("min:"));
        assert!(output.contains("max:"));
    }

    struct SingleRepeatedDistribution;
    impl Entry for SingleRepeatedDistribution {
        fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
            writer.value("latency", &RepeatedDistributionValue);
        }
    }

    struct RepeatedDistributionValue;
    impl Value for RepeatedDistributionValue {
        fn write(&self, writer: impl ValueWriter) {
            writer.metric(
                [Observation::Repeated {
                    total: 5000.0,
                    occurrences: 1000,
                }],
                unit::None::UNIT,
                [],
                MetricFlags::upcast(&Distribution),
            );
        }
    }
}
