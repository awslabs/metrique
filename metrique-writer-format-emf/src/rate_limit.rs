// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::{
    sync::OnceLock,
    time::{Duration, Instant},
};

// only pub(crate) so that the macro calls can all use the same epoch static
#[doc(hidden)]
pub(crate) fn time_since_arbitrary_epoch() -> Duration {
    static EPOCH: OnceLock<Instant> = OnceLock::new();
    Instant::now().duration_since(*EPOCH.get_or_init(Instant::now))
}

/// `rate_limited!(duration, expr)` will cause `expr` to only be evaluated at most once every `duration` across all
/// threads.
///
/// Designed to be used to rate limit background logs for error conditions. If we didn't rate limit, we can end up
/// flooding the application logs with repeated error messages. Instead, we want to log the first occurrence and then
/// at regular intervals to indicate the problem persists.
///
/// Note that the rate limiting applies to each unique code location of the macro call, not to all code sites using it.
macro_rules! rate_limited {
    ($interval:expr, $call:expr) => {{
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT_CALL: AtomicU64 = AtomicU64::new(u64::MIN);
        let interval = $interval;
        assert!(
            interval >= Duration::from_secs(1),
            "only second-level granularity supported for rate limiting"
        );

        let time = $crate::rate_limit::time_since_arbitrary_epoch();
        let next = NEXT_CALL.load(Ordering::Relaxed);
        if next <= time.as_secs() {
            let new_next = time
                .checked_add(interval)
                .unwrap_or(Duration::MAX)
                .as_secs();
            if NEXT_CALL
                .compare_exchange(next, new_next, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
            {
                $call;
            }
        }
    }};
}
pub(crate) use rate_limited;

#[cfg(test)]
mod test {
    use std::{cell::Cell, time::Duration};

    #[test]
    fn calls_at_least_once() {
        let counter = Cell::new(0u64);
        let incr = || rate_limited!(Duration::MAX, counter.set(counter.get() + 1));
        incr();
        assert_eq!(counter.get(), 1);
        for _ in 0..1000 {
            incr();
        }
        assert_eq!(counter.get(), 1);
    }
}
