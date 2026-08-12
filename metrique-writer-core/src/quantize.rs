// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Reduce the precision of metric values, trading a bounded amount of accuracy for a
//! smaller set of distinct values.
//!
//! Emitting values at full precision produces a large set of distinct values. Quantization
//! shrinks that set: nearby values collapse onto a shared representative, so a stream of
//! metrics contains fewer unique numbers and compresses better, at the cost of a precisely
//! bounded amount of accuracy.
//!
//! Quantization is always opt-in. Nothing in this module changes how a value is written
//! unless you explicitly ask for it.
//!
//! # How it works
//!
//! A [`Quantizer`] retains the leading `N` significant bits of a value and clears the rest.
//! Equivalently, it rounds the value to the nearest value representable with an `N`-bit
//! significand — quantizing to `N` bits is exactly the operation of storing the number as a
//! tiny floating-point value and reading it back.
//!
//! Consider `1000` (`0b11_1110_1000`) quantized to 4 significant bits. The value is 10 bits
//! wide, so the low 6 bits are discarded:
//!
//! ```text
//! 1000 = 0b1111101000
//!          ^^^^          the 4 bits that are kept
//!              ^^^^^^    the 6 bits that are discarded
//!
//! floor    -> 0b01111000000 =  960
//! midpoint -> 0b01111100000 =  992
//! ceil     -> 0b10000000000 = 1024
//! ```
//!
//! Because the discarded bits are always the *least* significant ones, the spacing between
//! adjacent representable values scales with the magnitude of the value. Small values are
//! quantized finely and large values coarsely: at 4 bits, `1000` moves by at most 64, while
//! `1_000_000` moves by at most 65536.
//!
//! # Bounded relative error
//!
//! The consequence of magnitude-scaled spacing is that error is bounded *relative* to the
//! value rather than in absolute terms. This is what makes the scheme suitable for
//! quantities that span many orders of magnitude, such as latencies and byte sizes, where a
//! fixed absolute error would be far too coarse for small values and pointlessly fine for
//! large ones.
//!
//! For `N` significant bits, the bound is:
//!
//! - [`Rounding::Floor`] and [`Rounding::Ceil`]: relative error is strictly less than
//!   `2^(1-N)`.
//! - [`Rounding::Midpoint`]: absolute relative error is at most `2^-N`.
//!
//! Every bound is a power of two, so the table below is exact rather than rounded:
//!
//! | significant bits | `Floor` / `Ceil` (`< 2^(1-N)`) | `Midpoint` (`<= 2^-N`) | exact below |
//! |---|---|---|---|
//! | 1 | 100% | 50% | 2 |
//! | 2 | 50% | 25% | 4 |
//! | 3 | 25% | 12.5% | 8 |
//! | 4 | 12.5% | 6.25% | 16 |
//! | 5 | 6.25% | 3.125% | 32 |
//! | 6 | 3.125% | 1.5625% | 64 |
//! | 7 | 1.5625% | 0.78125% | 128 |
//! | 8 (default) | 0.78125% | 0.390625% | 256 |
//! | 10 | 0.1953125% | 0.09765625% | 1024 |
//! | 11 | 0.09765625% | 0.048828125% | 2048 |
//! | 12 | 0.048828125% | 0.0244140625% | 4096 |
//! | 16 | 0.0030517578125% | 0.00152587890625% | 65536 |
//! | 24 | 0.0000119209289550781% | 0.0000059604644775391% | 16777216 |
//! | 32 | 0.0000000465661287308% | 0.0000000232830643654% | 4294967296 |
//! | 52 | 0.0000000000000444089% | 0.0000000000000222045% | 2^52 |
//!
//! [`SignificantBits::max_relative_error`] returns these values programmatically.
//!
//! ## Choosing a bit count
//!
//! The `exact below` column is the practical lever. Values narrower than `N` bits are
//! returned completely unchanged, so `N = 8` leaves every value below 256 exactly as it was.
//! That is good for correctness — small counters are never disturbed — but it also means
//! those values contribute nothing to the reduction in distinct values. Lowering `N` widens
//! the range of values that are actually affected.
//!
//! `4..=11` is the practical tuning band. Below 4, the error becomes large enough
//! (12.5% and up) that only order-of-magnitude questions can be answered. Above about 12,
//! the value set barely shrinks. Values outside that band are supported, but they are
//! deliberate extremes rather than defaults.
//!
//! The number of distinct values falls off steeply, because each binade is reduced to
//! `2^(N-1)` representatives regardless of how many distinct inputs landed in it. On a stream
//! of latencies and byte sizes spanning several orders of magnitude, dropping to 8 bits
//! typically collapses tens of thousands of distinct values into a few thousand; 4 bits takes
//! that into the low hundreds. Going below 4 continues to help, but each additional bit
//! removed roughly doubles the error while yielding progressively less, which is why the
//! recommended band bottoms out there.
//!
//! If you think in decimal significant digits, `N = ceil(log2(2 * 10^digits))` is the
//! smallest bit count whose relative error is below `10^-digits`:
//!
//! | decimal digits | significant bits |
//! |---|---|
//! | 1 | 5 |
//! | 2 | 8 |
//! | 3 | 11 |
//! | 4 | 15 |
//!
//! # The bucket lattice
//!
//! The set of representable values forms a lattice that is uniform within each binade — each
//! band between consecutive powers of two — and doubles in spacing at every binade boundary.
//! This makes the layout a piecewise-linear approximation of a logarithmic scale, a
//! structure usually called *log-linear quantization*.
//!
//! Each binade holds exactly `2^(N-1)` representable values, so `N` controls how finely each
//! power-of-two band is subdivided:
//!
//! ```text
//! N = 3 significant bits, so 4 representable values per binade
//!
//! binade [8, 16):    8   10   12   14        spacing 2
//! binade [16, 32):  16   20   24   28        spacing 4
//! binade [32, 64):  32   40   48   56        spacing 8
//! ```
//!
//! At `N = 1` each binade collapses to the single power of two that starts it, which is why
//! one significant bit means 100% worst-case error.
//!
//! # Relation to other schemes
//!
//! Schemes that hold relative error *exactly* constant place bucket boundaries at `γ^i` for
//! a fixed base `γ`. OpenTelemetry exponential histograms, Prometheus native histograms, and
//! DDSketch all work this way.
//!
//! The log-linear lattice used here instead *bounds* relative error. Error is largest at the
//! bottom of each binade, where the spacing has just doubled, and smallest at the top, just
//! before it doubles again. In exchange, quantizing is a shift and a mask rather than a
//! logarithm, which is what makes it cheap enough to run on every value at emission time.
//!
//! The same segment-and-mantissa structure — an exponent selecting a segment, a mantissa
//! interpolating linearly within it — has been in continuous use since µ-law and A-law
//! companding were standardized in ITU-T G.711 in the 1960s.
//!
//! # Applying it
//!
//! There are three ways to turn quantization on, in increasing order of blast radius.
//!
//! ## One field, declared in the struct
//!
//! The usual choice. The settings live next to the field they affect, so anyone reading the
//! struct can see which metrics are approximate and by how much.
//!
//! ```
//! use metrique::unit_of_work::metrics;
//!
//! #[metrics(rename_all = "PascalCase")]
//! struct RequestMetrics {
//!     // Within 0.390625% of the true value.
//!     #[metrics(quantize(bits = 8))]
//!     latency_nanos: u64,
//!
//!     // Within 6.25%, and never above the true value.
//!     #[metrics(quantize(bits = 4, rounding = floor))]
//!     payload_bytes: u64,
//!
//!     // Left exact: a count that has to reconcile.
//!     request_count: u64,
//! }
//! ```
//!
//! ## One value, configured at runtime
//!
//! When the bit count comes from configuration rather than being fixed at compile time, wrap
//! the value with [`MetricValue::quantized`](crate::value::MetricValue::quantized).
//!
//! ```
//! use metrique_writer_core::quantize::{Rounding, SignificantBits};
//! use metrique_writer_core::value::MetricValue;
//!
//! # fn configured_bits() -> u8 { 8 }
//! let bits = SignificantBits::new(configured_bits())?;
//! let latency = 1_234_567u64.quantized(bits, Rounding::Midpoint);
//! # Ok::<(), metrique_writer_core::quantize::SignificantBitsError>(())
//! ```
//!
//! ## A whole stream, without touching field declarations
//!
//! `metrique-writer` provides `with_quantization`, which applies a quantizer to every entry
//! passing through a stream. It takes a policy naming the metrics to quantize — there is
//! deliberately no "quantize everything" option, because a stream almost always carries some
//! values whose precision matters. See `metrique_writer::stream::QuantizationPolicy`.
//!
//! # Behaviour under aggregation
//!
//! Metric backends sum and average these values across many records, so the *direction* of the
//! error matters as much as its size.
//!
//! [`Rounding::Floor`] and [`Rounding::Ceil`] move every value the same way. That bias does not
//! cancel as more records arrive: an average computed from floor-quantized values is
//! persistently low, by roughly the mean distance from a value to the bottom of its bucket, no
//! matter how many samples contribute.
//!
//! [`Rounding::Midpoint`] moves values in both directions, so the errors largely cancel. The
//! cancellation is not perfect — values are not uniformly distributed inside a bucket, so the
//! mean of a bucket's contents does not sit exactly at its midpoint — but the residual bias is
//! far smaller than either one-sided mode's.
//!
//! Prefer [`Rounding::Midpoint`] (the default) for anything that feeds an average, a
//! percentile, or a sum. Reach for a one-sided mode when a single record's value has to carry a
//! guarantee on its own.
//!
//! # What not to quantize
//!
//! Quantization is lossy and must not be applied to values whose exact magnitude carries
//! meaning:
//!
//! - **Timestamps.** Quantizing a Unix epoch value moves it by hours or days.
//! - **Counts used for exact accounting**, such as billing quantities or request totals
//!   that must reconcile.
//! - **Identifiers and enumerations** that happen to be numeric.
//!
//! Small counters are a softer case: they are left exact below `exact_below()` anyway, so
//! quantizing them is usually harmless but also usually pointless.

