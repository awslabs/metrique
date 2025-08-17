// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use metrique_writer_core::unit::{NegativeScale, PositiveScale};

/// convert a metrics::Unit to an metrique_writer_core::Unit
#[cfg(feature = "metrics_rs_024")]
pub(crate) fn metrics_024_unit_to_metrique_unit(
    unit: Option<metrics::Unit>,
) -> metrique_writer_core::Unit {
    match unit {
        Some(u) => match u {
            metrics::Unit::Count => metrique_writer_core::Unit::Count,
            metrics::Unit::Percent => metrique_writer_core::Unit::Percent,
            metrics::Unit::Seconds => metrique_writer_core::Unit::Second(NegativeScale::One),
            metrics::Unit::Milliseconds => metrique_writer_core::Unit::Second(NegativeScale::Milli),
            metrics::Unit::Microseconds => metrique_writer_core::Unit::Second(NegativeScale::Micro),
            metrics::Unit::Nanoseconds => metrique_writer_core::Unit::Custom("Nanoseconds"),
            metrics::Unit::Tebibytes => metrique_writer_core::Unit::Custom("Tebibytes"),
            metrics::Unit::Gibibytes => metrique_writer_core::Unit::Custom("Gibibytes"),
            metrics::Unit::Mebibytes => metrique_writer_core::Unit::Custom("Mebibytes"),
            metrics::Unit::Kibibytes => metrique_writer_core::Unit::Custom("Kibibytes"),
            metrics::Unit::Bytes => metrique_writer_core::Unit::Byte(PositiveScale::One),
            metrics::Unit::TerabitsPerSecond => {
                metrique_writer_core::Unit::BitPerSecond(PositiveScale::Tera)
            }
            metrics::Unit::GigabitsPerSecond => {
                metrique_writer_core::Unit::BitPerSecond(PositiveScale::Giga)
            }
            metrics::Unit::MegabitsPerSecond => {
                metrique_writer_core::Unit::BitPerSecond(PositiveScale::Mega)
            }
            metrics::Unit::KilobitsPerSecond => {
                metrique_writer_core::Unit::BitPerSecond(PositiveScale::Kilo)
            }
            metrics::Unit::BitsPerSecond => {
                metrique_writer_core::Unit::BitPerSecond(PositiveScale::One)
            }
            metrics::Unit::CountPerSecond => metrique_writer_core::Unit::Custom("Count/Second"),
        },
        None => metrique_writer_core::Unit::None,
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_metrics_024_unit_to_metrique_unit() {
        let map = [
            (metrique_writer_core::Unit::Count, metrics::Unit::Count),
            (metrique_writer_core::Unit::Percent, metrics::Unit::Percent),
            (
                metrique_writer_core::Unit::Second(NegativeScale::Micro),
                metrics::Unit::Microseconds,
            ),
            (
                metrique_writer_core::Unit::Second(NegativeScale::Milli),
                metrics::Unit::Milliseconds,
            ),
            (
                metrique_writer_core::Unit::Custom("Nanoseconds"),
                metrics::Unit::Nanoseconds,
            ),
            (
                metrique_writer_core::Unit::Second(NegativeScale::One),
                metrics::Unit::Seconds,
            ),
            (
                metrique_writer_core::Unit::Byte(PositiveScale::One),
                metrics::Unit::Bytes,
            ),
            (
                metrique_writer_core::Unit::Custom("Kibibytes"),
                metrics::Unit::Kibibytes,
            ),
            (
                metrique_writer_core::Unit::Custom("Mebibytes"),
                metrics::Unit::Mebibytes,
            ),
            (
                metrique_writer_core::Unit::Custom("Gibibytes"),
                metrics::Unit::Gibibytes,
            ),
            (
                metrique_writer_core::Unit::Custom("Tebibytes"),
                metrics::Unit::Tebibytes,
            ),
            (
                metrique_writer_core::Unit::Byte(PositiveScale::One),
                metrics::Unit::Bytes,
            ),
            (
                metrique_writer_core::Unit::BitPerSecond(PositiveScale::One),
                metrics::Unit::BitsPerSecond,
            ),
            (
                metrique_writer_core::Unit::BitPerSecond(PositiveScale::Kilo),
                metrics::Unit::KilobitsPerSecond,
            ),
            (
                metrique_writer_core::Unit::BitPerSecond(PositiveScale::Mega),
                metrics::Unit::MegabitsPerSecond,
            ),
            (
                metrique_writer_core::Unit::BitPerSecond(PositiveScale::Giga),
                metrics::Unit::GigabitsPerSecond,
            ),
            (
                metrique_writer_core::Unit::BitPerSecond(PositiveScale::Tera),
                metrics::Unit::TerabitsPerSecond,
            ),
            (
                metrique_writer_core::Unit::Custom("Count/Second"),
                metrics::Unit::CountPerSecond,
            ),
        ];
        assert_eq!(
            metrics_024_unit_to_metrique_unit(None),
            metrique_writer_core::Unit::None
        );
        for (metrique_unit, metrics_unit) in map {
            assert_eq!(
                metrics_024_unit_to_metrique_unit(Some(metrics_unit)),
                metrique_unit
            );
        }
    }
}
