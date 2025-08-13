// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

#![deny(missing_docs)]
#![doc = include_str!("../README.md")]
#![cfg_attr(docsrs, feature(doc_auto_cfg))]

use std::{
    cell::RefCell,
    fmt::Debug,
    ops::Add,
    time::{Duration, Instant as StdInstant, SystemTime as StdSystemTime, SystemTimeError},
};

/// Module containing fake time sources for testing
///
/// To enable this module, you must enable the `test-util` feature.
#[cfg(feature = "test-util")]
pub mod fakes;

/// Trait for providing custom time sources
///
/// Implementors of this trait can be used to provide custom time behavior
/// for testing or specialized use cases.
pub trait Time: Send + Sync + Debug {
    /// Get the current system time
    fn now(&self) -> StdSystemTime;

    /// Get the current instant
    fn instant(&self) -> StdInstant;
}

/// Tokio-specific time source implementations
///
/// This module provides integration with tokio's time utilities, including
/// support for tokio's time pause/advance functionality for testing.
///
/// This requires that the `tokio` feature be enabled.
#[cfg(feature = "tokio")]
pub mod tokio {
    use std::time::SystemTime;

    use tokio::time::Instant as TokioInstant;

    use crate::{Time, TimeSource};
    use std::time::Instant as StdInstant;

    impl TimeSource {
        /// Create a new TimeSource that uses tokio's time utilities
        ///
        /// This allows integration with tokio's time pause/advance functionality
        /// for testing time-dependent code.
        ///
        /// This requires that the `tokio` feature be enabled.
        ///
        /// # Arguments
        ///
        /// * `starting_timestamp` - The initial system time to use
        ///
        /// # Returns
        ///
        /// A new TimeSource that uses tokio's time utilities
        ///
        /// # Examples
        ///
        /// ```
        /// # #[tokio::main(flavor = "current_thread")]
        /// # async fn main() {
        /// use std::time::{Duration, UNIX_EPOCH};
        /// use metrique_timesource::TimeSource;
        ///
        /// tokio::time::pause();
        /// let ts = TimeSource::tokio(UNIX_EPOCH);
        /// let start = ts.instant();
        ///
        /// tokio::time::advance(Duration::from_secs(5)).await;
        /// assert_eq!(start.elapsed(), Duration::from_secs(5));
        /// # }
        /// ```
        pub fn tokio(starting_timestamp: SystemTime) -> Self {
            TimeSource::custom(TokioTime::initialize_at(starting_timestamp))
        }
    }

    /// A time source implementation that uses tokio's time utilities
    ///
    /// This time source integrates with tokio's time pause/advance functionality,
    /// making it useful for testing time-dependent code.
    ///
    /// This requires that the `tokio` feature be enabled.
    #[derive(Copy, Clone, Debug)]
    pub struct TokioTime {
        start_time: TokioInstant,
        start_system_time: SystemTime,
    }

    impl TokioTime {
        /// Initialize a new TokioTime with the current system time
        ///
        /// # Returns
        ///
        /// A new TokioTime instance initialized with the current system time
        ///
        /// # Examples
        ///
        /// ```
        /// use metrique_timesource::tokio::TokioTime;
        /// use metrique_timesource::TimeSource;
        ///
        /// let time = TokioTime::initialize();
        /// let ts = TimeSource::custom(time);
        /// ```
        pub fn initialize() -> Self {
            Self::initialize_at(SystemTime::now())
        }

        /// Initialize a new TokioTime with a specific system time
        ///
        /// # Arguments
        ///
        /// * `initial_time` - The initial system time to use
        ///
        /// # Returns
        ///
        /// A new TokioTime instance initialized with the specified system time
        ///
        /// # Examples
        ///
        /// ```
        /// # #[tokio::main(flavor = "current_thread")]
        /// # async fn main() {
        /// use std::time::{Duration, UNIX_EPOCH};
        /// use metrique_timesource::tokio::TokioTime;
        /// use metrique_timesource::TimeSource;
        ///
        /// tokio::time::pause();
        /// let time = TokioTime::initialize_at(UNIX_EPOCH);
        /// let ts = TimeSource::custom(time);
        ///
        /// assert_eq!(ts.system_time(), UNIX_EPOCH);
        /// # }
        /// ```
        pub fn initialize_at(initial_time: SystemTime) -> Self {
            Self {
                start_time: TokioInstant::now(),
                start_system_time: initial_time,
            }
        }
    }

