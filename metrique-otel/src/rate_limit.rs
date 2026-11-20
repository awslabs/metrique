// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Cross-thread rate limiting for noisy log output: [`RateLimiter`] for a limit owned by a value
//! (such as one sink), and `rate_limited!` for a limit shared by every caller of one call site.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Whole seconds since the Unix epoch. If the system clock is set back, calls stay limited
/// until it catches up; that is acceptable for log output.
fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Allows one call per interval, shared by every thread that holds a reference to it.
///
/// `next_call_secs` only holds the next allowed time and guards no other data, so `Relaxed` is
/// sufficient: the compare-exchange lets one caller through per interval.
pub(crate) struct RateLimiter {
    next_call_secs: AtomicU64,
    interval_secs: u64,
}

impl RateLimiter {
    pub(crate) const fn new(interval: Duration) -> Self {
        assert!(
            interval.as_secs() >= 1,
            "only second-level granularity supported for rate limiting"
        );
        Self {
            next_call_secs: AtomicU64::new(0),
            interval_secs: interval.as_secs(),
        }
    }

    /// Returns `true` for the first call in each interval and `false` for the rest.
    pub(crate) fn try_acquire(&self) -> bool {
        self.try_acquire_at(now_secs())
    }

    /// [`try_acquire`](Self::try_acquire) at `now_secs`, in whole seconds since the Unix epoch.
    fn try_acquire_at(&self, now_secs: u64) -> bool {
        let next = self.next_call_secs.load(Ordering::Relaxed);
        // Limited calls return here without the compare-exchange, which would otherwise take
        // the cache line exclusively on every call.
        if next > now_secs {
            return false;
        }
        self.next_call_secs
            .compare_exchange(
                next,
                now_secs.saturating_add(self.interval_secs),
                Ordering::Relaxed,
                Ordering::Relaxed,
            )
            .is_ok()
    }
}

macro_rules! rate_limited {
    ($interval:expr, $call:expr) => {{
        static LIMITER: $crate::rate_limit::RateLimiter =
            $crate::rate_limit::RateLimiter::new($interval);
        if LIMITER.try_acquire() {
            $call;
        }
    }};
}
pub(crate) use rate_limited;

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    const MINUTE: Duration = Duration::from_secs(60);

    /// Each case is a sequence of `(seconds, allowed)` calls against one fresh one-minute
    /// limiter.
    #[rstest]
    #[case::first_call_allowed(&[(0, true)])]
    #[case::rest_of_window_rejected(&[(0, true), (59, false)])]
    #[case::next_window_allowed(&[(0, true), (60, true), (60, false)])]
    #[case::late_first_call(&[(1_000, true), (1_059, false), (1_060, true)])]
    fn allows_one_call_per_interval(#[case] calls: &[(u64, bool)]) {
        let limiter = RateLimiter::new(MINUTE);
        for &(secs, allowed) in calls {
            assert_eq!(limiter.try_acquire_at(secs), allowed, "call at {secs}s");
        }
    }

    #[test]
    fn saturates_at_u64_max() {
        let limiter = RateLimiter::new(MINUTE);
        assert!(limiter.try_acquire_at(u64::MAX - 1));
        assert!(!limiter.try_acquire_at(u64::MAX - 1));
    }

    #[test]
    fn limiters_are_independent() {
        let first = RateLimiter::new(MINUTE);
        let second = RateLimiter::new(MINUTE);
        assert!(first.try_acquire());
        assert!(second.try_acquire());
    }

    #[test]
    fn macro_call_site_fires_once_per_interval() {
        let mut fired = 0;
        for _ in 0..3 {
            rate_limited!(MINUTE, fired += 1);
        }
        assert_eq!(fired, 1);
    }

    #[test]
    fn macro_call_sites_are_limited_separately() {
        let (mut first, mut second) = (0, 0);
        for _ in 0..3 {
            rate_limited!(MINUTE, first += 1);
            rate_limited!(MINUTE, second += 1);
        }
        assert_eq!((first, second), (1, 1));
    }
}
