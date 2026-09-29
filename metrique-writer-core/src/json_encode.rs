// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Shared JSON scalar encoders used by the `metrique-writer-format-json` crate's native
//! renderer and (in a later change) the crate's object string-fallback.
//!
//! These live in core so both paths share one implementation — a value renders the same whether
//! it appears at the top level of a JSON entry or nested inside an object rendered through the
//! fallback. Only the scalar primitives are shared; array/object framing and the `_aws` envelope
//! stay in their respective renderers, and other JSON-shaped formats (e.g. EMF) keep their own
//! encoders.
//!
//! The shared implementation is not a promise of byte-identical output across every path. Two
//! deliberate carve-outs (see `docs/object-values-rfc.md`):
//! - A [`Observation::Repeated`]'s `count` depends on the `multiplicity` passed by the caller.
//!   The native renderer passes the entry's sampling multiplicity; the object string-fallback
//!   (added in a later change) has none and passes `None`, so a sampled observation can differ
//!   in `count`.
//! - The pre-existing scalar-array `StringCapture` path in [`crate::value`] is a separate regime
//!   and is unchanged; it does not route through these functions.
//!
//! Stability: these functions are public API — `metrique-writer-format-json` depends on them —
//! so their signatures are covered by `metrique-writer-core`'s semver guarantee. They are
//! generic over [`std::fmt::Write`] and return [`std::fmt::Result`], so a caller may render into
//! any text sink and observe write failures rather than have them silently swallowed. Writing
//! into a [`String`] is infallible, so callers with a `String` buffer can discard the result.

use std::fmt::{self, Write};

use crate::value::Observation;

/// Push a scalar observation value into the buffer.
///
/// `multiplicity` scales a [`Observation::Repeated`]'s occurrence count for sampling; pass `None`
/// (equivalent to `1`) outside a sampling context, such as the object string fallback.
pub fn push_observation<W: Write + ?Sized>(
    buf: &mut W,
    obs: Observation,
    multiplicity: Option<u64>,
) -> fmt::Result {
    // This match is intentionally exhaustive with no wildcard: `Observation` is
    // `#[non_exhaustive]` but defined in this crate, so a wildcard would be
    // `unreachable_patterns` (a hard error under `-D warnings`). Adding a variant is meant to
    // break this build rather than silently render `null`.
    match obs {
        Observation::Unsigned(v) => buf.write_str(itoa::Buffer::new().format(v)),
        Observation::Floating(v) => push_float(buf, v),
        Observation::Repeated { total, occurrences } => {
            let mult = multiplicity.unwrap_or(1);
            buf.write_str("{\"total\":")?;
            push_float(buf, total)?;
            buf.write_str(",\"count\":")?;
            buf.write_str(itoa::Buffer::new().format(occurrences.saturating_mul(mult)))?;
            buf.write_char('}')
        }
    }
}

/// Push a float value, clamping infinities to ±`f64::MAX` and writing `null` for NaN.
pub(crate) fn push_float<W: Write + ?Sized>(buf: &mut W, v: f64) -> fmt::Result {
    // `f64::MIN == -f64::MAX`; spell it as `f64::MIN` so the bound is obviously in range.
    let v = v.clamp(f64::MIN, f64::MAX);
    if v.is_nan() {
        buf.write_str("null")
    } else {
        // We use `dtoa` over `ryu` because `dtoa` emits decimal notation for typical
        // magnitudes, which is easier to script against and more portable across downstream
        // metric consumers. Note it still falls back to scientific notation at the extremes
        // (e.g. `f64::MAX` renders as `1.79...e308`).
        let mut buffer = dtoa::Buffer::new();
        let s = buffer.format_finite(v);
        // Strip trailing ".0" for cleaner integer-like output
        buf.write_str(s.strip_suffix(".0").unwrap_or(s))
    }
}

