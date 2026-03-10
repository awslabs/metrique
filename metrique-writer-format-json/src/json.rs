// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::borrow::Cow;
use std::io;
use std::time::SystemTime;

use metrique_writer::sample::DefaultRng;
use metrique_writer_core::entry::EntryConfig;
use metrique_writer_core::format::Format;
use metrique_writer_core::sample::SampledFormat;
use metrique_writer_core::stream::IoStreamError;
use metrique_writer_core::value::{MetricFlags, Observation, Value, ValueWriter};
use metrique_writer_core::{Entry, EntryWriter, Unit, ValidationError, ValidationErrorBuilder};
use rand::rngs::ThreadRng;
use rand::{Rng, RngCore};

/// How to format metric observations in JSON output.
///
/// There are two fundamentally different output shapes:
///
/// - [`Scalar`](ObservationFormat::Scalar): Human-friendly output using `"value"` / `"values"`
///   fields. A [`RepeatedFormat`] controls how aggregated observations are rendered.
/// - [`Histogram`](ObservationFormat::Histogram): Statistical output using parallel
///   `"values"` / `"counts"` arrays, preserving per-observation multiplicity.
///
/// Histogram contract:
/// - `values` and `counts` are parallel arrays.
/// - For each index `i`, `counts[i]` is the frequency/weight of `values[i]`.
/// - Expanded representation is equivalent to repeating each `values[i]`
///   `counts[i]` times.
/// - Non-finite floating-point observations are encoded as `null` in `values`.
///   Their corresponding `counts` entry is still emitted.
///
/// ## Example
///
/// Given one entry with sampled multiplicity = `5`:
/// - property: `Operation = "GetItem"`
/// - metric `Latency = Observation::Repeated { total: 150.0, occurrences: 3 }`
/// - metric `PayloadSize = [10, 20]` (two floating-point observations)
///
/// #### `Scalar(RepeatedFormat::TotalAndCount)` (default):
/// ```json
/// {
///   "metrics": {
///     "Latency": { "value": { "total": 150, "count": 15 } },
///     "PayloadSize": { "values": [10, 20] }
///   },
///   "properties": { "Operation": "GetItem" }
/// }
/// ```
///
/// #### `Scalar(RepeatedFormat::Mean)`:
/// ```json
/// {
///   "metrics": {
///     "Latency": { "value": 50 },
///     "PayloadSize": { "values": [10, 20] }
///   },
///   "properties": { "Operation": "GetItem" }
/// }
/// ```
///
/// #### `Histogram`:
/// ```json
/// {
///   "metrics": {
///     "Latency": { "values": [50], "counts": [15] },
///     "PayloadSize": { "values": [10, 20], "counts": [5, 5] }
///   },
///   "properties": { "Operation": "GetItem" }
/// }
/// ```
#[derive(Debug, Clone, Copy)]
pub enum ObservationFormat {
    /// Emit single observations as `"value": X`, multiple as `"values": [...]`.
    ///
    /// The inner [`RepeatedFormat`] controls how `Observation::Repeated` values
    /// are rendered (see its docs for details).
    ///
    /// Note: when sampling is used, scalar non-repeated observations do not
    /// include an explicit multiplicity field. If sampled JSON output is consumed
    /// directly for weighted aggregation, prefer [`ObservationFormat::Histogram`].
    Scalar(RepeatedFormat),
    /// Emit all metrics as `{"values": [...], "counts": [...]}`.
    ///
    /// Each observation gets a corresponding count entry, i.e. `values` and `counts` are parallel arrays
    /// where `counts[i]` is the frequency/weight of `values[i]`.
    ///
    /// For simple observations, each count is the sampling multiplicity (or `1`).
    /// For repeated observations, the value is the mean and the count is
    /// `occurrences * multiplicity`.
    Histogram,
}

