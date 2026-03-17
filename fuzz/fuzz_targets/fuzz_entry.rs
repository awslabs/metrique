//! Shared fuzz entry types used by all formatter fuzz targets.

use std::borrow::Cow;
use std::time::{Duration, SystemTime};

use arbitrary::{Arbitrary, Unstructured};

use metrique_writer_core::{
    config::{AllowSplitEntries, EntryDimensions},
    unit::{NegativeScale, PositiveScale},
    Entry, EntryWriter, MetricFlags, Observation, Unit, ValueWriter,
};

const EMPTY_FIELD_NAME_RATE_PERCENT: u8 = 5;

/// Field-name string for fuzzing.
///
/// Most generated names are forced non-empty to avoid spending too much time in
/// expected validation failures, but we keep a small empty-name probability to
/// still exercise that error path.
#[derive(Debug)]
pub struct FuzzFieldName(pub String);

impl<'a> Arbitrary<'a> for FuzzFieldName {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        let mut s: String = u.arbitrary()?;
        if s.is_empty() {
            // Keep empty names rarely (~5%) for validation-path coverage.
            if u.int_in_range(0..=99)? < EMPTY_FIELD_NAME_RATE_PERCENT {
                return Ok(Self(s));
            }

            s = u.arbitrary::<String>().unwrap_or_default();
            if s.is_empty() {
                s.push('x');
            }
        }
        Ok(Self(s))
    }
}

/// Wrapper for `Unit` (foreign `#[non_exhaustive]` type).
#[derive(Debug, Clone, Copy)]
pub struct FuzzUnit(pub Unit);

impl<'a> Arbitrary<'a> for FuzzUnit {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        let tag: u8 = u.arbitrary()?;
        Ok(Self(match tag % 11 {
            0 => Unit::None,
            1 => Unit::Count,
            2 => Unit::Percent,
            3 => Unit::Second(NegativeScale::Micro),
            4 => Unit::Second(NegativeScale::Milli),
            5 => Unit::Second(NegativeScale::One),
            6 => Unit::Byte(PositiveScale::One),
            7 => Unit::Byte(PositiveScale::Kilo),
            8 => Unit::Byte(PositiveScale::Mega),
            9 => Unit::Bit(PositiveScale::One),
            _ => Unit::Bit(PositiveScale::Kilo),
        }))
    }
}

/// A single field in our fuzzed entry.
#[derive(Debug, Arbitrary)]
pub enum FuzzField {
    /// A string property like `writer.value("key", &"some string")`
    StringProperty { name: FuzzFieldName, value: String },
    /// A metric with one or more observations
    Metric {
        name: FuzzFieldName,
        observations: Vec<FuzzObservation>,
        dimensions: Vec<(String, String)>,
        unit: FuzzUnit,
    },
}

#[derive(Debug, Arbitrary)]
pub enum FuzzObservation {
    Unsigned(u64),
    Floating(f64),
    Repeated { total: f64, occurrences: u64 },
}

impl FuzzObservation {
    pub fn to_observation(&self) -> Observation {
        match *self {
            FuzzObservation::Unsigned(v) => Observation::Unsigned(v),
            FuzzObservation::Floating(v) => Observation::Floating(v),
            FuzzObservation::Repeated { total, occurrences } => {
                Observation::Repeated { total, occurrences }
            }
        }
    }
}

#[derive(Debug, Arbitrary)]
pub struct FuzzTimestamp {
    pub before_epoch: bool,
    pub secs: u64,
}

impl FuzzTimestamp {
    pub fn to_system_time(&self) -> SystemTime {
        let duration = Duration::from_secs(self.secs);
        if self.before_epoch {
            SystemTime::UNIX_EPOCH
                .checked_sub(duration)
                .unwrap_or(SystemTime::UNIX_EPOCH)
        } else {
            SystemTime::UNIX_EPOCH
                .checked_add(duration)
                .unwrap_or(SystemTime::UNIX_EPOCH)
        }
    }
}

/// Wrapper for `EntryDimensions` (foreign type that can't derive `Arbitrary`).
/// Stores `EntryDimensions` directly because `writer.config()` borrows it for `'a`.
#[derive(Debug)]
pub struct FuzzEntryDimensions(pub EntryDimensions);

impl<'a> Arbitrary<'a> for FuzzEntryDimensions {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        let sets: Vec<Vec<String>> = u.arbitrary()?;
        let sets: Vec<Cow<'static, [Cow<'static, str>]>> = sets
            .into_iter()
            .map(|dims| Cow::Owned(dims.into_iter().map(Cow::Owned).collect()))
            .collect();
        Ok(Self(EntryDimensions::new(Cow::Owned(sets))))
    }
}

/// Fuzzed entry that exercises the full `EntryWriter` interface.
///
/// This is a format-agnostic entry: it writes metrics directly without
/// format-specific wrappers (like EMF flags). Format-specific fuzz targets
/// can wrap this to add their own behavior.
#[derive(Debug, Arbitrary)]
pub struct FuzzEntry {
    pub timestamp: Option<FuzzTimestamp>,
    pub allow_split_entries: bool,
    pub entry_dimensions: Option<FuzzEntryDimensions>,
    pub fields: Vec<FuzzField>,
}

impl Entry for FuzzEntry {
    fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
        if self.allow_split_entries {
            writer.config(&const { AllowSplitEntries::new() });
        }
        if let Some(dims) = &self.entry_dimensions {
            writer.config(&dims.0);
        }
        if let Some(timestamp) = &self.timestamp {
            writer.timestamp(timestamp.to_system_time());
        }
        for field in &self.fields {
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
                    writer.value(name.0.as_str(), &metric);
                }
            }
        }
    }
}

pub struct FuzzMetricValue<'a> {
    pub observations: &'a [FuzzObservation],
    pub dimensions: &'a [(String, String)],
    pub unit: Unit,
}

impl metrique_writer_core::value::Value for FuzzMetricValue<'_> {
    fn write(&self, writer: impl ValueWriter) {
        writer.metric(
            self.observations
                .iter()
                .map(FuzzObservation::to_observation),
            self.unit,
            self.dimensions
                .iter()
                .map(|(key, value)| (key.as_str(), value.as_str())),
            MetricFlags::empty(),
        );
    }
}