use std::fmt;

/// The smallest number of significant bits a [`SignificantBits`] may hold.
pub const MIN_SIGNIFICANT_BITS: u8 = 1;

/// The largest number of significant bits a [`SignificantBits`] may hold.
///
/// The ceiling is 52 because metric values are carried as `f64` in several places and an
/// `f64` significand holds 53 bits. Asking to retain more than 52 significant bits would
/// silently do nothing to those values while still altering integer ones, which would make
/// the documented error bound untrue for some values but not others. Rejecting the request
/// keeps the bound uniform.
pub const MAX_SIGNIFICANT_BITS: u8 = 52;

/// The error returned when a significant bit count is outside `1..=52`.
///
/// See [`MAX_SIGNIFICANT_BITS`] for why the upper bound is 52.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SignificantBitsError {
    requested: u8,
}

impl SignificantBitsError {
    /// The out-of-range bit count that was requested.
    pub const fn requested(&self) -> u8 {
        self.requested
    }
}

impl fmt::Display for SignificantBitsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "significant bits must be in {MIN_SIGNIFICANT_BITS}..={MAX_SIGNIFICANT_BITS}, got {}",
            self.requested
        )
    }
}

impl std::error::Error for SignificantBitsError {}

/// The number of leading significant bits a [`Quantizer`] retains.
///
/// Valid values are `1..=52`; see [`MAX_SIGNIFICANT_BITS`] for the reason behind the ceiling.
/// Larger values keep more precision and shrink the set of distinct values less.
///
/// ```
/// use metrique_writer_core::quantize::{Rounding, SignificantBits};
///
/// let bits = SignificantBits::new(8).unwrap();
/// assert_eq!(bits.get(), 8);
///
/// // Values narrower than 8 bits are never modified.
/// assert_eq!(bits.exact_below(), 256);
///
/// // Exactly 2^-8 for midpoint rounding.
/// assert_eq!(bits.max_relative_error(Rounding::Midpoint), 0.00390625);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SignificantBits(u8);

impl Default for SignificantBits {
    fn default() -> Self {
        Self::DEFAULT
    }
}

impl SignificantBits {
    /// Eight significant bits: relative error below 0.78125%, or 0.390625% with
    /// [`Rounding::Midpoint`]. Values below 256 are left exact.
    ///
    /// This is a deliberately conservative default. It sits at the precise end of the
    /// practical `4..=11` band, so turning quantization on with the default is unlikely to
    /// disturb anything that was previously graphed, while still collapsing the long tail of
    /// large values.
    pub const DEFAULT: Self = Self(8);

    /// Retain `bits` leading significant bits.
    ///
    /// Returns [`SignificantBitsError`] if `bits` is outside `1..=52`. Out-of-range input is
    /// rejected rather than clamped, so a mistaken configuration surfaces immediately
    /// instead of silently emitting values with a different error bound than intended.
    ///
    /// ```
    /// use metrique_writer_core::quantize::SignificantBits;
    ///
    /// assert!(SignificantBits::new(8).is_ok());
    /// assert!(SignificantBits::new(0).is_err());
    /// assert!(SignificantBits::new(53).is_err());
    /// ```
    pub const fn new(bits: u8) -> Result<Self, SignificantBitsError> {
        if bits < MIN_SIGNIFICANT_BITS || bits > MAX_SIGNIFICANT_BITS {
            Err(SignificantBitsError { requested: bits })
        } else {
            Ok(Self(bits))
        }
    }

    /// The number of significant bits retained.
    pub const fn get(self) -> u8 {
        self.0
    }

    /// Values strictly below this threshold are returned unchanged by a [`Quantizer`].
    ///
    /// This is `2^bits`: any value that fits within the retained bits has nothing to
    /// discard.
    ///
    /// ```
    /// use metrique_writer_core::quantize::SignificantBits;
    ///
    /// assert_eq!(SignificantBits::new(1).unwrap().exact_below(), 2);
    /// assert_eq!(SignificantBits::new(8).unwrap().exact_below(), 256);
    /// ```
    pub const fn exact_below(self) -> u64 {
        1u64 << self.0
    }

    /// The largest relative error quantizing can introduce, as a fraction of the true value.
    ///
    /// For [`Rounding::Floor`] and [`Rounding::Ceil`] this is `2^(1-bits)` and the true error
    /// is strictly less than the returned value. For [`Rounding::Midpoint`] it is `2^-bits`
    /// and the true error can reach it exactly.
    ///
    /// ```
    /// use metrique_writer_core::quantize::{Rounding, SignificantBits};
    ///
    /// let bits = SignificantBits::new(4).unwrap();
    /// assert_eq!(bits.max_relative_error(Rounding::Floor), 0.125);
    /// assert_eq!(bits.max_relative_error(Rounding::Ceil), 0.125);
    /// assert_eq!(bits.max_relative_error(Rounding::Midpoint), 0.0625);
    /// ```
    pub fn max_relative_error(self, rounding: Rounding) -> f64 {
        let exponent = match rounding {
            Rounding::Floor | Rounding::Ceil => 1 - i32::from(self.0),
            Rounding::Midpoint => -i32::from(self.0),
        };
        f64::from(exponent).exp2()
    }
}

/// Which value within a bucket a [`Quantizer`] emits.
///
/// The choice determines what you can promise about the emitted value relative to the true
/// one. [`Midpoint`](Rounding::Midpoint) is the default because it halves the worst-case
/// error at no cost to the number of distinct values produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
#[non_exhaustive]
pub enum Rounding {
    /// Emit the bottom of the bucket. The emitted value is always less than or equal to the
    /// true value, and the relative error is strictly less than `2^(1-bits)`.
    ///
    /// Choose this when a value must never overstate the truth — for instance a figure
    /// feeding a headroom or capacity calculation that should err toward caution.
    ///
    /// Because every value in a bucket maps to the bucket's lower edge, the emitted numbers
    /// have trailing zero bits. Note that the error is one-directional: an average taken
    /// over many `Floor`-quantized values is biased low, and that bias does not cancel out
    /// with more samples. Prefer [`Midpoint`](Rounding::Midpoint) for values that are
    /// aggregated.
    Floor,

    /// Emit the top of the bucket. The emitted value is always greater than or equal to the
    /// true value, and the relative error is strictly less than `2^(1-bits)`.
    ///
    /// Choose this when a value must never understate the truth. As with
    /// [`Floor`](Rounding::Floor), the error is one-directional and biases aggregates —
    /// upward, in this case.
    Ceil,

    /// Emit the middle of the bucket. The absolute relative error is at most `2^-bits`, but
    /// the emitted value may be either above or below the true value.
    ///
    /// This is the default. Like [`Floor`](Rounding::Floor), it maps an entire bucket onto a
    /// single representative, so it produces exactly as many distinct values and exactly the
    /// same value-histogram entropy — while halving the worst-case error and largely
    /// cancelling the directional bias the one-sided modes introduce into aggregates. Its
    /// representatives sit mid-bucket rather than on a power-of-two-aligned edge, so the
    /// encoded form tends to be a percent or two larger than `Floor`'s after compression;
    /// the number of distinct values, which dominates, is identical.
    ///
    /// The tradeoff is that no one-sided promise holds: an emitted value may exceed the true
    /// value. If a consumer depends on the value never overstating, use
    /// [`Floor`](Rounding::Floor) explicitly.
    #[default]
    Midpoint,
}