    impl Time for TokioTime {
        fn now(&self) -> SystemTime {
            self.start_system_time + self.start_time.elapsed()
        }

        fn instant(&self) -> StdInstant {
            TokioInstant::now().into_std()
        }
    }

    #[cfg(test)]
    mod test {
        use std::time::{Duration, UNIX_EPOCH};

        use crate::{SystemTime, TimeSource, get_time_source, set_time_source, tokio::TokioTime};

        #[tokio::test]
        async fn tokio_time_source() {
            tokio::time::pause();
            let ts = TimeSource::custom(TokioTime::initialize_at(UNIX_EPOCH));
            let start = ts.instant();
            assert_eq!(ts.system_time(), UNIX_EPOCH);
            tokio::time::advance(Duration::from_secs(1)).await;
            assert_eq!(ts.system_time(), UNIX_EPOCH + Duration::from_secs(1));
            assert_eq!(start.elapsed(), Duration::from_secs(1))
        }

        #[tokio::test]
        async fn with_tokio_ts() {
            struct MyMetric {
                start: SystemTime,
                end: Option<SystemTime>,
            }
            impl MyMetric {
                fn init() -> Self {
                    MyMetric {
                        start: get_time_source(None).system_time(),
                        end: None,
                    }
                }

                fn finish(&mut self) {
                    self.end = Some(get_time_source(None).system_time());
                }
            }

            tokio::time::pause();
            let start_time = UNIX_EPOCH + Duration::from_secs(1234);
            let _guard = set_time_source(TimeSource::custom(TokioTime::initialize_at(start_time)));
            let mut metric = MyMetric::init();
            assert_eq!(metric.start, start_time);
            tokio::time::advance(Duration::from_secs(5)).await;
            metric.finish();

            assert_eq!(
                metric.end.unwrap().duration_since(metric.start).unwrap(),
                Duration::from_secs(5)
            );
        }
    }
}

/// Enum representing different time source options
///
/// TimeSource provides a unified interface for accessing time, whether from the system
/// clock or from a custom time source for testing.
#[derive(Clone)]
pub enum TimeSource {
    /// Use the system time
    System,
    #[cfg(feature = "custom-timesource")]
    /// Use a custom time source
    Custom(std::sync::Arc<dyn Time + Send + Sync>),
}

impl std::fmt::Debug for TimeSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::System => write!(f, "TimeSource::System"),
            #[cfg(feature = "custom-timesource")]
            Self::Custom(_) => write!(f, "TimeSource::Custom(...)"),
        }
    }
}

impl TimeSource {
    /// Get the current [`SystemTime`] from this time source
    ///
    /// # Returns
    ///
    /// A wrapped SystemTime that maintains a reference to this time source
    ///
    /// # Examples
    ///
    /// ```
    /// use metrique_timesource::TimeSource;
    ///
    /// let ts = TimeSource::System;
    /// let now = ts.system_time();
    /// ```
    pub fn system_time(&self) -> SystemTime {
        match self {
            Self::System => SystemTime::new(StdSystemTime::now(), self),
            #[cfg(feature = "custom-timesource")]
            Self::Custom(ts) => SystemTime::new(ts.now(), self),
        }
    }

    /// Get the current instant from this time source
    ///
    /// # Returns
    ///
    /// A wrapped Instant that maintains a reference to this time source
    ///
    /// # Examples
    ///
    /// ```
    /// use metrique_timesource::TimeSource;
    /// use std::time::Duration;
    ///
    /// let ts = TimeSource::System;
    /// let start = ts.instant();
    /// // Do some work
    /// let elapsed = start.elapsed();
    /// ```
    pub fn instant(&self) -> Instant {
        match self {
            Self::System => Instant::new(StdInstant::now(), self),
            #[cfg(feature = "custom-timesource")]
            Self::Custom(ts) => Instant::new(ts.instant(), self),
        }
    }

