// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::hash::{Hash, Hasher};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use metrique_writer_core::{
    Observation, Unit,
    unit::{NegativeScale, PositiveScale},
};
use opentelemetry::{
    KeyValue,
    metrics::{Counter, Gauge, Histogram, MeterProvider, UpDownCounter},
};
use opentelemetry_sdk::metrics::SdkMeterProvider;
use papaya::Equivalent;

use crate::flags::InstrumentKind;

/// Minimum time between repeated bad-value warnings, process-wide. NaN /
/// non-finite / out-of-range observations are dropped, but we only surface
/// one log line per interval so a misconfigured field can't drown the log.
const BAD_VALUE_WARN_INTERVAL: Duration = Duration::from_secs(60);

static LAST_BAD_VALUE_WARN: OnceLock<Mutex<Instant>> = OnceLock::new();

#[cold]
fn warn_bad_value(name: &str, kind: InstrumentKind, value: f64, reason: &'static str) {
    let mu =
        LAST_BAD_VALUE_WARN.get_or_init(|| Mutex::new(Instant::now() - BAD_VALUE_WARN_INTERVAL));
    let mut last = mu.lock().expect("OTel bad-value warn mutex poisoned");
    if last.elapsed() >= BAD_VALUE_WARN_INTERVAL {
        tracing::warn!(
            metric = name,
            kind = ?kind,
            value = value,
            reason = reason,
            "metrique-otel: dropping out-of-range observation"
        );
        *last = Instant::now();
    }
}

/// Cache of OTel instruments keyed by `(scope, name, kind)`. Each
/// `CachedInstrument` variant is `Arc`-backed inside the OTel SDK, so cloning
/// a hit is cheap and recording is internally synchronized, so no external
/// locking is required around the recording itself.
///
/// Storage is a lock-free [`papaya::HashMap`]: steady-state lookups never
/// take a lock, and the first-sight insert path uses `get_or_insert_with` to
/// stay race-safe without serializing readers.
pub(crate) struct InstrumentCache {
    meter_provider: SdkMeterProvider,
    instruments: papaya::HashMap<InstrumentKey, CachedInstrument>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct InstrumentKey {
    pub(crate) scope: &'static str,
    pub(crate) name: String,
    pub(crate) kind: InstrumentKind,
}

/// Borrowed counterpart to [`InstrumentKey`] used for cache lookups. Lets
/// `record` probe the map without first allocating an owned `String` for the
/// metric name; the owned key is only constructed on the insert path.
struct InstrumentKeyRef<'a> {
    scope: &'static str,
    name: &'a str,
    kind: InstrumentKind,
}

// Must match `#[derive(Hash)]` on `InstrumentKey`: same field order, and
// `Hash for String` delegates to `Hash for str`, so the two hashes agree.
impl Hash for InstrumentKeyRef<'_> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.scope.hash(state);
        self.name.hash(state);
        self.kind.hash(state);
    }
}

impl Equivalent<InstrumentKey> for InstrumentKeyRef<'_> {
    fn equivalent(&self, key: &InstrumentKey) -> bool {
        self.scope == key.scope && self.kind == key.kind && self.name == key.name.as_str()
    }
}

#[derive(Clone)]
pub(crate) enum CachedInstrument {
    Counter(Counter<u64>),
    UpDownCounter(UpDownCounter<i64>),
    Histogram(Histogram<f64>),
    Gauge(Gauge<f64>),
}

impl InstrumentCache {
    pub(crate) fn new(meter_provider: SdkMeterProvider) -> Self {
        Self {
            meter_provider,
            instruments: papaya::HashMap::new(),
        }
    }

