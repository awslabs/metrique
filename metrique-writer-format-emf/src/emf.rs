// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use hashbrown::Equivalent;
use hashbrown::hash_map::EntryRef;
use itertools::Itertools;
use metrique_writer::sample::DefaultRng;
use metrique_writer::value::{FlagConstructor, ForceFlag, MetricOptions};
use metrique_writer_core::config::AllowUnroutableEntries;
use metrique_writer_core::format::Format;
use metrique_writer_core::sample::SampledFormat;
use metrique_writer_core::stream::IoStreamError;
use metrique_writer_core::{
    Entry, EntryConfig, MetricFlags, Observation, Unit, ValidationError, ValidationErrorBuilder,
    Value,
};
use rand::rngs::ThreadRng;
use rand::{Rng, RngCore};
use std::any::Any;
use std::fmt::{Display, Write};
use std::mem;
use std::num::NonZero;
use std::ops::Deref;
use std::time::Duration;
use std::{borrow::Cow, io, time::SystemTime};

use smallvec::{SmallVec, smallvec};

use crate::json_string::JsonString as _;
use crate::rate_limit::rate_limited;

use super::buf::{PrefixedStringBuf, write_all_vectored};

#[derive(Clone, Default)]
struct Validation {
    // validations are on-when-false to make Default enable all validations
    skip_validate_unique: bool,
    skip_validate_dimensions_exist: bool,
    skip_validate_names: bool,
}

/// The Amazon [Embedded Metric Format](https://docs.aws.amazon.com/AmazonCloudWatch/latest/monitoring/CloudWatch_Embedded_Metric_Format_Specification.html).
///
/// EMF is a format that allows for emitting CloudWatch Metrics from specially-formatted JSON CloudWatch Logs log events.
/// To emit the metrics, you just need to emit the log lines created by this struct into some CloudWatch Logs Log Stream
/// in your AWS account, no extra configuration needed. Most users would pipe the logs to a rotating file then use
/// [CloudWatch Agent] to upload the logs, but any way of calling [PutLogEvents] will work.
///
/// EMF requires there to be a timestamp in metrics. If your entry has an `#[entry(timestamp)]` field,
/// (or if you call [`EntryWriter::timestamp()`](metrique_writer::EntryWriter::timestamp) directly), that will
/// be used as the timestamp. Otherwise, a timestamp will be generated from [`SystemTime::now`] when
/// [`format`] is called.
///
/// [`SystemTime::now`]: std::time::SystemTime::now
/// [`format`]: [Emf::format]
///
/// EMF publishes metrics under a namespace and dimension-set, which you must pass to your builder as parameters.
///
/// ## Dimensions
///
/// This formatter creates a single EMF directive, which only has a single dimension-set. Therefore, if metrics are passed with metric-specific dimensions,
/// for example via [WithDimension](metrique_writer::value::WithDimension), there are currently 3 options:
/// 1. An error will occur (this is the default).
/// 2. If `allow_ignored_dimensions` is used, the metric will be emitted without the extra dimensions,
///    with just the `default_dimensions`.
/// 3. If the `AllowSplitEntries` config is enabled, there will be a separate entry
///    generated for each set of dimension values. This is generally the right thing to
///    do when emitting time-based metrics as it will cause them to be emitted correctly, but
///    the wrong thing to do when emitting unit-of-work based metrics.
///
/// In a future version, there might be additional options.
///
/// If you want to emit metrics with specific dimensions, you can add additional directives using the [`directive`](EmfBuilder::directive)
/// function. EMF ignores missing metrics and missing dimensions in dimension-sets, so if you emit a metric only
/// under specific conditions there is no problem with having the directive.
///
/// ## Metric emission format - scalar vs. histogram
///
/// The EMF formatter can emit metrics in 2 different forms:
///
/// 1. The traditional, "scalar" metrics. For example `{ "_aws": {...}, "MyMetric": 2 }`
/// 2. "Histogram" metrics. For example
///    `{ "_aws": {...}, "MyMetric": { "Values": [0.01, 0.17], "Counts": [10, 20] } }`,
///    which emits the `0.01` data point with multiplicity 10 and `0.17` data point with multiplicity 20.
///
/// The "histogram" form allows for emitting metrics with a large `Count` using O(1) cost. It is used
/// Therefore, it is used when there are multiple [`Observation`]s for a single metric, an
/// [`Observation::Repeated`], or [sampling].
///
/// CloudWatch EMF and CloudWatch Metrics support both forms equally well and "native" CloudWatch Metrics
/// statistics like sums, averages, and percentiles will work the same in both forms, with no extra
/// setup needed.
///
/// However, other tools that process the emitted logs, including [CloudWatch Logs Insights] queries as well as
/// any custom tooling that processes the log JSONs must to be adapted to support the format that
/// is being emitted.
///
/// [sampling]: Emf::with_sampling
/// [CloudWatch Logs Insights]: https://docs.aws.amazon.com/AmazonCloudWatch/latest/logs/AnalyzingLogData.html
///
/// ## Handling of non-finite floating-point values
///
/// Floating-point infinities are clamped (replaced with +f64::MAX or -f64::MAX).
///
/// NaN observations are skipped. If a metric has only NaN observations, it will be skipped.
/// In these cases, a rate-limited [`tracing`] error will be generated.
///
/// All observations other than the ones containing the NaN will be emitted as usual.
///
/// ## Examples
///
/// Here is an example of using [`Emf`] to format an [`Entry`] as a string:
///
/// ```
/// # use metrique_writer::{
/// #    Entry, EntryWriter,
/// #    format::{Format as _},
/// # };
/// # use metrique_writer_format_emf::Emf;
/// # use std::time::{Duration, SystemTime};
///
/// #[derive(Entry)]
/// #[entry(rename_all = "PascalCase")]
/// struct MyMetrics {
///     #[entry(timestamp)]
///     start: SystemTime,
///     my_field: u32,
/// }
///
/// let mut emf = Emf::all_validations("MyApp".to_string(), vec![vec![]]);
/// let mut output = Vec::new();
///
/// emf.format(&MyMetrics {
///     start: SystemTime::UNIX_EPOCH, // use SystemTime::now() in the real world
///     my_field: 4,
/// }, &mut output).unwrap();
///
/// let output = String::from_utf8(output).unwrap();
/// assert_json_diff::assert_json_eq!(serde_json::from_str::<serde_json::Value>(&output).unwrap(),
///     serde_json::json!({
///         "_aws": {
///             "CloudWatchMetrics": [
///                  {"Namespace": "MyApp", "Dimensions": [[]], "Metrics": [{"Name": "MyField"}]},
///             ],
///             "Timestamp": 0,
///         },
///         "MyField": 4,
///     })
/// );
/// ```
///
/// [CloudWatch Agent]: https://docs.aws.amazon.com/AmazonCloudWatch/latest/monitoring/Install-CloudWatch-Agent.html
/// [PutLogEvents]: https://docs.aws.amazon.com/AmazonCloudWatchLogs/latest/APIReference/API_PutLogEvents.html
#[derive(Clone)]
pub struct Emf {
    state: State,
    validation: Validation,
    validation_map_base: hashbrown::HashMap<SCow<'static>, LineData>,
}

#[derive(Clone)]
struct State {
    namespaces: Vec<JsonEncodedString>,
    each_dimensions_str: Vec<JsonEncodedArray>,
    log_group_and_timestamp: LogGroupNameAndTimestampString,
    dimension_set_map: hashbrown::HashMap<DimensionSet, MetricsForDimensionSet>,

    // buf that string fields can be added to
    string_fields_buf: PrefixedStringBuf,
    // buf that fields can be added to
    fields_buf: PrefixedStringBuf,
    // buf that metrics can be added to
    metrics_buf: PrefixedStringBuf,
    // buf that dimensions are added to. Used internally in `finish` and reset, not accumulator.
    dimensions_buf: PrefixedStringBuf,
    // index after the namespace in dimensions_buf
    after_namespace_index: usize,
    // internal buf used to publish metric counts. reset to empty between calls
    counts_buf: PrefixedStringBuf,
    // buf of extra declarations
    decl_buf: PrefixedStringBuf,
    allow_ignored_dimensions: bool,
}

/// Serde declaration of EMF's MetricDirective type
#[derive(serde::Serialize, Clone, Debug)]
pub struct MetricDirective<'a> {
    /// A DimensionSet array containing the dimension sets this metric will be emitted at
    #[serde(rename = "Dimensions")]
    pub dimensions: Vec<Vec<&'a str>>,
    /// The list of metrics to be emitted. This array MUST NOT contain more than
    /// 100 MetricDefinition objects.
    #[serde(rename = "Metrics")]
    pub metrics: Vec<MetricDefinition<'a>>,
    /// The namespace of the metrics to be emitted
    #[serde(rename = "Namespace")]
    pub namespace: &'a str,
}

/// Serde declaration of EMF's MetricDefinition type
#[derive(serde::Serialize, Copy, Clone, Debug)]
pub struct MetricDefinition<'a> {
    /// The name of the metric to be emitted
    #[serde(rename = "Name")]
    pub name: &'a str,
    /// The unit of the metric to be emitted
    #[serde(rename = "Unit")]
    pub unit: Unit,
    /// The storage resolution of the metric to be emitted
    ///
    /// If None, the metrics will be stored at the default resolution
    /// of 1/minute
    #[serde(rename = "StorageResolution")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub storage_resolution: Option<StorageResolution>,
}

/// Serde declaration of EMF's Unit type
#[derive(Copy, Clone, Debug)]
pub enum StorageResolution {
    /// Store the metric at a high storage resolution of 1/second
    Second = 1,
    /// Store the metric at a standard storage resolution of 1/minute. This is the default.
    Minute = 60,
}

/// Contains a JSON string that has been JSON-encoded
#[derive(Clone)]
struct JsonEncodedString {
    encoded_str: String,
}

impl JsonEncodedString {
    fn encode(input: &str) -> Self {
        JsonEncodedString {
            encoded_str: serde_json::to_string(input).expect("everything here is valid"),
        }
    }
}

impl Display for JsonEncodedString {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.encoded_str.fmt(f)
    }
}

/// Contains a JSON array that has been JSON-encoded
#[derive(Clone)]
struct JsonEncodedArray {
    encoded_array: String,
}

impl JsonEncodedArray {
    fn encode(input: &[String]) -> Self {
        JsonEncodedArray {
            encoded_array: serde_json::to_string(input).expect("everything here is valid"),
        }
    }

    fn extend_with_strings<'a>(self, extend_with: impl Iterator<Item = &'a str>) -> Self {
        let mut encoded_array = self.encoded_array;
        let mut first: bool = encoded_array.len() == 2; // []
        encoded_array.pop();
        for name in extend_with {
            if !first {
                encoded_array.push(',');
            }
            first = false;
            encoded_array.json_string(name);
        }
        encoded_array.push(']');
        JsonEncodedArray { encoded_array }
    }
}

impl Display for JsonEncodedArray {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.encoded_array.fmt(f)
    }
}

/// Contains the log group name and timestamp string
/// It is either
/// `],"LogGroupName":"log group","Timestamp":`
/// or
/// `],"Timestamp":`
#[derive(Clone)]
struct LogGroupNameAndTimestampString {
    encoded: String,
}

impl LogGroupNameAndTimestampString {
    fn new(log_group: Option<String>) -> Self {
        LogGroupNameAndTimestampString {
            encoded: if let Some(log_group) = log_group {
                format!(
                    r#"],"LogGroupName":{},"Timestamp":"#,
                    serde_json::to_string(&log_group).expect("everything here is valid")
                )
            } else {
                r#"],"Timestamp":"#.to_string()
            },
        }
    }
}

trait PushJsonSafeString {
    fn push_json_safe_string<'a>(&'a mut self, s: &JsonEncodedString) -> &'a mut Self;
    fn push_json_safe_array<'a>(&'a mut self, s: &JsonEncodedArray) -> &'a mut Self;
    fn push_json_safe_log_group_and_timestamp<'a>(
        &'a mut self,
        s: &LogGroupNameAndTimestampString,
        timestamp_str: &str,
    ) -> &'a mut Self;
}

impl PushJsonSafeString for PrefixedStringBuf {
    fn push_json_safe_string<'a>(&'a mut self, s: &JsonEncodedString) -> &'a mut Self {
        self.push_raw_str(&s.encoded_str)
    }

    fn push_json_safe_array<'a>(&'a mut self, s: &JsonEncodedArray) -> &'a mut Self {
        self.push_raw_str(&s.encoded_array)
    }

    fn push_json_safe_log_group_and_timestamp<'a>(
        &'a mut self,
        s: &LogGroupNameAndTimestampString,
        timestamp_str: &str,
    ) -> &'a mut Self {
        self.push_raw_str(&s.encoded).push_raw_str(timestamp_str)
    }
}

impl serde::Serialize for StorageResolution {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        (*self as u32).serialize(serializer)
    }
}

impl Emf {
    /// Create a builder for [`Emf`]
    ///
    /// This defaults to disabling some validations when debug assertions are disabled.
    pub fn builder(namespace: String, default_dimensions: Vec<Vec<String>>) -> EmfBuilder {
        assert!(
            !default_dimensions.is_empty(),
            "Without dimension sets no metrics can be published. Pass `default_dimensions=vec![vec![]]` to publish without dimensions"
        );
        EmfBuilder {
            namespaces: vec![namespace],
            default_dimensions,
            allow_ignored_dimensions: false,
            extra_directives: String::new(),
            log_group_name: None,
            #[cfg(debug_assertions)]
            validation: Validation::default(),
            #[cfg(not(debug_assertions))]
            validation: Validation {
                skip_validate_unique: true,
                skip_validate_dimensions_exist: true,
                skip_validate_names: true,
            },
        }
    }

    /// Turn on all validations for the Emf format.
    ///
    /// This is *expensive*. It can be 3-5x slower and take up significantly more memory depending on the entry
    /// contents. It's recommended to only enable all validations in debug builds. This is exactly what
    /// [`Emf::builder`] does.
    pub fn all_validations(namespace: String, default_dimensions: Vec<Vec<String>>) -> Self {
        Self::builder(namespace, default_dimensions).build()
    }

