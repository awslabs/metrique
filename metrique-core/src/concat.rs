//! Utilities for concatenating strings
//!
//! This exists because const generics is insufficiently useful. It should
//! be greatly simplified once const generics is better.

use std::{borrow::Cow, marker::PhantomData};

use self::private::SealedMaybeConstStr;

/// A trait representing a constant string lifted to a constant
pub trait ConstStr {
    /// The constant string value
    const VAL: &'static str;
}

/// An empty constant string
pub struct EmptyConstStr;

impl ConstStr for EmptyConstStr {
    const VAL: &'static str = "";
}

impl<T: ConstStr> MaybeConstStr for T {
    const MAYBE_VAL: &'static str = Self::VAL;
    const LEN: usize = const { Self::VAL.len() };
    const HAVE_VAL: bool = true;
    fn extend(into: &mut String) {
        into.push_str(Self::VAL);
    }
}

impl<T: ConstStr> SealedMaybeConstStr for T {}

/// A trait representing a string that is possibly a constant
///
/// The associated constants are implementation details and
/// not to be used.
// The `extend` function will extend the content string (with a known length)
// into an input buffer.
//
// If `HAVE_VAL` is true, then `MAYBE_VAL` contains the same value as the result of
// extracting this by running:
// ```
// let mut buf = String::with_capacity(Self::LEN);
// Self::extend(&mut buf);
// &buf[..]
// ```
//
// If `HAVE_VAL` is false, then `MAYBE_VAL` contains garbage.
//
// Ideally, const generics would be better and we would not need this hack.
pub trait MaybeConstStr: SealedMaybeConstStr {
    #[doc(hidden)]
    const MAYBE_VAL: &'static str;
    #[doc(hidden)]
    const LEN: usize = 0;
    #[doc(hidden)]
    const HAVE_VAL: bool;
    /// Extend the value into a given string
    fn extend(into: &mut String);
}

mod private {
    pub trait SealedMaybeConstStr {}
}

/// The concatenation of 2 constant strings
pub struct Concatenated<S, T>(S, T);

const fn concatenate_strings<const N: usize>(x: &[u8], y: &[u8]) -> [u8; N] {
    let mut buf = [0; N];
    if N != x.len() + y.len() {
        return buf;
    }
    let mut i = 0;
    while i < x.len() {
        buf[i] = x[i];
        i += 1;
    }
    let mut i = 0;
    while i < y.len() {
        buf[x.len() + i] = y[i];
        i += 1;
    }
    buf
}

struct ConcatenatedLen<X: MaybeConstStr, Y: MaybeConstStr, const N: usize>(
    X,
    Y,
    PhantomData<[u8; N]>,
);
impl<S: MaybeConstStr, T: MaybeConstStr, const N: usize> MaybeConstStr
    for ConcatenatedLen<S, T, N>
{
    const MAYBE_VAL: &str = const {
        let buf =
            const { &concatenate_strings::<N>(S::MAYBE_VAL.as_bytes(), T::MAYBE_VAL.as_bytes()) };
        match std::str::from_utf8(buf) {
            Ok(res) => res,
            Err(_) => panic!(),
        }
    };
    const HAVE_VAL: bool = S::HAVE_VAL && T::HAVE_VAL;
    const LEN: usize = S::LEN + T::LEN;
    fn extend(into: &mut String) {
        S::extend(into);
        T::extend(into);
    }
}
impl<S: MaybeConstStr, T: MaybeConstStr, const N: usize> SealedMaybeConstStr
    for ConcatenatedLen<S, T, N>
{
}