/// How to render `Observation::Repeated` in [`ObservationFormat::Scalar`] mode.
///
/// A repeated observation aggregates multiple measurements into a single
/// `{ total, occurrences }` pair (e.g. from a histogram bucket or timer).
///
/// Given `Observation::Repeated { total: 150.0, occurrences: 3 }`:
///
/// - [`TotalAndCount`](RepeatedFormat::TotalAndCount):
///   `{"total": 150, "count": 3}`
/// - [`Mean`](RepeatedFormat::Mean):
///   `50`
#[derive(Debug, Clone, Copy, Default)]
pub enum RepeatedFormat {
    /// Emit as `{"total": f64, "count": u64}`.
    #[default]
    TotalAndCount,
    /// Emit the mean (`total / occurrences`) as a scalar.
    Mean,
}

impl Default for ObservationFormat {
    fn default() -> Self {
        Self::Scalar(RepeatedFormat::default())
    }
}

// Maximum buffer size before shrinking on clear. Prevents one large entry from
// permanently bloating memory.
const MAX_BUF_RETAIN: usize = 1024 * 1024;

/// A pure JSON formatter for metrique metrics.
///
/// Outputs one JSON object per entry as a single line, followed by a newline.
///
/// The output structure is:
/// ```json
/// {
///   "timestamp": 1705312800000,
///   "metrics": {
///     "Latency": { "value": 42.5, "unit": "Milliseconds" },
///     "Count": { "value": 10 }
///   },
///   "properties": {
///     "Operation": "GetItem",
///     "Region": "us-east-1"
///   }
/// }
/// ```
///
/// ## Handling of non-finite floating-point values
///
/// Infinities are clamped to `f64::MAX` / `-f64::MAX`. NaN observations are
/// skipped entirely.
#[derive(Debug)]
pub struct Json {
    observation_format: ObservationFormat,
    // Reusable string buffers, cleared between entries, capacity stays warm.
    // Each entry writes ,"key":value fragments into these. The leading comma is
    // stripped when assembling the final output.
    metrics_buf: String,
    properties_buf: String,
    // Used for histogram counts
    counts_buf: String,
}

impl Json {
    /// Create a new JSON formatter with default settings.
    pub fn new() -> Self {
        Self {
            observation_format: ObservationFormat::default(),
            metrics_buf: String::with_capacity(2048),
            properties_buf: String::with_capacity(2048),
            counts_buf: String::with_capacity(256),
        }
    }

    /// Set how metric observations are formatted.
    pub fn with_observation_format(mut self, format: ObservationFormat) -> Self {
        self.observation_format = format;
        self
    }

    /// Wrap this formatter with support for sampling using the default RNG.
    ///
    /// When sampling is active, metrics are emitted with a multiplicity that
    /// compensates for dropped entries, keeping aggregate statistics unbiased.
    ///
    /// For sampled output that will be consumed directly from JSON, prefer
    /// [`ObservationFormat::Histogram`] so multiplicity is represented explicitly
    /// in `counts`.
    pub fn with_sampling(self) -> SampledJson {
        SampledJson {
            json: self,
            rng: Default::default(),
        }
    }

    /// Wrap this formatter with support for sampling using an explicit RNG.
    ///
    /// For sampled output that will be consumed directly from JSON, prefer
    /// [`ObservationFormat::Histogram`] so multiplicity is represented explicitly
    /// in `counts`.
    pub fn with_sampling_and_rng<R>(self, rng: R) -> SampledJson<R> {
        SampledJson { json: self, rng }
    }

    fn format_with_multiplicity(
        &mut self,
        entry: &impl Entry,
        output: &mut impl io::Write,
        multiplicity: Option<u64>,
    ) -> Result<(), IoStreamError> {
        self.clear_buffers();

        let mut writer = JsonEntryWriter {
            timestamp: None,
            metrics_buf: &mut self.metrics_buf,
            properties_buf: &mut self.properties_buf,
            counts_buf: &mut self.counts_buf,
            observation_format: self.observation_format,
            multiplicity,
            error: ValidationErrorBuilder::default(),
        };

        entry.write(&mut writer);

        let timestamp = writer.timestamp;
        let error = writer.error;

        // Check accumulated validation errors
        error.build()?;

        // Assemble final JSON and write to output
        let timestamp = timestamp.unwrap_or_else(SystemTime::now);
        let millis = timestamp
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();

        output.write_all(b"{\"timestamp\":")?;
        output.write_all(itoa::Buffer::new().format(millis).as_bytes())?;

        if !self.metrics_buf.is_empty() {
            output.write_all(b",\"metrics\":{")?;
            // Skip leading comma, each value wrote ,"name":{...}
            output.write_all(&self.metrics_buf.as_bytes()[1..])?;
            output.write_all(b"}")?;
        }

        if !self.properties_buf.is_empty() {
            output.write_all(b",\"properties\":{")?;
            output.write_all(&self.properties_buf.as_bytes()[1..])?;
            output.write_all(b"}")?;
        }

        output.write_all(b"}\n")?;
        Ok(())
    }

