//! Fuzz target for the EMF (Embedded Metric Format) formatter.
//!
//! Invariants tested:
//! - Successful formatting always produces one or more valid, newline-delimited JSON objects.
//! - Formatter state reuse across entries does not corrupt output.
//! - Both regular and sampled paths are exercised, with EMF-specific flag modes
//!   (HighStorageResolution, NoMetric) applied to metrics.

#![no_main]

mod fuzz_entry;

use arbitrary::Unstructured;
use libfuzzer_sys::fuzz_target;

use metrique_writer_core::format::Format;
use metrique_writer_core::sample::SampledFormat;
use metrique_writer_core::{Entry, EntryWriter};
use metrique_writer_format_emf::{Emf, HighStorageResolution, NoMetric};

use fuzz_entry::{
    FuzzEntry, FuzzField, FuzzMetricValue, arbitrary_sample_rate, arbitrary_string,
};

/// EMF-specific flag mode applied on top of base fuzz entries.
#[derive(Debug, Clone, Copy)]
enum FuzzMetricFlagMode {
    None,
    HighStorageResolution,
    NoMetric,
    HighThenNoMetric,
    NoMetricThenHigh,
}

impl<'a> arbitrary::Arbitrary<'a> for FuzzMetricFlagMode {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        let tag: u8 = u.arbitrary()?;
        Ok(match tag % 5 {
            0 => FuzzMetricFlagMode::None,
            1 => FuzzMetricFlagMode::HighStorageResolution,
            2 => FuzzMetricFlagMode::NoMetric,
            3 => FuzzMetricFlagMode::HighThenNoMetric,
            _ => FuzzMetricFlagMode::NoMetricThenHigh,
        })
    }
}

/// Wrapper around `FuzzEntry` that applies EMF-specific flag modes to metrics.
#[derive(Debug)]
struct EmfFuzzEntry {
    inner: FuzzEntry,
    /// One flag mode per metric field. Non-metric fields use index but ignore the flag.
    flag_modes: Vec<FuzzMetricFlagMode>,
}

impl Entry for EmfFuzzEntry {
    fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
        // Delegate config and timestamps to the base entry's logic,
        // but handle fields ourselves to apply EMF flags.
        if self.inner.allow_split_entries {
            writer.config(&const { metrique_writer_core::config::AllowSplitEntries::new() });
        }
        if let Some(entry_dimensions) = &self.inner.entry_dimensions {
            writer.config(entry_dimensions);
        }
        for timestamp in &self.inner.timestamps {
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
                    writer.value(name.as_str(), &value.as_str());
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
                        unit: *unit,
                    };
                    match flag_mode {
                        FuzzMetricFlagMode::None => writer.value(name.as_str(), &metric),
                        FuzzMetricFlagMode::HighStorageResolution => {
                            writer.value(name.as_str(), &HighStorageResolution::from(metric));
                        }
                        FuzzMetricFlagMode::NoMetric => {
                            writer.value(name.as_str(), &NoMetric::from(metric));
                        }
                        FuzzMetricFlagMode::HighThenNoMetric => {
                            writer.value(
                                name.as_str(),
                                &NoMetric::from(HighStorageResolution::from(metric)),
                            );
                        }
                        FuzzMetricFlagMode::NoMetricThenHigh => {
                            writer.value(
                                name.as_str(),
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

#[derive(Debug)]
struct FuzzEmfConfig {
    namespace: String,
    default_dimensions: Vec<Vec<String>>,
    extra_namespace: Option<String>,
    log_group_name: Option<String>,
    allow_ignored_dimensions: bool,
}

impl<'a> arbitrary::Arbitrary<'a> for FuzzEmfConfig {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        let namespace = arbitrary_string(u, 48)?;
        let extra_namespace = if u.arbitrary::<bool>()? {
            Some(arbitrary_string(u, 48)?)
        } else {
            None
        };
        let log_group_name = if u.arbitrary::<bool>()? {
            Some(arbitrary_string(u, 64)?)
        } else {
            None
        };

        // Keep at least one default dimension set to match common EMF setup.
        let set_count = (u.arbitrary::<u8>()? % 4) + 1;
        let mut default_dimensions = Vec::with_capacity(set_count as usize);
        for _ in 0..set_count {
            let dim_count = u.arbitrary::<u8>()? % 5;
            let mut dims = Vec::with_capacity(dim_count as usize);
            for _ in 0..dim_count {
                dims.push(arbitrary_string(u, 32)?);
            }
            default_dimensions.push(dims);
        }

        Ok(Self {
            namespace,
            default_dimensions,
            extra_namespace,
            log_group_name,
            allow_ignored_dimensions: u.arbitrary()?,
        })
    }
}

fn build_emf(config: &FuzzEmfConfig) -> Emf {
    let mut builder = Emf::builder(config.namespace.clone(), config.default_dimensions.clone())
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
    // 1–4 entries to format through the same formatter instance.
    let entry_count = match u.arbitrary::<u8>() {
        Ok(n) => (n % 4) as usize + 1,
        Err(_) => return,
    };
    let mut entries = Vec::with_capacity(entry_count);
    for _ in 0..entry_count {
        let Ok(entry) = u.arbitrary::<FuzzEntry>() else {
            return;
        };
        let flags: Vec<FuzzMetricFlagMode> = (0..entry.fields.len())
            .map(|_| u.arbitrary().unwrap_or(FuzzMetricFlagMode::None))
            .collect();
        entries.push(EmfFuzzEntry {
            inner: entry,
            flag_modes: flags,
        });
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
        let Ok(rate) = arbitrary_sample_rate(&mut u) else {
            return;
        };
        output.clear();
        let result = sampled.format_with_sample_rate(entry, &mut output, rate);
        if let Ok(()) = result {
            assert_valid_json_lines(&output, &format!("sampled entry {i}"));
        }
    }
});