impl<S: MaybeConstStr, T: MaybeConstStr> MaybeConstStr for Concatenated<S, T> {
    // For strings over length 100, `HAVE_VAL = false` so allocate. It is possible
    // to change the 100 for some other value, you need to change the length of
    // the match initializing MAYBE_VAL.
    const HAVE_VAL: bool = S::HAVE_VAL && T::HAVE_VAL && (S::LEN + T::LEN) <= 100;
    const LEN: usize = S::LEN + T::LEN;
    fn extend(into: &mut String) {
        S::extend(into);
        T::extend(into);
    }
    // Hack since const generics are less ideal than we want.
    const MAYBE_VAL: &str = match S::MAYBE_VAL.len() + T::MAYBE_VAL.len() {
        0 => ConcatenatedLen::<S, T, 0>::MAYBE_VAL,
        1 => ConcatenatedLen::<S, T, 1>::MAYBE_VAL,
        2 => ConcatenatedLen::<S, T, 2>::MAYBE_VAL,
        3 => ConcatenatedLen::<S, T, 3>::MAYBE_VAL,
        4 => ConcatenatedLen::<S, T, 4>::MAYBE_VAL,
        5 => ConcatenatedLen::<S, T, 5>::MAYBE_VAL,
        6 => ConcatenatedLen::<S, T, 6>::MAYBE_VAL,
        7 => ConcatenatedLen::<S, T, 7>::MAYBE_VAL,
        8 => ConcatenatedLen::<S, T, 8>::MAYBE_VAL,
        9 => ConcatenatedLen::<S, T, 9>::MAYBE_VAL,
        10 => ConcatenatedLen::<S, T, 10>::MAYBE_VAL,
        11 => ConcatenatedLen::<S, T, 11>::MAYBE_VAL,
        12 => ConcatenatedLen::<S, T, 12>::MAYBE_VAL,
        13 => ConcatenatedLen::<S, T, 13>::MAYBE_VAL,
        14 => ConcatenatedLen::<S, T, 14>::MAYBE_VAL,
        15 => ConcatenatedLen::<S, T, 15>::MAYBE_VAL,
        16 => ConcatenatedLen::<S, T, 16>::MAYBE_VAL,
        17 => ConcatenatedLen::<S, T, 17>::MAYBE_VAL,
        18 => ConcatenatedLen::<S, T, 18>::MAYBE_VAL,
        19 => ConcatenatedLen::<S, T, 19>::MAYBE_VAL,
        20 => ConcatenatedLen::<S, T, 20>::MAYBE_VAL,
        21 => ConcatenatedLen::<S, T, 21>::MAYBE_VAL,
        22 => ConcatenatedLen::<S, T, 22>::MAYBE_VAL,
        23 => ConcatenatedLen::<S, T, 23>::MAYBE_VAL,
        24 => ConcatenatedLen::<S, T, 24>::MAYBE_VAL,
        25 => ConcatenatedLen::<S, T, 25>::MAYBE_VAL,
        26 => ConcatenatedLen::<S, T, 26>::MAYBE_VAL,
        27 => ConcatenatedLen::<S, T, 27>::MAYBE_VAL,
        28 => ConcatenatedLen::<S, T, 28>::MAYBE_VAL,
        29 => ConcatenatedLen::<S, T, 29>::MAYBE_VAL,
        30 => ConcatenatedLen::<S, T, 30>::MAYBE_VAL,
        31 => ConcatenatedLen::<S, T, 31>::MAYBE_VAL,
        32 => ConcatenatedLen::<S, T, 32>::MAYBE_VAL,
        33 => ConcatenatedLen::<S, T, 33>::MAYBE_VAL,
        34 => ConcatenatedLen::<S, T, 34>::MAYBE_VAL,
        35 => ConcatenatedLen::<S, T, 35>::MAYBE_VAL,
        36 => ConcatenatedLen::<S, T, 36>::MAYBE_VAL,
        37 => ConcatenatedLen::<S, T, 37>::MAYBE_VAL,
        38 => ConcatenatedLen::<S, T, 38>::MAYBE_VAL,
        39 => ConcatenatedLen::<S, T, 39>::MAYBE_VAL,
        40 => ConcatenatedLen::<S, T, 40>::MAYBE_VAL,
        41 => ConcatenatedLen::<S, T, 41>::MAYBE_VAL,
        42 => ConcatenatedLen::<S, T, 42>::MAYBE_VAL,
        43 => ConcatenatedLen::<S, T, 43>::MAYBE_VAL,
        44 => ConcatenatedLen::<S, T, 44>::MAYBE_VAL,
        45 => ConcatenatedLen::<S, T, 45>::MAYBE_VAL,
        46 => ConcatenatedLen::<S, T, 46>::MAYBE_VAL,
        47 => ConcatenatedLen::<S, T, 47>::MAYBE_VAL,
        48 => ConcatenatedLen::<S, T, 48>::MAYBE_VAL,
        49 => ConcatenatedLen::<S, T, 49>::MAYBE_VAL,
        50 => ConcatenatedLen::<S, T, 50>::MAYBE_VAL,
        51 => ConcatenatedLen::<S, T, 51>::MAYBE_VAL,
        52 => ConcatenatedLen::<S, T, 52>::MAYBE_VAL,
        53 => ConcatenatedLen::<S, T, 53>::MAYBE_VAL,
        54 => ConcatenatedLen::<S, T, 54>::MAYBE_VAL,
        55 => ConcatenatedLen::<S, T, 55>::MAYBE_VAL,
        56 => ConcatenatedLen::<S, T, 56>::MAYBE_VAL,
        57 => ConcatenatedLen::<S, T, 57>::MAYBE_VAL,
        58 => ConcatenatedLen::<S, T, 58>::MAYBE_VAL,
        59 => ConcatenatedLen::<S, T, 59>::MAYBE_VAL,
        60 => ConcatenatedLen::<S, T, 60>::MAYBE_VAL,
        61 => ConcatenatedLen::<S, T, 61>::MAYBE_VAL,
        62 => ConcatenatedLen::<S, T, 62>::MAYBE_VAL,
        63 => ConcatenatedLen::<S, T, 63>::MAYBE_VAL,
        64 => ConcatenatedLen::<S, T, 64>::MAYBE_VAL,
        65 => ConcatenatedLen::<S, T, 65>::MAYBE_VAL,
        66 => ConcatenatedLen::<S, T, 66>::MAYBE_VAL,
        67 => ConcatenatedLen::<S, T, 67>::MAYBE_VAL,
        68 => ConcatenatedLen::<S, T, 68>::MAYBE_VAL,
        69 => ConcatenatedLen::<S, T, 69>::MAYBE_VAL,
        70 => ConcatenatedLen::<S, T, 70>::MAYBE_VAL,
        71 => ConcatenatedLen::<S, T, 71>::MAYBE_VAL,
        72 => ConcatenatedLen::<S, T, 72>::MAYBE_VAL,
        73 => ConcatenatedLen::<S, T, 73>::MAYBE_VAL,
        74 => ConcatenatedLen::<S, T, 74>::MAYBE_VAL,
        75 => ConcatenatedLen::<S, T, 75>::MAYBE_VAL,
        76 => ConcatenatedLen::<S, T, 76>::MAYBE_VAL,
        77 => ConcatenatedLen::<S, T, 77>::MAYBE_VAL,
        78 => ConcatenatedLen::<S, T, 78>::MAYBE_VAL,
        79 => ConcatenatedLen::<S, T, 79>::MAYBE_VAL,
        80 => ConcatenatedLen::<S, T, 80>::MAYBE_VAL,
        81 => ConcatenatedLen::<S, T, 81>::MAYBE_VAL,
        82 => ConcatenatedLen::<S, T, 82>::MAYBE_VAL,
        83 => ConcatenatedLen::<S, T, 83>::MAYBE_VAL,
        84 => ConcatenatedLen::<S, T, 84>::MAYBE_VAL,
        85 => ConcatenatedLen::<S, T, 85>::MAYBE_VAL,
        86 => ConcatenatedLen::<S, T, 86>::MAYBE_VAL,
        87 => ConcatenatedLen::<S, T, 87>::MAYBE_VAL,
        88 => ConcatenatedLen::<S, T, 88>::MAYBE_VAL,
        89 => ConcatenatedLen::<S, T, 89>::MAYBE_VAL,
        90 => ConcatenatedLen::<S, T, 90>::MAYBE_VAL,
        91 => ConcatenatedLen::<S, T, 91>::MAYBE_VAL,
        92 => ConcatenatedLen::<S, T, 92>::MAYBE_VAL,
        93 => ConcatenatedLen::<S, T, 93>::MAYBE_VAL,
        94 => ConcatenatedLen::<S, T, 94>::MAYBE_VAL,
        95 => ConcatenatedLen::<S, T, 95>::MAYBE_VAL,
        96 => ConcatenatedLen::<S, T, 96>::MAYBE_VAL,
        97 => ConcatenatedLen::<S, T, 97>::MAYBE_VAL,
        98 => ConcatenatedLen::<S, T, 98>::MAYBE_VAL,
        99 => ConcatenatedLen::<S, T, 99>::MAYBE_VAL,
        100 => ConcatenatedLen::<S, T, 100>::MAYBE_VAL,
        _ => "",
    };
}
impl<S: MaybeConstStr, T: MaybeConstStr> SealedMaybeConstStr for Concatenated<S, T> {}