    /// Clear the buffers keeping the allocated capacity
    #[inline(always)]
    fn clear_buffers(&mut self) {
        self.metrics_buf.truncate(0);
        self.metrics_buf.shrink_to(MAX_BUF_RETAIN);
        self.properties_buf.truncate(0);
        self.properties_buf.shrink_to(MAX_BUF_RETAIN);
        self.counts_buf.truncate(0);
        self.counts_buf.shrink_to(MAX_BUF_RETAIN);
    }
}

impl Default for Json {
    fn default() -> Self {
        Self::new()
    }
}

impl Format for Json {
    fn format(
        &mut self,
        entry: &impl Entry,
        output: &mut impl io::Write,
    ) -> Result<(), IoStreamError> {
        self.format_with_multiplicity(entry, output, None)
    }
}

struct JsonEntryWriter<'b> {
    timestamp: Option<SystemTime>,
    metrics_buf: &'b mut String,
    properties_buf: &'b mut String,
    counts_buf: &'b mut String,
    observation_format: ObservationFormat,
    multiplicity: Option<u64>,
    error: ValidationErrorBuilder,
}

impl<'a, 'b> EntryWriter<'a> for JsonEntryWriter<'b> {
    fn timestamp(&mut self, timestamp: SystemTime) {
        if self.timestamp.is_some() {
            self.error.invalid_mut("timestamp set more than once");
        }
        self.timestamp = Some(timestamp);
    }

    fn value(&mut self, name: impl Into<Cow<'a, str>>, value: &(impl Value + ?Sized)) {
        let name = name.into();
        if name.is_empty() {
            self.error
                .extend_mut(ValidationError::invalid("name can't be empty").for_field(""));
            return;
        }
        let writer = JsonValueWriter {
            name: name.as_ref(),
            metrics_buf: self.metrics_buf,
            properties_buf: self.properties_buf,
            counts_buf: self.counts_buf,
            observation_format: self.observation_format,
            multiplicity: self.multiplicity,
            error: &mut self.error,
        };
        value.write(writer);
    }

    fn config(&mut self, _config: &'a dyn EntryConfig) {
        // Currently there's no EntryConfig that is JSON-specific or relevant to the JSON format.
    }
}

struct JsonValueWriter<'b, 'c> {
    name: &'c str,
    metrics_buf: &'b mut String,
    properties_buf: &'b mut String,
    counts_buf: &'b mut String,
    observation_format: ObservationFormat,
    multiplicity: Option<u64>,
    error: &'b mut ValidationErrorBuilder,
}

impl<'b, 'c> ValueWriter for JsonValueWriter<'b, 'c> {
    fn string(self, value: &str) {
        let buf = self.properties_buf;
        buf.push(',');
        push_json_string(buf, self.name);
        buf.push(':');
        push_json_string(buf, value);
    }

