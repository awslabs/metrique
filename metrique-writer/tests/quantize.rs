// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! End-to-end tests for quantized values inside a derived [`Entry`].

use std::time::{Duration, SystemTime};

use metrique_writer::quantize::{Bits, Rounding, SignificantBits, rounding};
use metrique_writer::unit::Microsecond;
use metrique_writer::value::{MetricValue as _, Quantized};
use metrique_writer::{Entry, test_util::to_test_entry};

#[derive(Entry)]
#[entry(rename_all = "PascalCase")]
struct RequestMetrics {
    #[entry(timestamp)]
    timestamp: SystemTime,

    /// Eight significant bits, default midpoint rounding.
    latency_nanos: Quantized<u64, Bits<8>>,

    /// Four significant bits, never overstating.
    payload_bytes: Quantized<u64, Bits<4, rounding::Floor>>,

    /// Not quantized: an exact count.
    request_count: u64,

    /// Not quantized: an exact latency, for comparison.
    exact_nanos: u64,
}

#[test]
fn only_quantized_fields_are_reduced() {
    let entry = RequestMetrics {
        timestamp: SystemTime::UNIX_EPOCH,
        latency_nanos: 1_234_567u64.into(),
        payload_bytes: 1000u64.into(),
        request_count: 1000,
        exact_nanos: 1_234_567,
    };

    let entry = to_test_entry(&entry);

    // 1_234_567 at 8 significant bits, midpoint.
    assert_eq!(entry.metrics["LatencyNanos"], 1_232_896);
    // 1000 at 4 significant bits, floor.
    assert_eq!(entry.metrics["PayloadBytes"], 960);
    // Untouched.
    assert_eq!(entry.metrics["RequestCount"], 1000);
    assert_eq!(entry.metrics["ExactNanos"], 1_234_567);

    assert!(entry.timestamp.is_some(), "the timestamp must survive");
}

#[test]
fn quantized_values_stay_within_their_documented_bound() {
    let bits = SignificantBits::new(8).unwrap();
    let bound = bits.max_relative_error(Rounding::Midpoint);

    for raw in [1u64, 255, 256, 1000, 99_999, 1_234_567, 8_589_934_591] {
        #[derive(Entry)]
        struct Single {
            value: Quantized<u64, Bits<8>>,
        }

        let entry = to_test_entry(&Single { value: raw.into() });
        let emitted = entry.metrics["value"].as_u64();

        let error = (emitted as f64 - raw as f64).abs() / raw as f64;
        assert!(
            error <= bound,
            "raw={raw} emitted={emitted} error={error} bound={bound}"
        );
    }
}

#[test]
fn builder_method_matches_the_type_level_form() {
    #[derive(Entry)]
    struct ViaType {
        value: Quantized<u64, Bits<8, rounding::Floor>>,
    }

    #[derive(Entry)]
    struct ViaBuilder {
        value: Quantized<u64>,
    }

    let bits = SignificantBits::new(8).unwrap();

    let by_type = to_test_entry(&ViaType {
        value: 1_234_567u64.into(),
    });
    let by_builder = to_test_entry(&ViaBuilder {
        value: 1_234_567u64.quantized(bits, Rounding::Floor),
    });

    assert_eq!(by_type.metrics["value"], by_builder.metrics["value"]);
}

#[test]
fn converting_the_unit_first_applies_the_bound_in_that_unit() {
    #[derive(Entry)]
    struct Converted {
        // 3ms expressed in microseconds, then quantized: 3000us -> 2992us at 8 bits, floor.
        value: Quantized<
            metrique_writer::unit::WithUnit<Duration, Microsecond>,
            Bits<8, rounding::Floor>,
        >,
    }

    let entry = to_test_entry(&Converted {
        value: Duration::from_millis(3).with_unit::<Microsecond>().into(),
    });

    assert_eq!(entry.metrics["value"], 2992);
}

#[test]
fn quantized_optional_fields_are_skipped_when_absent() {
    #[derive(Entry)]
    struct Optional {
        present: Option<Quantized<u64, Bits<4, rounding::Floor>>>,
        absent: Option<Quantized<u64, Bits<4, rounding::Floor>>>,
    }

    let entry = to_test_entry(&Optional {
        present: Some(1000u64.into()),
        absent: None,
    });

    assert_eq!(entry.metrics["present"], 960);
    assert!(
        !entry.metrics.contains_key("absent"),
        "an absent optional should emit nothing"
    );
}