    /// Turn off all optional validations for the Emf format
    ///
    /// This is substantially faster than [`Emf::all_validations()`] because it can avoid storing a hash map
    /// of seen fields. It's still recommended to enable validations during tests and in debug builds.
    ///
    /// Note that skipping validations is **only intended to be used to improve performance for code
    /// that has tests that demonstrate that it is only creating valid metrics**, NOT to intentionally
    /// pass invalid input to the formatter. Metric records that cause an error with `all_validations`
    /// are considered a program error, and might start failing in newer versions of this library.
    pub fn no_validations(namespace: String, default_dimensions: Vec<Vec<String>>) -> Self {
        Self::builder(namespace, default_dimensions)
            .skip_all_validations(true)
            .build()
    }

    // every sample's counts are emitted with multiplicity `multiplicity` to account for sampling.
    fn format_with_multiplicity(
        &mut self,
        entry: &impl Entry,
        output: &mut impl io::Write,
        multiplicity: Option<u64>,
    ) -> Result<(), IoStreamError> {
        self.state.string_fields_buf.clear();
        self.state.fields_buf.clear();
        self.state.metrics_buf.clear();
        self.state.decl_buf.clear();
        self.state.dimension_set_map.clear();

        // counts_buf is cleared when returning
        let mut writer = EntryWriter {
            validation_map: if self.validation.skip_validate_dimensions_exist {
                hashbrown::HashMap::new()
            } else {
                self.validation_map_base.clone()
            },
            entry_dimensions: None,
            state: &mut self.state,
            multiplicity,
            timestamp: None,
            validations: &self.validation,
            error: ValidationErrorBuilder::default(),
            allow_split_entries: false,
            is_allow_unroutable_entries: false,
        };

        entry.write(&mut writer);
        writer.finish(output)
    }

    /// Wrap the given `Emf` with support for sampling with the default RNG.
    ///
    /// Sampling is designed to solve the problem that a high rate of metric emission can
    /// consume a large amount of resources. It allows a sampling combinator like
    /// [`sample_by_congress_at_fixed_entries_per_second`] or [`sample_by_fixed_fraction`]
    /// to give every [`Entry`] emission event a "sample rate", and only emit the
    /// [`Entry`] with that probability (and drop it otherwise). The more sophisticated
    /// sampling implementations vary the sample rate to ensure that a rate limit is
    /// not significantly exceeded and that rare events (for example, error entries) remain
    /// sampled.
    ///
    /// To ensure that aggregate statistics (for example, sums or averages) of sampled
    /// metrics remain accurate, especially with per-metric sample rates, every metric
    /// that is "successfully" sampled is counted with a "weight" that is equivalent to
    /// the inverse of its rate. For example, if the sample rate of an entry is 0.5,
    /// a "successfully" sampled entry is supposed to count with a "weight" of 2.
    /// If the sample rate is 0.01, the sampled entry will count with a "weight" of 100.
    ///
    /// The way this weighting is done in the EMF formatter is by making each metric count as
    /// **duplicate datapoints**.
    ///
    /// ### Integer Multiplicity (and the RNG)
    ///
    /// Since in CloudWatch, the multiplicity of a datapoint is required to be an integer,
    /// the EMF emitter will randomly pick between either the floor or the
    /// ceiling of the inverse rate with the right probability to ensure the statistics are
    /// unbiased.
    ///
    /// For example, if a metric with a sample rate of 0.3 is sampled, it will be emitted with a
    /// multiplicity of 3 with probability 2/3 and multiplicity of 4 with probability 1/3,
    /// which leads to the following distribution:
    ///
    ///  - 70% - metric is not sampled
    ///  - 30% - metric is sampled
    ///    - 20% - metric is sampled with multiplicity 3
    ///    - 10% - metric is sampled with multiplicity 4
    ///
    /// And as you can see, the expected multiplicity is `0.2 * 3 + 0.1 * 4 = 1`, which ensures
    /// there is no bias.
    ///
    /// The RNG that is picked here (in this function to be the [ThreadRng], or the caller-specified
    /// RNG passed in [`with_sampling_and_rng`]) is solely used to pick the integer multiplicity.
    /// The actual "sampling" RNG used to pick which entries are sampled is a part of the sampler
    /// implementation - to customize it, you would need to for example use
    /// [`CongressSampleBuilder::build_with_rng`].
    ///
    /// Of course, if you want determinism (for example, for testing), you need to pass both RNGs,
    /// and of course, the precise way random numbers are selected is not stable between different
    /// versions of this crate.
    ///
    /// ### Metric Emission Format
    ///
    /// Using `with_sampling` will make all metrics be emitted in the histogram format (even if the
    /// sampling rate is 1). CloudWatch Metrics will handle the histogram format just fine, but
    /// you should make sure that any queries (using [CloudWatch Logs Insights]
    /// or any other way to read the logs directly) you have processing the metrics expect
    /// that format. Read the [`Emf`] docs for more details.
    ///
    /// [CloudWatch Logs Insights]: https://docs.aws.amazon.com/AmazonCloudWatch/latest/logs/AnalyzingLogData.html
    /// [`sample_by_congress_at_fixed_entries_per_second`]: metrique_writer::sample::SampledFormatExt::sample_by_congress_at_fixed_entries_per_second
    /// [`sample_by_fixed_fraction`]: metrique_writer::sample::SampledFormatExt::sample_by_fixed_fraction
    /// [`CongressSampleBuilder::build_with_rng`]: metrique_writer::sample::CongressSampleBuilder::build_with_rng
    /// [`with_sampling_and_rng`]: Self::with_sampling_and_rng
    pub fn with_sampling(self) -> SampledEmf {
        Self::with_sampling_and_rng(self, Default::default())
    }

    /// Wrap the given `Emf` with support for sampling with an explicit RNG.
    ///
    /// The RNG is only used to pick the integer sampling multiplicity,
    /// a separate RNG is used for the actual sampling. See [`Self::with_sampling`].
    pub fn with_sampling_and_rng<R>(self, rng: R) -> SampledEmf<R> {
        SampledEmf { emf: self, rng }
    }
}

/// Builder for [`Emf`]
#[derive(Clone)]
pub struct EmfBuilder {
    default_dimensions: Vec<Vec<String>>,
    extra_directives: String,
    namespaces: Vec<String>,
    validation: Validation,
    allow_ignored_dimensions: bool,
    log_group_name: Option<String>,
}

impl EmfBuilder {
    /// Build an [`Emf`] for formatting metrics with the input configuration
    ///
    /// ## Example
    ///
    /// This will publish the metrics to both the `MyApp` and the `MyApp2` namespaces,
    /// under the dimension sets [["Operation"], ["Operation", "Status"]]:
    ///
    /// ```
    /// # use metrique_writer::{
    /// #    Entry, EntryWriter,
    /// #    format::{Format as _},
    /// # };
    /// # use metrique_writer_format_emf::Emf;
    /// # use std::time::{Duration, SystemTime};
    ///
    /// #[derive(Entry)]
    /// #[entry(rename_all = "PascalCase")]
    /// struct MyMetrics {
    ///     #[entry(timestamp)]
    ///     start: SystemTime,
    ///     my_field: u32,
    ///     operation: &'static str,
    ///     status: &'static str,
    /// }
    ///
    /// let dimension_sets = vec![
    ///     vec!["Operation".into()],
    ///     vec!["Operation".into(), "Status".into()]
    /// ];
    /// let mut emf = Emf::builder("MyApp".to_string(), dimension_sets)
    ///     .add_namespace("MyApp2".to_string())
    ///     .build();
    /// let mut output = Vec::new();
    ///
    /// emf.format(&MyMetrics {
    ///     start: SystemTime::UNIX_EPOCH, // use SystemTime::now() in the real world
    ///     my_field: 4,
    ///     operation: "Foo",
    ///     status: "200",
    /// }, &mut output).unwrap();
    ///
    /// let output = String::from_utf8(output).unwrap();
    /// assert_json_diff::assert_json_eq!(serde_json::from_str::<serde_json::Value>(&output).unwrap(),
    ///     serde_json::json!({
    ///         "_aws": {
    ///             "CloudWatchMetrics": [
    ///                  {"Namespace": "MyApp", "Dimensions": [["Operation"], ["Operation", "Status"]], "Metrics": [{"Name": "MyField"}]},
    ///                  {"Namespace": "MyApp2", "Dimensions": [["Operation"], ["Operation", "Status"]], "Metrics": [{"Name": "MyField"}]},
    ///             ],
    ///             "Timestamp": 0,
    ///         },
    ///         "MyField": 4,
    ///         "Operation": "Foo",
    ///         "Status": "200",
    ///     })
    /// );
    /// ```
    pub fn build(self) -> Emf {
        let mut validation_map = hashbrown::HashMap::new();
        for dimension_set in &self.default_dimensions {
            for dimension in dimension_set {
                validation_map.entry_ref(dimension).or_insert(LineData {
                    kind: LineKind::UnfoundDimension,
                });
            }
        }
        assert!(
            !self.namespaces.is_empty(),
            "must publish to at least 1 namespace"
        );

        let each_dimensions_str: Vec<JsonEncodedArray> = self
            .default_dimensions
            .iter()
            .map(|x| JsonEncodedArray::encode(x))
            .collect();
        let namespaces: Vec<JsonEncodedString> = self
            .namespaces
            .iter()
            .map(|x| JsonEncodedString::encode(x))
            .collect();
        let first_ns: &JsonEncodedString = &namespaces[0];
        let dimensions_after_ns = r#","Dimensions":["#;
        let dimensions_prefix = &format!(
            r#"{{"_aws":{{"CloudWatchMetrics":[{{"Namespace":{first_ns}{dimensions_after_ns}"#
        );
        Emf {
            state: State {
                namespaces,
                each_dimensions_str,
                dimension_set_map: hashbrown::HashMap::new(),
                after_namespace_index: dimensions_prefix.len() - dimensions_after_ns.len(),
                dimensions_buf: PrefixedStringBuf::new(dimensions_prefix, 256),
                fields_buf: PrefixedStringBuf::new("}", 2048),
                string_fields_buf: PrefixedStringBuf::new("", 2048),
                counts_buf: PrefixedStringBuf::new(r#"],"Counts":["#, 256),
                metrics_buf: PrefixedStringBuf::new(r#"],"Metrics":["#, 2048),
                decl_buf: PrefixedStringBuf::new(&self.extra_directives, 256),
                allow_ignored_dimensions: self.allow_ignored_dimensions,
                log_group_and_timestamp: LogGroupNameAndTimestampString::new(self.log_group_name),
            },
            validation_map_base: validation_map,
            validation: self.validation,
        }
    }

    /// Adds a custom EMF directive, to allow emitting specific metrics with custom dimensions.
    ///
    /// This function is intended to allow emitting the precise desired metric directive, and
    /// currently everything has to be specified manually.
    ///
    /// Note that EMF skips metric definitions that refer to metrics that don't exist, so if
    /// you have a metric that appears only in some entries it is OK to emit a directive for it.
    ///
    /// ## Example
    ///
    /// This will publish all metrics with dimensions `["Operation"]`, and will also publish
    /// the `time_taken` field with dimensions `["Operation", "MachineType"]`.
    ///
    /// ```
    /// # use metrique_writer::{
    /// #    Entry, EntryWriter,
    /// #    format::{Format as _},
    /// #    unit::{Unit, NegativeScale},
    /// # };
    /// # use metrique_writer_format_emf::{
    /// #     Emf, MetricDefinition,
    /// #     MetricDirective, StorageResolution,
    /// # };
    /// # use std::time::{Duration, SystemTime};
    ///
    /// #[derive(Entry)]
    /// #[entry(rename_all = "PascalCase")]
    /// struct MyMetrics {
    ///     #[entry(timestamp)]
    ///     start: SystemTime,
    ///     my_field: u32,
    ///     time_taken: Duration,
    ///     operation: &'static str,
    ///     machine_type: &'static str,
    /// }
    ///
    /// let dimension_sets = vec![vec!["Operation".into()]];
    /// let mut emf = Emf::builder("MyApp".to_string(), dimension_sets)
    ///     .directive(MetricDirective {
    ///         dimensions: vec![vec!["Operation", "MachineType"]],
    ///         metrics: vec![MetricDefinition {
    ///             name: "TimeTaken",
    ///             // there is no "unit rescaling" done, you must get the correct unit used,
    ///             // which for Duration is milliseconds
    ///             unit: Unit::Second(NegativeScale::Milli),
    ///             storage_resolution: None,
    ///         }],
    ///         namespace: "MyApp",
    ///     })
    ///     .build();
    /// let mut output = Vec::new();
    ///
    /// emf.format(&MyMetrics {
    ///     start: SystemTime::UNIX_EPOCH, // use SystemTime::now() in the real world
    ///     my_field: 4,
    ///     time_taken: Duration::from_micros(1_200),
    ///     operation: "Foo",
    ///     machine_type: "r7g.2xlarge",
    /// }, &mut output).unwrap();
    ///
    /// let output = String::from_utf8(output).unwrap();
    /// assert_json_diff::assert_json_eq!(serde_json::from_str::<serde_json::Value>(&output).unwrap(),
    ///     serde_json::json!({
    ///         "_aws": {
    ///             "CloudWatchMetrics": [
    ///                  {"Namespace": "MyApp", "Dimensions": [["Operation"]], "Metrics": [
    ///                      {"Name": "MyField"},
    ///                      {"Name": "TimeTaken", "Unit": "Milliseconds"}
    ///                  ]},
    ///                  {"Namespace": "MyApp", "Dimensions": [["Operation", "MachineType"]], "Metrics": [
    ///                      {"Name": "TimeTaken", "Unit": "Milliseconds"}
    ///                  ]},
    ///             ],
    ///             "Timestamp": 0,
    ///         },
    ///         "MyField": 4,
    ///         "TimeTaken": 1.2,
    ///         "Operation": "Foo",
    ///         "MachineType": "r7g.2xlarge",
    ///     })
    /// );
    /// ```
    pub fn directive(mut self, directive: MetricDirective) -> Self {
        self.extra_directives.push(',');
        // injection-safe because this is pushing directly after serialization
        self.extra_directives
            .push_str(&serde_json::to_string(&directive).expect("nothing that can fail here"));
        self
    }

    /// Enable ignored dimensions mode, in which per-metric dimensions are skipped
    pub fn allow_ignored_dimensions(mut self, allow: bool) -> Self {
        self.allow_ignored_dimensions = allow;
        self
    }

    /// If `skip` is true, turns skipping validations on.
    ///
    /// Note that skipping validations is **only intended to be used to improve performance for code
    /// that has tests that demonstrate that it is only creating valid metrics**, NOT to intentionally
    /// pass invalid input to the formatter. Metric records that cause an error with `all_validations`
    /// are considered a program error, and might start failing in newer versions of this library.
    pub fn skip_all_validations(mut self, skip: bool) -> Self {
        self.validation.skip_validate_unique |= skip;
        self.validation.skip_validate_dimensions_exist |= skip;
        self.validation.skip_validate_names |= skip;
        self
    }

    /// Add an additional namespace to this builder
    ///
    /// All metrics will be published to all namespaces by creating multiple
    /// MetricDirective objects.
    ///
    /// ## Examples
    ///
    /// This will publish the metrics to both the `MyApp` and the `MyApp2` namespaces:
    ///
    /// ```
    /// # use metrique_writer::{
    /// #    Entry, EntryWriter,
    /// #    format::{Format as _},
    /// # };
    /// # use metrique_writer_format_emf::Emf;
    /// # use std::time::{Duration, SystemTime};
    ///
    /// #[derive(Entry)]
    /// #[entry(rename_all = "PascalCase")]
    /// struct MyMetrics {
    ///     #[entry(timestamp)]
    ///     start: SystemTime,
    ///     my_field: u32,
    /// }
    ///
    /// let mut emf = Emf::builder("MyApp".to_string(), vec![vec![]])
    ///     .add_namespace("MyApp2".to_string())
    ///     .build();
    /// let mut output = Vec::new();
    ///
    /// emf.format(&MyMetrics {
    ///     start: SystemTime::UNIX_EPOCH, // use SystemTime::now() in the real world
    ///     my_field: 4,
    /// }, &mut output).unwrap();
    ///
    /// let output = String::from_utf8(output).unwrap();
    /// assert_json_diff::assert_json_eq!(serde_json::from_str::<serde_json::Value>(&output).unwrap(),
    ///     serde_json::json!({
    ///         "_aws": {
    ///             "CloudWatchMetrics": [
    ///                  {"Namespace": "MyApp", "Dimensions": [[]], "Metrics": [{"Name": "MyField"}]},
    ///                  {"Namespace": "MyApp2", "Dimensions":[[]], "Metrics": [{"Name": "MyField"}]},
    ///             ],
    ///             "Timestamp": 0,
    ///         },
    ///         "MyField": 4,
    ///     })
    /// );
    /// ```
    pub fn add_namespace(mut self, namespace: impl Into<String>) -> Self {
        self.namespaces.push(namespace.into());
        self
    }

    /// Set the log group name for this builder.
    ///
    /// This is used when publishing to [CloudWatch Agent via TCP or UDP][cwa-tcp-udp],
    /// to select the destination log group name.
    ///
    /// When publishing via the file-based CloudWatch Agent interface, the log group name
    /// is instead selected by CloudWatch Agent configuration, so this is not needed.
    ///
    /// This field is not read by EMF itself (outside of the CloudWatch Agent).
    ///
    /// [cwa-tcp-udp]: https://docs.aws.amazon.com/AmazonCloudWatch/latest/monitoring/CloudWatch_Embedded_Metric_Format_Generation_CloudWatch_Agent.html#CloudWatch_Embedded_Metric_Format_Generation_CloudWatch_Agent_Send_Logs
    ///
    /// ## Examples
    ///
    /// This will publish the metrics to the `Foo` log group:
    ///
    /// ```
    /// # use metrique_writer::{
    /// #    Entry, EntryWriter,
    /// #    format::{Format as _},
    /// # };
    /// # use metrique_writer_format_emf::Emf;
    /// # use std::time::{Duration, SystemTime};
    ///
    /// #[derive(Entry)]
    /// #[entry(rename_all = "PascalCase")]
    /// struct MyMetrics {
    ///     #[entry(timestamp)]
    ///     start: SystemTime,
    ///     my_field: u32,
    /// }
    ///
    /// let mut emf = Emf::builder("MyApp".to_string(), vec![vec![]])
    ///     .log_group_name("Foo".to_string())
    ///     .build();
    /// let mut output = Vec::new();
    ///
    /// emf.format(&MyMetrics {
    ///     start: SystemTime::UNIX_EPOCH, // use SystemTime::now() in the real world
    ///     my_field: 4,
    /// }, &mut output).unwrap();
    ///
    /// let output = String::from_utf8(output).unwrap();
    /// assert_json_diff::assert_json_eq!(serde_json::from_str::<serde_json::Value>(&output).unwrap(),
    ///     serde_json::json!({
    ///         "_aws": {
    ///             "CloudWatchMetrics": [
    ///                  {"Namespace": "MyApp", "Dimensions": [[]], "Metrics": [{"Name": "MyField"}]},
    ///             ],
    ///             "LogGroupName": "Foo",
    ///             "Timestamp": 0,
    ///         },
    ///         "MyField": 4,
    ///     })
    /// );
    /// ```
    pub fn log_group_name(mut self, log_group_name: impl Into<String>) -> Self {
        self.log_group_name = Some(log_group_name.into());
        self
    }
}

#[derive(Clone)]
enum LineKind {
    // this is a string
    String,
    // this is a metric
    Metric { indexes: bit_set::BitSet<u32> },
    // this is a dimension that needs to be filled
    UnfoundDimension,
}

#[derive(Clone)]
struct LineData {
    kind: LineKind,
}

#[derive(Clone, Hash, PartialEq, Eq, Debug)]
struct DimensionSet {
    entry: SmallVec<[(String, String); 2]>,
}

#[derive(Hash, PartialEq, Eq, Debug)]
struct DimensionSetKey<'a> {
    entry: SmallVec<[(&'a str, &'a str); 2]>,
}

impl<'a> FromIterator<(&'a str, &'a str)> for DimensionSetKey<'a> {
    fn from_iter<T: IntoIterator<Item = (&'a str, &'a str)>>(iter: T) -> Self {
        let mut res = DimensionSetKey {
            entry: FromIterator::from_iter(iter),
        };
        res.entry.sort();
        res
    }
}

/// A version of Cow<'a, str> that implements Equivalent and From to allow use in an hashbrown::HashMap
#[derive(Clone, Hash, PartialEq, Eq)]
struct SCow<'a>(Cow<'a, str>);