    pub(crate) fn record(
        &self,
        scope: &'static str,
        name: &str,
        kind: InstrumentKind,
        observations: impl IntoIterator<Item = Observation>,
        unit: Unit,
        attributes: &[KeyValue],
    ) {
        let map = self.instruments.pin();
        // Probe with a borrowed key so a steady-state hit pays no allocation.
        // Only construct the owned `InstrumentKey` on the cold insert path.
        let instrument = if let Some(inst) = map.get(&InstrumentKeyRef { scope, name, kind }) {
            inst.clone()
        } else {
            // Instrument unit is fixed at creation time. If the same metric
            // name is later recorded with a different unit, the original wins,
            // mirroring the OTEL SDK's own behavior.
            let meter = self.meter_provider.meter(scope);
            let unit_str = unit_to_otel(unit);
            let owned_key = InstrumentKey {
                scope,
                name: name.to_owned(),
                kind,
            };
            // Race-safe insert: under contention the closure can run more than
            // once and only one instrument is kept. Building a fresh handle is
            // cheap and the discard path is rare (first sight of a key).
            map.get_or_insert_with(owned_key, || match kind {
                InstrumentKind::Counter => CachedInstrument::Counter(
                    meter
                        .u64_counter(name.to_owned())
                        .with_unit(unit_str)
                        .build(),
                ),
                InstrumentKind::UpDownCounter => CachedInstrument::UpDownCounter(
                    meter
                        .i64_up_down_counter(name.to_owned())
                        .with_unit(unit_str)
                        .build(),
                ),
                InstrumentKind::Histogram => CachedInstrument::Histogram(
                    meter
                        .f64_histogram(name.to_owned())
                        .with_unit(unit_str)
                        .build(),
                ),
                InstrumentKind::Gauge => CachedInstrument::Gauge(
                    meter.f64_gauge(name.to_owned()).with_unit(unit_str).build(),
                ),
            })
            .clone()
        };

        const NON_FINITE_OR_NEGATIVE: &str = "non-finite or negative";
        const NON_FINITE: &str = "non-finite";

        match &instrument {
            CachedInstrument::Counter(c) => {
                for obs in observations {
                    let v = match obs {
                        Observation::Unsigned(v) => v,
                        Observation::Floating(v) => {
                            if !v.is_finite() || v < 0.0 {
                                warn_bad_value(name, kind, v, NON_FINITE_OR_NEGATIVE);
                                continue;
                            }
                            v as u64
                        }
                        Observation::Repeated { total, .. } => {
                            if !total.is_finite() || total < 0.0 {
                                warn_bad_value(name, kind, total, NON_FINITE_OR_NEGATIVE);
                                continue;
                            }
                            total as u64
                        }
                        _ => continue,
                    };
                    c.add(v, attributes);
                }
            }
            CachedInstrument::UpDownCounter(c) => {
                for obs in observations {
                    let v = match obs {
                        Observation::Unsigned(v) => v as i64,
                        Observation::Floating(v) => {
                            if !v.is_finite() {
                                warn_bad_value(name, kind, v, NON_FINITE);
                                continue;
                            }
                            v as i64
                        }
                        Observation::Repeated { total, occurrences } if occurrences > 0 => {
                            let mean = total / occurrences as f64;
                            if !mean.is_finite() {
                                warn_bad_value(name, kind, mean, NON_FINITE);
                                continue;
                            }
                            mean as i64
                        }
                        _ => continue,
                    };
                    c.add(v, attributes);
                }
            }
            CachedInstrument::Histogram(h) => {
                for obs in observations {
                    let v = match obs {
                        Observation::Unsigned(v) => v as f64,
                        Observation::Floating(v) => {
                            if !v.is_finite() {
                                warn_bad_value(name, kind, v, NON_FINITE);
                                continue;
                            }
                            v
                        }
                        // Repeated has already collapsed the distribution to
                        // (total, occurrences); we can't recover individual
                        // samples. Record the mean once: bucketing is lossy
                        // and, critically, the OTel histogram's `count` will
                        // report 1 here rather than `occurrences`, so any
                        // count-based aggregation downstream will undercount.
                        // The `sum` (mean x 1) is also no longer the true sum
                        // (which would be `total`). Users that need faithful
                        // distributions or accurate counts should keep raw
                        // `Floating` observations and avoid pre-summing.
                        Observation::Repeated { total, occurrences } if occurrences > 0 => {
                            let mean = total / occurrences as f64;
                            if !mean.is_finite() {
                                warn_bad_value(name, kind, mean, NON_FINITE);
                                continue;
                            }
                            mean
                        }
                        _ => continue,
                    };
                    h.record(v, attributes);
                }
            }
            CachedInstrument::Gauge(g) => {
                for obs in observations {
                    let v = match obs {
                        Observation::Unsigned(v) => v as f64,
                        Observation::Floating(v) => {
                            if !v.is_finite() {
                                warn_bad_value(name, kind, v, NON_FINITE);
                                continue;
                            }
                            v
                        }
                        Observation::Repeated { total, occurrences } if occurrences > 0 => {
                            let mean = total / occurrences as f64;
                            if !mean.is_finite() {
                                warn_bad_value(name, kind, mean, NON_FINITE);
                                continue;
                            }
                            mean
                        }
                        _ => continue,
                    };
                    g.record(v, attributes);
                }
            }
        }
    }
}

