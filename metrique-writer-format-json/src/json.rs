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
use metrique_writer_core::value::{
    MetricFlags, ObjectValue, Observation, Value, ValueWriter, write_json_object,
};
use metrique_writer_core::{Entry, EntryWriter, Unit, ValidationError, ValidationErrorBuilder};
use rand::rngs::ThreadRng;
use rand::{Rng, RngCore};

// Maximum buffer size before shrinking on clear. Prevents one large entry from
// permanently bloating memory.
const MAX_BUF_RETAIN: usize = 1024 * 1024;

/// A pure JSON formatter for metrique metrics.
///
/// Outputs one JSON object per entry as a single line, followed by a newline.
///
/// The output structure is (timestamp is milliseconds since Unix epoch):
/// ```json
/// {
///   "timestamp": 1705312800000,
///   "metrics": {
///     "Latency": { "value": 42.5, "unit": "Milliseconds" },
///     "Count": { "value": 10 },
///     "BackendLatency": { "value": { "total": 150, "count": 3 }, "unit": "Milliseconds" },
///     "ResponseTimes": { "values": [1, 2, 3], "unit": "Milliseconds" }
///   },
///   "properties": {
///     "Operation": "GetItem",
///     "Region": "us-east-1"
///   }
/// }
/// ```
///
/// Single observations are emitted as `"value": X`, multiple observations as
/// `"values": [...]`. Repeated observations (e.g. from histogram buckets) are
/// emitted as `{"total": f64, "count": u64}`.
///
/// ## Object-valued fields
///
/// A field that opts into object rendering (`#[metrics(format = AsObject)]`) is emitted in
/// `properties` as a native nested JSON object, and a list of such objects as a native JSON array
/// — not as a JSON-encoded string. Objects are not metrics: per-member units, dimensions, and
/// flags are ignored, and a member's observation count is not scaled by the sampling multiplicity
/// (unlike a top-level metric). A member whose value writes nothing (e.g. a `None` or an empty
/// distribution) is omitted rather than emitted as `null`. Consumers that assume `properties`
/// values are always strings should account for object and array values.
///
/// ```
/// use metrique_writer_format_json::Json;
///
/// let format = Json::new();
/// ```
///
/// ## Handling of non-finite floating-point values
///
/// JSON has no representation for infinity or NaN, so these are handled as follows:
///
/// - **Infinities** are clamped to `f64::MAX` / `-f64::MAX`. This preserves the
///   data point for debugging while staying within valid JSON. Note that this means the output value
///   is technically different from the input.
/// - **NaN** observations are serialized as JSON `null`.
#[derive(Debug)]
pub struct Json {
    // Reusable string buffers, cleared between entries, capacity stays warm.
    // Each entry writes ,"key":value fragments into these. The leading comma is
    // stripped when assembling the final output.
    metrics_buf: String,
    properties_buf: String,
}

impl Json {
    /// Create a new JSON formatter with default settings.
    pub fn new() -> Self {
        Self {
            metrics_buf: String::with_capacity(2048),
            properties_buf: String::with_capacity(2048),
        }
    }

    /// Wrap this formatter with support for sampling using the default RNG.
    ///
    /// When sampling is active, metrics are emitted with a multiplicity that
    /// compensates for dropped entries, keeping aggregate statistics unbiased.
    pub fn with_sampling(self) -> SampledJson {
        SampledJson {
            json: self,
            rng: Default::default(),
        }
    }

    /// Wrap this formatter with support for sampling using an explicit RNG.
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

    /// Clear buffers and shrink overly large retained capacity.
    #[inline(always)]
    fn clear_buffers(&mut self) {
        self.metrics_buf.clear();
        self.metrics_buf.shrink_to(MAX_BUF_RETAIN);
        self.properties_buf.clear();
        self.properties_buf.shrink_to(MAX_BUF_RETAIN);
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
    multiplicity: Option<u64>,
    error: &'b mut ValidationErrorBuilder,
}

/// Adapter for writing individual elements inside a JSON array.
///
/// String values are JSON-escaped. Metric values write their observations as
/// numeric scalars (single observation) or nested sub-arrays (multiple observations).
struct JsonArrayElementWriter<'a>(&'a mut String);