    fn metric<'a>(
        self,
        distribution: impl IntoIterator<Item = Observation>,
        unit: Unit,
        _dimensions: impl IntoIterator<Item = (&'a str, &'a str)>,
        _flags: MetricFlags<'_>,
    ) {
        // JSON v0 intentionally ignores per-metric dimensions and metric flags.
        // EMF interprets these for CloudWatch-specific behavior, while pure JSON
        // currently focuses on value/unit serialization only.
        let buf = self.metrics_buf;
        let mut obs = distribution.into_iter();

        let Some(first) = obs.next() else {
            return; // no observations, skip metric
        };

        // Write ,"MetricName":{
        buf.push(',');
        push_json_string(buf, self.name);
        buf.push_str(":{");

        match self.observation_format {
            ObservationFormat::Histogram => {
                buf.push_str("\"values\":[");
                let counts = self.counts_buf;
                counts.truncate(0);
                push_histogram_observation(buf, counts, first, self.multiplicity);
                for ob in obs {
                    buf.push(',');
                    counts.push(',');
                    push_histogram_observation(buf, counts, ob, self.multiplicity);
                }
                buf.push_str("],\"counts\":[");
                buf.push_str(counts);
                buf.push(']');
            }
            ObservationFormat::Scalar(repeated) => {
                if let Some(second) = obs.next() {
                    buf.push_str("\"values\":[");
                    push_observation(buf, first, repeated, self.multiplicity);
                    push_observation_comma(buf, second, repeated, self.multiplicity);
                    for ob in obs {
                        push_observation_comma(buf, ob, repeated, self.multiplicity);
                    }
                    buf.push(']');
                } else {
                    buf.push_str("\"value\":");
                    push_observation(buf, first, repeated, self.multiplicity);
                }
            }
        }

        if unit != Unit::None {
            buf.push_str(",\"unit\":");
            push_json_string(buf, unit.name());
        }

        buf.push('}');
    }

    fn error(self, error: ValidationError) {
        self.error.extend_mut(error.for_field(self.name));
    }
}

/// Push a comma followed by an observation (for array items after the first).
fn push_observation_comma(
    buf: &mut String,
    obs: Observation,
    repeated: RepeatedFormat,
    multiplicity: Option<u64>,
) {
    buf.push(',');
    push_observation(buf, obs, repeated, multiplicity);
}

/// Push a scalar observation value into the buffer.
fn push_observation(
    buf: &mut String,
    obs: Observation,
    repeated: RepeatedFormat,
    multiplicity: Option<u64>,
) {
    match obs {
        Observation::Unsigned(v) => {
            buf.push_str(itoa::Buffer::new().format(v));
        }
        Observation::Floating(v) => {
            push_float(buf, v);
        }
        Observation::Repeated { total, occurrences } => {
            let mult = multiplicity.unwrap_or(1);
            match repeated {
                RepeatedFormat::TotalAndCount => {
                    buf.push_str("{\"total\":");
                    push_float(buf, total);
                    buf.push_str(",\"count\":");
                    buf.push_str(itoa::Buffer::new().format(occurrences.saturating_mul(mult)));
                    buf.push('}');
                }
                RepeatedFormat::Mean => {
                    push_float(buf, repeated_mean(total, occurrences));
                }
            }
        }
        _ => {
            buf.push_str("null");
        }
    }
}

/// Push a histogram observation value and append its matching count.
fn push_histogram_observation(
    values_buf: &mut String,
    counts_buf: &mut String,
    obs: Observation,
    multiplicity: Option<u64>,
) {
    let mult = multiplicity.unwrap_or(1);
    match obs {
        Observation::Unsigned(v) => {
            values_buf.push_str(itoa::Buffer::new().format(v));
            counts_buf.push_str(itoa::Buffer::new().format(mult));
        }
        Observation::Floating(v) => {
            push_float(values_buf, v);
            counts_buf.push_str(itoa::Buffer::new().format(mult));
        }
        Observation::Repeated { total, occurrences } => {
            push_float(values_buf, repeated_mean(total, occurrences));
            counts_buf.push_str(itoa::Buffer::new().format(occurrences.saturating_mul(mult)));
        }
        _ => {
            values_buf.push_str("null");
            counts_buf.push_str(itoa::Buffer::new().format(mult));
        }
    }
}

fn repeated_mean(total: f64, occurrences: u64) -> f64 {
    if occurrences == 0 {
        0.0
    } else {
        total / occurrences as f64
    }
}

/// Push a float value, clamping infinities and writing null for NaN.
fn push_float(buf: &mut String, v: f64) {
    let v = v.clamp(-f64::MAX, f64::MAX);
    if v.is_nan() {
        buf.push_str("null");
    } else {
        let mut buffer = ryu::Buffer::new();
        let s = buffer.format_finite(v);
        // Strip trailing ".0" for cleaner integer-like output
        buf.push_str(s.strip_suffix(".0").unwrap_or(s));
    }
}

