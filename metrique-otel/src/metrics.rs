// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::{collections::HashMap, sync::RwLock};

use metrique_writer_core::{
    Observation, Unit,
    unit::{NegativeScale, PositiveScale},
};
use opentelemetry::{
    KeyValue,
    metrics::{Counter, Gauge, Histogram, MeterProvider, UpDownCounter},
};
use opentelemetry_sdk::metrics::SdkMeterProvider;

use crate::flags::InstrumentKind;

/// Cache of OTel instruments keyed by `(name, kind)`. Each `CachedInstrument`
/// variant is `Arc`-backed inside the OTel SDK, so cloning a hit is cheap and
/// recording is internally synchronized — no external locking required around
/// the recording itself. Reads take the read lock and clone the handle out;
/// the write lock is taken only on first sight of a new `(name, kind)` pair.
pub(crate) struct InstrumentCache {
    meter_provider: SdkMeterProvider,
    instruments: RwLock<HashMap<InstrumentKey, CachedInstrument>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct InstrumentKey {
    pub(crate) scope: &'static str,
    pub(crate) name: String,
    pub(crate) kind: InstrumentKind,
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
            instruments: RwLock::new(HashMap::new()),
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
        let key = InstrumentKey {
            scope,
            name: name.to_owned(),
            kind,
        };
        // Read-fast-path: clone the `Arc`-backed handle out and drop the lock
        // before doing any recording. Steady-state lookups never take the
        // write lock.
        let instrument = if let Some(inst) = self
            .instruments
            .read()
            .expect("instrument cache poisoned")
            .get(&key)
        {
            inst.clone()
        } else {
            // Instrument unit is fixed at creation time. If the same metric
            // name is later recorded with a different unit, the original wins
            // — that mirrors the OTEL SDK's own behavior.
            let meter = self.meter_provider.meter(scope);
            let unit_str = unit_to_otel(unit);
            let fresh = match kind {
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
            };
            // Race-safe insert: if another writer beat us, keep their entry.
            self.instruments
                .write()
                .expect("instrument cache poisoned")
                .entry(key)
                .or_insert(fresh)
                .clone()
        };

        match &instrument {
            CachedInstrument::Counter(c) => {
                for obs in observations {
                    let v = match obs {
                        Observation::Unsigned(v) => v,
                        // Counters are non-negative; clamp at 0 rather than
                        // emitting a panic for an out-of-spec observation.
                        Observation::Floating(v) => v.max(0.0) as u64,
                        Observation::Repeated { total, .. } => total.max(0.0) as u64,
                        _ => continue,
                    };
                    c.add(v, attributes);
                }
            }
            CachedInstrument::UpDownCounter(c) => {
                for obs in observations {
                    let v = match obs {
                        Observation::Unsigned(v) => v as i64,
                        Observation::Floating(v) => v as i64,
                        Observation::Repeated { total, .. } => total as i64,
                        _ => continue,
                    };
                    c.add(v, attributes);
                }
            }
            CachedInstrument::Histogram(h) => {
                for obs in observations {
                    let v = match obs {
                        Observation::Unsigned(v) => v as f64,
                        Observation::Floating(v) => v,
                        // Repeated has already collapsed the distribution to
                        // (total, occurrences); we can't recover individual
                        // samples. Record the mean once — bucketing is lossy
                        // but count and sum stay sensible. Users that need
                        // faithful distributions should keep raw `Floating`
                        // observations and avoid pre-summing.
                        Observation::Repeated { total, occurrences } if occurrences > 0 => {
                            total / occurrences as f64
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
                        Observation::Floating(v) => v,
                        Observation::Repeated { total, occurrences } if occurrences > 0 => {
                            total / occurrences as f64
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