impl ValueWriter for JsonArrayElementWriter<'_> {
    fn string(self, value: &str) {
        push_json_string(self.0, value);
    }

    fn metric<'a>(
        self,
        distribution: impl IntoIterator<Item = Observation>,
        _unit: Unit,
        _dimensions: impl IntoIterator<Item = (&'a str, &'a str)>,
        _flags: MetricFlags<'_>,
    ) {
        let buf = self.0;
        let mut iter = distribution.into_iter();
        let Some(first) = iter.next() else { return };
        match iter.next() {
            None => push_observation(buf, first, None),
            Some(second) => {
                buf.push('[');
                push_observation(buf, first, None);
                buf.push(',');
                push_observation(buf, second, None);
                for obs in iter {
                    buf.push(',');
                    push_observation(buf, obs, None);
                }
                buf.push(']');
            }
        }
    }

    fn error(self, _error: ValidationError) {}

    fn values<'a, V: Value + 'a>(self, values: impl IntoIterator<Item = &'a V>) {
        // Arrays don't nest here: a list inside an element becomes one joined element.
        metrique_writer_core::value::write_values_as_string(self, values)
    }

    fn object<O: ObjectValue + ?Sized>(self, object: &O) {
        // An object inside an array renders as a raw `{...}` element. Its own members — including
        // nested objects and object arrays — recurse natively through core's encoder. (A bare
        // array nested directly as an array element, not inside an object, still joins via
        // `values()` above; `AsObject` cannot produce that `Vec<Vec<_>>` shape.)
        write_json_object(self.0, object);
    }
}

impl<'b, 'c> ValueWriter for JsonValueWriter<'b, 'c> {
    fn string(self, value: &str) {
        let buf = self.properties_buf;
        buf.push(',');
        push_json_string(buf, self.name);
        buf.push(':');
        push_json_string(buf, value);
    }