/// Return the value of a given [MaybeConstStr]. If possible, will return
/// the value without allocating. It might not be always possible due
/// to const eval limitations.
pub fn const_str_value<S: MaybeConstStr>() -> Cow<'static, str> {
    if S::HAVE_VAL {
        Cow::Borrowed(S::MAYBE_VAL)
    } else {
        let mut buf = String::with_capacity(S::LEN);
        S::extend(&mut buf);
        Cow::Owned(buf)
    }
}

#[cfg(test)]
mod test {
    use std::borrow::Cow;

    use crate::concat::{Concatenated, ConstStr, const_str_value};

    struct ConstFoo;
    impl ConstStr for ConstFoo {
        const VAL: &str = "Foo_";
    }

    struct ConstBar;
    impl ConstStr for ConstBar {
        const VAL: &str = "Bar";
    }

    struct ConstMinus;
    impl ConstStr for ConstMinus {
        const VAL: &str = "-";
    }

    type MinusConcated<U, V> = Concatenated<U, Concatenated<ConstMinus, V>>;
    type VL1 = MinusConcated<ConstFoo, ConstBar>;
    type VL2 = MinusConcated<VL1, VL1>;
    type VL3 = MinusConcated<VL2, VL2>;
    type VL4 = MinusConcated<VL3, VL3>;
    type VL5 = MinusConcated<VL4, VL4>;
    type VL6 = MinusConcated<VL5, VL5>;