impl<'a> Deref for SCow<'a> {
    type Target = Cow<'a, str>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Equivalent<SCow<'_>> for String {
    fn equivalent(&self, key: &SCow<'_>) -> bool {
        **self == *key.0
    }
}

impl Equivalent<SCow<'_>> for &'_ str {
    fn equivalent(&self, key: &SCow<'_>) -> bool {
        **self == *key.0
    }
}

impl std::borrow::Borrow<str> for SCow<'_> {
    fn borrow(&self) -> &str {
        &self.0
    }
}

impl From<&'_ String> for SCow<'_> {
    fn from(value: &String) -> Self {
        SCow(Cow::Owned(value.clone()))
    }
}

impl<'a> From<&'a str> for SCow<'a> {
    fn from(value: &'a str) -> Self {
        SCow(Cow::Borrowed(value))
    }
}

impl<'a> From<&'_ SCow<'a>> for SCow<'a> {
    fn from(value: &SCow<'a>) -> Self {
        match value {
            SCow(Cow::Borrowed(s)) => SCow(Cow::Borrowed(s)),
            SCow(Cow::Owned(o)) => SCow(Cow::Owned(o.clone())),
        }
    }
}

impl Equivalent<DimensionSet> for DimensionSetKey<'_> {
    fn equivalent(&self, key: &DimensionSet) -> bool {
        self.entry.len() == key.entry.len()
            && self
                .entry
                .iter()
                .zip(&key.entry)
                .all(|((n1, v1), (n2, v2))| n1 == n2 && v1 == v2)
    }
}

impl From<&'_ DimensionSetKey<'_>> for DimensionSet {
    fn from(val: &'_ DimensionSetKey<'_>) -> Self {
        Self {
            entry: val
                .entry
                .iter()
                .map(|&(k, v)| (k.to_owned(), v.to_owned()))
                .collect(),
        }
    }
}

#[derive(Clone)]
struct MetricsForDimensionSet {
    fields_buf: PrefixedStringBuf,
    metrics_buf: PrefixedStringBuf,
    // an index into "metrics_buf" after the end of the namespace
    after_namespace_index: usize,
    index: NonZero<usize>,
}

impl MetricsForDimensionSet {
    fn new(
        namespace_str: &JsonEncodedString,
        each_dimensions_str: &[JsonEncodedArray],
        variable_dimensions: &DimensionSetKey<'_>,
        index: NonZero<usize>,
    ) -> Self {
        let dimensions_str = each_dimensions_str
            .iter()
            .map(|dim_s| {
                dim_s.to_owned().extend_with_strings(
                    variable_dimensions.entry.iter().map(|&(name, _value)| name),
                )
            })
            .join(",");
        let mut metrics_buf = String::with_capacity(2048);
        write!(
            metrics_buf,
            r#"{{"_aws":{{"CloudWatchMetrics":[{{"Namespace":{namespace_str}"#
        )
        .ok();
        let after_namespace_index = metrics_buf.len();
        write!(
            metrics_buf,
            r#","Dimensions":[{dimensions_str}],"Metrics":["#
        )
        .ok();
        let mut fields_buf = String::with_capacity(2048);
        fields_buf.push('}');
        // push the strings for the variable dimensions
        for (name, value) in &variable_dimensions.entry {
            fields_buf.push(',');
            fields_buf.json_string(name);
            fields_buf.push(':');
            fields_buf.json_string(value);
        }
        Self {
            fields_buf: PrefixedStringBuf::from_prefix(fields_buf),
            metrics_buf: PrefixedStringBuf::from_prefix(metrics_buf),
            after_namespace_index,
            index,
        }
    }
}

pub use metrique_writer_core::config::{AllowSplitEntries, EntryDimensions};

struct EntryWriter<'a> {
    validation_map: hashbrown::HashMap<SCow<'a>, LineData>,
    state: &'a mut State,
    entry_dimensions: Option<Vec<JsonEncodedArray>>,
    validations: &'a Validation,
    timestamp: Option<SystemTime>,
    multiplicity: Option<u64>,
    error: ValidationErrorBuilder,
    allow_split_entries: bool,
    is_allow_unroutable_entries: bool,
}

impl<'a> metrique_writer_core::EntryWriter<'a> for EntryWriter<'a> {
    fn timestamp(&mut self, timestamp: SystemTime) {
        if self.timestamp.replace(timestamp).is_some() {
            self.error.invalid_mut("multiple timestamps written");
        }
    }

    fn value(&mut self, name: impl Into<Cow<'a, str>>, value: &(impl Value + ?Sized)) {
        let name = name.into();
        if self.validate_name(&name) {
            value.write(ValueWriter {
                name: SCow(name),
                entry: self,
            });
        }
    }

    fn config(&mut self, config: &'a dyn EntryConfig) {
        if let Some(dimensions) = (config as &dyn Any).downcast_ref::<EntryDimensions>() {
            if !self.state.dimension_set_map.is_empty() {
                self.error.invalid_mut("entry dimensions must be configured before emitting a metric with custom dimensions");
                return;
            }
            if self.entry_dimensions.is_some() {
                self.error
                    .invalid_mut("entry dimensions cannot be set twice");
                return;
            }
            if dimensions.is_empty() {
                self.error.invalid_mut("entry dimensions cannot be empty");
                return;
            }
            if !self.validations.skip_validate_unique
                || !self.validations.skip_validate_dimensions_exist
            {
                for dim_set in dimensions.dim_sets() {
                    for dim in dim_set {
                        match self.validation_map.entry_ref(dim) {
                            hashbrown::hash_map::EntryRef::Occupied(e) => match e.get() {
                                LineData {
                                    kind: LineKind::UnfoundDimension | LineKind::String,
                                } => {}
                                LineData {
                                    kind: LineKind::Metric { .. },
                                } => {
                                    if !self.validations.skip_validate_unique {
                                        self.error.extend_mut(
                                            ValidationError::invalid("duplicate field")
                                                .for_field(dim),
                                        );
                                    }
                                }
                            },
                            hashbrown::hash_map::EntryRef::Vacant(v) => {
                                v.insert(LineData {
                                    kind: LineKind::UnfoundDimension,
                                });
                            }
                        }
                    }
                }
            }
            // FIXME: this does a bunch of allocations. If there are performance problems here, it's probably
            // good to do some caching since there are probably not many EntryDimensions values (one for
            // every metric "shape", of which I expect to be O(1)).
            let dimensions: Vec<JsonEncodedArray> = self
                .state
                .each_dimensions_str
                .iter()
                .flat_map(|d| {
                    dimensions
                        .dim_sets()
                        .map(|e| d.clone().to_owned().extend_with_strings(e))
                })
                .collect();
            self.entry_dimensions = Some(dimensions);
        }
        if (config as &dyn Any)
            .downcast_ref::<AllowSplitEntries>()
            .is_some()
        {
            self.allow_split_entries = true;
        }
        if (config as &dyn Any)
            .downcast_ref::<AllowUnroutableEntries>()
            .is_some()
        {
            self.is_allow_unroutable_entries = true;
        }
    }
}