    fn values<'a, V: Value + 'a>(self, values: impl IntoIterator<Item = &'a V>) {
        let buf = self.properties_buf;
        buf.push(',');
        push_json_string(buf, self.name);
        buf.push_str(":[");
        let mut wrote_any = false;
        for value in values {
            let before = buf.len();
            if wrote_any {
                buf.push(',');
            }
            let after_sep = buf.len();
            value.write(JsonArrayElementWriter(buf));
            if buf.len() > after_sep {
                wrote_any = true;
            } else {
                buf.truncate(before);
            }
        }
        buf.push(']');
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

        if let Some(second) = obs.next() {
            buf.push_str("\"values\":[");
            push_observation(buf, first, self.multiplicity);
            push_observation_comma(buf, second, self.multiplicity);
            for ob in obs {
                push_observation_comma(buf, ob, self.multiplicity);
            }
            buf.push(']');
        } else {
            buf.push_str("\"value\":");
            push_observation(buf, first, self.multiplicity);
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

    fn object<O: ObjectValue + ?Sized>(self, object: &O) {
        // Objects are not metrics: render into the properties section as a raw nested object.
        let buf = self.properties_buf;
        buf.push(',');
        push_json_string(buf, self.name);
        buf.push(':');
        write_json_object(buf, object);
    }
}

/// Push a comma followed by an observation (for array items after the first).
fn push_observation_comma(buf: &mut String, obs: Observation, multiplicity: Option<u64>) {
    buf.push(',');
    push_observation(buf, obs, multiplicity);
}

/// Push a scalar observation value into the buffer.
fn push_observation(buf: &mut String, obs: Observation, multiplicity: Option<u64>) {
    match obs {
        Observation::Unsigned(v) => {
            buf.push_str(itoa::Buffer::new().format(v));
        }
        Observation::Floating(v) => {
            push_float(buf, v);
        }
        Observation::Repeated { total, occurrences } => {
            let mult = multiplicity.unwrap_or(1);
            buf.push_str("{\"total\":");
            push_float(buf, total);
            buf.push_str(",\"count\":");
            buf.push_str(itoa::Buffer::new().format(occurrences.saturating_mul(mult)));
            buf.push('}');
        }
        _ => {
            buf.push_str("null");
        }
    }
}

/// Push a float value, clamping infinities and writing null for NaN.
fn push_float(buf: &mut String, v: f64) {
    let v = v.clamp(-f64::MAX, f64::MAX);
    if v.is_nan() {
        buf.push_str("null");
    } else {
        // We use `dtoa` over `ryu` because `dtoa` always emits decimal notation
        // (no scientific notation), which is easier to script against and more portable
        // across downstream metric consumers.
        let mut buffer = dtoa::Buffer::new();
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
#[derive(Debug)]
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
    fn test_sampled_rate_1_0() {
        let mut format =
            Json::new().with_sampling_and_rng(rand_chacha::ChaChaRng::seed_from_u64(0));
        let mut output = Vec::new();
        format
            .format_with_sample_rate(&RepeatedEntry, &mut output, 1.0)
            .unwrap();

        // At rate 1.0 (no sampling), multiplicity should be 1,
        // so count == occurrences (3), not scaled up.
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
    fn test_sampled_invalid_rate() {
        let mut format =
            Json::new().with_sampling_and_rng(rand_chacha::ChaChaRng::seed_from_u64(0));
        let mut output = Vec::new();
        let result = format.format_with_sample_rate(&SimpleEntry, &mut output, -0.5);
        assert!(result.is_err());

        let result = format.format_with_sample_rate(&SimpleEntry, &mut output, f32::NAN);
        assert!(result.is_err());
    }

    struct VecEntry {
        plugins: Vec<String>,
    }

    impl Entry for VecEntry {
        fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
            writer.timestamp(SystemTime::UNIX_EPOCH);
            writer.value("Plugins", &self.plugins);
        }
    }

    #[test]
    fn test_vec_emits_json_array() {
        let mut format = Json::new();
        let mut output = Vec::new();
        format
            .format(
                &VecEntry {
                    plugins: vec!["auth".into(), "cache".into()],
                },
                &mut output,
            )
            .unwrap();
        let json = parse_output(&output);
        assert_eq!(
            json["properties"]["Plugins"],
            serde_json::json!(["auth", "cache"])
        );
    }

    #[test]
    fn test_single_element_vec_in_json() {
        let mut format = Json::new();
        let mut output = Vec::new();
        format
            .format(
                &VecEntry {
                    plugins: vec!["only".into()],
                },
                &mut output,
            )
            .unwrap();
        let json = parse_output(&output);
        assert_eq!(json["properties"]["Plugins"], serde_json::json!(["only"]));
    }

    #[test]
    fn test_empty_vec_in_json() {
        let mut format = Json::new();
        let mut output = Vec::new();
        format
            .format(&VecEntry { plugins: vec![] }, &mut output)
            .unwrap();
        let json = parse_output(&output);
        assert_eq!(json["properties"]["Plugins"], serde_json::json!([]));
    }

    #[test]
    fn test_vec_with_none_elements_in_json() {
        struct VecOptionEntry {
            tags: Vec<Option<String>>,
        }
        impl Entry for VecOptionEntry {
            fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
                writer.timestamp(SystemTime::UNIX_EPOCH);
                writer.value("Tags", &self.tags);
            }
        }

        let mut format = Json::new();
        let mut output = Vec::new();
        format
            .format(
                &VecOptionEntry {
                    tags: vec![Some("a".into()), None, Some("c".into())],
                },
                &mut output,
            )
            .unwrap();
        let json = parse_output(&output);
        assert_eq!(json["properties"]["Tags"], serde_json::json!(["a", "c"]));
    }

    #[test]
    fn test_vec_u64_emits_json_array() {
        struct VecU64Entry {
            counts: Vec<u64>,
        }
        impl Entry for VecU64Entry {
            fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
                writer.timestamp(SystemTime::UNIX_EPOCH);
                writer.value("Counts", &self.counts);
            }
        }

        let mut format = Json::new();
        let mut output = Vec::new();
        format
            .format(
                &VecU64Entry {
                    counts: vec![10, 20, 30],
                },
                &mut output,
            )
            .unwrap();
        let json = parse_output(&output);
        assert_eq!(
            json["properties"]["Counts"],
            serde_json::json!([10, 20, 30])
        );
    }

    #[test]
    fn test_vec_multi_observation_nests_sub_arrays_in_json() {
        struct MultiObsValue(Vec<u64>);
        impl Value for MultiObsValue {
            fn write(&self, writer: impl ValueWriter) {
                writer.metric(
                    self.0.iter().map(|&v| Observation::Unsigned(v)),
                    Unit::None,
                    [],
                    MetricFlags::empty(),
                );
            }
        }
        struct VecMultiObsEntry {
            data: Vec<MultiObsValue>,
        }
        impl Entry for VecMultiObsEntry {
            fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
                writer.timestamp(SystemTime::UNIX_EPOCH);
                writer.value("Data", &self.data);
            }
        }

        let mut format = Json::new();
        let mut output = Vec::new();
        format
            .format(
                &VecMultiObsEntry {
                    data: vec![MultiObsValue(vec![1, 2, 3]), MultiObsValue(vec![4, 5])],
                },
                &mut output,
            )
            .unwrap();
        let json = parse_output(&output);
        assert_eq!(
            json["properties"]["Data"],
            serde_json::json!([[1, 2, 3], [4, 5]])
        );
    }