impl Rounding {
    /// A short, stable name for this mode, suitable for logs and diagnostics.
    pub const fn as_str(self) -> &'static str {
        match self {
            Rounding::Floor => "floor",
            Rounding::Ceil => "ceil",
            Rounding::Midpoint => "midpoint",
        }
    }
}

impl fmt::Display for Rounding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Type-level [`Rounding`] markers, for specifying a mode in a type parameter.
///
/// These mirror the [`Rounding`] variants and exist so that quantization settings can be
/// carried in a type rather than a value, as [`Bits`] does.
pub mod rounding {
    use super::Rounding;

    /// A [`Rounding`] mode known at compile time.
    pub trait RoundingTag {
        /// The mode this tag denotes.
        const ROUNDING: Rounding;
    }

    /// Type-level [`Rounding::Floor`].
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    pub struct Floor;

    /// Type-level [`Rounding::Ceil`].
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    pub struct Ceil;

    /// Type-level [`Rounding::Midpoint`].
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    pub struct Midpoint;

    impl RoundingTag for Floor {
        const ROUNDING: Rounding = Rounding::Floor;
    }

    impl RoundingTag for Ceil {
        const ROUNDING: Rounding = Rounding::Ceil;
    }

    impl RoundingTag for Midpoint {
        const ROUNDING: Rounding = Rounding::Midpoint;
    }
}

use rounding::RoundingTag;

/// Quantization settings expressed as a type rather than a value.
///
/// `Bits<N>` denotes `N` significant bits with the default [`Rounding::Midpoint`];
/// `Bits<N, R>` denotes `N` bits with the mode named by the [`RoundingTag`] `R`. The bit
/// count is validated at compile time, so an out-of-range `N` fails to build rather than
/// failing at runtime.
///
/// ```
/// use metrique_writer_core::quantize::{Bits, Rounding, rounding};
///
/// assert_eq!(Bits::<8>::BITS.get(), 8);
/// assert_eq!(Bits::<8>::ROUNDING, Rounding::Midpoint);
/// assert_eq!(Bits::<11, rounding::Floor>::ROUNDING, Rounding::Floor);
/// ```
///
/// `Bits::<53>::BITS` does not compile, because 53 is above [`MAX_SIGNIFICANT_BITS`]:
///
/// ```compile_fail
/// use metrique_writer_core::quantize::Bits;
///
/// let _ = Bits::<53>::BITS;
/// ```
///
/// Neither does `Bits::<0>::BITS`:
///
/// ```compile_fail
/// use metrique_writer_core::quantize::Bits;
///
/// let _ = Bits::<0>::BITS;
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Bits<const N: u8, R: RoundingTag = rounding::Midpoint>(std::marker::PhantomData<R>);

impl<const N: u8, R: RoundingTag> Bits<N, R> {
    /// The significant bit count `N`, validated at compile time.
    pub const BITS: SignificantBits = match SignificantBits::new(N) {
        Ok(bits) => bits,
        Err(_) => panic!("significant bits must be in 1..=52"),
    };

    /// The rounding mode denoted by `R`.
    pub const ROUNDING: Rounding = R::ROUNDING;

    /// The [`Quantizer`] these settings describe.
    pub const QUANTIZER: Quantizer = Quantizer::new(Self::BITS, Self::ROUNDING);
}

/// Reduces values to a bounded number of significant bits.
///
/// A `Quantizer` pairs a [`SignificantBits`] count with a [`Rounding`] mode. Given a value, it
/// discards the bits below the leading `N` significant ones and returns the representative of
/// the resulting bucket selected by the rounding mode.
///
/// # Semantics
///
/// For a value `v` of bit width `w` retaining `b` bits, where `shift = w - b`,
/// `low = (v >> shift) << shift` is the bottom of `v`'s bucket, and `step = 1 << shift` is
/// the bucket's width:
///
/// | mode | result | guarantee |
/// |---|---|---|
/// | [`Rounding::Floor`] | `low` | `q(v) <= v`, error `< 2^(1-b) * v` |
/// | [`Rounding::Ceil`] | `low` if `low == v`, else `low + step` | `q(v) >= v`, error `< 2^(1-b) * v` |
/// | [`Rounding::Midpoint`] | `low + step / 2` | `abs(q(v) - v) <= 2^-b * v` |
///
/// When `w <= b` there is nothing to discard and `v` is returned unchanged. This is why
/// values below [`SignificantBits::exact_below`] always survive intact.
///
/// # Properties
///
/// - **Idempotent.** `q(q(v)) == q(v)` in every mode, so a value that has already been
///   quantized is not disturbed by quantizing it again with the same settings.
/// - **Monotonic.** `a <= b` implies `q(a) <= q(b)`.
/// - **Never panics.** Arithmetic saturates at [`u64::MAX`], which still satisfies the
///   [`Rounding::Ceil`] guarantee.
/// - **Zero is a fixed point** in every mode.
///
/// [`Rounding::Midpoint`] is applied unconditionally, including to values that already sit
/// exactly on a bucket floor. Returning such a value untouched would reduce its error to
/// zero, but it would also make the bucket produce two distinct outputs instead of one, which
/// works against the goal of shrinking the set of distinct values. [`Rounding::Ceil`], by
/// contrast, must leave them alone, both to keep its error tight and to stay idempotent.
///
/// # Examples
///
/// ```
/// use metrique_writer_core::quantize::{Quantizer, Rounding, SignificantBits};
///
/// let bits = SignificantBits::new(4).unwrap();
///
/// let floor = Quantizer::new(bits, Rounding::Floor);
/// let midpoint = Quantizer::new(bits, Rounding::Midpoint);
/// let ceil = Quantizer::new(bits, Rounding::Ceil);
///
/// // 1000 is 0b1111101000: keep the top 4 bits, discard the low 6.
/// assert_eq!(floor.quantize_u64(1000), 960);
/// assert_eq!(midpoint.quantize_u64(1000), 992);
/// assert_eq!(ceil.quantize_u64(1000), 1024);
///
/// // Small values are untouched in every mode.
/// assert_eq!(floor.quantize_u64(15), 15);
/// assert_eq!(midpoint.quantize_u64(15), 15);
/// assert_eq!(ceil.quantize_u64(15), 15);
/// ```
///
/// The error always stays within the documented bound:
///
/// ```
/// use metrique_writer_core::quantize::{Quantizer, Rounding, SignificantBits};
///
/// let bits = SignificantBits::new(8).unwrap();
/// let quantizer = Quantizer::new(bits, Rounding::Midpoint);
///
/// let value = 1_234_567u64;
/// let quantized = quantizer.quantize_u64(value);
///
/// let error = (quantized as f64 - value as f64).abs() / value as f64;
/// assert!(error <= bits.max_relative_error(Rounding::Midpoint));
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Quantizer {
    bits: SignificantBits,
    rounding: Rounding,
}

impl Default for Quantizer {
    /// [`SignificantBits::DEFAULT`] with [`Rounding::Midpoint`]: relative error at most
    /// 0.390625%, values below 256 untouched.
    fn default() -> Self {
        Self::new(SignificantBits::DEFAULT, Rounding::Midpoint)
    }
}

impl Quantizer {
    /// Build a quantizer retaining `bits` significant bits, using `rounding` to pick each
    /// bucket's representative.
    pub const fn new(bits: SignificantBits, rounding: Rounding) -> Self {
        Self { bits, rounding }
    }

    /// The number of significant bits retained.
    pub const fn significant_bits(self) -> SignificantBits {
        self.bits
    }

    /// The rounding mode used to pick a bucket's representative.
    pub const fn rounding(self) -> Rounding {
        self.rounding
    }

    /// The largest relative error this quantizer can introduce.
    ///
    /// Shorthand for [`SignificantBits::max_relative_error`] with this quantizer's mode.
    pub fn max_relative_error(self) -> f64 {
        self.bits.max_relative_error(self.rounding)
    }

