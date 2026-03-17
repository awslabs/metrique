//! Fuzz target for the EMF (Embedded Metric Format) formatter.
//!
//! Invariants tested:
//! - Successful formatting always produces one or more valid, newline-delimited JSON objects.
//! - Formatter state reuse across entries does not corrupt output.
//! - Both regular and sampled paths are exercised, with EMF-specific flag modes
//!   (HighStorageResolution, NoMetric) applied to metrics.

#![no_main]

mod fuzz_entry;

use arbitrary::{Arbitrary, Unstructured};
use libfuzzer_sys::fuzz_target;

use metrique_writer_core::format::Format;
use metrique_writer_core::sample::SampledFormat;
use metrique_writer_core::{Entry, EntryWriter};
use metrique_writer_format_emf::{Emf, HighStorageResolution, NoMetric};

use fuzz_entry::{FuzzEntry, FuzzField, FuzzMetricValue};

/// EMF-specific flag mode applied on top of base fuzz entries.
#[derive(Debug, Clone, Copy, Arbitrary)]
enum FuzzMetricFlagMode {
    None,
    HighStorageResolution,
    NoMetric,
    HighThenNoMetric,
    NoMetricThenHigh,
}

/// Wrapper around `FuzzEntry` that applies EMF-specific flag modes to metrics.
#[derive(Debug, Arbitrary)]
struct EmfFuzzEntry {
    inner: FuzzEntry,
    flag_modes: Vec<FuzzMetricFlagMode>,
}

impl Entry for EmfFuzzEntry {
    fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
        // Delegate config and timestamps to the base entry's logic,
        // but handle fields here to apply EMF-specific flag modes.
        if self.inner.allow_split_entries {
            writer.config(&const { metrique_writer_core::config::AllowSplitEntries::new() });
        }
        if let Some(dims) = &self.inner.entry_dimensions {
            writer.config(&dims.0);
        }
        if let Some(timestamp) = &self.inner.timestamp {
            writer.timestamp(timestamp.to_system_time());
        }
        for (i, field) in self.inner.fields.iter().enumerate() {
            let flag_mode = self
                .flag_modes
                .get(i)
                .copied()
                .unwrap_or(FuzzMetricFlagMode::None);
            match field {
                FuzzField::StringProperty { name, value } => {
                    writer.value(name.0.as_str(), &value.as_str());
                }
                FuzzField::Metric {
                    name,
                    observations,
                    dimensions,
                    unit,
                } => {
                    let metric = FuzzMetricValue {
                        observations,
                        dimensions,
                        unit: unit.0,
                    };
                    match flag_mode {
                        FuzzMetricFlagMode::None => writer.value(name.0.as_str(), &metric),
                        FuzzMetricFlagMode::HighStorageResolution => {
                            writer.value(name.0.as_str(), &HighStorageResolution::from(metric));
                        }
                        FuzzMetricFlagMode::NoMetric => {
                            writer.value(name.0.as_str(), &NoMetric::from(metric));
                        }
                        FuzzMetricFlagMode::HighThenNoMetric => {
                            writer.value(
                                name.0.as_str(),
                                &NoMetric::from(HighStorageResolution::from(metric)),
                            );
                        }
                        FuzzMetricFlagMode::NoMetricThenHigh => {
                            writer.value(
                                name.0.as_str(),
                                &HighStorageResolution::from(NoMetric::from(metric)),
                            );
                        }
                    }
                }
            }
        }
    }
}

/// EMF can produce multiple newline-delimited JSON documents (split entries).
fn assert_valid_json_lines(output: &[u8], context: &str) {
    let mut saw_document = false;
    for line in output.split(|&b| b == b'\n') {
        if line.is_empty() {
            continue;
        }
        saw_document = true;
        let parsed = serde_json::from_slice::<serde_json::Value>(line).unwrap_or_else(|_| {
            panic!(
                "EMF produced invalid JSON ({context}): {}",
                String::from_utf8_lossy(line),
            )
        });
        assert!(
            parsed.is_object(),
            "EMF produced non-object JSON ({context}): {}",
            String::from_utf8_lossy(line),
        );
    }
    assert!(
        saw_document,
        "EMF returned success but emitted no JSON documents ({context})",
    );
}

#[derive(Debug, Arbitrary)]
struct FuzzEmfConfig {
    namespace: String,
    default_dimensions: Vec<Vec<String>>,
    extra_namespace: Option<String>,
    log_group_name: Option<String>,
    allow_ignored_dimensions: bool,
}

fn build_emf(config: &FuzzEmfConfig) -> Emf {
    // Keep generation broad while normalizing invalid empty input
    // into "publish without dimensions".
    let default_dimensions = if config.default_dimensions.is_empty() {
        vec![vec![]]
    } else {
        config.default_dimensions.clone()
    };
    let mut builder = Emf::builder(config.namespace.clone(), default_dimensions)
        .allow_ignored_dimensions(config.allow_ignored_dimensions);
    if let Some(extra) = &config.extra_namespace {
        builder = builder.add_namespace(extra.clone());
    }
    if let Some(log_group_name) = &config.log_group_name {
        builder = builder.log_group_name(log_group_name.clone());
    }
    builder.build()
}

fuzz_target!(|data: &[u8]| {
    let mut u = Unstructured::new(data);
    let Ok(config) = u.arbitrary::<FuzzEmfConfig>() else {
        return;
    };
    let Ok(entries) = u.arbitrary::<Vec<EmfFuzzEntry>>() else {
        return;
    };
    if entries.is_empty() {
        return;
    }

    // Regular EMF path, format all entries through the same formatter.
    let mut format = build_emf(&config);
    let mut output = Vec::new();
    for (i, entry) in entries.iter().enumerate() {
        output.clear();
        let result = format.format(entry, &mut output);
        if let Ok(()) = result {
            assert_valid_json_lines(&output, &format!("entry {i}"));
        }
    }

    // Sampled EMF path, same entries, fresh formatter.
    let mut sampled = build_emf(&config).with_sampling();
    for (i, entry) in entries.iter().enumerate() {
        let Ok(rate) = u.arbitrary::<f32>() else {
            return;
        };
        output.clear();
        let result = sampled.format_with_sample_rate(entry, &mut output, rate);
        if let Ok(()) = result {
            assert_valid_json_lines(&output, &format!("sampled entry {i}"));
        }
    }
});