    /// Create a new TimeSource with a custom time implementation
    ///
    /// This method is only available when the `custom-timesource` feature is enabled.
    ///
    /// # Arguments
    ///
    /// * `custom` - An implementation of the `Time` trait
    ///
    /// # Returns
    ///
    /// A new TimeSource that uses the provided custom time implementation
    ///
    /// # Examples
    ///
    /// ```
    /// use metrique_timesource::{TimeSource, fakes::StaticTimeSource};
    /// use std::time::{SystemTime, UNIX_EPOCH};
    ///
    /// let static_time = StaticTimeSource::at_time(UNIX_EPOCH);
    /// let ts = TimeSource::custom(static_time);
    /// assert_eq!(ts.system_time(), UNIX_EPOCH);
    /// ```
    #[cfg(feature = "custom-timesource")]
    pub fn custom(custom: impl Time + 'static) -> TimeSource {
        Self::Custom(std::sync::Arc::new(custom))
    }
}

impl Default for TimeSource {
    fn default() -> Self {
        Self::System
    }
}

// Thread-local time source override
thread_local! {
    static THREAD_LOCAL_TIME_SOURCE: RefCell<Option<TimeSource>> = const { RefCell::new(None) };
}

/// Guard for thread-local time source override
#[must_use]
pub struct ThreadLocalTimeSourceGuard {
    previous: Option<TimeSource>,
}

impl Drop for ThreadLocalTimeSourceGuard {
    fn drop(&mut self) {
        THREAD_LOCAL_TIME_SOURCE.with(|cell| {
            *cell.borrow_mut() = self.previous.take();
        });
    }
}

#[cfg(feature = "custom-timesource")]
/// Set a thread-local time source override and return a guard
/// When the guard is dropped, the thread-local override will be cleared
///
/// # Examples
/// ```
/// use metrique_timesource::{TimeSource, fakes::StaticTimeSource, time_source, set_time_source};
/// use std::time::UNIX_EPOCH;
///
/// let ts = TimeSource::custom(StaticTimeSource::at_time(UNIX_EPOCH));
/// let _guard = set_time_source(ts);
///
/// assert_eq!(time_source().system_time(), UNIX_EPOCH);
/// ```
pub fn set_time_source(time_source: TimeSource) -> ThreadLocalTimeSourceGuard {
    let previous = THREAD_LOCAL_TIME_SOURCE.with(|cell| cell.borrow_mut().replace(time_source));
    ThreadLocalTimeSourceGuard { previous }
}

#[cfg(feature = "custom-timesource")]
/// Run a closure with a thread-local time source override
pub fn with_time_source<F, R>(time_source: TimeSource, f: F) -> R
where
    F: FnOnce() -> R,
{
    let _guard = set_time_source(time_source);
    f()
}

/// Get the current time source, following the priority order:
/// 1. Explicitly provided time source
/// 2. Thread-local override
/// 3. System default
#[inline]
pub fn get_time_source(ts: Option<TimeSource>) -> TimeSource {
    // 1. Explicitly provided time source
    if let Some(ts) = ts {
        return ts;
    }

    #[cfg(feature = "custom-timesource")]
    {
        // 2. Thread-local override
        let thread_local = THREAD_LOCAL_TIME_SOURCE.with(|cell| cell.borrow().clone());
        if let Some(ts) = thread_local {
            return ts;
        }
    }

    // 3. System default
    TimeSource::System
}

/// Get the current time source
///
/// This is a convenience function that calls `get_time_source(None)`.
///
/// # Returns
///
/// The current time source, which will be either the thread-local override
/// if one is set, or the system default.
///
/// # Examples
///
/// ```
/// use metrique_timesource::time_source;
///
/// let ts = time_source();
/// let now = ts.system_time();
/// ```
#[inline]
pub fn time_source() -> TimeSource {
    get_time_source(None)
}