    /// Quantize an unsigned integer.
    ///
    /// Values below [`SignificantBits::exact_below`] are returned unchanged. See the
    /// [type-level docs](Quantizer#semantics) for the exact rule applied to larger values.
    ///
    /// ```
    /// use metrique_writer_core::quantize::{Quantizer, Rounding, SignificantBits};
    ///
    /// let quantizer = Quantizer::new(SignificantBits::new(3).unwrap(), Rounding::Floor);
    ///
    /// // Within a binade the lattice is uniform: [8, 16) has spacing 2.
    /// assert_eq!(quantizer.quantize_u64(8), 8);
    /// assert_eq!(quantizer.quantize_u64(9), 8);
    /// assert_eq!(quantizer.quantize_u64(10), 10);
    /// assert_eq!(quantizer.quantize_u64(11), 10);
    ///
    /// // Crossing into [16, 32) doubles the spacing to 4.
    /// assert_eq!(quantizer.quantize_u64(16), 16);
    /// assert_eq!(quantizer.quantize_u64(19), 16);
    /// assert_eq!(quantizer.quantize_u64(20), 20);
    /// ```
    pub const fn quantize_u64(self, value: u64) -> u64 {
        let bits = self.bits.get() as u32;
        // Bit width of `value`; 0 when `value` is 0.
        let width = u64::BITS - value.leading_zeros();

        if width <= bits {
            // Nothing to discard: the value is already representable exactly.
            return value;
        }

        // `width > bits` guarantees `shift >= 1`, so `shift - 1` below cannot underflow.
        let shift = width - bits;
        let low = (value >> shift) << shift;

        match self.rounding {
            Rounding::Floor => low,
            Rounding::Ceil => {
                if low == value {
                    // Already on the lattice. Returning `value` rather than the next point up
                    // keeps the error at zero and keeps the operation idempotent.
                    value
                } else {
                    // Saturates instead of wrapping near `u64::MAX`. `u64::MAX >= value`
                    // holds, so the `Ceil` guarantee survives saturation.
                    low.saturating_add(1u64 << shift)
                }
            }
            // `low + step / 2` cannot exceed `u64::MAX` (the largest possible `low` is
            // `2^64 - step`, leaving room for `step / 2`), but saturate anyway so that this
            // can never panic on a metrics path.
            Rounding::Midpoint => low.saturating_add(1u64 << (shift - 1)),
        }
    }

    /// Quantize a floating point value.
    ///
    /// This is the same operation as [`quantize_u64`](Quantizer::quantize_u64), applied to the
    /// significand instead of to an integer: the low `53 - bits` significand bits are cleared,
    /// and the rounding mode selects a representative from the resulting bucket.
    ///
    /// # Values that are passed through unchanged
    ///
    /// - Zero, in either sign.
    /// - Infinities and `NaN`, which have no significand to reduce.
    /// - Subnormals, whose magnitude is below [`f64::MIN_POSITIVE`]. A subnormal already
    ///   carries fewer than 53 significant bits, so masking could discard it entirely and
    ///   report a 100% error. Passing them through keeps the documented bound true of every
    ///   value this function actually modifies. Subnormals are smaller than `1e-307`, so no
    ///   realistic metric value is affected.
    ///
    /// # Negative values
    ///
    /// The mode's promise is about the value, not its magnitude, so the two one-sided modes
    /// swap when the input is negative: making a negative number's magnitude larger makes the
    /// number smaller. [`Rounding::Floor`] on `-1000.0` therefore returns `-1024.0`, which is
    /// still less than or equal to the input.
    ///
    /// Note that most metric formats reject negative observations regardless.
    ///
    /// # Relationship to [`quantize_u64`](Quantizer::quantize_u64)
    ///
    /// For a magnitude at or above [`SignificantBits::exact_below`], the two functions agree
    /// exactly: both see the same binade and the same bucket width.
    ///
    /// Below that threshold they differ, and deliberately so. An integer smaller than the
    /// retained bit width has no bits to discard, so `quantize_u64` returns it untouched. A
    /// float has all 53 significand bits available at every magnitude, so there is no
    /// equivalent regime — the lattice keeps subdividing, and `quantize_f64(3.0)` at 2
    /// significant bits lands on `3.5` rather than staying at `3.0`. Both results are inside
    /// the documented error bound.
    ///
    /// # Examples
    ///
    /// ```
    /// use metrique_writer_core::quantize::{Quantizer, Rounding, SignificantBits};
    ///
    /// let bits = SignificantBits::new(4).unwrap();
    ///
    /// // The same buckets as the integer path, for values above `exact_below`.
    /// assert_eq!(Quantizer::new(bits, Rounding::Floor).quantize_f64(1000.0), 960.0);
    /// assert_eq!(Quantizer::new(bits, Rounding::Midpoint).quantize_f64(1000.0), 992.0);
    /// assert_eq!(Quantizer::new(bits, Rounding::Ceil).quantize_f64(1000.0), 1024.0);
    ///
    /// // Fractional values work the same way, one binade at a time.
    /// assert_eq!(Quantizer::new(bits, Rounding::Floor).quantize_f64(0.1), 0.09375);
    ///
    /// // Special values are left alone.
    /// let quantizer = Quantizer::new(bits, Rounding::Midpoint);
    /// assert!(quantizer.quantize_f64(f64::NAN).is_nan());
    /// assert_eq!(quantizer.quantize_f64(f64::INFINITY), f64::INFINITY);
    /// assert_eq!(quantizer.quantize_f64(0.0), 0.0);
    /// ```
    ///
    /// One-sided modes keep their promise across the sign change:
    ///
    /// ```
    /// use metrique_writer_core::quantize::{Quantizer, Rounding, SignificantBits};
    ///
    /// let bits = SignificantBits::new(4).unwrap();
    ///
    /// assert_eq!(Quantizer::new(bits, Rounding::Floor).quantize_f64(-1000.0), -1024.0);
    /// assert_eq!(Quantizer::new(bits, Rounding::Ceil).quantize_f64(-1000.0), -960.0);
    /// ```
    pub fn quantize_f64(self, value: f64) -> f64 {
        // `NaN` and the infinities have no significand to reduce; zero has nothing to discard.
        if !value.is_finite() || value == 0.0 {
            return value;
        }

        let magnitude = value.abs();

        // Subnormals hold fewer than 53 significant bits, so the mask below could erase them
        // outright and blow past the documented bound. See the doc comment.
        if magnitude < f64::MIN_POSITIVE {
            return value;
        }

        let bits = u32::from(self.bits.get());
        // Retain the implicit leading one plus `bits - 1` explicit significand bits.
        // `bits <= 52` guarantees `drop >= 1`, so `drop - 1` below cannot underflow, and
        // `drop <= 52` guarantees the mask never reaches the exponent field.
        let drop = 53 - bits;
        let mask = u64::MAX << drop;

        let raw = magnitude.to_bits();
        let low_bits = raw & mask;
        let already_on_lattice = low_bits == raw;

        // The promise is about the value, not the magnitude. For a negative input, growing the
        // magnitude shrinks the value, so the one-sided modes exchange roles.
        let rounding = if value.is_sign_negative() {
            match self.rounding {
                Rounding::Floor => Rounding::Ceil,
                Rounding::Ceil => Rounding::Floor,
                Rounding::Midpoint => Rounding::Midpoint,
            }
        } else {
            self.rounding
        };

        // Incrementing the raw bit pattern by `1 << drop` advances exactly one lattice step.
        // IEEE-754 orders positive finite values monotonically by bit pattern, and a carry out
        // of the significand lands on the first value of the next binade, which is where the
        // step doubles.
        let quantized = match rounding {
            Rounding::Floor => low_bits,
            Rounding::Ceil => {
                if already_on_lattice {
                    raw
                } else {
                    low_bits + (1u64 << drop)
                }
            }
            Rounding::Midpoint => low_bits + (1u64 << (drop - 1)),
        };

        let quantized = f64::from_bits(quantized);

        // `Ceil` inside the largest binade can carry into the exponent encoding for infinity.
        // Clamping to the largest finite value still satisfies `q(v) >= v` and keeps the result
        // usable by formats that reject infinities.
        let quantized = if quantized.is_finite() {
            quantized
        } else {
            f64::MAX
        };

        if value.is_sign_negative() {
            -quantized
        } else {
            quantized
        }
    }
}

/// Supplies the [`Quantizer`] that a wrapper such as
/// [`Quantized`](crate::value::Quantized) should apply.
///
/// Two kinds of implementation exist. [`Quantizer`] itself carries its settings as data, which
/// is what you want when the bit count comes from configuration. [`Bits`] carries them in the
/// type, which lets the settings be named in a struct field's type and validated at compile
/// time.
///
/// ```
/// use metrique_writer_core::quantize::{Bits, Quantizer, QuantizerSource, Rounding, SignificantBits, rounding};
///
/// // Settings as data.
/// let runtime = Quantizer::new(SignificantBits::new(8).unwrap(), Rounding::Floor);
/// assert_eq!(runtime.quantizer(), runtime);
///
/// // The same settings as a type. `Bits` is zero-sized, so it costs nothing to carry.
/// let type_level = Bits::<8, rounding::Floor>::default();
/// assert_eq!(type_level.quantizer(), runtime);
/// assert_eq!(std::mem::size_of_val(&type_level), 0);
/// ```
pub trait QuantizerSource {
    /// The quantizer to apply.
    fn quantizer(&self) -> Quantizer;
}

