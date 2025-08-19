// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

#[cfg(feature = "metrics_rs_024")]
use metrique_writer_core::unit::{NegativeScale, PositiveScale};

/// convert a metrics_024::Unit to an metrique_writer_core::Unit
#[cfg(feature = "metrics_rs_024")]
pub(crate) fn metrics_024_unit_to_metrique_unit(
    unit: Option<metrics_024::Unit>,
) -> metrique_writer_core::Unit {
    match unit {
        Some(u) => match u {
            metrics_024::Unit::Count => metrique_writer_core::Unit::Count,
            metrics_024::Unit::Percent => metrique_writer_core::Unit::Percent,
            metrics_024::Unit::Seconds => metrique_writer_core::Unit::Second(NegativeScale::One),
            metrics_024::Unit::Milliseconds => {
                metrique_writer_core::Unit::Second(NegativeScale::Milli)
            }
            metrics_024::Unit::Microseconds => {
                metrique_writer_core::Unit::Second(NegativeScale::Micro)
            }
            metrics_024::Unit::Nanoseconds => metrique_writer_core::Unit::Custom("Nanoseconds"),
            metrics_024::Unit::Tebibytes => metrique_writer_core::Unit::Custom("Tebibytes"),
            metrics_024::Unit::Gibibytes => metrique_writer_core::Unit::Custom("Gibibytes"),
            metrics_024::Unit::Mebibytes => metrique_writer_core::Unit::Custom("Mebibytes"),
            metrics_024::Unit::Kibibytes => metrique_writer_core::Unit::Custom("Kibibytes"),
            metrics_024::Unit::Bytes => metrique_writer_core::Unit::Byte(PositiveScale::One),
            metrics_024::Unit::TerabitsPerSecond => {
                metrique_writer_core::Unit::BitPerSecond(PositiveScale::Tera)
            }
            metrics_024::Unit::GigabitsPerSecond => {
                metrique_writer_core::Unit::BitPerSecond(PositiveScale::Giga)
            }
            metrics_024::Unit::MegabitsPerSecond => {
                metrique_writer_core::Unit::BitPerSecond(PositiveScale::Mega)
            }
            metrics_024::Unit::KilobitsPerSecond => {
                metrique_writer_core::Unit::BitPerSecond(PositiveScale::Kilo)
            }
            metrics_024::Unit::BitsPerSecond => {
                metrique_writer_core::Unit::BitPerSecond(PositiveScale::One)
            }
            metrics_024::Unit::CountPerSecond => metrique_writer_core::Unit::Custom("Count/Second"),
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
            (metrique_writer_core::Unit::Count, metrics_024::Unit::Count),
            (
                metrique_writer_core::Unit::Percent,
                metrics_024::Unit::Percent,
            ),
            (
                metrique_writer_core::Unit::Second(NegativeScale::Micro),
                metrics_024::Unit::Microseconds,
            ),
            (
                metrique_writer_core::Unit::Second(NegativeScale::Milli),
                metrics_024::Unit::Milliseconds,
            ),
            (
                metrique_writer_core::Unit::Custom("Nanoseconds"),
                metrics_024::Unit::Nanoseconds,
            ),
            (
                metrique_writer_core::Unit::Second(NegativeScale::One),
                metrics_024::Unit::Seconds,
            ),
            (
                metrique_writer_core::Unit::Byte(PositiveScale::One),
                metrics_024::Unit::Bytes,
            ),
            (
                metrique_writer_core::Unit::Custom("Kibibytes"),
                metrics_024::Unit::Kibibytes,
            ),
            (
                metrique_writer_core::Unit::Custom("Mebibytes"),
                metrics_024::Unit::Mebibytes,
            ),
            (
                metrique_writer_core::Unit::Custom("Gibibytes"),
                metrics_024::Unit::Gibibytes,
            ),
            (
                metrique_writer_core::Unit::Custom("Tebibytes"),
                metrics_024::Unit::Tebibytes,
            ),
            (
                metrique_writer_core::Unit::Byte(PositiveScale::One),
                metrics_024::Unit::Bytes,
            ),
            (
                metrique_writer_core::Unit::BitPerSecond(PositiveScale::One),
                metrics_024::Unit::BitsPerSecond,
            ),
            (
                metrique_writer_core::Unit::BitPerSecond(PositiveScale::Kilo),
                metrics_024::Unit::KilobitsPerSecond,
            ),
            (
                metrique_writer_core::Unit::BitPerSecond(PositiveScale::Mega),
                metrics_024::Unit::MegabitsPerSecond,
            ),
            (
                metrique_writer_core::Unit::BitPerSecond(PositiveScale::Giga),
                metrics_024::Unit::GigabitsPerSecond,
            ),
            (
                metrique_writer_core::Unit::BitPerSecond(PositiveScale::Tera),
                metrics_024::Unit::TerabitsPerSecond,
            ),
            (
                metrique_writer_core::Unit::Custom("Count/Second"),
                metrics_024::Unit::CountPerSecond,
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