/// Map a `metrique` [`Unit`] to the UCUM-flavored string the OTEL semantic
/// conventions expect on the wire (e.g. `ms`, `By`, `%`, `1` for dimensionless).
pub(crate) fn unit_to_otel(unit: Unit) -> &'static str {
    match unit {
        Unit::None | Unit::Count => "1",
        Unit::Percent => "%",
        Unit::Second(NegativeScale::Micro) => "us",
        Unit::Second(NegativeScale::Milli) => "ms",
        Unit::Second(NegativeScale::One) => "s",
        Unit::Byte(scale) => match scale {
            PositiveScale::One => "By",
            PositiveScale::Kilo => "KBy",
            PositiveScale::Mega => "MBy",
            PositiveScale::Giga => "GBy",
            PositiveScale::Tera => "TBy",
            _ => "By",
        },
        Unit::BytePerSecond(scale) => match scale {
            PositiveScale::One => "By/s",
            PositiveScale::Kilo => "KBy/s",
            PositiveScale::Mega => "MBy/s",
            PositiveScale::Giga => "GBy/s",
            PositiveScale::Tera => "TBy/s",
            _ => "By/s",
        },
        Unit::Bit(scale) => match scale {
            PositiveScale::One => "bit",
            PositiveScale::Kilo => "Kbit",
            PositiveScale::Mega => "Mbit",
            PositiveScale::Giga => "Gbit",
            PositiveScale::Tera => "Tbit",
            _ => "bit",
        },
        Unit::BitPerSecond(scale) => match scale {
            PositiveScale::One => "bit/s",
            PositiveScale::Kilo => "Kbit/s",
            PositiveScale::Mega => "Mbit/s",
            PositiveScale::Giga => "Gbit/s",
            PositiveScale::Tera => "Tbit/s",
            _ => "bit/s",
        },
        Unit::Custom(s) => s,
        // `Unit` is `#[non_exhaustive]`; fall back to dimensionless for
        // unknown future variants rather than panicking.
        _ => "1",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opentelemetry_sdk::metrics::data::{AggregatedMetrics, MetricData};
    use opentelemetry_sdk::metrics::{InMemoryMetricExporter, PeriodicReader};

    fn fresh() -> (InstrumentCache, SdkMeterProvider, InMemoryMetricExporter) {
        let exporter = InMemoryMetricExporter::default();
        let reader = PeriodicReader::builder(exporter.clone()).build();
        let mp = SdkMeterProvider::builder().with_reader(reader).build();
        let cache = InstrumentCache::new(mp.clone());
        (cache, mp, exporter)
    }

    fn metric_exists(exporter: &InMemoryMetricExporter, name: &str) -> bool {
        let exported = exporter
            .get_finished_metrics()
            .expect("get_finished_metrics");
        exported
            .iter()
            .flat_map(|rm| rm.scope_metrics())
            .flat_map(|sm| sm.metrics())
            .any(|m| m.name() == name)
    }

    #[test]
    fn counter_drops_negative_nan_and_infinite_floating() {
        let (cache, mp, exporter) = fresh();
        cache.record(
            "metrics_test",
            "BadCounter",
            InstrumentKind::Counter,
            [
                Observation::Floating(-1.0),
                Observation::Floating(f64::NAN),
                Observation::Floating(f64::INFINITY),
                Observation::Floating(f64::NEG_INFINITY),
            ],
            Unit::Count,
            &[],
        );
        mp.force_flush().expect("force_flush");
        assert!(
            !metric_exists(&exporter, "BadCounter"),
            "out-of-range Counter observations should be dropped, not emitted"
        );
    }

    #[test]
    fn counter_drops_negative_repeated() {
        let (cache, mp, exporter) = fresh();
        cache.record(
            "metrics_test",
            "BadCounterRepeated",
            InstrumentKind::Counter,
            [Observation::Repeated {
                total: -3.0,
                occurrences: 2,
            }],
            Unit::Count,
            &[],
        );
        mp.force_flush().expect("force_flush");
        assert!(
            !metric_exists(&exporter, "BadCounterRepeated"),
            "negative Repeated total on Counter should be dropped"
        );
    }

    #[test]
    fn up_down_counter_drops_nan_floating() {
        let (cache, mp, exporter) = fresh();
        cache.record(
            "metrics_test",
            "BadUpDown",
            InstrumentKind::UpDownCounter,
            [Observation::Floating(f64::NAN)],
            Unit::None,
            &[],
        );
        mp.force_flush().expect("force_flush");
        assert!(
            !metric_exists(&exporter, "BadUpDown"),
            "NaN UpDownCounter observations should be dropped"
        );
    }

    #[test]
    fn up_down_counter_repeated_uses_mean_not_total() {
        let (cache, mp, exporter) = fresh();
        cache.record(
            "metrics_test",
            "Level",
            InstrumentKind::UpDownCounter,
            [Observation::Repeated {
                total: 15.0,
                occurrences: 3,
            }],
            Unit::None,
            &[],
        );
        mp.force_flush().expect("force_flush");

        let exported = exporter
            .get_finished_metrics()
            .expect("get_finished_metrics");
        let mut total = 0i64;
        let mut found = false;
        for rm in &exported {
            for sm in rm.scope_metrics() {
                for m in sm.metrics() {
                    if m.name() == "Level"
                        && let AggregatedMetrics::I64(MetricData::Sum(s)) = m.data()
                    {
                        for dp in s.data_points() {
                            total += dp.value();
                        }
                        found = true;
                    }
                }
            }
        }
        assert!(found, "expected 'Level' UpDownCounter in exported metrics");
        assert_eq!(
            total, 5,
            "UpDownCounter Repeated should record mean (15/3 = 5), not total (15)"
        );
    }

    #[test]
    fn histogram_drops_nan_floating() {
        let (cache, mp, exporter) = fresh();
        cache.record(
            "metrics_test",
            "BadHist",
            InstrumentKind::Histogram,
            [Observation::Floating(f64::NAN)],
            Unit::None,
            &[],
        );
        mp.force_flush().expect("force_flush");
        assert!(
            !metric_exists(&exporter, "BadHist"),
            "NaN Histogram observations should be dropped"
        );
    }

    #[test]
    fn gauge_drops_nan_floating() {
        let (cache, mp, exporter) = fresh();
        cache.record(
            "metrics_test",
            "BadGauge",
            InstrumentKind::Gauge,
            [Observation::Floating(f64::NAN)],
            Unit::None,
            &[],
        );
        mp.force_flush().expect("force_flush");
        assert!(
            !metric_exists(&exporter, "BadGauge"),
            "NaN Gauge observations should be dropped"
        );
    }
}