/// Push a JSON-escaped string with surrounding quotes into the buffer.
fn push_json_string(buf: &mut String, s: &str) {
    buf.push('"');
    let bytes = s.as_bytes();
    let mut start = 0;
    for (i, &b) in bytes.iter().enumerate() {
        let escape = match b {
            b'"' => "\\\"",
            b'\\' => "\\\\",
            b'\n' => "\\n",
            b'\r' => "\\r",
            b'\t' => "\\t",
            0x00..=0x1f => {
                buf.push_str(&s[start..i]);
                start = i + 1;
                use std::fmt::Write;
                let _ = write!(buf, "\\u{:04x}", b);
                continue;
            }
            _ => continue,
        };
        buf.push_str(&s[start..i]);
        buf.push_str(escape);
        start = i + 1;
    }
    buf.push_str(&s[start..]);
    buf.push('"');
}

/// A wrapper around [`Json`] that supports sampling. Datapoints are emitted with
/// multiplicity equal to either `floor(1/rate)` or `ceil(1/rate)` to ensure
/// statistics are unbiased.
///
/// See [`Json::with_sampling`].
pub struct SampledJson<R = DefaultRng<ThreadRng>> {
    json: Json,
    rng: R,
}

impl<R> Format for SampledJson<R> {
    fn format(
        &mut self,
        entry: &impl Entry,
        output: &mut impl io::Write,
    ) -> Result<(), IoStreamError> {
        self.json.format(entry, output)
    }
}

/// Return (n, alpha) such that 1/rate = alpha * n + (1-alpha) * (n+1).
fn rate_to_n_alpha(rate: f32) -> (u64, f64) {
    let rate = rate as f64;
    let inv_rate = 1.0 / rate;
    let inv_rate_int = inv_rate as u64;
    (inv_rate_int, (inv_rate_int + 1) as f64 - inv_rate)
}

fn rate_to_n<R: RngCore>(rate: f32, rng: &mut R) -> u64 {
    if rate < 1.0 / (i64::MAX as f32) {
        u64::MAX
    } else {
        let (n, alpha) = rate_to_n_alpha(rate);
        if rng.random::<f64>() < alpha {
            n
        } else {
            n.saturating_add(1)
        }
    }
}

impl<R: RngCore> SampledFormat for SampledJson<R> {
    fn format_with_sample_rate(
        &mut self,
        entry: &impl Entry,
        output: &mut impl io::Write,
        rate: f32,
    ) -> Result<(), IoStreamError> {
        if rate <= 0.0 || rate.is_nan() {
            return Err(IoStreamError::Validation(ValidationError::invalid(
                "format with non-positive sample rate",
            )));
        }
        let n = rate_to_n(rate, &mut self.rng);
        self.json.format_with_multiplicity(entry, output, Some(n))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use metrique_writer::value::{FlagConstructor, ForceFlag, MetricOptions, WithDimension};
    use rand::SeedableRng;
    use std::time::{Duration, SystemTime};

    struct SimpleEntry;
    impl Entry for SimpleEntry {
        fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
            writer.timestamp(SystemTime::UNIX_EPOCH + Duration::from_secs(1705312800));
            writer.value("Latency", &42.5f64);
            writer.value("Count", &10u64);
            writer.value("Operation", &"GetItem");
        }
    }

    fn parse_output(output: &[u8]) -> serde_json::Value {
        serde_json::from_slice(output).unwrap()
    }

    fn expected(json: &str) -> serde_json::Value {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn test_simple_entry() {
        let mut format = Json::new();
        let mut output = Vec::new();
        format.format(&SimpleEntry, &mut output).unwrap();

        assert_eq!(
            parse_output(&output),
            expected(
                r#"{
                "timestamp": 1705312800000,
                "metrics": {
                    "Latency": { "value": 42.5 },
                    "Count": { "value": 10 }
                },
                "properties": { "Operation": "GetItem" }
            }"#
            ),
        );
    }

