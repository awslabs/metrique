// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Cross-thread rate limiting for noisy log call sites.

use std::sync::OnceLock;
use std::time::{Duration, Instant};

#[doc(hidden)]
pub fn time_since_arbitrary_epoch() -> Duration {
    static EPOCH: OnceLock<Instant> = OnceLock::new();
    Instant::now().duration_since(*EPOCH.get_or_init(Instant::now))
}

macro_rules! rate_limited {
    ($interval:expr, $call:expr) => {{
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT_CALL: AtomicU64 = AtomicU64::new(u64::MIN);
        let interval: std::time::Duration = $interval;
        assert!(
            interval >= std::time::Duration::from_secs(1),
            "only second-level granularity supported for rate limiting"
        );

        let time = $crate::rate_limit::time_since_arbitrary_epoch();
        let next = NEXT_CALL.load(Ordering::Relaxed);
        if next <= time.as_secs() {
            let new_next = time
                .checked_add(interval)
                .unwrap_or(std::time::Duration::MAX)
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