/// `Instant` wrapper
///
/// This may be freely converted into `std::time::Instant` with `.into()`. However,
/// this will cause `elapsed()` to no longer return correct results if a custom time source is used.
///
/// When `custom-timesource` is not enabled, this is exactly the same size as `Instant`. When `custom-timesource` _is_ enabled, it retains a pointer
/// to the timesource it came from to allow `elapsed()` to work properly.
#[derive(Clone)]
#[cfg_attr(not(feature = "custom-timesource"), derive(Copy), repr(transparent))]
pub struct Instant {
    value: StdInstant,
    #[cfg(feature = "custom-timesource")]
    time_source: TimeSource,
}

impl From<Instant> for StdInstant {
    fn from(instant: Instant) -> std::time::Instant {
        instant.as_std()
    }
}

impl std::fmt::Debug for Instant {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.value.fmt(f)
    }
}

impl Instant {
    /// Create a new Instant from the given TimeSource
    ///
    /// # Arguments
    ///
    /// * `ts` - The TimeSource to use
    ///
    /// # Returns
    ///
    /// A new Instant representing the current time from the given TimeSource
    ///
    /// # Examples
    ///
    /// ```
    /// use metrique_timesource::{Instant, TimeSource};
    ///
    /// let ts = TimeSource::System;
    /// let now = Instant::now(&ts);
    /// ```
    pub fn now(ts: &TimeSource) -> Self {
        ts.instant()
    }

    /// Returns the amount of time elapsed since this instant was created
    ///
    /// # Returns
    ///
    /// The elapsed time as a Duration
    ///
    /// # Examples
    ///
    /// ```
    /// use metrique_timesource::{TimeSource, time_source};
    /// use std::thread;
    /// use std::time::Duration;
    ///
    /// let ts = time_source();
    /// let start = ts.instant();
    /// thread::sleep(Duration::from_millis(10));
    /// let elapsed = start.elapsed();
    /// assert!(elapsed.as_millis() >= 10);
    /// ```
    pub fn elapsed(&self) -> Duration {
        #[cfg(not(feature = "custom-timesource"))]
        let ts = TimeSource::System;
        #[cfg(feature = "custom-timesource")]
        let ts = &self.time_source;

        ts.instant().as_std() - self.value
    }

    /// Convert this Instant to a std::time::Instant
    ///
    /// # Returns
    ///
    /// A std::time::Instant representing the same point in time
    ///
    /// # Note
    ///
    /// After conversion, elapsed() will no longer respect custom time sources
    /// if they were being used.
    pub fn as_std(&self) -> StdInstant {
        self.value
    }

    fn new(std: StdInstant, ts: &TimeSource) -> Self {
        #[cfg(not(feature = "custom-timesource"))]
        let _ = ts;
        Self {
            value: std,
            #[cfg(feature = "custom-timesource")]
            time_source: ts.clone(),
        }
    }
}

/// `SystemTime` wrapper
///
/// This may be freely converted into `std::time::SystemTime` with `.into()`. However,
/// this will cause `elapsed()` to no longer return correct results if a custom time source is used.
///
/// When `custom-timesource` is not enabled, this is exactly the same size as `SystemTime`. When `custom-timesource` _is_ enabled, it retains a pointer
/// to the timesource it came from to allow `elapsed()` to work properly.
#[derive(Clone)]
#[cfg_attr(not(feature = "custom-timesource"), derive(Copy), repr(transparent))]
pub struct SystemTime {
    value: StdSystemTime,
    #[cfg(feature = "custom-timesource")]
    time_source: TimeSource,
}

impl PartialEq for SystemTime {
    fn eq(&self, other: &SystemTime) -> bool {
        self.value.eq(&other.value)
    }
}

impl Eq for SystemTime {}

impl PartialOrd for SystemTime {
    fn partial_cmp(&self, other: &SystemTime) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for SystemTime {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.value.cmp(&other.value)
    }
}

impl PartialEq<StdSystemTime> for SystemTime {
    fn eq(&self, other: &StdSystemTime) -> bool {
        self.value.eq(other)
    }
}

impl PartialOrd<StdSystemTime> for SystemTime {
    fn partial_cmp(&self, other: &StdSystemTime) -> Option<std::cmp::Ordering> {
        Some(self.value.cmp(other))
    }
}

impl Add<Duration> for SystemTime {
    type Output = Self;

