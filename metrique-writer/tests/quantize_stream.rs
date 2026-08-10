// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Tests for the stream-level quantization decorator.

use std::time::{Duration, SystemTime};

use metrique_writer::quantize::{Quantizer, Rounding, SignificantBits};
use metrique_writer::stream::QuantizationPolicy;
use metrique_writer::test_util::to_test_entry;
use metrique_writer::{Entry, entry::QuantizedEntry};

fn quantizer(bits: u8, rounding: Rounding) -> Quantizer {
    Quantizer::new(SignificantBits::new(bits).unwrap(), rounding)
}

#[derive(Entry)]
#[entry(rename_all = "PascalCase")]
struct RequestMetrics {
    #[entry(timestamp)]
    timestamp: SystemTime,
    latency_nanos: u64,
    downstream_nanos: u64,
    request_count: u64,
    fault_count: u64,
}

fn sample() -> RequestMetrics {
    RequestMetrics {
        // A real epoch timestamp: quantizing this would move it by years.
        timestamp: SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000),
        latency_nanos: 1_234_567,
        downstream_nanos: 1000,
        request_count: 1000,
        fault_count: 1000,
    }
}

#[test]
fn only_named_metrics_are_quantized() {
    let policy = QuantizationPolicy::for_metrics(
        quantizer(4, Rounding::Floor),
        ["LatencyNanos", "DownstreamNanos"],
    );

    let entry = to_test_entry(QuantizedEntry::new(sample(), policy));

    // 1_234_567 at 4 bits, floor.
    assert_eq!(entry.metrics["LatencyNanos"], 1_179_648);
    assert_eq!(entry.metrics["DownstreamNanos"], 960);
    // Not named, so untouched.
    assert_eq!(entry.metrics["RequestCount"], 1000);
    assert_eq!(entry.metrics["FaultCount"], 1000);
}

#[test]
fn a_predicate_policy_selects_by_name() {
    let policy = QuantizationPolicy::matching(quantizer(4, Rounding::Floor), |name| {
        name.ends_with("Nanos")
    });

    let entry = to_test_entry(QuantizedEntry::new(sample(), policy));

    assert_eq!(entry.metrics["LatencyNanos"], 1_179_648);
    assert_eq!(entry.metrics["DownstreamNanos"], 960);
    assert_eq!(entry.metrics["RequestCount"], 1000);
    assert_eq!(entry.metrics["FaultCount"], 1000);
}

#[test]
fn an_empty_policy_quantizes_nothing() {
    let policy =
        QuantizationPolicy::for_metrics(quantizer(1, Rounding::Floor), Vec::<String>::new());

    let entry = to_test_entry(QuantizedEntry::new(sample(), policy));

    assert_eq!(entry.metrics["LatencyNanos"], 1_234_567);
    assert_eq!(entry.metrics["DownstreamNanos"], 1000);
    assert_eq!(entry.metrics["RequestCount"], 1000);
    assert_eq!(entry.metrics["FaultCount"], 1000);
}

#[test]
fn the_timestamp_is_never_quantized_even_when_the_filter_matches_everything() {
    // A policy that matches every name, including whatever the timestamp field is called.
    // Timestamps travel through `EntryWriter::timestamp`, not `value`, so the decorator has no
    // way to reach one.
    let policy = QuantizationPolicy::matching(quantizer(1, Rounding::Floor), |_| true);

    let original = to_test_entry(sample());
    let quantized = to_test_entry(QuantizedEntry::new(sample(), policy));

    assert_eq!(
        original.timestamp, quantized.timestamp,
        "the timestamp must be identical"
    );
    assert!(quantized.timestamp.is_some());

    // The values, by contrast, were reduced.
    assert_eq!(quantized.metrics["LatencyNanos"], 1_048_576);
}

#[test]
fn a_policy_matching_everything_still_leaves_small_values_exact() {
    // `exact_below` at 8 bits is 256: values *strictly below* 256 are returned unchanged even
    // under a policy that matches them. 256 itself is 9 bits wide, so it does get quantized —
    // this test pins that boundary.
    let policy = QuantizationPolicy::matching(quantizer(8, Rounding::Midpoint), |_| true);

    #[derive(Entry)]
    struct Small {
        one: u64,
        two_fifty_five: u64,
        two_fifty_six: u64,
    }

    let entry = to_test_entry(QuantizedEntry::new(
        Small {
            one: 1,
            two_fifty_five: 255,
            two_fifty_six: 256,
        },
        policy,
    ));

    assert_eq!(entry.metrics["one"], 1);
    assert_eq!(entry.metrics["two_fifty_five"], 255);
    // 256 is 0b100000000: one bit too wide, so midpoint moves it half a step to 257.
    assert_eq!(entry.metrics["two_fifty_six"], 257);
}