impl EntryWriter<'_> {
    fn finish(mut self, output: &mut impl io::Write) -> Result<(), IoStreamError> {
        if !self.validations.skip_validate_dimensions_exist && !self.is_allow_unroutable_entries {
            for (dim, value) in self.validation_map.iter_mut() {
                if let LineData {
                    kind: LineKind::UnfoundDimension,
                } = value
                {
                    self.error
                        .extend_mut(ValidationError::invalid("missing dimension").for_field(dim));
                }
            }
        }

        let timestamp = self.timestamp.unwrap_or_else(SystemTime::now);
        let unix = timestamp
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default();
        let mut timestamp_buf = itoa::Buffer::new();
        let timestamp_str = timestamp_buf.format(unix.as_millis());
        self.error.build()?;
        self.state
            .decl_buf
            // safe because timestamp is a number
            .push_json_safe_log_group_and_timestamp(
                &self.state.log_group_and_timestamp,
                timestamp_str,
            );
        self.state.string_fields_buf.push_raw_str("}\n");

        let mut emitted_any_dimension_metrics = false;

        for entry in self.state.dimension_set_map.values_mut() {
            entry.metrics_buf.push_raw_str("]}");
            let metrics_len = entry.metrics_buf.as_str().len();
            for namespace in &self.state.namespaces[1..] {
                entry
                    .metrics_buf
                    .push_raw_str(r#",{"Namespace":"#)
                    .push_json_safe_string(namespace)
                    .extend_from_within_range(entry.after_namespace_index, metrics_len);
            }
            entry
                .metrics_buf
                // safe because timestamp is a number
                .push_json_safe_log_group_and_timestamp(
                    &self.state.log_group_and_timestamp,
                    timestamp_str,
                );
            let buf: SmallVec<[_; 3]> = smallvec![
                entry.metrics_buf.as_ref(),
                entry.fields_buf.as_ref(),
                self.state.string_fields_buf.as_ref(),
            ];
            if entry.fields_buf.is_empty() {
                // skip metric line with no metrics
                continue;
            }
            emitted_any_dimension_metrics = true;
            write_all_vectored(buf, output)?;
        }

        // if we emitted any dimensioned line and there are no fields with no dimensions,
        // the "no-dimensions" line is redundant. However, make sure we emit at least
        // 1 line to ensure there is always some kind of life sign.
        if !emitted_any_dimension_metrics || !self.state.fields_buf.is_empty() {
            self.state.dimensions_buf.clear();
            let mut first = true;
            for dimension in self
                .entry_dimensions
                .as_deref()
                .unwrap_or(&self.state.each_dimensions_str)
            {
                if !mem::replace(&mut first, false) {
                    self.state.dimensions_buf.push(',');
                }
                self.state.dimensions_buf.push_json_safe_array(dimension);
            }
            self.state.metrics_buf.push_raw_str("]}");
            let metrics_len = self.state.metrics_buf.as_str().len();
            for namespace in &self.state.namespaces[1..] {
                self.state
                    .metrics_buf
                    .push_raw_str(r#",{"Namespace":"#)
                    .push_json_safe_string(namespace)
                    // safe because dimensions_buf[after_namespace_index..]
                    // contains valid dimensions
                    .push_raw_str(
                        &self.state.dimensions_buf.as_str()[self.state.after_namespace_index..],
                    )
                    // safe because this is valid JSON
                    .extend_from_within_range(0, metrics_len);
            }
            // it's OK to write each line with a separate call to `write_all_vectored`,
            // since nothing bad occurs if lines are split.
            let buf: SmallVec<[_; 5]> = smallvec![
                self.state.dimensions_buf.as_ref(),
                self.state.metrics_buf.as_ref(),
                self.state.decl_buf.as_ref(),
                self.state.fields_buf.as_ref(),
                self.state.string_fields_buf.as_ref(),
            ];
            write_all_vectored(buf, output)?;
        }
        Ok(())
    }

    fn validate_name(&mut self, name: &str) -> bool {
        if !self.validations.skip_validate_names {
            if name.is_empty() {
                self.error
                    .extend_mut(ValidationError::invalid("name can't be empty").for_field(""));
                return false;
            }
            if name == "_aws" {
                self.error
                    .extend_mut(ValidationError::invalid("name can't be `_aws`").for_field("_aws"));
                return false;
            }
        }
        true
    }
}

struct FiniteFloat(f64);

fn clamp_to_finite(float: f64, name_for_log: &str) -> Option<FiniteFloat> {
    let float = float.clamp(-f64::MAX, f64::MAX);
    if !float.is_finite() {
        rate_limited!(
            Duration::from_secs(1),
            tracing::error!(
                message="skipping emitting metric with NaN value",
                metric=%name_for_log,
            )
        );
        None
    } else {
        Some(FiniteFloat(float))
    }
}

struct MetricSkipped;

struct ValueWriter<'a, 'e> {
    name: SCow<'e>,
    entry: &'a mut EntryWriter<'e>,
}

impl ValueWriter<'_, '_> {
    fn write_float(buf: &mut PrefixedStringBuf, v: FiniteFloat) {
        assert!(v.0.is_finite(), "should be checked by the caller");
        let mut dtoa_buf = dtoa::Buffer::new();
        let as_str = dtoa_buf.format_finite(v.0);
        // injection-safe since this is a number
        buf.push_raw_str(as_str.strip_suffix(".0").unwrap_or(as_str));
    }

    // return Err(MetricSkipped) if the observation has been skipped due to being NaN
    fn write_observation(
        buf: &mut PrefixedStringBuf,
        counts: &mut PrefixedStringBuf,
        observation: Observation,
        multiplicity: Option<u64>,
        // used purely for logging if there is a NaN
        name_for_log: &str,
    ) -> Result<(), MetricSkipped> {
        let multiplicity = multiplicity.unwrap_or(1);
        match observation {
            Observation::Unsigned(v) => {
                buf.push_integer(v);
                counts.push_integer(multiplicity);
                Ok(())
            }
            Observation::Floating(v) => {
                if let Some(v) = clamp_to_finite(v, name_for_log) {
                    Self::write_float(buf, v);
                    counts.push_integer(multiplicity);
                    Ok(())
                } else {
                    Err(MetricSkipped)
                }
            }
            Observation::Repeated { total, occurrences } => {
                let mean = if occurrences == 0 {
                    0.0
                } else {
                    total / occurrences as f64
                };
                if let Some(mean) = clamp_to_finite(mean, name_for_log) {
                    Self::write_float(buf, mean);
                    counts.push_integer(occurrences.saturating_mul(multiplicity));
                    Ok(())
                } else {
                    Err(MetricSkipped)
                }
            }
            _ => {
                // shouldn't actually happen unless there is a version mismatch,
                // but Observation is `#[non_exhaustive]`. Do something reasonable.
                rate_limited!(
                    Duration::from_secs(1),
                    tracing::error!(
                        message="skipping emitting metric due to unknown observation type",
                        metric=%name_for_log,
                    )
                );
                Err(MetricSkipped)
            }
        }
    }

    // return Err(MetricSkipped) and writes only to `buf` and `counts_buf`
    // (not touching `fields_buf`) if the metric is NaN
    fn write_metric_value(
        name: &str,
        fields_buf: &mut PrefixedStringBuf,
        counts_buf: &mut PrefixedStringBuf,
        first: Observation,
        mut distribution: impl Iterator<Item = Observation>,
        multiplicity: Option<u64>,
    ) -> Result<(), MetricSkipped> {
        let buf: &mut PrefixedStringBuf = fields_buf;
        buf.push(',').json_string(name).push(':');
        match (first, distribution.next()) {
            (Observation::Unsigned(v), None) if multiplicity.is_none() => {
                buf.push_integer(v);
                Ok(())
            }
            (Observation::Floating(v), None) if multiplicity.is_none() => {
                if let Some(v) = clamp_to_finite(v, name) {
                    Self::write_float(buf, v);
                    Ok(())
                } else {
                    Err(MetricSkipped)
                }
            }
            (first, second) => {
                let counts = counts_buf;
                buf.push_raw_str(r#"{"Values":["#);
                let mut wrote_anything = false;
                counts.clear(); // clear before to make sure there is no risk
                let mut wrote =
                    Self::write_observation(buf, counts, first, multiplicity, name).is_ok();
                wrote_anything |= wrote;
                for observation in second.into_iter().chain(distribution) {
                    if wrote {
                        buf.push(',');
                        counts.push(',');
                    }
                    wrote = Self::write_observation(buf, counts, observation, multiplicity, name)
                        .is_ok();
                    wrote_anything |= wrote;
                }
                // injection-safe because this is a comma-separated list of numbers
                buf.push_raw_str(counts.as_str());
                counts.clear(); // clear after to ensure this is not too big
                buf.push_raw_str("]}");
                if wrote_anything {
                    Ok(())
                } else {
                    Err(MetricSkipped)
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn write_metric(
        name: &str,
        fields_buf: &mut PrefixedStringBuf,
        metrics_buf: &mut PrefixedStringBuf,
        counts_buf: &mut PrefixedStringBuf,
        distribution: impl IntoIterator<Item = Observation>,
        unit: Unit,
        flags: MetricFlags<'_>,
        multiplicity: Option<u64>,
    ) -> Result<(), ValidationError> {
        let mut distribution = distribution.into_iter();
        let Some(first) = distribution.next() else {
            return Ok(()); // skip metric with no observations
        };

        // If write_metric_value skips a NaN metric, it will have already
        // written the metric name, so the buffer looks like
        //                                        `...,"MetricName":
        //                                           /             ^
        // buffer is at the comma before the truncation  | buffer is at the : when it detects the NaN
        //
        // There is always a comma, since `fields_buf` always contains at least the `}`
        // that closes the `_aws` block (and possibly other fields).
        let fields_buf_index = fields_buf.as_str().len();
        if let Err(MetricSkipped) = Self::write_metric_value(
            name,
            fields_buf,
            counts_buf,
            first,
            distribution,
            multiplicity,
        ) {
            // skipping this metric, truncate the metric name
            fields_buf.truncate(fields_buf_index);
            return Ok(()); // skip metric with only NaN observations
        }

        let flags = flags.downcast();

        if let Some(EmfOptions {
            storage_mode: StorageMode::NoMetric,
            ..
        }) = flags
        {
            // don't emit metric in no-emf mode
            return Ok(());
        }

        // (*) comma-adding logic
        if !metrics_buf.is_empty() {
            metrics_buf.push(',');
        }
        metrics_buf.push_raw_str(r#"{"Name":"#).json_string(name);
        if unit != Unit::None {
            metrics_buf
                .push_raw_str(r#","Unit":"#)
                .json_string(unit.name());
        }
        if let Some(EmfOptions {
            storage_mode: StorageMode::HighStorageResolution,
            ..
        }) = flags
        {
            metrics_buf.push_raw_str(r#","StorageResolution":1}"#);
        } else {
            metrics_buf.push('}');
        }

        Ok(())
    }

    // pass BufKind::FieldsBuf for dimension definitions, BufKind::DeclKind for dimension uses
    fn validate_string(&mut self) {
        match self.entry.validation_map.entry_ref(&self.name) {
            EntryRef::Occupied(mut occupied_entry) => {
                match occupied_entry.get_mut() {
                    LineData {
                        kind: LineKind::Metric { .. } | LineKind::String,
                    } => {
                        // duplicate metric
                        self.entry.error.extend_mut(
                            ValidationError::invalid("duplicate field").for_field(&self.name),
                        );
                    }
                    LineData {
                        kind: kind @ LineKind::UnfoundDimension,
                    } => {
                        *kind = LineKind::String;
                    }
                }
            }
            EntryRef::Vacant(vacant_entry) => {
                vacant_entry.insert(LineData {
                    kind: LineKind::String,
                });
            }
        }
    }
}

impl metrique_writer_core::ValueWriter for ValueWriter<'_, '_> {
    fn string(mut self, value: &str) {
        self.entry
            .state
            .string_fields_buf
            .push(',')
            .json_string(&self.name)
            .push(':')
            .json_string(value);

        if !self.entry.validations.skip_validate_unique {
            self.validate_string();
        }
    }

    fn metric<'a>(
        self,
        distribution: impl IntoIterator<Item = Observation>,
        unit: Unit,
        dimensions: impl IntoIterator<Item = (&'a str, &'a str)>,
        flags: MetricFlags<'_>,
    ) {
        let mut dimensions = dimensions.into_iter().peekable();
        let is_global = self.entry.state.allow_ignored_dimensions || dimensions.peek().is_none();
        if !is_global && !self.entry.allow_split_entries {
            self.entry.error.extend_mut(
                ValidationError::invalid("can't use per-metric dimensions without split entries - you probably want to remove WithDimensions<>")
                    .for_field(&self.name),
            );
        }
        let (metrics_buf, fields_buf, index) = if is_global {
            (
                &mut self.entry.state.metrics_buf,
                &mut self.entry.state.fields_buf,
                0,
            )
        } else {
            let key = DimensionSetKey::from_iter(dimensions);
            let index = NonZero::new(self.entry.state.dimension_set_map.len() + 1).unwrap();
            let each_dimensions_str = self
                .entry
                .entry_dimensions
                .as_deref()
                .unwrap_or(&self.entry.state.each_dimensions_str);
            let val = self
                .entry
                .state
                .dimension_set_map
                .entry_ref(&key)
                .or_insert_with(|| {
                    MetricsForDimensionSet::new(
                        &self.entry.state.namespaces[0],
                        each_dimensions_str,
                        &key,
                        index,
                    )
                });
            (&mut val.metrics_buf, &mut val.fields_buf, val.index.into())
        };
        if !self.entry.validations.skip_validate_unique && !self.entry.is_allow_unroutable_entries {
            // either the field is a true duplicate, or the field is an UnfoundDimension that is referred to as a metric
            match self
                .entry
                .validation_map
                .entry_ref(&self.name)
                .or_insert_with(|| LineData {
                    kind: LineKind::Metric {
                        indexes: bit_set::BitSet::new(),
                    },
                })
                .kind
            {
                LineKind::UnfoundDimension => {
                    self.entry.error.extend_mut(
                        ValidationError::invalid("can't use metric in dimension field")
                            .for_field(&self.name),
                    );
                }
                LineKind::Metric { ref mut indexes } => {
                    if !indexes.insert(index) {
                        self.entry.error.extend_mut(
                            ValidationError::invalid("duplicate field").for_field(&self.name),
                        );
                    }
                }
                LineKind::String => {
                    self.entry.error.extend_mut(
                        ValidationError::invalid("duplicate field").for_field(&self.name),
                    );
                }
            }
        }

        if let Err(err) = Self::write_metric(
            &self.name,
            fields_buf,
            metrics_buf,
            &mut self.entry.state.counts_buf,
            distribution,
            unit,
            flags,
            self.entry.multiplicity,
        ) {
            self.error(err);
        }
    }

    fn error(self, error: ValidationError) {
        self.entry.error.extend_mut(error.for_field(&self.name));
    }
}

impl Format for Emf {
    fn format(
        &mut self,
        entry: &impl Entry,
        output: &mut impl io::Write,
    ) -> Result<(), IoStreamError> {
        self.format_with_multiplicity(entry, output, None)
    }
}

// ordering is "who wins"
#[derive(Clone, Copy, Debug, PartialOrd, Ord, PartialEq, Eq)]
enum StorageMode {
    HighStorageResolution,
    NoMetric,
}

/// The [MetricOptions] for [Emf] formatters.
#[derive(Debug)]
struct EmfOptions {
    storage_mode: StorageMode,
}

impl MetricOptions for EmfOptions {
    fn try_merge(&self, other: &dyn MetricOptions) -> Option<MetricFlags<'static>> {
        (other as &dyn Any).downcast_ref::<EmfOptions>().map(|x| {
            MetricFlags::upcast(match std::cmp::max(x.storage_mode, self.storage_mode) {
                StorageMode::HighStorageResolution => &EmfOptions {
                    storage_mode: StorageMode::HighStorageResolution,
                },
                StorageMode::NoMetric => &EmfOptions {
                    storage_mode: StorageMode::NoMetric,
                },
            })
        })
    }
}

/// Creates options for high storage resolution
pub struct HighStorageResolutionCtor;

impl FlagConstructor for HighStorageResolutionCtor {
    fn construct() -> MetricFlags<'static> {
        MetricFlags::upcast(&EmfOptions {
            storage_mode: StorageMode::HighStorageResolution,
        })
    }
}

/// Creates options for emitting a value to the JSON but not
/// emitting EMF metric metadata for it (which will make it readable by
/// code parsing the JSON, but will prevent it from creating a CloudWatch
/// metric).
pub struct NoMetricCtor;

impl FlagConstructor for NoMetricCtor {
    fn construct() -> MetricFlags<'static> {
        MetricFlags::upcast(&EmfOptions {
            storage_mode: StorageMode::NoMetric,
        })
    }
}

/// Wrapper type to force a metric value, entry, or metric stream to be written to EMF in high
/// (1/second) storage resolution.
///
/// ```
/// # use metrique_writer_format_emf::HighStorageResolution;
/// # use std::time::Duration;
/// struct MyEntry {
///    my_timer_metric: HighStorageResolution<Duration>,
/// }
/// ```
pub type HighStorageResolution<T> = ForceFlag<T, HighStorageResolutionCtor>;

/// Wrapper type to force a metric value, entry, or metric stream to be present
/// in the JSON but emit no EMF metadata and therefore not be present in CloudWatch.
///
/// To make moving between `NoMetric` and emitting EMF as easy as possible, metrics
/// emitted using `NoMetric` are still emitted in histogram format when
/// [sampling][Emf::with_sampling] is used, and appear only in the JSON
/// corresponding to their dimensions when [AllowSplitEntries] is used.
///
/// ```
/// # use metrique_writer_format_emf::NoMetric;
/// # use std::time::Duration;
/// struct MyEntry {
///    my_metric_no_metric: NoMetric<Duration>,
/// }
/// ```
pub type NoMetric<T> = ForceFlag<T, NoMetricCtor>;

/// A wrapper around [Emf] that allows sampling. Datapoints are emitted with multiplicity
/// equal to either `floor(1/rate)` or `ceil(1/rate)` to ensure statistics are unbiased.
/// See the docs for [Emf::with_sampling] and [Emf::with_sampling_and_rng].
pub struct SampledEmf<R = DefaultRng<ThreadRng>> {
    emf: Emf,
    rng: R,
}

impl<R> Format for SampledEmf<R> {
    fn format(
        &mut self,
        entry: &impl Entry,
        output: &mut impl io::Write,
    ) -> Result<(), IoStreamError> {
        self.emf.format(entry, output)
    }
}

/// return an (n, alpha) such that
/// 1 / rate = alpha * n + (1-alpha) * (n+1)
fn rate_to_n_alpha(rate: f32) -> (u64, f64) {
    let rate = rate as f64;

    // inv_rate = floor(1/rate)
    let inv_rate = 1.0 / rate;
    let inv_rate_int = inv_rate as u64; // checked no overflow earlier

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

impl<R: RngCore> SampledFormat for SampledEmf<R> {
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
        self.emf.format_with_multiplicity(entry, output, Some(n))
    }
}

#[cfg(test)]
mod tests {
    use assert_approx_eq::assert_approx_eq;
    use assert_json_diff::assert_json_eq;
    use rand::SeedableRng;

    use super::*;
    use core::{f32, f64};
    use metrique_writer::value::{Distribution, Mean};
    use metrique_writer::{EntryIoStreamExt, FormatExt};
    use metrique_writer_core::{
        EntryIoStream, EntryWriter, MetricValue,
        unit::{BitPerSecond, Kilobyte, Millisecond, NegativeScale, Second},
        value::WithDimension,
    };
    use rstest::rstest;
    use std::time::Duration;

    // The normal Value implementations don't support empty Value, but it is legal, so test it.
    struct EmptyValue;
    impl Value for EmptyValue {
        fn write(&self, writer: impl metrique_writer_core::ValueWriter) {
            writer.metric(vec![], Unit::Count, vec![], MetricFlags::empty());
        }
    }
    impl MetricValue for EmptyValue {
        type Unit = metrique_writer_core::unit::Count;
    }

    #[test]
    fn test_rate_to_n_alpha() {
        assert_eq!(rate_to_n_alpha(0.5), (2, 1.0));

        // [1/0.4] = 2.5 = 2 * 0.5 + 3 * 0.5

        let (n, alpha) = rate_to_n_alpha(0.4);
        assert_eq!(n, 2);
        assert_approx_eq!(alpha, 0.5, 0.001);

        // [1/.225] = 40 / 9 = 4 * (5/9) + 5 * (1-5/9)

        let (n, alpha) = rate_to_n_alpha(0.225);
        assert_eq!(n, 4);
        assert_approx_eq!(alpha, 0.55555, 0.001);
    }

    #[test]
    fn test_rate_to_n() {
        // check that we get the right distribution. Use a fixed rng to make sure the
        // test is not flaky.
        let mut rng = rand_chacha::ChaChaRng::seed_from_u64(0);
        let mut total = 0;
        const SAMPLES: usize = 10_000;
        const RATE: f32 = 0.4;
        for _ in 0..SAMPLES {
            // sample with probability RATE
            if rng.random::<f64>() >= RATE as f64 {
                continue;
            }
            match rate_to_n(RATE, &mut rng) {
                // each sample counts as n
                n @ (2 | 3) => total += n,
                n => panic!("must be 2 or 3, found {n}"),
            }
        }
        // check that we get approximately SAMPLES samples
        assert_approx_eq!((total as f64) / (SAMPLES as f64), 1.0, 0.01);
    }

    #[test]
    fn test_validation_errors() {
        struct TestEntry;
        impl Entry for TestEntry {
            fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
                writer.timestamp(
                    SystemTime::UNIX_EPOCH + Duration::from_secs_f64(1749475336.0157819),
                );
                writer.timestamp(
                    SystemTime::UNIX_EPOCH + Duration::from_secs_f64(1749475336.0157819),
                );
                writer.config(const { &EntryDimensions::new(Cow::Borrowed(&[])) });
                writer.value("AWSAccountId", "012345678901");
                writer.value("AWSAccountId", "012345678901");
                writer.value(
                    "WithDimension",
                    &WithDimension::new_with_dimensions(&2.0, [("Dim", "Val")]),
                );
                writer.value("_aws", "some string value");
                writer.value("", "some string value");
                writer.value("MyDimension", &2u64);
                writer.value("Metric", &2u64);
                writer.value("Metric", &3u64);
                // check that metrics can't be duplicated even if they are NAN
                writer.value("NaNMetric", &f64::NAN);
                writer.value("NaNMetric", &1.0);
                writer
                    .config(const { &EntryDimensions::new(Cow::Borrowed(&[Cow::Borrowed(&[])])) });
            }
        }

        let mut emf = Emf::builder(
            "TestNS".to_string(),
            vec![
                vec![],
                vec!["MyDimension".to_string(), "MyOtherDimension".to_string()],
            ],
        )
        .skip_all_validations(false)
        .build();
        let errors = format!("{}", emf.format(&TestEntry, &mut vec![]).unwrap_err());
        assert!(errors.contains("multiple timestamps written"));
        assert!(errors.contains("for `AWSAccountId`: duplicate field"));
        assert!(errors.contains("for `_aws`: name can't be `_aws`"));
        assert!(errors.contains("for ``: name can't be empty"));
        assert!(errors.contains("for `Metric`: duplicate field"));
        assert!(errors.contains("for `NaNMetric`: duplicate field"));
        assert!(errors.contains("for `MyDimension`: can't use metric in dimension field"));
        assert!(errors.contains("for `MyOtherDimension`: missing dimension"));
        assert!(errors.contains("entry dimensions cannot be empty"));
        assert!(errors.contains(
            "entry dimensions must be configured before emitting a metric with custom dimensions"
        ));
        assert!(errors.contains("multiple timestamps written"));
    }

    #[test]
    fn test_validation_errors_multiple_config() {
        struct TestEntry;
        impl Entry for TestEntry {
            fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
                writer.value("Metric", &2u64);
                writer.config(
                    const {
                        &EntryDimensions::new(Cow::Borrowed(&[Cow::Borrowed(&[Cow::Borrowed(
                            "Metric",
                        )])]))
                    },
                );
                writer.config(
                    const {
                        &EntryDimensions::new(Cow::Borrowed(&[Cow::Borrowed(&[Cow::Borrowed(
                            "Metric",
                        )])]))
                    },
                );
            }
        }

        let mut emf: Emf = Emf::builder(
            "TestNS".to_string(),
            vec![
                vec![],
                vec!["MyDimension".to_string(), "MyOtherDimension".to_string()],
            ],
        )
        .skip_all_validations(true) // emitted even with no validations
        .build();
        let errors = format!("{}", emf.format(&TestEntry, &mut vec![]).unwrap_err());
        assert!(errors.contains("entry dimensions cannot be set twice"));

        let mut emf: Emf = Emf::builder(
            "TestNS".to_string(),
            vec![
                vec![],
                vec!["MyDimension".to_string(), "MyOtherDimension".to_string()],
            ],
        )
        .skip_all_validations(false) // emitted even with
        .build();
        let errors = format!("{}", emf.format(&TestEntry, &mut vec![]).unwrap_err());
        assert!(errors.contains("for `Metric`: duplicate field")); // with validations, duplicate field is emitted as well
        assert!(errors.contains("entry dimensions cannot be set twice"));
    }

    #[test]
    fn test_validation_errors_dimensions() {
        struct TestEntry;
        impl Entry for TestEntry {
            fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
                writer.timestamp(
                    SystemTime::UNIX_EPOCH + Duration::from_secs_f64(1749475336.0157819),
                );
                writer.value("WithDimension1", "012345678901");
                writer.value(
                    "WithDimension1",
                    &WithDimension::new_with_dimensions(&2.0, [("Dim", "Val")]),
                );
                writer.value(
                    "WithDimension2",
                    &WithDimension::new_with_dimensions(&2.0, [("Dim", "Val")]),
                );
                writer.value("WithDimension2", "012345678901");

                writer.value("MyOtherDimension", "foo");

                writer.value(
                    "_aws",
                    &WithDimension::new_with_dimensions(&2.0, [("Dim", "Val")]),
                );
                writer.value(
                    "",
                    &WithDimension::new_with_dimensions(&2.0, [("Dim", "Val")]),
                );

                writer.value(
                    "WithDimension3",
                    &WithDimension::new_with_dimensions(&2.0, [("Dim", "Val")]),
                );
                writer.value(
                    "WithDimension3",
                    &WithDimension::new_with_dimensions(&2.0, [("Dim", "Val")]),
                );

                writer.value("WithDimensionOK", &2.0);
                writer.value(
                    "WithDimensionOK",
                    &WithDimension::new_with_dimensions(&2.0, [("Dim", "Val")]),
                );

                writer.value("WithDimension4", &2.0);
                writer.value(
                    "WithDimension4",
                    &WithDimension::new_with_dimensions(&2.0, [("Dim", "Val")]),
                );
                writer.value("WithDimension4", &2.0);

                // duplicate empty value, illegal
                writer.value(
                    "WithDimension5",
                    &WithDimension::new_with_dimensions(&EmptyValue, [("Dim", "Val")]),
                );
                writer.value(
                    "WithDimension5",
                    &WithDimension::new_with_dimensions(&EmptyValue, [("Dim", "Val")]),
                );

                // duplicate empty + non-empty value, illegal as well
                writer.value(
                    "WithDimension6",
                    &WithDimension::new_with_dimensions(&EmptyValue, [("Dim", "Val")]),
                );
                writer.value(
                    "WithDimension6",
                    &WithDimension::new_with_dimensions(&2.0, [("Dim", "Val")]),
                );

                writer.value(
                    "MyDimension",
                    &WithDimension::new_with_dimensions(&2.0, [("Dim", "Val")]),
                );
            }
        }

        fn check(allow_ignored: bool) {
            let mut emf = Emf::builder(
                "TestNS".to_string(),
                vec![
                    vec![],
                    vec!["MyDimension".to_string(), "MyOtherDimension".to_string()],
                ],
            )
            .skip_all_validations(false)
            .allow_ignored_dimensions(allow_ignored)
            .build();
            let errors = format!("{}", emf.format(&TestEntry, &mut vec![]).unwrap_err());
            assert!(errors.contains("for `WithDimension1`: duplicate field"));
            assert!(errors.contains("for `WithDimension2`: duplicate field"));
            assert!(errors.contains("for `WithDimension3`: duplicate field"));
            assert!(errors.contains("for `WithDimension4`: duplicate field"));
            assert!(errors.contains("for `WithDimension5`: duplicate field"));
            assert!(errors.contains("for `WithDimension6`: duplicate field"));
            assert_eq!(
                errors.contains("for `WithDimensionOK`: duplicate field"),
                allow_ignored
            );
            assert!(errors.contains("for `_aws`: name can't be `_aws`"));
            assert!(errors.contains("for ``: name can't be empty"));
            assert!(errors.contains("for `MyDimension`: can't use metric in dimension field"));
            assert!(errors.contains("for `MyDimension`: missing dimension"));
        }

        check(false);
        check(true);
    }

    #[rstest]
    #[case(false)]
    #[case(true)]
    fn test_validation_errors_dimensions_no_split(#[case] preset_split_entries: bool) {
        struct TestEntry;
        impl Entry for TestEntry {
            fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
                writer.timestamp(
                    SystemTime::UNIX_EPOCH + Duration::from_secs_f64(1749475336.0157819),
                );
                writer.value("Normal", &1.0);
                writer.value(
                    "WithDimension",
                    &WithDimension::new_with_dimensions(&2.0, [("Dim", "Val")]),
                );
            }
        }

        fn check(skip_validations: bool, preset_split_entries: bool) {
            let mut emf = Emf::builder("TestNS".to_string(), vec![vec![]])
                .skip_all_validations(skip_validations)
                .build();
            if preset_split_entries {
                struct ForceSplitEntries;
                impl Entry for ForceSplitEntries {
                    fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
                        writer.config(&const { AllowSplitEntries::new() });
                    }
                }
                emf.format(&ForceSplitEntries, &mut vec![]).unwrap();
            }
            let errors = format!("{}", emf.format(&TestEntry, &mut vec![]).unwrap_err());
            assert!(errors.contains("for `WithDimension`: can't use per-metric dimensions without split entries - you probably want to remove WithDimensions<>"));
            assert!(!errors.contains("Normal"));
        }

        check(false, preset_split_entries);
        check(true, preset_split_entries);
    }

    #[test]
    fn test_sampling_bad_rate() {
        struct TestEntry;
        impl Entry for TestEntry {
            fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
                writer.timestamp(SystemTime::UNIX_EPOCH);
                writer.value(
                    "SomeRepeatedDuration",
                    &Mean::<Millisecond>::from_iter([1u32, 2, 3]),
                );
            }
        }
        let mut format = Emf::no_validations("MyNS".into(), vec![vec![]]).with_sampling();
        assert!(
            format
                .format_with_sample_rate(&TestEntry, &mut vec![], 1.0)
                .is_ok()
        );
        assert!(
            format
                .format_with_sample_rate(&TestEntry, &mut vec![], 0.0015)
                .is_ok()
        );
        let mut infty = vec![];
        assert!(
            format
                .format_with_sample_rate(&TestEntry, &mut infty, 1e-30)
                .is_ok()
        );
        // check that we handle values above u64::MAX in a sane enough way
        // since we are creating JSONs with string processing this test shouldn't break that often.
        assert!(
            String::from_utf8(infty).unwrap().contains(
                r#""SomeRepeatedDuration":{"Values":[2],"Counts":[18446744073709551615]}"#
            )
        );
        assert!(
            format
                .format_with_sample_rate(&TestEntry, &mut vec![], f32::NAN)
                .is_err()
        );
        assert!(
            format
                .format_with_sample_rate(&TestEntry, &mut vec![], 0.0)
                .is_err()
        );
        assert!(
            format
                .format_with_sample_rate(&TestEntry, &mut vec![], -1.0)
                .is_err()
        );
    }

    #[test]
    fn test_missing_timestamp() {
        struct TestEntry;
        impl Entry for TestEntry {
            fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
                writer.value("Metric", &2u64);
            }
        }

        let mut emf = Emf::all_validations("TestNS".to_string(), vec![vec![]]);
        let mut buf = vec![];
        emf.format(&TestEntry, &mut buf).unwrap();
        let emf: serde_json::Value = serde_json::from_slice(&buf).unwrap();
        let now = i64::try_from(
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_millis(),
        )
        .unwrap();
        let json_now = emf
            .get("_aws")
            .unwrap()
            .get("Timestamp")
            .unwrap()
            .as_i64()
            .unwrap();
        // more than 1 billion milliseconds difference = probably not a flaky test
        if now.abs_diff(json_now) > 1_000_000_000 {
            assert!(false, "time is not sane {now} {json_now}");
        }
    }

    #[rstest]
    #[case(None)]
    #[case(Some(1))]
    #[case(Some(2))]
    fn formats_all_features(#[case] sample_rate: Option<u64>) {
        struct TestEntry;
        impl Entry for TestEntry {
            fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
                writer.timestamp(
                    SystemTime::UNIX_EPOCH + Duration::from_secs_f64(1749475336.0157819),
                );
                // test that a NaN works even if it's the first field
                writer.value("NaN", &f64::NAN);
                writer.value("AWSAccountId", "012345678901");
                writer.value("API", "MyAPI");
                writer.value("StringProp", "some string value");
                writer.value("HighResCount", &HighStorageResolution::from(1234u64));
                writer.value("BasicIntCount", &1234u64);
                writer.value("NoMetric", &NoMetric::from(&1235u64));
                writer.value("BasicFloatCount", &5.4321f64);
                writer.value("SomeDuration", &Duration::from_micros(12345678));
                writer.value(
                    "SomeRepeatedDuration",
                    &Mean::<Millisecond>::from_iter([1u32, 2, 3]),
                );
                writer.value(
                    "Nothing",
                    &Observation::Repeated {
                        total: 0.0,
                        occurrences: 0,
                    },
                );
                writer.value(
                    "RepeatedDuration",
                    &Distribution::<_, 2>::from_iter([
                        Duration::from_micros(10),
                        Duration::from_micros(170),
                    ]),
                );
                writer.value("CounterWithUnit", &99u64.with_unit::<Kilobyte>());
                writer.value(
                    "MeanValue",
                    &WithDimension::new_with_dimensions(
                        Mean::<Second>::from_iter([1u8, 2, 3]),
                        [("Ignored", "X")],
                    ),
                );
                writer.value(
                    "DistributionWithNonFinite",
                    &Distribution::<f64>::from_iter([
                        f64::NAN,
                        f64::INFINITY,
                        -f64::INFINITY,
                        f64::NAN,
                        1.0,
                    ]),
                );
                writer.value("OtherNaN", &f64::NAN);
                writer.value(
                    "DistributionWithOnlyNaN",
                    &Distribution::<f64>::from_iter([f64::NAN, f64::NAN]),
                );
                writer.value(
                    "ComplexDistribution",
                    &Distribution::<_, 3>::from_iter([456u32, 789u32, 123u32])
                        .with_unit::<BitPerSecond>(),
                );
                writer.value("NoObservations", &Distribution::<u64, 0>::from_iter([]));
            }
        }

        fn check(format: Emf, extra: &str, sample_rate: Option<u64>) {
            // format multiple times to make sure that buffers are cleared.
            // the RNG does not matter since the rate is an integer
            let mut sampled_format = format.with_sampling();
            for _ in 0..3 {
                let mut output = Vec::new();

                if let Some(sample_rate) = sample_rate {
                    sampled_format
                        .format_with_sample_rate(
                            &TestEntry,
                            &mut output,
                            1.0 / (sample_rate as f32),
                        )
                        .unwrap();
                } else {
                    sampled_format.format(&TestEntry, &mut output).unwrap();
                }
                let json: serde_json::Value = serde_json::from_slice(&output).unwrap();

                let expected = format!(
                    "{}{}{}{}",
                    match sample_rate {
                        None =>
                            r#"
            {
                "BasicFloatCount": 5.4321,
                "BasicIntCount": 1234,
                "NoMetric": 1235,
                "HighResCount": 1234,
                "ComplexDistribution": {"Values": [456,789,123], "Counts": [1,1,1]},
                "CounterWithUnit": 99,
                "MeanValue": {"Values":[2], "Counts":[3]},
                "RepeatedDuration": {"Values":[0.01, 0.17], "Counts": [1, 1]},
                "SomeDuration": 12345.678,
                "SomeRepeatedDuration": {"Values":[2], "Counts":[3]},
                "Nothing": {"Values":[0], "Counts":[0]},
                "DistributionWithNonFinite": {"Values":[1.7976931348623157e308,-1.7976931348623157e308,1], "Counts":[1,1,1]},"#,
                        Some(1) =>
                            r#"
            {
                "BasicFloatCount": {"Values": [5.4321], "Counts": [1]},
                "BasicIntCount": {"Values": [1234], "Counts": [1]},
                "NoMetric": {"Values": [1235], "Counts": [1]},
                "HighResCount": {"Values": [1234], "Counts": [1]},
                "ComplexDistribution": {"Values": [456,789,123], "Counts": [1,1,1]},
                "CounterWithUnit": {"Values": [99], "Counts": [1]},
                "MeanValue": {"Values": [2], "Counts": [3]},
                "RepeatedDuration": {"Values": [0.01, 0.17], "Counts": [1, 1]},
                "SomeDuration": {"Values": [12345.678], "Counts": [1]},
                "SomeRepeatedDuration": {"Values": [2], "Counts": [3]},
                "Nothing": {"Values": [0], "Counts": [0]},
                "DistributionWithNonFinite": {"Values":[1.7976931348623157e308,-1.7976931348623157e308,1], "Counts":[1,1,1]},"#,
                        Some(2) =>
                            r#"
            {
                "BasicFloatCount": {"Values": [5.4321], "Counts": [2]},
                "BasicIntCount": {"Values": [1234], "Counts": [2]},
                "NoMetric": {"Values": [1235], "Counts": [2]},
                "HighResCount": {"Values": [1234], "Counts": [2]},
                "ComplexDistribution": {"Values": [456,789,123], "Counts": [2,2,2]},
                "CounterWithUnit": {"Values": [99], "Counts": [2]},
                "MeanValue": {"Values": [2], "Counts": [6]},
                "RepeatedDuration": {"Values": [0.01, 0.17], "Counts": [2, 2]},
                "SomeDuration": {"Values": [12345.678], "Counts": [2]},
                "SomeRepeatedDuration": {"Values": [2], "Counts": [6]},
                "Nothing": {"Values": [0], "Counts": [0]},
                "DistributionWithNonFinite": {"Values":[1.7976931348623157e308,-1.7976931348623157e308,1], "Counts":[2,2,2]},"#,
                        _ => panic!("unknown sample rate"),
                    },
                    r#"
                "API": "MyAPI",
                "AWSAccountId": "012345678901",
                "StringProp": "some string value",
                "_aws": {
                    "CloudWatchMetrics": [
                        {
                            "Namespace": "TestNS",
                            "Dimensions": [
                                [
                                ],
                                [
                                    "AWSAccountId"
                                ]
                            ],
                            "Metrics": [
                                {
                                    "Name": "HighResCount",
                                    "StorageResolution": 1
                                },
                                {
                                    "Name": "BasicIntCount"
                                },
                                {
                                    "Name": "BasicFloatCount"
                                },
                                {
                                    "Name": "SomeDuration",
                                    "Unit": "Milliseconds"
                                },
                                {
                                    "Name": "SomeRepeatedDuration",
                                    "Unit": "Milliseconds"
                                },
                                {
                                    "Name": "Nothing"
                                },
                                {
                                    "Name": "RepeatedDuration",
                                    "Unit": "Milliseconds"
                                },
                                {
                                    "Name": "CounterWithUnit",
                                    "Unit": "Kilobytes"
                                },
                                {
                                    "Name": "MeanValue",
                                    "Unit": "Seconds"
                                },
                                {
                                    "Name": "DistributionWithNonFinite"
                                },
                                {
                                    "Name": "ComplexDistribution",
                                    "Unit": "Bits/Second"
                                }
                            ]
                        }"#,
                    extra,
                    r#"
                    ],
                    "Timestamp": 1749475336015
                }
            }
            "#
                );
                assert_json_diff::assert_json_eq!(
                    json,
                    serde_json::from_str::<serde_json::Value>(&expected).unwrap()
                );
            }
        }
        check(
            Emf::builder(
                "TestNS".to_string(),
                vec![vec![], vec!["AWSAccountId".to_string()]],
            )
            .directive(MetricDirective {
                dimensions: vec![vec!["API"]],
                metrics: vec![MetricDefinition {
                    name: "MeanValue",
                    unit: Unit::Second(NegativeScale::One),
                    storage_resolution: Some(StorageResolution::Second),
                }],
                namespace: "TestNS",
            })
            .allow_ignored_dimensions(true)
            .skip_all_validations(true)
            .build(),
            r#"                        ,{ "Namespace": "TestNS", "Dimensions": [["API"]], "Metrics": [{"Name": "MeanValue", "Unit": "Seconds", "StorageResolution": 1}]}"#,
            sample_rate,
        );

        check(
            Emf::builder(
                "TestNS".to_string(),
                vec![vec![], vec!["AWSAccountId".to_string()]],
            )
            .directive(MetricDirective {
                dimensions: vec![vec!["API"]],
                metrics: vec![MetricDefinition {
                    name: "MeanValue",
                    unit: Unit::Second(NegativeScale::One),
                    storage_resolution: Some(StorageResolution::Second),
                }],
                namespace: "TestNS",
            })
            .allow_ignored_dimensions(true)
            .skip_all_validations(false)
            .build(),
            r#"                        ,{ "Namespace": "TestNS", "Dimensions": [["API"]], "Metrics": [{"Name": "MeanValue", "Unit": "Seconds", "StorageResolution": 1}]}"#,
            sample_rate,
        );
    }

    #[test]
    fn formats_all_features_dimensions() {
        struct TestEntry;
        impl Entry for TestEntry {
            fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
                writer.timestamp(
                    SystemTime::UNIX_EPOCH + Duration::from_secs_f64(1749475336.0157819),
                );
                writer.config(const { &AllowSplitEntries::new() });
                writer.value("AWSAccountId", "012345678901");
                writer.value("API", "MyAPI");
                writer.value("StringProp", "some string value");
                writer.value("BasicIntCount", &1234u64);
                writer.value("NoMetric", &NoMetric::from(&1235u64));
                writer.value(
                    "DimensionedIntCount",
                    &WithDimension::new_with_dimensions(1235u64, [("Kind", "Bar")]),
                );
                writer.value(
                    "MeanValue",
                    &Mean::<Second>::from_iter([1u8, 2, 3, 4, 5, 6]),
                );
                writer.value(
                    "MeanValue",
                    &WithDimension::new_with_dimensions(
                        Mean::<Second>::from_iter([1u8, 2, 3]),
                        [("Kind", "Foo")],
                    ),
                );
                writer.value(
                    "MeanValue",
                    &WithDimension::new_with_dimensions(&EmptyValue, [("Kind", "Empty")]),
                );
                writer.value(
                    "MeanValue",
                    &WithDimension::new_with_dimensions(
                        Mean::<Second>::from_iter([4u8, 5, 6]),
                        [("Kind", "Bar")],
                    ),
                );
                writer.value(
                    "MeanValue",
                    &WithDimension::new_with_dimensions(
                        Mean::<Second>::from_iter([4u8, 5, 6]),
                        [("Kind", "Bar"), ("Type", "Baz")],
                    ),
                );
            }
        }

        fn check(mut format: Emf, extra: &str) {
            // format multiple times to make sure that buffers are cleared.
            for _ in 0..3 {
                let mut output = Vec::new();

                format.format(&TestEntry, &mut output).unwrap();
                let mut output: Vec<&[u8]> = output.split(|c| *c == b'\n').collect();
                assert_eq!(output.pop().unwrap(), b"");
                assert_eq!(output.len(), 4);
                output.sort();
                let json: serde_json::Value = serde_json::from_slice(&output[0]).unwrap();

                let expected = r#"
                {
                    "API": "MyAPI",
                    "AWSAccountId": "012345678901",
                    "StringProp": "some string value",
                    "Kind": "Bar",
                    "Type": "Baz",
                    "MeanValue": {"Values":[5], "Counts":[3]},
                    "_aws": {
                        "CloudWatchMetrics": [
                            {
                                "Namespace": "TestNS",
                                "Dimensions": [
                                    [
                                        "Kind",
                                        "Type"
                                    ],
                                    [
                                        "AWSAccountId",
                                        "Kind",
                                        "Type"
                                    ]
                                ],
                                "Metrics": [
                                    {
                                        "Name": "MeanValue",
                                        "Unit": "Seconds"
                                    }
                                ]
                            }
                        ],
                        "Timestamp": 1749475336015
                    }
                }
                "#;
                assert_json_diff::assert_json_eq!(
                    json,
                    serde_json::from_str::<serde_json::Value>(&expected).unwrap()
                );

                let json: serde_json::Value = serde_json::from_slice(&output[1]).unwrap();
                let expected: &str = r#"
            {
                "API": "MyAPI",
                "AWSAccountId": "012345678901",
                "StringProp": "some string value",
                "Kind": "Bar",
                "MeanValue": {"Values":[5], "Counts":[3]},
                "DimensionedIntCount": 1235,
                "_aws": {
                    "CloudWatchMetrics": [
                        {
                            "Namespace": "TestNS",
                            "Dimensions": [
                                [
                                    "Kind"
                                ],
                                [
                                    "AWSAccountId",
                                    "Kind"
                                ]
                            ],
                            "Metrics": [
                                {
                                    "Name": "DimensionedIntCount"
                                },
                                {
                                    "Name": "MeanValue",
                                    "Unit": "Seconds"
                                }
                            ]
                        }
                    ],
                    "Timestamp": 1749475336015
                }
            }
            "#;
                assert_json_diff::assert_json_eq!(
                    json,
                    serde_json::from_str::<serde_json::Value>(&expected).unwrap()
                );
                let expected: &str = r#"
            {
                "API": "MyAPI",
                "AWSAccountId": "012345678901",
                "StringProp": "some string value",
                "Kind": "Foo",
                "MeanValue": {"Values":[2], "Counts":[3]},
                "_aws": {
                    "CloudWatchMetrics": [
                        {
                            "Namespace": "TestNS",
                            "Dimensions": [
                                [
                                    "Kind"
                                ],
                                [
                                    "AWSAccountId",
                                    "Kind"
                                ]
                            ],
                            "Metrics": [
                                {
                                    "Name": "MeanValue",
                                    "Unit": "Seconds"
                                }
                            ]
                        }
                    ],
                    "Timestamp": 1749475336015
                }
            }
            "#;
                let json: serde_json::Value = serde_json::from_slice(&output[2]).unwrap();

                assert_json_diff::assert_json_eq!(
                    json,
                    serde_json::from_str::<serde_json::Value>(&expected).unwrap()
                );

                let expected = format!(
                    "{}{}{}",
                    r#"
            {
                "API": "MyAPI",
                "AWSAccountId": "012345678901",
                "StringProp": "some string value",
                "MeanValue": {"Values":[3.5], "Counts":[6]},
                "BasicIntCount": 1234,
                "NoMetric": 1235,
                "_aws": {
                    "CloudWatchMetrics": [
                        {
                            "Namespace": "TestNS",
                            "Dimensions": [
                                [
                                ],
                                [
                                    "AWSAccountId"
                                ]
                            ],
                            "Metrics": [
                                {
                                    "Name": "BasicIntCount"
                                },
                                {
                                    "Name": "MeanValue",
                                    "Unit": "Seconds"
                                }
                            ]
                        }"#,
                    extra,
                    r#"
                    ],
                    "Timestamp": 1749475336015
                }
            }
            "#
                );
                let json: serde_json::Value = serde_json::from_slice(&output[3]).unwrap();
                assert_json_diff::assert_json_eq!(
                    json,
                    serde_json::from_str::<serde_json::Value>(&expected).unwrap()
                );
            }
        }
        check(
            Emf::builder(
                "TestNS".to_string(),
                vec![vec![], vec!["AWSAccountId".to_string()]],
            )
            .directive(MetricDirective {
                dimensions: vec![vec!["API"]],
                metrics: vec![MetricDefinition {
                    name: "MeanValue",
                    unit: Unit::Second(NegativeScale::One),
                    storage_resolution: Some(StorageResolution::Second),
                }],
                namespace: "TestNS",
            })
            .skip_all_validations(true)
            .build(),
            r#"                        ,{ "Namespace": "TestNS", "Dimensions": [["API"]], "Metrics": [{"Name": "MeanValue", "Unit": "Seconds", "StorageResolution": 1}]}"#,
        );

        check(
            Emf::builder(
                "TestNS".to_string(),
                vec![vec![], vec!["AWSAccountId".to_string()]],
            )
            .directive(MetricDirective {
                dimensions: vec![vec!["API"]],
                metrics: vec![MetricDefinition {
                    name: "MeanValue",
                    unit: Unit::Second(NegativeScale::One),
                    storage_resolution: Some(StorageResolution::Second),
                }],
                namespace: "TestNS",
            })
            .skip_all_validations(false)
            .build(),
            r#"                        ,{ "Namespace": "TestNS", "Dimensions": [["API"]], "Metrics": [{"Name": "MeanValue", "Unit": "Seconds", "StorageResolution": 1}]}"#,
        );

        check(
            Emf::no_validations(
                "TestNS".to_string(),
                vec![vec![], vec!["AWSAccountId".to_string()]],
            ),
            "",
        );

        check(
            Emf::all_validations(
                "TestNS".to_string(),
                vec![vec![], vec!["AWSAccountId".to_string()]],
            ),
            "",
        );
    }

    #[test]
    fn formats_all_features_dimensions_per_entry() {
        struct TestEntry;
        impl Entry for TestEntry {
            fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
                writer.timestamp(
                    SystemTime::UNIX_EPOCH + Duration::from_secs_f64(1749475336.0157819),
                );
                writer.config(const { &AllowSplitEntries::new() });
                writer.value("AWSAccountId", "012345678901");
                writer.value("API", "MyAPI");
                writer.value("BasicIntCount", &1234u64);
                writer.config(
                    const {
                        &EntryDimensions::new(Cow::Borrowed(&[
                            Cow::Borrowed(&[Cow::Borrowed("API")]),
                            Cow::Borrowed(&[Cow::Borrowed("API"), Cow::Borrowed("StringProp")]),
                        ]))
                    },
                );
                // put the StringProp after the config, to check that validation path as well
                writer.value("StringProp", "some string value");
                writer.value(
                    "MeanValue",
                    &Mean::<Second>::from_iter([1u8, 2, 3, 4, 5, 6]),
                );
                writer.value(
                    "MeanValue",
                    &WithDimension::new_with_dimensions(
                        Mean::<Second>::from_iter([1u8, 2, 3]),
                        [("Kind", "Foo")],
                    ),
                );
            }
        }

        fn check(mut format: Emf, log_group_name: &str, extra: &str) {
            // format multiple times to make sure that buffers are cleared.
            for _ in 0..3 {
                let mut output = Vec::new();

                format.format(&TestEntry, &mut output).unwrap();
                let mut output: Vec<&[u8]> = output.split(|c| *c == b'\n').collect();
                assert_eq!(output.pop().unwrap(), b"");
                assert_eq!(output.len(), 2);
                output.sort();
                let expected: String = format!(
                    "{}{}{}",
                    r#"
            {
                "API": "MyAPI",
                "AWSAccountId": "012345678901",
                "StringProp": "some string value",
                "Kind": "Foo",
                "MeanValue": {"Values":[2], "Counts":[3]},
                "_aws": {
                    "CloudWatchMetrics": [
                        {
                            "Namespace": "TestNS",
                            "Dimensions": [
                                ["API", "Kind"],
                                ["API", "StringProp", "Kind"],
                                ["AWSAccountId", "API", "Kind"],
                                ["AWSAccountId", "API", "StringProp", "Kind"]
                            ],
                            "Metrics": [
                                {
                                    "Name": "MeanValue",
                                    "Unit": "Seconds"
                                }
                            ]
                        }
                    ],"#,
                    log_group_name,
                    r#"
                    "Timestamp": 1749475336015
                }
            }
            "#
                );
                let json: serde_json::Value = serde_json::from_slice(&output[0]).unwrap();

                assert_json_diff::assert_json_eq!(
                    json,
                    serde_json::from_str::<serde_json::Value>(&expected).unwrap()
                );

                let expected = format!(
                    "{}{}{}{}{}",
                    r#"
            {
                "API": "MyAPI",
                "AWSAccountId": "012345678901",
                "StringProp": "some string value",
                "MeanValue": {"Values":[3.5], "Counts":[6]},
                "BasicIntCount": 1234,
                "_aws": {
                    "CloudWatchMetrics": [
                        {
                            "Namespace": "TestNS",
                            "Dimensions": [
                                ["API"],
                                ["API", "StringProp"],
                                ["AWSAccountId", "API"],
                                ["AWSAccountId", "API", "StringProp"]
                            ],
                            "Metrics": [
                                {
                                    "Name": "BasicIntCount"
                                },
                                {
                                    "Name": "MeanValue",
                                    "Unit": "Seconds"
                                }
                            ]
                        }"#,
                    extra,
                    r"],",
                    log_group_name,
                    r#""Timestamp": 1749475336015
                }
            }
            "#
                );
                let json: serde_json::Value = serde_json::from_slice(&output[1]).unwrap();
                assert_json_diff::assert_json_eq!(
                    json,
                    serde_json::from_str::<serde_json::Value>(&expected).unwrap()
                );
            }
        }
        check(
            Emf::builder(
                "TestNS".to_string(),
                vec![vec![], vec!["AWSAccountId".to_string()]],
            )
            .directive(MetricDirective {
                dimensions: vec![vec!["API"]],
                metrics: vec![MetricDefinition {
                    name: "MeanValue",
                    unit: Unit::Second(NegativeScale::One),
                    storage_resolution: Some(StorageResolution::Second),
                }],
                namespace: "TestNS",
            })
            .skip_all_validations(true)
            .build(),
            "",
            r#"                        ,{ "Namespace": "TestNS", "Dimensions": [["API"]], "Metrics": [{"Name": "MeanValue", "Unit": "Seconds", "StorageResolution": 1}]}"#,
        );

        check(
            Emf::builder(
                "TestNS".to_string(),
                vec![vec![], vec!["AWSAccountId".to_string()]],
            )
            .directive(MetricDirective {
                dimensions: vec![vec!["API"]],
                metrics: vec![MetricDefinition {
                    name: "MeanValue",
                    unit: Unit::Second(NegativeScale::One),
                    storage_resolution: Some(StorageResolution::Second),
                }],
                namespace: "TestNS",
            })
            .skip_all_validations(false)
            .build(),
            "",
            r#"                        ,{ "Namespace": "TestNS", "Dimensions": [["API"]], "Metrics": [{"Name": "MeanValue", "Unit": "Seconds", "StorageResolution": 1}]}"#,
        );

        check(
            Emf::no_validations(
                "TestNS".to_string(),
                vec![vec![], vec!["AWSAccountId".to_string()]],
            ),
            "",
            "",
        );

        check(
            Emf::all_validations(
                "TestNS".to_string(),
                vec![vec![], vec!["AWSAccountId".to_string()]],
            ),
            "",
            "",
        );

        check(
            Emf::builder(
                "TestNS".to_string(),
                vec![vec![], vec!["AWSAccountId".to_string()]],
            )
            .build(),
            "",
            "",
        );

        check(
            Emf::builder(
                "TestNS".to_string(),
                vec![vec![], vec!["AWSAccountId".to_string()]],
            )
            .log_group_name("Bar")
            .build(),
            r#""LogGroupName": "Bar","#,
            "",
        );
    }

    #[rstest]
    #[case(false)]
    #[case(true)]
    fn test_multiple_namespaces(#[case] split_entries: bool) {
        struct TestEntry {
            split_entries: bool,
        }
        impl Entry for TestEntry {
            fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
                writer.timestamp(
                    SystemTime::UNIX_EPOCH + Duration::from_secs_f64(1749475336.0157819),
                );
                if self.split_entries {
                    writer.config(const { &AllowSplitEntries::new() });
                }
                writer.value("AWSAccountId", "012345678901");
                writer.value("BasicIntCount", &1234u64);
                if self.split_entries {
                    writer.value(
                        "MeanValue",
                        &WithDimension::new_with_dimensions(
                            Mean::<Second>::from_iter([1u8, 2, 3]),
                            [("Kind", "Foo")],
                        ),
                    );
                }
            }
        }

        fn check(mut format: Emf, extra: &str, split_entries: bool) {
            // format multiple times to make sure that buffers are cleared.
            for _ in 0..3 {
                let mut output = Vec::new();

                format
                    .format(&TestEntry { split_entries }, &mut output)
                    .unwrap();
                let mut output: Vec<&[u8]> = output.split(|c| *c == b'\n').collect();
                assert_eq!(output.pop().unwrap(), b"");
                assert_eq!(output.len(), if split_entries { 2 } else { 1 });
                output.sort();
                if split_entries {
                    let expected: &str = r#"
            {
                "AWSAccountId": "012345678901",
                "Kind": "Foo",
                "MeanValue": {
                    "Counts": [3],
                    "Values": [2]
                },
                "_aws": {
                    "CloudWatchMetrics": [
                        {
                            "Dimensions": [["Kind"], ["AWSAccountId", "Kind"]],
                            "Metrics": [{"Name": "MeanValue", "Unit": "Seconds"}],
                            "Namespace": "TestNS"
                        },
                        {
                            "Dimensions": [["Kind"], ["AWSAccountId", "Kind"]],
                            "Metrics": [{"Name": "MeanValue", "Unit": "Seconds"}],
                            "Namespace": "OtherNS"
                        },
                        {
                            "Dimensions": [["Kind"], ["AWSAccountId", "Kind"]],
                            "Metrics": [{"Name": "MeanValue", "Unit": "Seconds"}],
                            "Namespace": "ThirdNS"
                        }
                    ],
                    "Timestamp": 1749475336015
                }
            }
            "#;
                    let json: serde_json::Value =
                        serde_json::from_slice(&output.remove(0)).unwrap();

                    assert_json_diff::assert_json_eq!(
                        json,
                        serde_json::from_str::<serde_json::Value>(&expected).unwrap()
                    );
                }
                let expected = format!(
                    "{}{}{}",
                    r#"
            {
                "BasicIntCount": 1234,
                "AWSAccountId": "012345678901",
                "_aws": {
                    "CloudWatchMetrics": [
                        {
                            "Namespace": "TestNS",
                            "Dimensions": [[], ["AWSAccountId"]],
                            "Metrics": [{ "Name": "BasicIntCount" }]
                        },
                        {
                            "Namespace": "OtherNS",
                            "Dimensions": [[], ["AWSAccountId"]],
                            "Metrics": [{ "Name": "BasicIntCount" }]
                        },
                        {
                            "Namespace": "ThirdNS",
                            "Dimensions": [[], ["AWSAccountId"]],
                            "Metrics": [{ "Name": "BasicIntCount" }]
                        }
                    "#,
                    extra,
                    r#"
                    ],
                    "Timestamp": 1749475336015
                }
            }
            "#
                );
                let json: serde_json::Value = serde_json::from_slice(&output[0]).unwrap();
                assert_json_diff::assert_json_eq!(
                    json,
                    serde_json::from_str::<serde_json::Value>(&expected).unwrap()
                );
            }
        }
        check(
            Emf::builder(
                "TestNS".to_string(),
                vec![vec![], vec!["AWSAccountId".to_string()]],
            )
            .skip_all_validations(false)
            .add_namespace("OtherNS".to_string())
            .add_namespace("ThirdNS".to_string())
            .build(),
            "",
            split_entries,
        );
        check(
            Emf::builder(
                "TestNS".to_string(),
                vec![vec![], vec!["AWSAccountId".to_string()]],
            )
            .skip_all_validations(true)
            .add_namespace("OtherNS".to_string())
            .add_namespace("ThirdNS".to_string())
            .build(),
            "",
            split_entries,
        );
        check(
            Emf::builder(
                "TestNS".to_string(),
                vec![vec![], vec!["AWSAccountId".to_string()]],
            )
            .add_namespace("OtherNS".to_string())
            .add_namespace("ThirdNS".to_string())
            .directive(MetricDirective {
                dimensions: vec![vec!["API"]],
                metrics: vec![MetricDefinition {
                    name: "MeanValue",
                    unit: Unit::Second(NegativeScale::One),
                    storage_resolution: Some(StorageResolution::Second),
                }],
                namespace: "TestNS",
            })
            .skip_all_validations(false)
            .build(),
            r#"                        ,{ "Namespace": "TestNS", "Dimensions": [["API"]], "Metrics": [{"Name": "MeanValue", "Unit": "Seconds", "StorageResolution": 1}]}"#,
            split_entries,
        );
    }

    #[test]
    fn formats_dimensions_only() {
        struct TestEntry;
        impl Entry for TestEntry {
            fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
                writer.timestamp(
                    SystemTime::UNIX_EPOCH + Duration::from_secs_f64(1749475336.0157819),
                );
                writer.config(const { &AllowSplitEntries::new() });
                writer.value("AWSAccountId", "012345678901");
                writer.value(
                    "MeanValue",
                    &WithDimension::new_with_dimensions(
                        Mean::<Second>::from_iter([1u8, 2, 3]),
                        [("Kind", "Foo")],
                    ),
                );
            }
        }

        fn check(mut format: Emf) {
            // format multiple times to make sure that buffers are cleared.
            for _ in 0..3 {
                let mut output = Vec::new();

                format.format(&TestEntry, &mut output).unwrap();
                let mut output: Vec<&[u8]> = output.split(|c| *c == b'\n').collect();
                assert_eq!(output.pop().unwrap(), b"");
                assert_eq!(output.len(), 1);
                output.sort();
                eprintln!("{}", str::from_utf8(&output[0]).unwrap());
                let json: serde_json::Value = serde_json::from_slice(&output[0]).unwrap();

                let expected = r#"
                {
                    "AWSAccountId": "012345678901",
                    "Kind": "Foo",
                    "MeanValue": {"Values":[2], "Counts":[3]},
                    "_aws": {
                        "CloudWatchMetrics": [
                            {
                                "Namespace": "TestNS",
                                "Dimensions": [
                                    [
                                        "Kind"
                                    ],
                                    [
                                        "AWSAccountId",
                                        "Kind"
                                    ]
                                ],
                                "Metrics": [
                                    {
                                        "Name": "MeanValue",
                                        "Unit": "Seconds"
                                    }
                                ]
                            }
                        ],
                        "Timestamp": 1749475336015
                    }
                }
                "#;
                assert_json_diff::assert_json_eq!(
                    json,
                    serde_json::from_str::<serde_json::Value>(&expected).unwrap()
                );
            }
        }

        check(Emf::no_validations(
            "TestNS".to_string(),
            vec![vec![], vec!["AWSAccountId".to_string()]],
        ));

        check(Emf::all_validations(
            "TestNS".to_string(),
            vec![vec![], vec!["AWSAccountId".to_string()]],
        ));
    }

    #[test]
    fn formats_dimensions_only_no_metric() {
        // check that if a dimension only contains NoMetric metrics, it is still emitted correctly
        struct TestEntry;
        impl Entry for TestEntry {
            fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
                writer.timestamp(
                    SystemTime::UNIX_EPOCH + Duration::from_secs_f64(1749475336.0157819),
                );
                writer.config(const { &AllowSplitEntries::new() });
                writer.value("AWSAccountId", "012345678901");
                writer.value(
                    "MeanValue",
                    &NoMetric::from(WithDimension::new_with_dimensions(
                        Mean::<Second>::from_iter([1u8, 2, 3]),
                        [("Kind", "Foo")],
                    )),
                );
            }
        }

        fn check(mut format: Emf) {
            // format multiple times to make sure that buffers are cleared.
            for _ in 0..3 {
                let mut output = Vec::new();

                format.format(&TestEntry, &mut output).unwrap();
                let mut output: Vec<&[u8]> = output.split(|c| *c == b'\n').collect();
                assert_eq!(output.pop().unwrap(), b"");
                assert_eq!(output.len(), 1);
                output.sort();
                let json: serde_json::Value = serde_json::from_slice(&output[0]).unwrap();

                let expected = r#"
                {
                    "AWSAccountId": "012345678901",
                    "Kind": "Foo",
                    "MeanValue": {"Values":[2], "Counts":[3]},
                    "_aws": {
                        "CloudWatchMetrics": [
                            {
                                "Namespace": "TestNS",
                                "Dimensions": [
                                    [
                                        "Kind"
                                    ],
                                    [
                                        "AWSAccountId",
                                        "Kind"
                                    ]
                                ],
                                "Metrics": []
                            }
                        ],
                        "Timestamp": 1749475336015
                    }
                }
                "#;
                assert_eq!(
                    json,
                    serde_json::from_str::<serde_json::Value>(&expected).unwrap()
                );
            }
        }

        check(Emf::no_validations(
            "TestNS".to_string(),
            vec![vec![], vec!["AWSAccountId".to_string()]],
        ));

        check(Emf::all_validations(
            "TestNS".to_string(),
            vec![vec![], vec!["AWSAccountId".to_string()]],
        ));
    }

    #[test]
    fn formats_empty() {
        // check that a metric entry with only a timestamp still produces a metric entry

        struct TestEntry;
        impl Entry for TestEntry {
            fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
                writer.timestamp(
                    SystemTime::UNIX_EPOCH + Duration::from_secs_f64(1749475336.0157819),
                );
            }
        }

        fn check(mut format: Emf) {
            // format multiple times to make sure that buffers are cleared.
            for _ in 0..3 {
                let mut output = Vec::new();

                format.format(&TestEntry, &mut output).unwrap();
                let mut output: Vec<&[u8]> = output.split(|c| *c == b'\n').collect();
                assert_eq!(output.pop().unwrap(), b"");
                assert_eq!(output.len(), 1);
                output.sort();
                let json: serde_json::Value = serde_json::from_slice(&output[0]).unwrap();

                let expected = r#"
                {
                    "_aws": {
                        "CloudWatchMetrics": [
                            {
                                "Namespace": "TestNS",
                                "Dimensions": [[]],
                                "Metrics": []
                            }
                        ],
                        "Timestamp": 1749475336015
                    }
                }
                "#;
                assert_json_diff::assert_json_eq!(
                    json,
                    serde_json::from_str::<serde_json::Value>(&expected).unwrap()
                );
            }
        }

        check(Emf::no_validations("TestNS".to_string(), vec![vec![]]));

        check(Emf::all_validations("TestNS".to_string(), vec![vec![]]));
    }

    const STORAGE_HIRES: &'static EmfOptions = &EmfOptions {
        storage_mode: StorageMode::HighStorageResolution,
    };
    const STORAGE_NO_METRIC: &'static EmfOptions = &EmfOptions {
        storage_mode: StorageMode::NoMetric,
    };

    #[rstest]
    #[case(STORAGE_HIRES, STORAGE_HIRES, STORAGE_HIRES)]
    #[case(STORAGE_HIRES, STORAGE_NO_METRIC, STORAGE_NO_METRIC)]
    #[case(STORAGE_NO_METRIC, STORAGE_HIRES, STORAGE_NO_METRIC)]
    fn test_try_merge(
        #[case] lhs: &EmfOptions,
        #[case] rhs: &EmfOptions,
        #[case] result: &EmfOptions,
    ) {
        assert_eq!(
            MetricFlags::upcast(lhs)
                .try_merge(MetricFlags::upcast(rhs))
                .downcast::<EmfOptions>()
                .unwrap()
                .storage_mode,
            result.storage_mode
        );
    }

    #[test]
    fn test_force_storage_resolution_emf() {
        use std::time::{Duration, SystemTime};

        struct TestEntry;
        impl Entry for TestEntry {
            fn write<'a>(&'a self, writer: &mut impl metrique_writer_core::EntryWriter<'a>) {
                // intentionally not testing with dimensions to ensure we only have 1 json line.
                writer.timestamp(SystemTime::UNIX_EPOCH + Duration::from_secs_f64(12345.6789));
                writer.value("Time", &Duration::from_millis(42));
                writer.value("Operation", "Foo");
                writer.value("BasicIntCount", &1234u64);
                writer.value("BasicFloatCount", &5.4321f64);
                writer.value("SomeDuration", &Duration::from_micros(12345678));
                writer.value(
                    "SomeRepeatedDuration",
                    &Mean::<Millisecond>::from_iter([1u32, 2, 3]),
                );
                writer.value(
                    "RepeatedDuration",
                    &Distribution::<_, 2>::from_iter([
                        Duration::from_micros(10),
                        Duration::from_micros(170),
                    ]),
                );
                // also check nested HighStorageResolution
                writer.value(
                    "CounterWithUnit",
                    &HighStorageResolution::from(99u64.with_unit::<Kilobyte>()),
                );
                writer.value("MeanValue", &Mean::<Second>::from_iter([1u8, 2, 3]));
            }
        }

        let mut output = Vec::new();
        let stream = Emf::all_validations("MyNS".to_owned(), vec![vec![]]).output_to(&mut output);
        let mut stream = HighStorageResolution::from(stream);
        stream.next(&TestEntry).unwrap();
        stream.flush().unwrap();

        let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
        let expected = r#"
{
    "_aws": {
        "CloudWatchMetrics": [
            {
                "Namespace": "MyNS",
                "Dimensions": [
                    []
                ],
                "Metrics": [
                    {
                        "Name": "Time",
                        "Unit": "Milliseconds",
                        "StorageResolution": 1
                    },
                    {
                        "Name": "BasicIntCount",
                        "StorageResolution": 1
                    },
                    {
                        "Name": "BasicFloatCount",
                        "StorageResolution": 1
                    },
                    {
                        "Name": "SomeDuration",
                        "Unit": "Milliseconds",
                        "StorageResolution": 1
                    },
                    {
                        "Name": "SomeRepeatedDuration",
                        "Unit": "Milliseconds",
                        "StorageResolution": 1
                    },
                    {
                        "Name": "RepeatedDuration",
                        "Unit": "Milliseconds",
                        "StorageResolution": 1
                    },
                    {
                        "Name": "CounterWithUnit",
                        "Unit": "Kilobytes",
                        "StorageResolution": 1
                    },
                    {
                        "Name": "MeanValue",
                        "Unit": "Seconds",
                        "StorageResolution": 1
                    }
                ]
            }
        ],
        "Timestamp": 12345678
    },
    "Time": 42,
    "BasicIntCount": 1234,
    "BasicFloatCount": 5.4321,
    "SomeDuration": 12345.678,
    "SomeRepeatedDuration": {
        "Values": [
            2
        ],
        "Counts": [
            3
        ]
    },
    "RepeatedDuration": {
        "Values": [
            0.01,
            0.17
        ],
        "Counts": [
            1,
            1
        ]
    },
    "CounterWithUnit": 99,
    "MeanValue": {
        "Values": [
            2
        ],
        "Counts": [
            3
        ]
    },
    "Operation": "Foo"
}
        "#;
        assert_json_diff::assert_json_eq!(
            json,
            serde_json::from_str::<serde_json::Value>(&expected).unwrap()
        );
    }

    #[rstest]
    #[case("Foo", "Region", true)]
    // merging property "_aws" is illegal
    #[case("Foo", "_aws", false)]
    // merging property "Error" will cause a conflict, which should make us bail out
    #[case("Foo", "Error", false)]
    #[case("Error", "Region", true)]
    fn test_report_error(#[case] dim: &str, #[case] merged_dim: &str, #[case] is_valid: bool) {
        struct MergeUsEast1<'a> {
            dim: &'a str,
        }
        impl Entry for MergeUsEast1<'_> {
            fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
                writer.value(self.dim, "us-east-1");
            }
        }
        let writer = Emf::all_validations("Foo".into(), vec![vec![dim.into()]]);

        let mut w1 = vec![];
        let res = writer
            .output_to(&mut w1)
            .merge_globals(MergeUsEast1 { dim: merged_dim })
            .report_error("basic error");
        if is_valid {
            res.unwrap();
        } else {
            res.unwrap_err();
            return;
        }
        let mut actual =
            serde_json::from_str::<serde_json::Value>(&String::from_utf8(w1).unwrap()).unwrap();
        actual["_aws"]["Timestamp"] = 0.into();
        let expected = serde_json::json!({
            "_aws": serde_json::json!({
                "CloudWatchMetrics": [{
                    "Namespace": "Foo",
                    "Dimensions": [[dim]],
                    "Metrics": []
                }],
                "Timestamp": 0,
            }),
            merged_dim: "us-east-1",
            "Error": "basic error"
        });
        assert_json_eq!(expected, actual);
    }
}