    #[test]
    fn main() {
        assert_eq!(
            const_str_value::<Concatenated<ConstFoo, ConstBar>>(),
            "Foo_Bar"
        );
        let vl6 = const_str_value::<VL6>();
        match vl6 {
            Cow::Owned(a) => assert_eq!(
                a,
                "Foo_-Bar-Foo_-Bar-Foo_-Bar-Foo_-Bar-Foo_-Bar-Foo_-Bar-Foo_-Bar-Foo_-Bar-Foo_-Bar-Foo_-Bar-Foo_-Bar-Foo_-Bar-Foo_-Bar-Foo_-Bar-Foo_-Bar-Foo_-Bar-Foo_-Bar-Foo_-Bar-Foo_-Bar-Foo_-Bar-Foo_-Bar-Foo_-Bar-Foo_-Bar-Foo_-Bar-Foo_-Bar-Foo_-Bar-Foo_-Bar-Foo_-Bar-Foo_-Bar-Foo_-Bar-Foo_-Bar-Foo_-Bar"
            ),
            _ => panic!(),
        };
        let vl5 = const_str_value::<VL5>();
        match vl5 {
            Cow::Owned(a) => assert_eq!(
                a,
                "Foo_-Bar-Foo_-Bar-Foo_-Bar-Foo_-Bar-Foo_-Bar-Foo_-Bar-Foo_-Bar-Foo_-Bar-Foo_-Bar-Foo_-Bar-Foo_-Bar-Foo_-Bar-Foo_-Bar-Foo_-Bar-Foo_-Bar-Foo_-Bar"
            ),
            _ => panic!(),
        };
        let vl4 = const_str_value::<VL4>();
        match vl4 {
            Cow::Borrowed(a) => assert_eq!(
                a,
                "Foo_-Bar-Foo_-Bar-Foo_-Bar-Foo_-Bar-Foo_-Bar-Foo_-Bar-Foo_-Bar-Foo_-Bar"
            ),
            _ => panic!(),
        };
    }
}