/// Push a JSON-escaped string with surrounding quotes into the buffer.
pub fn push_json_string<W: Write + ?Sized>(buf: &mut W, s: &str) -> fmt::Result {
    buf.write_char('"')?;
    let bytes = s.as_bytes();
    let mut start = 0;
    for (i, &b) in bytes.iter().enumerate() {
        let escape = match b {
            b'"' => "\\\"",
            b'\\' => "\\\\",
            b'\n' => "\\n",
            b'\r' => "\\r",
            b'\t' => "\\t",
            0x00..=0x1f => {
                buf.write_str(&s[start..i])?;
                start = i + 1;
                write!(buf, "\\u{:04x}", b)?;
                continue;
            }
            _ => continue,
        };
        buf.write_str(&s[start..i])?;
        buf.write_str(escape)?;
        start = i + 1;
    }
    buf.write_str(&s[start..])?;
    buf.write_char('"')
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::strategy::Strategy as _;

    fn obs(o: Observation, multiplicity: Option<u64>) -> String {
        let mut buf = String::new();
        // Writing into a `String` is infallible.
        let _ = push_observation(&mut buf, o, multiplicity);
        buf
    }

    fn js(s: &str) -> String {
        let mut buf = String::new();
        let _ = push_json_string(&mut buf, s);
        buf
    }

    #[test]
    fn repeated_renders_as_total_and_count_object() {
        assert_eq!(
            obs(
                Observation::Repeated {
                    total: 10.0,
                    occurrences: 4
                },
                None
            ),
            "{\"total\":10,\"count\":4}"
        );
    }

    #[test]
    fn repeated_count_scales_by_multiplicity() {
        assert_eq!(
            obs(
                Observation::Repeated {
                    total: 10.0,
                    occurrences: 4
                },
                Some(5)
            ),
            "{\"total\":10,\"count\":20}"
        );
    }

    #[test]
    fn integers_and_floats_render_without_trailing_zero() {
        assert_eq!(obs(Observation::Unsigned(42), None), "42");
        assert_eq!(obs(Observation::Floating(2.0), None), "2");
        assert_eq!(obs(Observation::Floating(1.5), None), "1.5");
    }

    #[test]
    fn repeated_count_does_not_overflow_on_saturating_mul() {
        // `occurrences.saturating_mul(mult)` is a deliberate overflow guard; pin it so a future
        // `*` regression is caught rather than panicking in debug builds.
        assert_eq!(
            obs(
                Observation::Repeated {
                    total: 1.0,
                    occurrences: u64::MAX,
                },
                Some(2),
            ),
            format!("{{\"total\":1,\"count\":{}}}", u64::MAX),
        );
    }

    #[test]
    fn repeated_with_zero_occurrences_does_not_panic() {
        // `Observation::Repeated` docs call out `occurrences == 0` as a case that must not panic.
        assert_eq!(
            obs(
                Observation::Repeated {
                    total: 5.0,
                    occurrences: 0,
                },
                None,
            ),
            "{\"total\":5,\"count\":0}",
        );
    }

    #[test]
    fn non_finite_floats_follow_the_json_policy() {
        // NaN has no JSON representation; infinities clamp to the finite extremes.
        assert_eq!(obs(Observation::Floating(f64::NAN), None), "null");
        let pos_inf = obs(Observation::Floating(f64::INFINITY), None);
        let neg_inf = obs(Observation::Floating(f64::NEG_INFINITY), None);
        // Clamp to exactly ±f64::MAX (not merely "starts with 1"). Assert the round-tripped
        // value rather than the textual form: dtoa does fall back to scientific notation at this
        // magnitude, so its "decimal notation" preference does not hold at the extremes.
        assert_eq!(
            pos_inf.parse::<f64>().ok(),
            Some(f64::MAX),
            "+inf clamps to f64::MAX"
        );
        assert_eq!(
            neg_inf.parse::<f64>().ok(),
            Some(-f64::MAX),
            "-inf clamps to -f64::MAX"
        );
    }

    #[test]
    fn strings_escape_control_and_special_characters() {
        assert_eq!(js("tab\tnl\nq\"bs\\end"), "\"tab\\tnl\\nq\\\"bs\\\\end\"");
        // A control char below 0x20 with no short escape uses \u00xx.
        assert_eq!(js("\u{1}"), "\"\\u0001\"");
        // Carriage return has a short escape (the `\r` arm).
        assert_eq!(js("a\rb"), "\"a\\rb\"");
    }

    #[test]
    fn strings_slice_by_byte_index_around_multibyte_characters() {
        // `push_json_string` slices the &str by byte index; this is sound only because no UTF-8
        // continuation byte matches an escape arm. Pin it with escapes on both sides of a
        // multibyte character (α is two bytes; 😀 is four).
        assert_eq!(js("α\t😀\"β"), "\"α\\t😀\\\"β\"");
    }

    #[test]
    fn write_failure_propagates_rather_than_truncating_silently() {
        // A fallible sink that errors mid-write must surface the error. This is why the encoders
        // return `fmt::Result` rather than swallowing writes.
        struct FailAfter(usize);
        impl fmt::Write for FailAfter {
            fn write_str(&mut self, s: &str) -> fmt::Result {
                if self.0 == 0 {
                    return Err(fmt::Error);
                }
                self.0 -= 1;
                let _ = s;
                Ok(())
            }
        }
        // The opening quote consumes the only successful write; the next write fails.
        assert!(push_json_string(&mut FailAfter(1), "abc").is_err());
        assert!(push_observation(&mut FailAfter(0), Observation::Unsigned(1), None).is_err());
    }

    proptest::proptest! {
        // For any input string, the escaped output must be valid JSON that round-trips back to
        // the original. Build the string from `any::<char>()` rather than `any::<String>()`:
        // the latter's default strategy is the regex `\PC*`, which excludes control characters
        // and so never exercises the escape arms. `any::<char>()`'s special-case set includes
        // NUL, tab, CR, LF, quote, and backslash, hitting every branch.
        #[test]
        fn push_json_string_always_produces_round_trippable_json(
            s in proptest::collection::vec(proptest::prelude::any::<char>(), 0..64)
                .prop_map(|cs| cs.into_iter().collect::<String>())
        ) {
            let mut buf = String::new();
            let _ = push_json_string(&mut buf, &s);
            match serde_json::from_str::<String>(&buf) {
                Ok(decoded) => proptest::prop_assert_eq!(decoded, s),
                Err(e) => proptest::prop_assert!(
                    false,
                    "escaped output was not valid JSON: {e} (output: {buf:?})"
                ),
            }
        }
    }
}