impl QuantizerSource for Quantizer {
    fn quantizer(&self) -> Quantizer {
        *self
    }
}

impl<const N: u8, R: RoundingTag> QuantizerSource for Bits<N, R> {
    fn quantizer(&self) -> Quantizer {
        Self::QUANTIZER
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A deliberately naive quantizer, written to be obviously correct rather than fast, used
    /// to check the shift-and-mask implementation.
    ///
    /// This computes the bit width with a division loop and the bucket with `u128`
    /// division and multiplication, so it shares no arithmetic with the real implementation.
    /// Note that computing the width as `log2(value).floor() + 1` would be a trap: `u64`
    /// values above `2^53` are not exactly representable as `f64`, so the conversion can
    /// round across a power-of-two boundary and yield a width that is off by one.
    fn reference_quantize_u64(value: u64, bits: u8, rounding: Rounding) -> u64 {
        if value == 0 {
            return 0;
        }

        let mut width = 0u32;
        let mut remaining = value;
        while remaining > 0 {
            width += 1;
            remaining /= 2;
        }

        if width <= u32::from(bits) {
            return value;
        }

        let shift = width - u32::from(bits);
        let step = 2u128.pow(shift);
        let value = u128::from(value);
        let low = (value / step) * step;

        let result = match rounding {
            Rounding::Floor => low,
            Rounding::Ceil => {
                if low == value {
                    low
                } else {
                    low + step
                }
            }
            Rounding::Midpoint => low + step / 2,
        };

        if result > u128::from(u64::MAX) {
            u64::MAX
        } else {
            result as u64
        }
    }

    /// Deterministic xorshift64*, so a failure is always reproducible.
    struct Rng(u64);

    impl Rng {
        fn new(seed: u64) -> Self {
            Self(seed)
        }

        fn next_u64(&mut self) -> u64 {
            let mut x = self.0;
            x ^= x >> 12;
            x ^= x << 25;
            x ^= x >> 27;
            self.0 = x;
            x.wrapping_mul(0x2545_F491_4F6C_DD1D)
        }
    }

    const ALL_MODES: [Rounding; 3] = [Rounding::Floor, Rounding::Ceil, Rounding::Midpoint];

    /// Values chosen to stress binade boundaries, the extremes, and dense small numbers.
    fn interesting_values() -> Vec<u64> {
        let mut values = vec![0, u64::MAX, u64::MAX - 1];

        // Dense small values, which should mostly be left exact.
        values.extend(0..512u64);

        // Every power of two and its immediate neighbours.
        for exponent in 0..64u32 {
            let power = 1u64 << exponent;
            values.push(power);
            values.push(power.saturating_sub(1));
            values.push(power.saturating_add(1));
        }

        // Values just below the top of each binade, where `Ceil` must carry into the next.
        for exponent in 1..64u32 {
            values.push((1u64 << exponent) - 1);
        }

        values.sort_unstable();
        values.dedup();
        values
    }

    fn quantizer(bits: u8, rounding: Rounding) -> Quantizer {
        Quantizer::new(SignificantBits::new(bits).unwrap(), rounding)
    }

    #[test]
    fn matches_reference_on_interesting_values() {
        for bits in MIN_SIGNIFICANT_BITS..=MAX_SIGNIFICANT_BITS {
            for rounding in ALL_MODES {
                let quantizer = quantizer(bits, rounding);
                for value in interesting_values() {
                    assert_eq!(
                        quantizer.quantize_u64(value),
                        reference_quantize_u64(value, bits, rounding),
                        "bits={bits} rounding={rounding} value={value}"
                    );
                }
            }
        }
    }

    #[test]
    fn matches_reference_on_random_values() {
        let mut rng = Rng::new(0x5EED_1234_ABCD_0001);
        for _ in 0..20_000 {
            let value = rng.next_u64();
            // Also exercise smaller magnitudes, which a uniform u64 rarely produces.
            let shifted = value >> (rng.next_u64() % 64);

            for bits in MIN_SIGNIFICANT_BITS..=MAX_SIGNIFICANT_BITS {
                for rounding in ALL_MODES {
                    let quantizer = quantizer(bits, rounding);
                    for candidate in [value, shifted] {
                        assert_eq!(
                            quantizer.quantize_u64(candidate),
                            reference_quantize_u64(candidate, bits, rounding),
                            "bits={bits} rounding={rounding} value={candidate}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn floor_never_overstates_and_ceil_never_understates() {
        for bits in MIN_SIGNIFICANT_BITS..=MAX_SIGNIFICANT_BITS {
            for value in interesting_values() {
                assert!(
                    quantizer(bits, Rounding::Floor).quantize_u64(value) <= value,
                    "floor overstated: bits={bits} value={value}"
                );
                assert!(
                    quantizer(bits, Rounding::Ceil).quantize_u64(value) >= value,
                    "ceil understated: bits={bits} value={value}"
                );
            }
        }
    }

    #[test]
    fn relative_error_stays_within_documented_bound() {
        for bits in MIN_SIGNIFICANT_BITS..=MAX_SIGNIFICANT_BITS {
            let significant = SignificantBits::new(bits).unwrap();
            for rounding in ALL_MODES {
                let quantizer = Quantizer::new(significant, rounding);
                let bound = significant.max_relative_error(rounding);

                for value in interesting_values() {
                    if value == 0 {
                        continue;
                    }
                    // u64::MAX saturation is the one case where `Ceil` cannot reach the next
                    // lattice point; it clamps downward, which stays within the bound anyway.
                    let quantized = quantizer.quantize_u64(value);
                    let error = (quantized as f64 - value as f64).abs() / value as f64;

                    match rounding {
                        // The one-sided bound is strict, but allow equality for the
                        // saturating edge case at u64::MAX.
                        Rounding::Floor | Rounding::Ceil => assert!(
                            error <= bound,
                            "bits={bits} rounding={rounding} value={value} error={error} bound={bound}"
                        ),
                        Rounding::Midpoint => assert!(
                            error <= bound,
                            "bits={bits} rounding={rounding} value={value} error={error} bound={bound}"
                        ),
                    }
                }
            }
        }
    }

    #[test]
    fn all_modes_are_idempotent() {
        let mut rng = Rng::new(0xD1CE_0000_0000_0007);
        let mut values = interesting_values();
        for _ in 0..5_000 {
            values.push(rng.next_u64() >> (rng.next_u64() % 64));
        }

        for bits in MIN_SIGNIFICANT_BITS..=MAX_SIGNIFICANT_BITS {
            for rounding in ALL_MODES {
                let quantizer = quantizer(bits, rounding);
                for &value in &values {
                    let once = quantizer.quantize_u64(value);
                    let twice = quantizer.quantize_u64(once);
                    assert_eq!(
                        once, twice,
                        "not idempotent: bits={bits} rounding={rounding} value={value}"
                    );
                }
            }
        }
    }

    #[test]
    fn values_below_exact_below_are_untouched() {
        for bits in MIN_SIGNIFICANT_BITS..=MAX_SIGNIFICANT_BITS {
            let significant = SignificantBits::new(bits).unwrap();
            let threshold = significant.exact_below();

            for rounding in ALL_MODES {
                let quantizer = Quantizer::new(significant, rounding);
                // Check the whole range for small bit counts, a sample for large ones.
                let limit = threshold.min(4096);
                for value in 0..limit {
                    assert_eq!(
                        quantizer.quantize_u64(value),
                        value,
                        "bits={bits} rounding={rounding} value={value}"
                    );
                }
                // And the largest exact value, just below the threshold.
                assert_eq!(quantizer.quantize_u64(threshold - 1), threshold - 1);
            }
        }
    }

    #[test]
    fn ceil_saturates_instead_of_overflowing() {
        for bits in MIN_SIGNIFICANT_BITS..=MAX_SIGNIFICANT_BITS {
            let quantizer = quantizer(bits, Rounding::Ceil);
            // u64::MAX has every bit set, so it is never on the lattice for bits < 64 and
            // the naive `low + step` would overflow.
            assert_eq!(quantizer.quantize_u64(u64::MAX), u64::MAX);
            assert_eq!(quantizer.quantize_u64(u64::MAX - 1), u64::MAX);
        }
    }

    #[test]
    fn zero_is_a_fixed_point() {
        for bits in MIN_SIGNIFICANT_BITS..=MAX_SIGNIFICANT_BITS {
            for rounding in ALL_MODES {
                assert_eq!(quantizer(bits, rounding).quantize_u64(0), 0);
            }
        }
    }

    #[test]
    fn quantizing_is_monotonic() {
        let mut rng = Rng::new(0xA5A5_0000_1111_2222);
        for bits in MIN_SIGNIFICANT_BITS..=MAX_SIGNIFICANT_BITS {
            for rounding in ALL_MODES {
                let quantizer = quantizer(bits, rounding);
                for _ in 0..2_000 {
                    let a = rng.next_u64() >> (rng.next_u64() % 64);
                    let b = rng.next_u64() >> (rng.next_u64() % 64);
                    let (low, high) = if a <= b { (a, b) } else { (b, a) };
                    assert!(
                        quantizer.quantize_u64(low) <= quantizer.quantize_u64(high),
                        "not monotonic: bits={bits} rounding={rounding} {low} vs {high}"
                    );
                }
            }
        }
    }

    #[test]
    fn floor_and_midpoint_produce_one_representative_per_bucket() {
        // The claim behind Midpoint being the default at no cost: Floor and Midpoint map a
        // whole bucket to a single value, so they yield the same number of distinct outputs.
        use std::collections::BTreeSet;

        for bits in [1u8, 2, 3, 4, 8] {
            let mut floor_outputs = BTreeSet::new();
            let mut midpoint_outputs = BTreeSet::new();
            let mut ceil_outputs = BTreeSet::new();

            for value in 0..100_000u64 {
                floor_outputs.insert(quantizer(bits, Rounding::Floor).quantize_u64(value));
                midpoint_outputs.insert(quantizer(bits, Rounding::Midpoint).quantize_u64(value));
                ceil_outputs.insert(quantizer(bits, Rounding::Ceil).quantize_u64(value));
            }

            assert_eq!(
                floor_outputs.len(),
                midpoint_outputs.len(),
                "floor and midpoint should have equal alphabet size at {bits} bits"
            );
            // Ceil keeps lattice-exact values as themselves *and* rounds others up, so it
            // produces strictly more distinct values.
            assert!(
                ceil_outputs.len() >= floor_outputs.len(),
                "ceil alphabet should not be smaller at {bits} bits"
            );
        }
    }

    #[test]
    fn quantizer_accessors_round_trip() {
        let bits = SignificantBits::new(11).unwrap();
        let quantizer = Quantizer::new(bits, Rounding::Ceil);
        assert_eq!(quantizer.significant_bits(), bits);
        assert_eq!(quantizer.rounding(), Rounding::Ceil);
        assert_eq!(
            quantizer.max_relative_error(),
            bits.max_relative_error(Rounding::Ceil)
        );
    }

    #[test]
    fn quantizer_default_is_eight_bit_midpoint() {
        let quantizer = Quantizer::default();
        assert_eq!(quantizer.significant_bits(), SignificantBits::DEFAULT);
        assert_eq!(quantizer.rounding(), Rounding::Midpoint);
    }

    #[test]
    fn bits_tag_produces_matching_quantizer() {
        assert_eq!(
            Bits::<8>::QUANTIZER,
            Quantizer::new(SignificantBits::new(8).unwrap(), Rounding::Midpoint)
        );
        assert_eq!(
            Bits::<11, rounding::Floor>::QUANTIZER,
            Quantizer::new(SignificantBits::new(11).unwrap(), Rounding::Floor)
        );
    }

    #[test]
    fn worked_example_from_module_docs() {
        // 1000 = 0b1111101000 at 4 significant bits.
        let bits = SignificantBits::new(4).unwrap();
        assert_eq!(
            Quantizer::new(bits, Rounding::Floor).quantize_u64(1000),
            960
        );
        assert_eq!(
            Quantizer::new(bits, Rounding::Midpoint).quantize_u64(1000),
            992
        );
        assert_eq!(
            Quantizer::new(bits, Rounding::Ceil).quantize_u64(1000),
            1024
        );
    }

    #[test]
    fn binade_spacing_doubles() {
        // The lattice diagram in the module docs, at 3 significant bits.
        let quantizer = quantizer(3, Rounding::Floor);
        let representatives = |range: std::ops::Range<u64>| {
            range
                .map(|v| quantizer.quantize_u64(v))
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>()
        };

        assert_eq!(representatives(8..16), vec![8, 10, 12, 14]);
        assert_eq!(representatives(16..32), vec![16, 20, 24, 28]);
        assert_eq!(representatives(32..64), vec![32, 40, 48, 56]);
    }

    #[test]
    fn each_binade_holds_two_to_the_bits_minus_one_values() {
        // The module docs claim `2^(N-1)` representable values per binade.
        for bits in 1u8..=8 {
            let quantizer = quantizer(bits, Rounding::Floor);
            let expected = 1usize << (bits - 1);

            // Check several binades well above `exact_below`, where quantizing actually bites.
            for exponent in u32::from(bits)..u32::from(bits) + 6 {
                let start = 1u64 << exponent;
                let end = start * 2;
                let distinct = (start..end)
                    .map(|v| quantizer.quantize_u64(v))
                    .collect::<std::collections::BTreeSet<_>>()
                    .len();
                assert_eq!(
                    distinct, expected,
                    "bits={bits} binade=[{start}, {end}) should hold {expected} values"
                );
            }
        }
    }

    #[test]
    fn one_bit_collapses_each_binade_to_its_power_of_two() {
        // The docs' explanation for why 1 bit means 100% worst-case error.
        let quantizer = quantizer(1, Rounding::Floor);
        for exponent in 1..20u32 {
            let start = 1u64 << exponent;
            for value in start..start * 2 {
                assert_eq!(quantizer.quantize_u64(value), start, "value={value}");
            }
        }
    }

    // ---------------------------------------------------------------------------------
    // quantize_f64
    // ---------------------------------------------------------------------------------

    /// Independent f64 reference: derive the bucket from the exponent with `powi`, rather than
    /// by masking the significand, so it shares no arithmetic with the implementation.
    ///
    /// # Precondition
    ///
    /// Only valid for magnitudes within `REFERENCE_EXPONENT_RANGE`. Outside it, `2f64.powi()`
    /// is not a usable way to build the lattice step: `powi` computes a negative exponent by
    /// reciprocating the positive one, so `2f64.powi(-1024)` evaluates `1.0 / inf` and
    /// underflows to zero, and a step of zero turns the division below into `NaN`. The
    /// extremes are covered instead by the bound, idempotence, saturation, and subnormal
    /// tests, and by `f64_agrees_with_u64_at_or_above_exact_below`, which checks the f64 path
    /// against the independently verified integer path.
    fn reference_quantize_f64(value: f64, bits: u8, rounding: Rounding) -> f64 {
        if !value.is_finite() || value == 0.0 {
            return value;
        }
        let magnitude = value.abs();
        if magnitude < f64::MIN_POSITIVE {
            return value;
        }

        let rounding = if value.is_sign_negative() {
            match rounding {
                Rounding::Floor => Rounding::Ceil,
                Rounding::Ceil => Rounding::Floor,
                Rounding::Midpoint => Rounding::Midpoint,
            }
        } else {
            rounding
        };

        // The binade of `magnitude`, and the lattice step inside it.
        //
        // `log2().floor()` is not trustworthy on its own: `f64::MAX.log2()` rounds to exactly
        // 1024.0, which would put the value one binade too high and make `step` infinite.
        // Correct the estimate by comparison instead. The `exponent < 1024` guard keeps the
        // second loop from evaluating `powi(1025)` forever once `powi` saturates to infinity.
        let mut exponent = magnitude.log2().floor() as i32;
        while 2f64.powi(exponent) > magnitude {
            exponent -= 1;
        }
        while exponent < 1024 && 2f64.powi(exponent + 1) <= magnitude {
            exponent += 1;
        }
        let step = 2f64.powi(exponent - i32::from(bits) + 1);

        let low = (magnitude / step).floor() * step;
        let quantized = match rounding {
            Rounding::Floor => low,
            Rounding::Ceil => {
                if low == magnitude {
                    magnitude
                } else {
                    low + step
                }
            }
            Rounding::Midpoint => low + step / 2.0,
        };

        let quantized = if quantized.is_finite() {
            quantized
        } else {
            f64::MAX
        };
        if value.is_sign_negative() {
            -quantized
        } else {
            quantized
        }
    }

    /// The magnitude range over which `reference_quantize_f64` is trustworthy. Chosen so that
    /// `2f64.powi(exponent - bits + 1)` stays comfortably inside the normal range for every
    /// supported bit count: the smallest step it ever builds is `2^(-500 - 52 + 1)`.
    const REFERENCE_EXPONENT_RANGE: std::ops::RangeInclusive<i32> = -500..=500;

    fn reference_is_valid_for(value: f64) -> bool {
        if !value.is_finite() || value == 0.0 {
            return false;
        }
        let magnitude = value.abs();
        if magnitude < f64::MIN_POSITIVE {
            return false;
        }
        let exponent = magnitude.log2().floor() as i32;
        REFERENCE_EXPONENT_RANGE.contains(&exponent)
    }

    fn interesting_floats() -> Vec<f64> {
        let mut values = vec![
            0.1,
            0.5,
            1.0,
            1.5,
            2.0,
            3.0,
            100.0,
            1000.0,
            1e6,
            1e15,
            1e100,
            1e-100,
            f64::MIN_POSITIVE,
            f64::MAX,
            std::f64::consts::PI,
            std::f64::consts::E,
        ];

        // Powers of two and their neighbours across a wide exponent range.
        for exponent in -60..60i32 {
            let power = 2f64.powi(exponent);
            values.push(power);
            // Just below the top of the binade, where `Ceil` must carry into the next.
            values.push(power * 1.9999999999999998);
            values.push(power * 1.5);
        }

        // Integral values, to compare against the integer path.
        for v in 1..2048u64 {
            values.push(v as f64);
        }

        values.retain(|v| v.is_finite() && *v != 0.0);
        values
    }

    #[test]
    fn f64_matches_reference() {
        for bits in MIN_SIGNIFICANT_BITS..=MAX_SIGNIFICANT_BITS {
            for rounding in ALL_MODES {
                let quantizer = quantizer(bits, rounding);
                for value in interesting_floats() {
                    for signed in [value, -value] {
                        if !reference_is_valid_for(signed) {
                            continue;
                        }
                        let actual = quantizer.quantize_f64(signed);
                        let expected = reference_quantize_f64(signed, bits, rounding);
                        assert_eq!(
                            actual.to_bits(),
                            expected.to_bits(),
                            "bits={bits} rounding={rounding} value={signed:e}: {actual:e} != {expected:e}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn f64_matches_reference_on_random_values() {
        let mut rng = Rng::new(0xBEEF_0F64_0000_0001);
        let mut checked = 0u32;

        while checked < 20_000 {
            let candidate = f64::from_bits(rng.next_u64());
            if !reference_is_valid_for(candidate) {
                continue;
            }
            checked += 1;

            for bits in MIN_SIGNIFICANT_BITS..=MAX_SIGNIFICANT_BITS {
                for rounding in ALL_MODES {
                    let quantizer = quantizer(bits, rounding);
                    let actual = quantizer.quantize_f64(candidate);
                    let expected = reference_quantize_f64(candidate, bits, rounding);
                    assert_eq!(
                        actual.to_bits(),
                        expected.to_bits(),
                        "bits={bits} rounding={rounding} value={candidate:e}"
                    );
                }
            }
        }
    }

    #[test]
    fn f64_agrees_with_u64_at_or_above_exact_below() {
        // At or above `exact_below` both paths see the same binade and the same bucket width,
        // so they must produce identical results.
        for bits in MIN_SIGNIFICANT_BITS..=MAX_SIGNIFICANT_BITS {
            let significant = SignificantBits::new(bits).unwrap();
            let threshold = significant.exact_below();

            for rounding in ALL_MODES {
                let quantizer = Quantizer::new(significant, rounding);

                let mut candidates: Vec<u64> = (threshold..threshold.saturating_mul(4))
                    .take(4096)
                    .collect();
                // A few large integral values that are still exact in f64.
                for exponent in 20..53u32 {
                    candidates.push(1u64 << exponent);
                    candidates.push((1u64 << exponent) + 1);
                    candidates.push((1u64 << exponent) - 1);
                }
                candidates.retain(|v| *v < (1u64 << 53) && *v >= threshold);

                for value in candidates {
                    let via_u64 = quantizer.quantize_u64(value);
                    let via_f64 = quantizer.quantize_f64(value as f64);
                    assert_eq!(
                        via_u64 as f64, via_f64,
                        "bits={bits} rounding={rounding} value={value}"
                    );
                }
            }
        }
    }

    #[test]
    fn f64_passes_through_special_values() {
        for bits in MIN_SIGNIFICANT_BITS..=MAX_SIGNIFICANT_BITS {
            for rounding in ALL_MODES {
                let quantizer = quantizer(bits, rounding);

                assert!(quantizer.quantize_f64(f64::NAN).is_nan());
                assert_eq!(quantizer.quantize_f64(f64::INFINITY), f64::INFINITY);
                assert_eq!(quantizer.quantize_f64(f64::NEG_INFINITY), f64::NEG_INFINITY);

                // Both signed zeros survive, sign included.
                assert_eq!(quantizer.quantize_f64(0.0).to_bits(), 0.0f64.to_bits());
                assert_eq!(quantizer.quantize_f64(-0.0).to_bits(), (-0.0f64).to_bits());
            }
        }
    }

    #[test]
    fn f64_passes_through_subnormals() {
        let subnormals = [
            f64::from_bits(1),
            f64::from_bits(2),
            f64::from_bits(0x000F_FFFF_FFFF_FFFF),
            -f64::from_bits(1),
        ];

        for bits in MIN_SIGNIFICANT_BITS..=MAX_SIGNIFICANT_BITS {
            for rounding in ALL_MODES {
                let quantizer = quantizer(bits, rounding);
                for value in subnormals {
                    assert_eq!(
                        quantizer.quantize_f64(value).to_bits(),
                        value.to_bits(),
                        "bits={bits} rounding={rounding} subnormal={value:e}"
                    );
                }
            }
        }
    }

    #[test]
    fn f64_one_sided_modes_bracket_the_true_value() {
        for bits in MIN_SIGNIFICANT_BITS..=MAX_SIGNIFICANT_BITS {
            for value in interesting_floats() {
                for signed in [value, -value] {
                    let floor = quantizer(bits, Rounding::Floor).quantize_f64(signed);
                    let ceil = quantizer(bits, Rounding::Ceil).quantize_f64(signed);

                    assert!(
                        floor <= signed,
                        "floor overstated: bits={bits} value={signed:e} -> {floor:e}"
                    );
                    // `f64::MAX` saturation is the one case where `Ceil` cannot reach the next
                    // lattice point.
                    if ceil != f64::MAX && ceil != -f64::MAX {
                        assert!(
                            ceil >= signed,
                            "ceil understated: bits={bits} value={signed:e} -> {ceil:e}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn f64_relative_error_stays_within_documented_bound() {
        for bits in MIN_SIGNIFICANT_BITS..=MAX_SIGNIFICANT_BITS {
            let significant = SignificantBits::new(bits).unwrap();
            for rounding in ALL_MODES {
                let quantizer = Quantizer::new(significant, rounding);
                let bound = significant.max_relative_error(rounding);

                for value in interesting_floats() {
                    for signed in [value, -value] {
                        let quantized = quantizer.quantize_f64(signed);
                        if !quantized.is_finite() {
                            continue;
                        }
                        let error = (quantized - signed).abs() / signed.abs();
                        assert!(
                            error <= bound * (1.0 + 1e-12),
                            "bits={bits} rounding={rounding} value={signed:e} error={error:e} bound={bound:e}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn f64_is_idempotent() {
        let mut rng = Rng::new(0xF10A_7000_0000_0001);
        let mut values = interesting_floats();
        for _ in 0..5_000 {
            // Random finite floats spanning the whole exponent range.
            let candidate = f64::from_bits(rng.next_u64());
            if candidate.is_finite() && candidate != 0.0 {
                values.push(candidate);
            }
        }

        for bits in MIN_SIGNIFICANT_BITS..=MAX_SIGNIFICANT_BITS {
            for rounding in ALL_MODES {
                let quantizer = quantizer(bits, rounding);
                for &value in &values {
                    let once = quantizer.quantize_f64(value);
                    let twice = quantizer.quantize_f64(once);
                    assert_eq!(
                        once.to_bits(),
                        twice.to_bits(),
                        "not idempotent: bits={bits} rounding={rounding} value={value:e}"
                    );
                }
            }
        }
    }

    #[test]
    fn f64_is_monotonic() {
        let mut rng = Rng::new(0x0FEE_1111_2222_3333);
        for bits in MIN_SIGNIFICANT_BITS..=MAX_SIGNIFICANT_BITS {
            for rounding in ALL_MODES {
                let quantizer = quantizer(bits, rounding);
                for _ in 0..2_000 {
                    let a = f64::from_bits(rng.next_u64());
                    let b = f64::from_bits(rng.next_u64());
                    if !a.is_finite() || !b.is_finite() {
                        continue;
                    }
                    let (low, high) = if a <= b { (a, b) } else { (b, a) };
                    assert!(
                        quantizer.quantize_f64(low) <= quantizer.quantize_f64(high),
                        "not monotonic: bits={bits} rounding={rounding} {low:e} vs {high:e}"
                    );
                }
            }
        }
    }

    #[test]
    fn f64_ceil_saturates_at_max_instead_of_going_infinite() {
        for bits in MIN_SIGNIFICANT_BITS..=MAX_SIGNIFICANT_BITS {
            let quantizer = quantizer(bits, Rounding::Ceil);
            // f64::MAX has every significand bit set, so it is never on the lattice and the
            // naive `low + step` would carry into the infinity encoding.
            let quantized = quantizer.quantize_f64(f64::MAX);
            assert!(quantized.is_finite(), "bits={bits} produced {quantized:e}");
            assert_eq!(quantized, f64::MAX);
        }
    }

    #[test]
    fn f64_binade_crossing_is_exact() {
        // Just below a power of two, `Ceil` must land exactly on it.
        let quantizer = quantizer(4, Rounding::Ceil);
        for exponent in -20..20i32 {
            let power = 2f64.powi(exponent);
            let just_below = power * 1.9999999999999998;
            let quantized = quantizer.quantize_f64(just_below);
            assert_eq!(
                quantized,
                power * 2.0,
                "exponent={exponent} value={just_below:e}"
            );
        }
    }

    #[test]
    fn f64_one_bit_collapses_to_powers_of_two() {
        let quantizer = quantizer(1, Rounding::Floor);
        for exponent in -30..30i32 {
            let power = 2f64.powi(exponent);
            assert_eq!(quantizer.quantize_f64(power), power);
            assert_eq!(quantizer.quantize_f64(power * 1.5), power);
            assert_eq!(quantizer.quantize_f64(power * 1.99), power);
        }
    }

    #[test]
    fn f64_worked_examples_from_docs() {
        let bits = SignificantBits::new(4).unwrap();
        assert_eq!(
            Quantizer::new(bits, Rounding::Floor).quantize_f64(1000.0),
            960.0
        );
        assert_eq!(
            Quantizer::new(bits, Rounding::Midpoint).quantize_f64(1000.0),
            992.0
        );
        assert_eq!(
            Quantizer::new(bits, Rounding::Ceil).quantize_f64(1000.0),
            1024.0
        );
        assert_eq!(
            Quantizer::new(bits, Rounding::Floor).quantize_f64(0.1),
            0.09375
        );
        // Sign flips the one-sided modes.
        assert_eq!(
            Quantizer::new(bits, Rounding::Floor).quantize_f64(-1000.0),
            -1024.0
        );
        assert_eq!(
            Quantizer::new(bits, Rounding::Ceil).quantize_f64(-1000.0),
            -960.0
        );
    }

    #[test]
    fn rejects_out_of_range_bits() {
        assert_eq!(SignificantBits::new(0).unwrap_err().requested(), 0);
        assert_eq!(SignificantBits::new(53).unwrap_err().requested(), 53);
        assert_eq!(SignificantBits::new(255).unwrap_err().requested(), 255);
    }

    #[test]
    fn accepts_full_valid_range() {
        for bits in MIN_SIGNIFICANT_BITS..=MAX_SIGNIFICANT_BITS {
            let parsed = SignificantBits::new(bits).expect("in range");
            assert_eq!(parsed.get(), bits);
        }
    }

    #[test]
    fn default_is_eight_bits() {
        assert_eq!(SignificantBits::DEFAULT.get(), 8);
        assert_eq!(SignificantBits::default(), SignificantBits::DEFAULT);
    }

    #[test]
    fn default_rounding_is_midpoint() {
        assert_eq!(Rounding::default(), Rounding::Midpoint);
    }

    #[test]
    fn exact_below_is_two_to_the_bits() {
        for bits in MIN_SIGNIFICANT_BITS..=MAX_SIGNIFICANT_BITS {
            let parsed = SignificantBits::new(bits).unwrap();
            assert_eq!(parsed.exact_below(), 1u64 << bits, "bits={bits}");
        }
    }

    #[test]
    fn max_relative_error_matches_powers_of_two() {
        for bits in MIN_SIGNIFICANT_BITS..=MAX_SIGNIFICANT_BITS {
            let parsed = SignificantBits::new(bits).unwrap();
            let one_sided = f64::from(1 - i32::from(bits)).exp2();
            let midpoint = f64::from(-i32::from(bits)).exp2();

            assert_eq!(parsed.max_relative_error(Rounding::Floor), one_sided);
            assert_eq!(parsed.max_relative_error(Rounding::Ceil), one_sided);
            assert_eq!(parsed.max_relative_error(Rounding::Midpoint), midpoint);
            // The midpoint bound is exactly half the one-sided bound.
            assert_eq!(midpoint * 2.0, one_sided);
        }
    }

    #[test]
    fn documented_error_table_is_exact() {
        // Every entry in the module-level table, verified against the documented value.
        let cases: &[(u8, f64, f64)] = &[
            (1, 1.0, 0.5),
            (2, 0.5, 0.25),
            (3, 0.25, 0.125),
            (4, 0.125, 0.0625),
            (5, 0.0625, 0.03125),
            (6, 0.03125, 0.015625),
            (7, 0.015625, 0.0078125),
            (8, 0.0078125, 0.00390625),
            (10, 0.001953125, 0.0009765625),
            (11, 0.0009765625, 0.00048828125),
            (12, 0.00048828125, 0.000244140625),
            (16, 0.000030517578125, 0.0000152587890625),
        ];

        for &(bits, one_sided, midpoint) in cases {
            let parsed = SignificantBits::new(bits).unwrap();
            assert_eq!(
                parsed.max_relative_error(Rounding::Floor),
                one_sided,
                "one-sided bound for {bits} bits"
            );
            assert_eq!(
                parsed.max_relative_error(Rounding::Midpoint),
                midpoint,
                "midpoint bound for {bits} bits"
            );
        }
    }

    #[test]
    fn decimal_digit_equivalence_table_is_correct() {
        // `ceil(log2(2 * 10^digits))` is the smallest bit count whose one-sided relative
        // error is below 10^-digits, as the module docs claim.
        for (digits, expected_bits) in [(1u32, 5u8), (2, 8), (3, 11), (4, 15)] {
            let derived = (2.0 * 10f64.powi(digits as i32)).log2().ceil() as u8;
            assert_eq!(derived, expected_bits, "digits={digits}");

            let bits = SignificantBits::new(expected_bits).unwrap();
            let target = 10f64.powi(-(digits as i32));
            assert!(
                bits.max_relative_error(Rounding::Floor) < target,
                "{expected_bits} bits should beat 10^-{digits}"
            );
            // One bit fewer should not be sufficient, confirming minimality.
            let weaker = SignificantBits::new(expected_bits - 1).unwrap();
            assert!(
                weaker.max_relative_error(Rounding::Floor) >= target,
                "{} bits should not beat 10^-{digits}",
                expected_bits - 1
            );
        }
    }

    #[test]
    fn bits_tag_carries_settings() {
        assert_eq!(Bits::<8>::BITS.get(), 8);
        assert_eq!(Bits::<8>::ROUNDING, Rounding::Midpoint);
        assert_eq!(Bits::<1, rounding::Floor>::BITS.get(), 1);
        assert_eq!(Bits::<1, rounding::Floor>::ROUNDING, Rounding::Floor);
        assert_eq!(Bits::<52, rounding::Ceil>::BITS.get(), 52);
        assert_eq!(Bits::<52, rounding::Ceil>::ROUNDING, Rounding::Ceil);
    }

    #[test]
    fn rounding_display_is_stable() {
        assert_eq!(Rounding::Floor.to_string(), "floor");
        assert_eq!(Rounding::Ceil.to_string(), "ceil");
        assert_eq!(Rounding::Midpoint.to_string(), "midpoint");
    }

    #[test]
    fn error_message_names_the_valid_range() {
        let message = SignificantBits::new(53).unwrap_err().to_string();
        assert_eq!(message, "significant bits must be in 1..=52, got 53");
    }
}
