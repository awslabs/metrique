// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::hash::{Hash, Hasher};

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