    fn add(mut self, rhs: Duration) -> Self::Output {
        self.value += rhs;
        self
    }
}

impl Debug for SystemTime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.value.fmt(f)
    }
}

impl SystemTime {
    /// See [`std::time::SystemTime::duration_since`]
    pub fn duration_since(
        &self,
        earlier: impl Into<StdSystemTime>,
    ) -> Result<Duration, SystemTimeError> {
        self.value.duration_since(earlier.into())
    }

    /// See [`std::time::SystemTime::elapsed`]
    pub fn elapsed(&self) -> Result<Duration, SystemTimeError> {
        let now = self.time_source().system_time();
        now.duration_since(self.value)
    }

    /// Convert this SystemTime to a std::time::SystemTime
    ///
    /// # Returns
    ///
    /// A std::time::SystemTime representing the same point in time
    ///
    /// # Note
    ///
    /// After conversion, elapsed() will no longer respect custom time sources
    /// if they were being used.
    pub fn as_std(&self) -> StdSystemTime {
        self.value
    }

    fn time_source(&self) -> &TimeSource {
        #[cfg(feature = "custom-timesource")]
        {
            &self.time_source
        }

        #[cfg(not(feature = "custom-timesource"))]
        &TimeSource::System
    }

    /// Creates this SystemTime from a std::time::SystemTime
    /// and a provided time source. This is useful for loading
    /// system times from an external source, that you want
    /// to interact with using this library's time sources.
    ///
    /// # Returns
    ///
    /// A SystemTime representing the same point in time,
    /// managed by the provided time source.
    ///
    /// # Example
    ///
    /// ```
    /// use metrique_timesource::{SystemTime, time_source};
    ///
    /// let now = std::time::SystemTime::now();
    /// let system_time = SystemTime::new(now, &time_source());
    /// ```
    pub fn new(std: StdSystemTime, ts: &TimeSource) -> Self {
        #[cfg(not(feature = "custom-timesource"))]
        let _ = ts;
        Self {
            value: std,
            #[cfg(feature = "custom-timesource")]
            time_source: ts.clone(),
        }
    }
}

impl From<SystemTime> for StdSystemTime {
    fn from(val: SystemTime) -> Self {
        val.value
    }
}

#[cfg(test)]
mod tests {

    use std::time::UNIX_EPOCH;

    use crate::{
        TimeSource, fakes, get_time_source, set_time_source, time_source, with_time_source,
    };

    #[test]
    fn test_default_time_source() {
        let ts = time_source();
        match ts {
            TimeSource::System => {} // Expected
            _ => panic!("Expected default time source to be System"),
        }
    }

    #[test]
    fn test_explicit_time_source() {
        let ts = fakes::StaticTimeSource::at_time(UNIX_EPOCH);
        let ts = TimeSource::custom(ts);
        let ts = get_time_source(Some(ts));
        match ts {
            TimeSource::Custom(_) => {} // Expected
            _ => panic!("Expected explicit time source to be used"),
        }
    }

    #[test]
    fn test_thread_local_time_source() {
        let ts = fakes::StaticTimeSource::at_time(UNIX_EPOCH);
        let ts = TimeSource::custom(ts);

        {
            let _guard = set_time_source(ts);
            let ts = get_time_source(None);
            assert_eq!(ts.system_time(), UNIX_EPOCH);
        }

        // After guard is dropped, should go back to default
        let ts = get_time_source(None);
        match ts {
            TimeSource::System => {} // Expected
            _ => panic!("Expected default time source after guard is dropped"),
        }
    }

    #[test]
    fn test_thread_local_time_source_scoped() {
        let ts = fakes::StaticTimeSource::at_time(UNIX_EPOCH);
        let thread_local = TimeSource::custom(ts);

        with_time_source(thread_local, || {
            let ts = get_time_source(None);
            match ts {
                TimeSource::Custom(_) => {} // Expected
                _ => panic!(),
            }
        });

        // After scope, should go back to default
        let ts = get_time_source(None);
        match ts {
            TimeSource::System => {} // Expected
            _ => panic!("Expected default time source after scope"),
        }
    }
}