    mod objects {
        use super::*;
        use metrique_writer_core::BoxEntry;
        use metrique_writer_core::descriptor::{FieldShape, KnownShape};
        use rstest::rstest;

        /// A `Value` that renders a single `ObjectValue` through `object()`. It calls the native
        /// path directly rather than going through the `AsObject` formatter; `AsObject`'s own
        /// `O`/`Vec`/`Option`/`Option<Vec>` dispatch is format-agnostic and covered by core's
        /// `as_object_render_matrix`. Here we exercise how the JSON format *renders* an object and
        /// an object array, with hand-written fixtures (no derive needed).
        struct DirectObject<O>(O);

        impl<O: ObjectValue> Value for DirectObject<O> {
            const SHAPE: FieldShape<'static> = FieldShape::Object;
            const UNIT: Unit = Unit::None;

            fn write(&self, writer: impl ValueWriter) {
                writer.object(&self.0);
            }
        }

        /// An entry with a fixed timestamp and a single field, so output is deterministic.
        struct Field<V>(&'static str, V);

        impl<V: Value + Send + 'static> Entry for Field<V> {
            fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
                writer.timestamp(SystemTime::UNIX_EPOCH);
                writer.value(self.0, &self.1);
            }
        }

        fn render_bytes(entry: &impl Entry) -> String {
            let mut output = Vec::new();
            Json::new().format(entry, &mut output).unwrap();
            String::from_utf8(output).unwrap()
        }