#[test]
fn policy_reports_what_it_applies_to() {
    let policy = QuantizationPolicy::for_metrics(quantizer(8, Rounding::Ceil), ["A", "B"]);

    assert!(policy.applies_to("A"));
    assert!(policy.applies_to("B"));
    assert!(!policy.applies_to("C"));
    assert_eq!(policy.quantizer(), quantizer(8, Rounding::Ceil));
}

#[test]
fn nested_entries_are_quantized_through_flatten() {
    #[derive(Entry)]
    #[entry(rename_all = "PascalCase")]
    struct Outer {
        outer_nanos: u64,
        #[entry(flatten)]
        inner: Inner,
    }

    #[derive(Entry)]
    #[entry(rename_all = "PascalCase")]
    struct Inner {
        inner_nanos: u64,
        inner_count: u64,
    }

    let policy = QuantizationPolicy::matching(quantizer(4, Rounding::Floor), |name| {
        name.ends_with("Nanos")
    });

    let entry = to_test_entry(QuantizedEntry::new(
        Outer {
            outer_nanos: 1000,
            inner: Inner {
                inner_nanos: 1000,
                inner_count: 1000,
            },
        },
        policy,
    ));

    assert_eq!(entry.metrics["OuterNanos"], 960);
    assert_eq!(entry.metrics["InnerNanos"], 960);
    assert_eq!(entry.metrics["InnerCount"], 1000);
}

#[test]
fn stream_and_format_decorators_agree() {
    use metrique_writer::{EntryIoStream, EntryIoStreamExt, FormatExt};
    use metrique_writer_format_emf::Emf;

    let policy = QuantizationPolicy::matching(quantizer(4, Rounding::Floor), |name| {
        name.ends_with("Nanos")
    });

    // Applied to the stream, after formatting is bound.
    let mut via_stream = Vec::new();
    {
        let mut stream = Emf::builder("Test".to_string(), vec![vec![]])
            .build()
            .output_to(&mut via_stream)
            .with_quantization(policy.clone());
        stream.next(&sample()).unwrap();
    }

    // Applied to the format, before it is bound to an output.
    let mut via_format = Vec::new();
    {
        let mut stream = Emf::builder("Test".to_string(), vec![vec![]])
            .build()
            .with_quantization(policy)
            .output_to(&mut via_format);
        stream.next(&sample()).unwrap();
    }

    assert_eq!(
        String::from_utf8(via_stream).unwrap(),
        String::from_utf8(via_format).unwrap(),
    );
}

#[test]
fn composes_with_sampling_in_either_order() {
    use metrique_writer::sample::SampledFormatExt;
    use metrique_writer::{EntryIoStream, EntryIoStreamExt, FormatExt};
    use metrique_writer_format_emf::Emf;

    let policy = QuantizationPolicy::matching(quantizer(4, Rounding::Floor), |name| {
        name.ends_with("Nanos")
    });

    // Sample first, then quantize the resulting stream.
    let mut buf = Vec::new();
    {
        let mut stream = Emf::builder("Test".to_string(), vec![vec![]])
            .build()
            .with_sampling()
            // Rate 1.0 keeps every entry, so the two orders are directly comparable.
            .sample_by_fixed_fraction(1.0)
            .output_to(&mut buf)
            .with_quantization(policy.clone());
        stream.next(&sample()).unwrap();
    }
    let sampled_then_quantized = String::from_utf8(buf).unwrap();

    // Quantize first, then sample. This relies on `Quantize` implementing `SampledFormat`.
    let mut buf = Vec::new();
    {
        let mut stream = Emf::builder("Test".to_string(), vec![vec![]])
            .build()
            .with_sampling()
            .with_quantization(policy)
            .sample_by_fixed_fraction(1.0)
            .output_to(&mut buf);
        stream.next(&sample()).unwrap();
    }
    let quantized_then_sampled = String::from_utf8(buf).unwrap();

    assert!(
        sampled_then_quantized.contains("1179648"),
        "expected a quantized latency, got {sampled_then_quantized}"
    );
    assert_eq!(sampled_then_quantized, quantized_then_sampled);
}