    struct RepeatedEntry;
    impl Entry for RepeatedEntry {
        fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
            writer.timestamp(SystemTime::UNIX_EPOCH + Duration::from_secs(1705312800));
            writer.value(
                "AvgLatency",
                &Observation::Repeated {
                    total: 150.0,
                    occurrences: 3,
                },
            );
        }
    }

    #[test]
    fn test_repeated_total_and_count() {
        let mut format = Json::new();
        let mut output = Vec::new();
        format.format(&RepeatedEntry, &mut output).unwrap();

        assert_eq!(
            parse_output(&output),
            expected(
                r#"{
                "timestamp": 1705312800000,
                "metrics": {
                    "AvgLatency": { "value": { "total": 150, "count": 3 } }
                }
            }"#
            ),
        );
    }

    #[test]
    fn test_repeated_mean_mode() {
        let mut format =
            Json::new().with_observation_format(ObservationFormat::Scalar(RepeatedFormat::Mean));
        let mut output = Vec::new();
        format.format(&RepeatedEntry, &mut output).unwrap();

        assert_eq!(
            parse_output(&output),
            expected(
                r#"{
                "timestamp": 1705312800000,
                "metrics": {
                    "AvgLatency": { "value": 50 }
                }
            }"#
            ),
        );
    }

    #[test]
    fn test_repeated_with_multiplicity() {
        let mut format = Json::new();
        let mut output = Vec::new();
        format
            .format_with_multiplicity(&RepeatedEntry, &mut output, Some(5))
            .unwrap();

        assert_eq!(
            parse_output(&output),
            expected(
                r#"{
                "timestamp": 1705312800000,
                "metrics": {
                    "AvgLatency": { "value": { "total": 150, "count": 15 } }
                }
            }"#
            ),
        );
    }

    struct DistributionEntry;
    impl Entry for DistributionEntry {
        fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
            writer.timestamp(SystemTime::UNIX_EPOCH);
            struct MultiObs;
            impl Value for MultiObs {
                fn write(&self, writer: impl ValueWriter) {
                    writer.metric(
                        [
                            Observation::Floating(1.0),
                            Observation::Floating(2.0),
                            Observation::Floating(3.0),
                        ],
                        Unit::Second(metrique_writer_core::unit::NegativeScale::Milli),
                        [],
                        MetricFlags::empty(),
                    );
                }
            }
            writer.value("ResponseTimes", &MultiObs);
        }
    }

    #[test]
    fn test_distribution_array() {
        let mut format = Json::new();
        let mut output = Vec::new();
        format.format(&DistributionEntry, &mut output).unwrap();

        assert_eq!(
            parse_output(&output),
            expected(
                r#"{
                "timestamp": 0,
                "metrics": {
                    "ResponseTimes": { "values": [1, 2, 3], "unit": "Milliseconds" }
                }
            }"#
            ),
        );
    }

    #[test]
    fn test_dimensions_are_ignored_in_json() {
        struct DimEntry;
        impl Entry for DimEntry {
            fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
                writer.timestamp(SystemTime::UNIX_EPOCH);
                writer.value("Count", &WithDimension::new(10u64, "Region", "us-east-1"));
            }
        }

        let mut format = Json::new();
        let mut output = Vec::new();
        format.format(&DimEntry, &mut output).unwrap();

        assert_eq!(
            parse_output(&output),
            expected(
                r#"{
                "timestamp": 0,
                "metrics": {
                    "Count": { "value": 10 }
                }
            }"#
            ),
        );
    }

    #[test]
    fn test_flags_are_ignored_in_json() {
        #[derive(Debug)]
        struct TestFlagOptions;
        impl MetricOptions for TestFlagOptions {}

        struct TestFlagCtor;
        impl FlagConstructor for TestFlagCtor {
            fn construct() -> MetricFlags<'static> {
                MetricFlags::upcast(&TestFlagOptions)
            }
        }

        type TestFlag<T> = ForceFlag<T, TestFlagCtor>;

        struct FlagEntry;
        impl Entry for FlagEntry {
            fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
                writer.timestamp(SystemTime::UNIX_EPOCH);
                writer.value("Count", &TestFlag::from(10u64));
            }
        }

        let mut format = Json::new();
        let mut output = Vec::new();
        format.format(&FlagEntry, &mut output).unwrap();

        assert_eq!(
            parse_output(&output),
            expected(
                r#"{
                "timestamp": 0,
                "metrics": {
                    "Count": { "value": 10 }
                }
            }"#
            ),
        );
    }

    #[test]
    fn test_histogram_encoding() {
        let mut format = Json::new().with_observation_format(ObservationFormat::Histogram);
        let mut output = Vec::new();
        format.format(&DistributionEntry, &mut output).unwrap();

        assert_eq!(
            parse_output(&output),
            expected(
                r#"{
                "timestamp": 0,
                "metrics": {
                    "ResponseTimes": {
                        "values": [1, 2, 3],
                        "counts": [1, 1, 1],
                        "unit": "Milliseconds"
                    }
                }
            }"#
            ),
        );
    }

    #[test]
    fn test_histogram_repeated_with_multiplicity() {
        let mut format = Json::new().with_observation_format(ObservationFormat::Histogram);
        let mut output = Vec::new();
        format
            .format_with_multiplicity(&RepeatedEntry, &mut output, Some(5))
            .unwrap();

        assert_eq!(
            parse_output(&output),
            expected(
                r#"{
                "timestamp": 1705312800000,
                "metrics": {
                    "AvgLatency": { "values": [50], "counts": [15] }
                }
            }"#
            ),
        );
    }

    #[test]
    fn test_histogram_distribution_with_multiplicity() {
        let mut format = Json::new().with_observation_format(ObservationFormat::Histogram);
        let mut output = Vec::new();
        format
            .format_with_multiplicity(&DistributionEntry, &mut output, Some(5))
            .unwrap();

        assert_eq!(
            parse_output(&output),
            expected(
                r#"{
                "timestamp": 0,
                "metrics": {
                    "ResponseTimes": {
                        "values": [1, 2, 3],
                        "counts": [5, 5, 5],
                        "unit": "Milliseconds"
                    }
                }
            }"#
            ),
        );
    }

    #[test]
    fn test_histogram_nan_emits_null_with_count() {
        struct NanDistributionEntry;
        impl Entry for NanDistributionEntry {
            fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
                writer.timestamp(SystemTime::UNIX_EPOCH);
                struct DistWithNan;
                impl Value for DistWithNan {
                    fn write(&self, writer: impl ValueWriter) {
                        writer.metric(
                            [Observation::Floating(f64::NAN), Observation::Floating(1.0)],
                            Unit::None,
                            [],
                            MetricFlags::empty(),
                        );
                    }
                }
                writer.value("Mixed", &DistWithNan);
            }
        }

        let mut format = Json::new().with_observation_format(ObservationFormat::Histogram);
        let mut output = Vec::new();
        format.format(&NanDistributionEntry, &mut output).unwrap();

        assert_eq!(
            parse_output(&output),
            expected(
                r#"{
                "timestamp": 0,
                "metrics": {
                    "Mixed": {
                        "values": [null, 1],
                        "counts": [1, 1]
                    }
                }
            }"#
            ),
        );
    }

    #[test]
    fn test_empty_entry() {
        struct EmptyEntry;
        impl Entry for EmptyEntry {
            fn write<'a>(&'a self, _writer: &mut impl EntryWriter<'a>) {}
        }

        let mut format = Json::new();
        let mut output = Vec::new();
        format.format(&EmptyEntry, &mut output).unwrap();

        let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
        assert!(json["timestamp"].is_number());
        assert!(json.get("metrics").is_none());
        assert!(json.get("properties").is_none());
    }

    #[test]
    fn test_json_string_escaping() {
        struct EscapeEntry;
        impl Entry for EscapeEntry {
            fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
                writer.timestamp(SystemTime::UNIX_EPOCH);
                writer.value("msg", &"hello \"world\"\nnewline");
            }
        }

        let mut format = Json::new();
        let mut output = Vec::new();
        format.format(&EscapeEntry, &mut output).unwrap();

        // Parse round-trips the escaping through serde, confirming correctness
        let json = parse_output(&output);
        assert_eq!(json["properties"]["msg"], "hello \"world\"\nnewline");
    }

    #[test]
    fn test_json_string_escaping_stress_roundtrip() {
        struct EscapeStressEntry;
        impl Entry for EscapeStressEntry {
            fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
                writer.timestamp(SystemTime::UNIX_EPOCH);
                writer.value("msg", &"quote:\" slash:\\ tab:\t newline:\n null:\u{0000} unit-sep:\u{001f} unicode:α");
            }
        }

        let mut format = Json::new();
        let mut output = Vec::new();
        format.format(&EscapeStressEntry, &mut output).unwrap();

        let json = parse_output(&output);
        assert_eq!(
            json["properties"]["msg"],
            "quote:\" slash:\\ tab:\t newline:\n null:\u{0000} unit-sep:\u{001f} unicode:α"
        );
    }

    #[test]
    fn test_output_is_single_line() {
        let mut format = Json::new();
        let mut output = Vec::new();
        format.format(&SimpleEntry, &mut output).unwrap();

        let s = String::from_utf8(output).unwrap();
        assert_eq!(s.lines().count(), 1);
        assert!(s.ends_with('\n'));
    }

    #[test]
    fn test_nan_and_infinity() {
        struct NanEntry;
        impl Entry for NanEntry {
            fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
                writer.value("nan_val", &f64::NAN);
                writer.value("inf_val", &f64::INFINITY);
                writer.value("neg_inf_val", &f64::NEG_INFINITY);
            }
        }

        let mut format = Json::new();
        let mut output = Vec::new();
        format.format(&NanEntry, &mut output).unwrap();

        let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
        assert!(json["metrics"]["nan_val"]["value"].is_null());
        assert!(json["metrics"]["inf_val"]["value"].as_f64().unwrap() > 1e300);
        assert!(json["metrics"]["neg_inf_val"]["value"].as_f64().unwrap() < -1e300);
    }

    #[test]
    fn test_buffer_reuse() {
        let mut format = Json::new();

        let mut output1 = Vec::new();
        format.format(&SimpleEntry, &mut output1).unwrap();
        let mut output2 = Vec::new();
        format.format(&SimpleEntry, &mut output2).unwrap();

        assert_eq!(output1, output2);
    }

    #[test]
    fn test_sampled_format_trait() {
        let mut format =
            Json::new().with_sampling_and_rng(rand_chacha::ChaChaRng::seed_from_u64(0));
        let mut output = Vec::new();
        format
            .format_with_sample_rate(&RepeatedEntry, &mut output, 0.1)
            .unwrap();

        // With sampling at 0.1, multiplicity should be ~10 (either 10 or 11),
        // so count = occurrences (3) * multiplicity > 3
        let json = parse_output(&output);
        let count = json["metrics"]["AvgLatency"]["value"]["count"]
            .as_u64()
            .unwrap();
        assert!(count >= 30, "expected count >= 30, got {count}");
    }

    #[test]
    fn test_sampled_histogram_format_trait() {
        let mut format = Json::new()
            .with_observation_format(ObservationFormat::Histogram)
            .with_sampling_and_rng(rand_chacha::ChaChaRng::seed_from_u64(0));
        let mut output = Vec::new();
        format
            .format_with_sample_rate(&RepeatedEntry, &mut output, 0.1)
            .unwrap();

        let json = parse_output(&output);
        assert_eq!(
            json["metrics"]["AvgLatency"]["values"],
            serde_json::json!([50])
        );
        let count = json["metrics"]["AvgLatency"]["counts"][0].as_u64().unwrap();
        assert!(count >= 30, "expected count >= 30, got {count}");
    }

    #[test]
    fn test_sampled_invalid_rate() {
        let mut format =
            Json::new().with_sampling_and_rng(rand_chacha::ChaChaRng::seed_from_u64(0));
        let mut output = Vec::new();
        let result = format.format_with_sample_rate(&SimpleEntry, &mut output, -0.5);
        assert!(result.is_err());

        let result = format.format_with_sample_rate(&SimpleEntry, &mut output, f32::NAN);
        assert!(result.is_err());
    }
}