        /// Format the entry via both the unboxed and the boxed (dyn) path, assert the two are
        /// byte-identical, and return the parsed JSON. The boxed path exercises the
        /// `DynValueWriter::object` / `values_dyn` element-shape bridge, which is the part most
        /// likely to diverge from the direct path, so every object test gets parity for free.
        fn render_checked<E: Entry + Send + 'static>(make: impl Fn() -> E) -> serde_json::Value {
            let unboxed = render_bytes(&make());
            let boxed = render_bytes(&BoxEntry::new(make()));
            assert_eq!(
                unboxed, boxed,
                "boxed and unboxed output differ:\n  unboxed: {unboxed}\n  boxed:   {boxed}"
            );
            serde_json::from_str(&unboxed)
                .unwrap_or_else(|e| panic!("output was not valid JSON: {e}\noutput: {unboxed}"))
        }

        #[derive(Clone, Copy)]
        struct Endpoint {
            host: &'static str,
            port: u64,
        }

        impl ObjectValue for Endpoint {
            fn write_object<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
                writer.value("Host", &self.host);
                writer.value("Port", &self.port);
            }
        }

        fn ep() -> Endpoint {
            Endpoint {
                host: "example.com",
                port: 443,
            }
        }

        /// An object field lands natively in `properties` (not as an escaped JSON string), and an
        /// object-only entry produces no `metrics` section — objects are not metrics.
        #[test]
        fn single_object_renders_natively() {
            let json = render_checked(|| Field("Endpoint", DirectObject(ep())));
            assert_eq!(
                json["properties"]["Endpoint"],
                serde_json::json!({"Host": "example.com", "Port": 443})
            );
            assert!(json.get("metrics").is_none());
        }

        #[test]
        fn array_of_objects_renders_as_json_array() {
            let json = render_checked(|| {
                Field(
                    "Endpoints",
                    vec![
                        DirectObject(Endpoint { host: "a", port: 1 }),
                        DirectObject(Endpoint { host: "b", port: 2 }),
                    ],
                )
            });
            assert_eq!(
                json["properties"]["Endpoints"],
                serde_json::json!([{"Host": "a", "Port": 1}, {"Host": "b", "Port": 2}])
            );
        }

        /// Length 9 crosses `VALUES_INLINE_CAPACITY` (8): the boxed receiver's buffer spills, but
        /// output must stay identical to the unboxed path.
        #[test]
        fn array_crossing_inline_capacity_matches_boxed() {
            let json = render_checked(|| {
                Field(
                    "E",
                    (0..9u64)
                        .map(|port| DirectObject(Endpoint { host: "h", port }))
                        .collect::<Vec<_>>(),
                )
            });
            assert_eq!(json["properties"]["E"].as_array().unwrap().len(), 9);
        }

        #[test]
        fn empty_object_array_renders_empty_array() {
            let json = render_checked(|| Field("E", Vec::<DirectObject<Endpoint>>::new()));
            assert_eq!(json["properties"]["E"], serde_json::json!([]));
        }

        #[test]
        fn all_empty_object_renders_empty_braces() {
            struct Empty;
            impl ObjectValue for Empty {
                fn write_object<'a>(&'a self, _writer: &mut impl EntryWriter<'a>) {}
            }
            // Key present with an empty object, distinct from an omitted field.
            let json = render_checked(|| Field("Obj", DirectObject(Empty)));
            assert_eq!(json["properties"]["Obj"], serde_json::json!({}));
        }

        #[derive(Clone, Copy)]
        struct Members {
            a: Option<u64>,
            b: Option<u64>,
            c: Option<u64>,
        }

        impl ObjectValue for Members {
            fn write_object<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
                writer.value("A", &self.a);
                writer.value("B", &self.b);
                writer.value("C", &self.c);
            }
        }

        /// A `None` member in first / middle / last / only position is omitted with no dangling
        /// `"Key":` and no stray comma — the member rollback in core's object encoder.
        #[rstest]
        #[case::first_absent(Members { a: None, b: Some(2), c: Some(3) }, serde_json::json!({"B": 2, "C": 3}))]
        #[case::middle_absent(Members { a: Some(1), b: None, c: Some(3) }, serde_json::json!({"A": 1, "C": 3}))]
        #[case::last_absent(Members { a: Some(1), b: Some(2), c: None }, serde_json::json!({"A": 1, "B": 2}))]
        #[case::all_absent(Members { a: None, b: None, c: None }, serde_json::json!({}))]
        fn none_members_are_omitted(#[case] members: Members, #[case] expected: serde_json::Value) {
            let json = render_checked(move || Field("Obj", DirectObject(members)));
            assert_eq!(json["properties"]["Obj"], expected);
        }

        #[derive(Clone)]
        struct Phase {
            name: &'static str,
            children: Vec<Phase>,
        }

        impl ObjectValue for Phase {
            fn write_object<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
                writer.value("Name", &self.name);
                writer.value(
                    "Children",
                    &self.children.iter().map(DirectObject).collect::<Vec<_>>(),
                );
            }
        }

        /// A recursive object/array tree nests natively to full depth: one `JSON.parse` recovers
        /// it, and boxed output matches unboxed.
        #[test]
        fn nested_object_tree_renders_natively_to_full_depth() {
            let tree = || Phase {
                name: "root",
                children: vec![
                    Phase {
                        name: "a",
                        children: vec![],
                    },
                    Phase {
                        name: "b",
                        children: vec![Phase {
                            name: "b1",
                            children: vec![],
                        }],
                    },
                ],
            };
            let json = render_checked(move || Field("Root", DirectObject(tree())));
            assert_eq!(
                json["properties"]["Root"],
                serde_json::json!({
                    "Name": "root",
                    "Children": [
                        {"Name": "a", "Children": []},
                        {"Name": "b", "Children": [{"Name": "b1", "Children": []}]}
                    ]
                })
            );
        }

        /// String members inside an object are JSON-escaped by the shared core encoder, so native
        /// object output round-trips through `JSON.parse` unchanged.
        #[test]
        fn object_member_strings_are_escaped() {
            struct Weird;
            impl ObjectValue for Weird {
                fn write_object<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
                    writer.value("Msg", &"a\"b\nc");
                }
            }
            let json = render_checked(|| Field("Obj", DirectObject(Weird)));
            assert_eq!(json["properties"]["Obj"]["Msg"], "a\"b\nc");
        }

        /// An object field surrounded by a scalar property and a metric: it lands in `properties`
        /// without disturbing the leading-comma stripping of either section.
        #[test]
        fn object_renders_alongside_other_fields() {
            struct E;
            impl Entry for E {
                fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
                    writer.timestamp(SystemTime::UNIX_EPOCH);
                    writer.value("Before", &"x");
                    writer.value("Obj", &DirectObject(ep()));
                    writer.value("After", &10u64);
                }
            }
            let json = render_checked(|| E);
            assert_eq!(json["properties"]["Before"], "x");
            assert_eq!(
                json["properties"]["Obj"],
                serde_json::json!({"Host": "example.com", "Port": 443})
            );
            assert_eq!(json["metrics"]["After"]["value"], 10);
        }

        /// A member whose distribution is empty writes nothing, so the member is omitted (not
        /// `"Gone":null`) via the same rollback that handles `None`.
        #[test]
        fn empty_distribution_member_is_omitted() {
            struct EmptyDist;
            impl Value for EmptyDist {
                const SHAPE: FieldShape<'static> = FieldShape::Known(KnownShape::U64);
                const UNIT: Unit = Unit::None;

                fn write(&self, writer: impl ValueWriter) {
                    writer.metric(std::iter::empty(), Unit::None, [], MetricFlags::empty());
                }
            }
            struct Obj;
            impl ObjectValue for Obj {
                fn write_object<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
                    writer.value("Keep", &1u64);
                    writer.value("Gone", &EmptyDist);
                }
            }
            let json = render_checked(|| Field("Obj", DirectObject(Obj)));
            assert_eq!(json["properties"]["Obj"], serde_json::json!({"Keep": 1}));
        }

        /// Design decision pinned: objects are not metrics, so a member that reports a
        /// `ValidationError` is omitted and the entry still formats `Ok` — matching the string
        /// fallback, which has no error channel. (At the top level the same error fails the entry.)
        #[test]
        fn member_validation_error_is_dropped_not_surfaced() {
            struct Failing;
            impl Value for Failing {
                const SHAPE: FieldShape<'static> = FieldShape::Known(KnownShape::U64);
                const UNIT: Unit = Unit::None;

                fn write(&self, writer: impl ValueWriter) {
                    writer.error(ValidationError::invalid("boom"));
                }
            }
            struct Obj;
            impl ObjectValue for Obj {
                fn write_object<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
                    writer.value("Ok", &1u64);
                    writer.value("Bad", &Failing);
                }
            }
            // `render_checked` would panic if `format` returned `Err`.
            let json = render_checked(|| Field("Obj", DirectObject(Obj)));
            assert_eq!(json["properties"]["Obj"], serde_json::json!({"Ok": 1}));
        }

        /// Under sampling a top-level `Repeated` scales its `count` by the multiplicity, but an
        /// object member is never scaled (object members pass `multiplicity = None`).
        #[test]
        fn sampled_object_member_count_is_not_scaled() {
            struct Latency;
            impl ObjectValue for Latency {
                fn write_object<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
                    writer.value(
                        "Count",
                        &Observation::Repeated {
                            total: 150.0,
                            occurrences: 3,
                        },
                    );
                }
            }
            let mut format =
                Json::new().with_sampling_and_rng(rand_chacha::ChaChaRng::seed_from_u64(0));
            let mut output = Vec::new();
            format
                .format_with_sample_rate(&Field("Obj", DirectObject(Latency)), &mut output, 0.1)
                .unwrap();
            let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
            // Unscaled: 3, not ~30 as a top-level Repeated would be at rate 0.1.
            assert_eq!(json["properties"]["Obj"]["Count"]["count"], 3);
        }
    }
}
