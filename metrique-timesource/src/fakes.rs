// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime},
};

use crate::Time;

/// Simple static timesource that will always return the same time
#[derive(Debug)]
pub struct StaticTimeSource {
    now: SystemTime,
    now_instant: Instant,
}

impl StaticTimeSource {
    /// Create a new StaticTimeSource that always returns the given time
    ///
    /// # Arguments
    ///
    /// * `time` - The SystemTime that this source will always return
    ///
    /// # Returns
    ///
    /// A new StaticTimeSource initialized with the given time
    ///
    /// # Examples
    ///
    /// ```
    /// use metrique_timesource::{TimeSource, fakes::StaticTimeSource};
    /// use std::time::UNIX_EPOCH;
    ///
    /// let static_time = StaticTimeSource::at_time(UNIX_EPOCH);
    /// let ts = TimeSource::custom(static_time);
    /// assert_eq!(ts.system_time(), UNIX_EPOCH);
    /// ```
    pub fn at_time(time: impl Into<SystemTime>) -> Self {
        Self {
            now: time.into(),
            now_instant: Instant::now(),
        }
    }
}

impl Time for StaticTimeSource {
    fn now(&self) -> SystemTime {
        self.now
    }

    fn instant(&self) -> Instant {
        self.now_instant
    }
}

/// Dummy timesource that is loaded with one time,
/// but you can clone it and further modify the time and elapsed Instant duration
/// via a shared handle
#[derive(Debug, Clone)]
pub struct ManuallyAdvancedTimeSource(Arc<Mutex<StaticTimeSource>>);

impl ManuallyAdvancedTimeSource {
    /// Create a new ManuallyAdvancedTimeSource that is started with the given time.
    ///
    /// You can subsequently call [`Self::update_time`] to modify the loaded time.
    ///
    /// # Arguments
    ///
    /// * `time` - The SystemTime that this source will initially return
    ///
    /// # Returns
    ///
    /// A new ManuallyAdvancedTimeSource initialized with the given time
    ///
    /// # Examples
    ///
    /// ```
    /// use metrique_timesource::{TimeSource, fakes::ManuallyAdvancedTimeSource};
    /// use std::time::UNIX_EPOCH;
    ///
    /// let dummy_time = ManuallyAdvancedTimeSource::at_time(UNIX_EPOCH);
    /// let ts = TimeSource::custom(dummy_time);
    /// assert_eq!(ts.system_time(), UNIX_EPOCH);
    /// ```
    pub fn at_time(time: impl Into<SystemTime>) -> Self {
        let ts = StaticTimeSource::at_time(time);
        Self(Arc::from(Mutex::from(ts)))
    }

    /// Update the SystemTime loaded into the ManuallyAdvancedTimeSource.
    ///
    /// # Examples
    ///
    /// ```
    /// use metrique_timesource::{TimeSource, fakes::ManuallyAdvancedTimeSource};
    /// use std::time::{Duration, UNIX_EPOCH};
    ///
    /// // initial time is UNIX_EPOCH
    /// let dummy_time = ManuallyAdvancedTimeSource::at_time(UNIX_EPOCH);
    /// let ts = TimeSource::custom(dummy_time.clone());
    /// assert_eq!(ts.system_time(), UNIX_EPOCH);
    ///
    /// let new_timestamp = UNIX_EPOCH + Duration::from_secs(100);
    /// dummy_time.update_time(new_timestamp);
    /// assert_eq!(ts.system_time(), new_timestamp);
    /// ```
    pub fn update_time(&self, time: impl Into<SystemTime>) {
        let mut guard = self.0.lock().unwrap();
        guard.now = time.into();
    }

    /// Update the Instant loaded into the ManuallyAdvancedTimeSource by
    /// moving it forward by a duration.
    ///
    /// # Examples
    ///
    /// ```
    /// use metrique_timesource::{TimeSource, fakes::ManuallyAdvancedTimeSource};
    /// use std::time::{Duration, UNIX_EPOCH};
    ///
    /// // initial time is UNIX_EPOCH
    /// let dummy_time = ManuallyAdvancedTimeSource::at_time(UNIX_EPOCH);
    /// let ts = TimeSource::custom(dummy_time.clone());
    /// let instant = ts.instant();
    ///
    /// let elapsed = Duration::from_secs(100);
    /// dummy_time.update_instant(elapsed);
    /// assert_eq!(instant.elapsed(), elapsed);
    /// ```
    pub fn update_instant(&self, elapsed: Duration) {
        let mut guard = self.0.lock().unwrap();
        guard.now_instant += elapsed;
    }
}

impl Time for ManuallyAdvancedTimeSource {
    fn now(&self) -> SystemTime {
        self.0.lock().unwrap().now
    }

    fn instant(&self) -> Instant {
        self.0.lock().unwrap().now_instant
    }
}
