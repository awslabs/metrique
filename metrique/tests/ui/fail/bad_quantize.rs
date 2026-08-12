// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use metrique::unit_of_work::metrics;

#[metrics]
struct BitsOutOfRange {
    // 53 is above the ceiling, because an f64 significand holds 53 bits.
    #[metrics(quantize(bits = 53))]
    too_many: u64,

    #[metrics(quantize(bits = 0))]
    too_few: u64,
}

#[metrics]
struct UnknownRounding {
    #[metrics(quantize(bits = 8, rounding = nearest))]
    unknown_mode: u64,
}

#[metrics]
struct DecimalDigitsInsteadOfBits {
    // Readers coming from decimal significant digits reach for this; the diagnostic should
    // name the equivalent bit count rather than just rejecting the key.
    #[metrics(quantize(digits = 2))]
    digits: u64,
}

#[metrics]
struct MissingBits {
    #[metrics(quantize(rounding = floor))]
    no_bits: u64,
}

#[metrics]
struct UnknownOption {
    #[metrics(quantize(bits = 8, precision = 2))]
    unknown_key: u64,
}

#[metrics]
struct NotAList {
    #[metrics(quantize = 8)]
    not_a_list: u64,
}

#[metrics]
struct QuantizeOnTimestamp {
    // Quantizing a timestamp would move it by hours; it must be rejected.
    #[metrics(timestamp, quantize(bits = 8))]
    timestamp: std::time::SystemTime,
}

#[metrics]
struct QuantizeOnFlatten {
    #[metrics(flatten, quantize(bits = 8))]
    nested: Nested,
}

#[metrics(subfield)]
struct Nested {
    inner: u64,
}

fn main() {}
