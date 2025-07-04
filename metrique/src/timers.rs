// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::{
    marker::PhantomData,
    ops::AddAssign,
    sync::{Arc, Mutex},
    time::{Duration, UNIX_EPOCH},
};

use metrique_core::{CloseValue, CloseValueRef};
use metrique_timesource::{Instant, SystemTime, TimeSource, time_source};
use metrique_writer_core::{
    Value,
    unit::{Millisecond, Second},
};
use metrique_writer_core::{unit::Microsecond, value::ValueFormatter};
use timestamp_to_str::{TimestampToStr, TimestampValueOld};

use crate::unit::AttachUnit;

/// Timestamp of a metric entry
///
/// This type should with `#[metrics(timestamp)]` attribute on the root of your metrics entry.
/// Unless otherwise specified, it will record the time that it was created at. If you need to record a different time,
/// use [`Timestamp::new`].
///
/// When used as a field (without the `#[metrics(timestamp)]` attribute), `Timestamp` will record a `value` field (not a metric)
/// containing the current timestamp. By default, `Timestamp` will report units of [`Millisecond`]. You can control the unit with the `unit` attribute.
#[derive(Debug)]
pub struct Timestamp {
    time: SystemTime,
}

impl Timestamp {
    /// Create a new timestamp at the current time.
    ///
    /// The time will be loaded from [`metrique_timesource::time_source`]
    ///
    /// This is the behavior of `Timestamp::default`
    pub fn now() -> Self {
        Self::new_from_time_source(time_source())
    }

    /// Create a new timestamp at a specific time
    pub fn new(time: SystemTime) -> Self {
        Self { time }
    }

    /// Create a new timestamp at a specific time from an explicit [`TimeSource`]
    ///
    /// # Examples
    /// ```rust
    /// use std::time::UNIX_EPOCH;
    /// use metrique_timesource::{TimeSource, fakes::StaticTimeSource};
    /// use metrique::timers::Timestamp;
    /// let ts = TimeSource::custom(StaticTimeSource::at_time(UNIX_EPOCH));
    /// let timestamp = Timestamp::new_from_time_source(ts);
    /// ```
    pub fn new_from_time_source(ts: TimeSource) -> Self {
        Self {
            time: ts.system_time(),
        }
    }
}

impl CloseValueRef for Timestamp {
    type Closed = TimestampValue;

    fn close_ref(&self) -> Self::Closed {
        TimestampValue::new(&self.time)
    }
}

impl Default for Timestamp {
    fn default() -> Self {
        Self::now()
    }
}

/// A `Timestamp` which records the time when the record is closed
///
/// When combined with [`Timestamp`], this is a useful tool for to record when
/// an event starts and stops for post-hoc analysis.
///
/// # Examples
/// ```rust
/// use metrique::unit_of_work::metrics;
/// use metrique::timers::{Timestamp, TimestampOnClose};
///
/// #[metrics]
/// struct TrackStartStop {
///     start: Timestamp,
///     stop: TimestampOnClose
/// }
/// ```
pub struct TimestampOnClose {
    time_source: TimeSource,
}

impl Default for TimestampOnClose {
    fn default() -> Self {
        Self {
            time_source: time_source(),
        }
    }
}

impl CloseValue for TimestampOnClose {
    type Closed = TimestampValue;

    fn close(self) -> Self::Closed {
        TimestampValue::new(&self.time_source.system_time())
    }
}

/// Formats a timestamp in `EpochSeconds` format
pub type EpochSeconds = TimestampFormat<Second>;

/// Formats a timestamp in `EpochMillis` format
pub type EpochMillis = TimestampFormat<Millisecond>;

/// Formats a timestamp in `EpochMicros` format
pub type EpochMicros = TimestampFormat<Microsecond>;

/// The type returned when `Timestamp` types are closed
#[derive(Copy, Clone)]
pub struct TimestampValue {
    duration_since_epoch: Duration,
}

impl Value for TimestampValue {
    fn write(&self, writer: impl metrique_writer_core::ValueWriter) {
        // by default, use the milliseconds format
        <Millisecond as TimestampToStr>::to_str(self.duration_since_epoch, |v| writer.string(v));
    }
}

impl TimestampValue {
    /// Create a new `TimestampValue` from a `SystemTime`
    pub fn new(ts: &SystemTime) -> Self {
        Self {
            duration_since_epoch: ts.duration_since(UNIX_EPOCH).unwrap_or_default(),
        }
    }

    /// Duration since the [`UNIX_EPOCH`] represented by this Timestamp
    pub fn duration_since_epoch(&self) -> Duration {
        self.duration_since_epoch
    }
}

impl From<TimestampValue> for std::time::SystemTime {
    fn from(value: TimestampValue) -> Self {
        UNIX_EPOCH + value.duration_since_epoch
    }
}

impl AttachUnit for TimestampValue {
    type Output<U> = TimestampValueOld<U>;

    fn make<U>(self) -> Self::Output<U> {
        TimestampValueOld {
            time: UNIX_EPOCH + self.duration_since_epoch,
            unit: Default::default(),
        }
    }
}

#[doc(hidden)]
pub struct TimestampFormat<Unit> {
    u: PhantomData<Unit>,
}

impl<U: TimestampToStr> ValueFormatter<TimestampValue> for TimestampFormat<U> {
    fn format_value(writer: impl metrique_writer_core::ValueWriter, value: &TimestampValue) {
        U::to_str(value.duration_since_epoch, |s| writer.string(s));
    }
}

/// Timestamps must be formatted as strings
mod timestamp_to_str {
    use std::{
        marker::PhantomData,
        time::{Duration, UNIX_EPOCH},
    };

    use metrique_writer_core::{
        Value,
        unit::{Microsecond, Millisecond, Second},
    };

    use crate::unit::AttachUnit;

    use super::TimestampValue;

    /// The value produced when a `Timestamp` is closed.
    pub struct TimestampValueOld<Unit> {
        pub(crate) time: std::time::SystemTime,
        pub(crate) unit: PhantomData<Unit>,
    }

    impl<U> Clone for TimestampValueOld<U> {
        fn clone(&self) -> Self {
            *self
        }
    }

    impl<U> Copy for TimestampValueOld<U> {}

    pub(super) trait TimestampToStr {
        fn to_str(value: Duration, f: impl FnOnce(&str));
    }

    impl TimestampToStr for Second {
        fn to_str(value: Duration, f: impl FnOnce(&str)) {
            let mut buf = ryu::Buffer::new();
            let value = buf.format(value.as_secs_f64());
            f(value)
        }
    }

    pub(crate) fn duration_as_millis_with_nano_precision(duration: Duration) -> f64 {
        //         milli
        // unit  * ----- = milli
        //         unit
        duration.as_secs_f64() * 1000.0
    }

    impl TimestampToStr for Millisecond {
        fn to_str(value: Duration, f: impl FnOnce(&str)) {
            let mut buf = ryu::Buffer::new();
            let value = buf.format(duration_as_millis_with_nano_precision(value));
            f(value)
        }
    }

    impl TimestampToStr for Microsecond {
        fn to_str(value: Duration, f: impl FnOnce(&str)) {
            let mut buf = itoa::Buffer::new();
            let value = buf.format(value.as_micros());
            f(value)
        }
    }

    impl<Ignore> From<TimestampValueOld<Ignore>> for std::time::SystemTime {
        fn from(value: TimestampValueOld<Ignore>) -> Self {
            value.time
        }
    }

    impl<U: TimestampToStr> Value for TimestampValueOld<U> {
        fn write(&self, writer: impl metrique_writer_core::ValueWriter) {
            U::to_str(
                self.time.duration_since(UNIX_EPOCH).unwrap_or_default(),
                |v| writer.string(v),
            );
        }
    }

    impl From<TimestampValue> for TimestampValueOld<Second> {
        fn from(value: TimestampValue) -> Self {
            value.make()
        }
    }

    impl From<TimestampValue> for TimestampValueOld<Microsecond> {
        fn from(value: TimestampValue) -> Self {
            value.make()
        }
    }

    impl From<TimestampValue> for TimestampValueOld<Millisecond> {
        fn from(value: TimestampValue) -> Self {
            value.make()
        }
    }
}

/// Timers record the time it takes to do a unit of work
///
/// They are started automatically and stop automatically when dropped (unless you call `Timer::stop` first.)
/// If you want a timer you can control explicitly, use [`Stopwatch`]
///
/// Unlike [`Stopwatch`], timer records a single continuous span of time. It cannot be restarted after it is stopped.
#[derive(Debug)]
pub struct Timer {
    start: Instant,
    duration: Option<Duration>,
}

impl Default for Timer {
    fn default() -> Self {
        Self {
            start: time_source().instant(),
            duration: None,
        }
    }
}

impl Timer {
    /// Creates a new timer that starts immediately using the default time source.
    ///
    /// # Example
    /// ```
    /// use metrique::timers::Timer;
    /// use std::thread::sleep;
    /// use std::time::Duration;
    ///
    /// let mut timer = Timer::start_now();
    /// sleep(Duration::from_millis(10));
    /// let elapsed = timer.stop();
    /// assert!(elapsed >= Duration::from_millis(10));
    /// ```
    pub fn start_now() -> Self {
        Self::start_now_with_timesource(time_source())
    }

    /// Creates a new timer that starts immediately using the specified time source.
    ///
    /// This is useful for testing with a mock time source.
    ///
    /// # Example
    /// ```
    /// # use metrique::timers::Timer;
    /// # use metrique_timesource::TimeSource;
    /// # use std::time::UNIX_EPOCH;
    /// #
    /// let time_source = TimeSource::tokio(UNIX_EPOCH);
    /// let timer = Timer::start_now_with_timesource(time_source);
    /// ```
    pub fn start_now_with_timesource(timesource: TimeSource) -> Self {
        Self {
            start: timesource.instant(),
            duration: None,
        }
    }

    /// Stops the timer and returns the elapsed duration.
    ///
    /// After calling this method, the timer will no longer update and will report
    /// the same duration when closed.
    ///
    /// **Important Note**: Although this returns the duration for convenience, you don't need to store it yourself. The duration
    /// will be recorded in the parent metric when it is closed.
    ///
    /// # Example
    /// ```no_run
    /// use metrique::timers::Timer;
    /// use std::thread::sleep;
    /// use std::time::Duration;
    ///
    /// let mut timer = Timer::start_now();
    /// sleep(Duration::from_millis(10));
    /// let elapsed = timer.stop();
    /// ```
    pub fn stop(&mut self) -> Duration {
        let time = self.start.elapsed();
        self.duration = Some(time);
        time
    }
}

impl CloseValueRef for Timer {
    type Closed = Duration;

    fn close_ref(&self) -> Self::Closed {
        self.duration.unwrap_or_else(|| self.start.elapsed())
    }
}

/// A guard that stops a timer when dropped.
///
/// This guard is returned by [`Stopwatch::start()`] and will add the elapsed time
/// to the stopwatch when dropped.
///
/// To customize this behavior, use [`Self::overwrite()`] or
/// [`Self::discard()`].
///
/// To explicitly stop this guard and access the Duration it flushed to the `Stopwatch`,
/// use [`Self::stop()`].
#[must_use]
pub struct TimerGuard<'a> {
    start: Option<Instant>,
    self_time: Option<Duration>,
    timer: &'a mut MaybeGuardedDuration,
}

impl TimerGuard<'_> {
    /// Explicitly stops the timer.
    ///
    /// This is equivalent to dropping the guard, but makes the intention clearer.
    ///
    /// **Important Note**: Although this returns the duration for convenience, you don't need to store it yourself. The duration
    /// will be recorded in the parent metric when it is closed.
    ///
    /// # Example
    /// ```
    /// use metrique::timers::Stopwatch;
    /// use std::thread::sleep;
    /// use std::time::Duration;
    ///
    /// let mut stopwatch = Stopwatch::new();
    /// let guard = stopwatch.start();
    /// sleep(Duration::from_millis(10));
    /// let elapsed = guard.stop(); // Explicitly stop timing
    /// ```
    pub fn stop(mut self) -> Duration {
        // we know that this is not None, since the only way to wipe the stored
        // fields is by calling `Self::clear()`, or `Self::discard()`, which both take self,
        // meaning stop can't be called afterwards
        self.stop_ref().unwrap()
    }

    fn stop_ref(&mut self) -> Option<Duration> {
        match self.self_time {
            Some(time) => Some(time),
            None => {
                let elapsed = self.start.as_ref().map(|start| start.elapsed());
                self.self_time = elapsed;
                elapsed
            }
        }
    }

    /// Flush loaded duration in this guard, overwriting any existing
    /// contents in the underlying [`Stopwatch`].
    ///
    /// # Example
    /// ```
    /// use metrique::timers::Stopwatch;
    /// use std::thread::sleep;
    /// use std::time::Duration;
    ///
    /// let mut stopwatch = Stopwatch::new();
    /// let guard = stopwatch.start();
    /// sleep(Duration::from_millis(10));
    /// let elapsed = guard.stop(); // Explicitly stop timing
    /// // stopwatch now contains 10 millis
    ///
    /// let guard = stopwatch.start();
    /// sleep(Duration::from_millis(30));
    /// // wipe the existing 10 millis in this guard,
    /// // and replace with 30ms in loaded guard
    /// guard.overwrite();
    /// // stopwatch contains 30ms
    /// ```
    pub fn overwrite(self) {
        self.timer.take();
    }

    /// Discard all loaded duration in this guard,
    /// preventing it from being flushed into the underlying `Stopwatch`.
    ///
    /// # Example
    /// ```
    /// use metrique::timers::Stopwatch;
    /// use std::thread::sleep;
    /// use std::time::Duration;
    ///
    /// let mut stopwatch = Stopwatch::new();
    /// let guard = stopwatch.start();
    /// sleep(Duration::from_millis(20));
    /// let elapsed = guard.stop(); // Explicitly stop timing
    /// // stopwatch now contains 20 millis
    ///
    /// let guard = stopwatch.start();
    /// sleep(Duration::from_millis(10));
    /// // wipe the new 10 millis in this guard, but not the already-loaded
    /// // 20 millis in the underlying timer
    /// guard.discard();
    /// // stopwatch still contains 20ms
    /// ```
    pub fn discard(mut self) {
        self.self_time.take();
        self.start.take();
    }
}

impl Drop for TimerGuard<'_> {
    fn drop(&mut self) {
        let self_time = self.stop_ref();
        // only update the underlying StopWatch if we haven't called
        // clear() or discard() on the guard explicitly
        if let Some(self_time) = self_time {
            *self.timer += self_time;
        }
    }
}

/// An owned guard that stops a timer when dropped.
///
/// This guard is returned by [`Stopwatch::start_owned()`] and will add the elapsed time
/// to the stopwatch when dropped.
///
/// To customize this behavior, use [`Self::overwrite()`] or
/// [`Self::discard()`].
///
/// To explicitly stop this guard and access the Duration it flushed to the `Stopwatch`,
/// use [`Self::stop()`].
#[must_use]
pub struct OwnedTimerGuard {
    start: Option<Instant>,
    self_time: Option<Duration>,
    timer: MaybeGuardedDuration,
}

impl OwnedTimerGuard {
    /// Explicitly stops the timer.
    ///
    /// This is equivalent to dropping the guard, but makes the intention clearer.
    ///
    /// **Important Note**: Although this returns the duration for convenience, you don't need to store it yourself. The duration
    /// will be recorded in the parent metric when it is closed.
    ///
    /// # Example
    /// ```
    /// use metrique::timers::Stopwatch;
    /// use std::thread::sleep;
    /// use std::time::Duration;
    ///
    /// let mut stopwatch = Stopwatch::new();
    /// let guard = stopwatch.start();
    /// sleep(Duration::from_millis(10));
    /// let elapsed = guard.stop(); // Explicitly stop timing
    /// ```
    pub fn stop(mut self) -> Duration {
        // we know that this is not None, since the only way to wipe the stored
        // fields is by calling `Self::clear()`, or `Self::discard()`, which both take self,
        // meaning stop can't be called afterwards
        self.stop_ref().unwrap()
    }

    fn stop_ref(&mut self) -> Option<Duration> {
        match self.self_time {
            Some(time) => Some(time),
            None => {
                let elapsed = self.start.as_ref().map(|start| start.elapsed());
                self.self_time = elapsed;
                elapsed
            }
        }
    }

    /// Flush loaded duration in this guard, overwriting any existing
    /// contents in the underlying [`Stopwatch`].
    ///
    /// # Example
    /// ```
    /// use metrique::timers::Stopwatch;
    /// use std::thread::sleep;
    /// use std::time::Duration;
    ///
    /// let mut stopwatch = Stopwatch::new();
    /// let guard = stopwatch.start();
    /// sleep(Duration::from_millis(10));
    /// let elapsed = guard.stop(); // Explicitly stop timing
    /// // stopwatch now contains 10 millis
    ///
    /// let guard = stopwatch.start();
    /// sleep(Duration::from_millis(30));
    /// // wipe the existing 10 millis in this guard,
    /// // and replace with 30ms in loaded guard
    /// guard.overwrite();
    /// // stopwatch contains 30ms
    /// ```
    pub fn overwrite(mut self) {
        self.timer.take();
    }

    /// Discard all loaded duration in this guard,
    /// preventing it from being flushed into the underlying `Stopwatch`.
    ///
    /// # Example
    /// ```
    /// use metrique::timers::Stopwatch;
    /// use std::thread::sleep;
    /// use std::time::Duration;
    ///
    /// let mut stopwatch = Stopwatch::new();
    /// let guard = stopwatch.start();
    /// sleep(Duration::from_millis(10));
    /// let elapsed = guard.stop(); // Explicitly stop timing
    /// // stopwatch now contains 10 millis
    ///
    /// let guard = stopwatch.start();
    /// sleep(Duration::from_millis(10));
    /// // wipe the new 10 millis in this guard, but not the already-loaded
    /// // 10 millis in the underlying timer
    /// guard.discard();
    /// // stopwatch still contains 10ms
    /// ```
    pub fn discard(mut self) {
        self.self_time.take();
        self.start.take();
    }
}

impl Drop for OwnedTimerGuard {
    fn drop(&mut self) {
        let self_time = self.stop_ref();
        // only update the underlying StopWatch if we haven't called
        // clear() or discard() on the guard explicitly
        if let Some(self_time) = self_time {
            self.timer += self_time;
        }
    }
}

/// The internal storage of a duration that might have [`TimerGuard`] or [`SharedTimerGuard`]
/// pointing to it. We default to a Exclusive duration, meaning a guard backed by a mutable pointer
/// can access it ( eg[`Stopwatch::start()`]).
/// We switch to an shared, mutex-backed version if the caller uses eg [`Stopwatch::start_owned()`].
#[derive(Debug)]
enum MaybeGuardedDuration {
    Exclusive(Option<Duration>),
    Shared(SharedDuration),
}

impl MaybeGuardedDuration {
    /// Pull the stored duration out of the state, leaving empty state behind
    fn take(&mut self) -> Option<Duration> {
        match self {
            MaybeGuardedDuration::Exclusive(duration) => duration.take(),
            MaybeGuardedDuration::Shared(shared_duration) => shared_duration
                .0
                .lock()
                .expect("owned timer guard panicked while holding lock")
                .take(),
        }
    }

    /// Converts the the stored duration into the shared variant if needed,
    /// then returns of clone of it
    fn shared_cloned(&mut self) -> SharedDuration {
        match self {
            MaybeGuardedDuration::Exclusive(duration) => {
                let shared_duration = SharedDuration(Arc::new(Mutex::new(duration.take())));
                *self = MaybeGuardedDuration::Shared(shared_duration.clone());
                shared_duration
            }
            MaybeGuardedDuration::Shared(shared_duration) => shared_duration.clone(),
        }
    }
}

impl Default for MaybeGuardedDuration {
    fn default() -> Self {
        MaybeGuardedDuration::Exclusive(Default::default())
    }
}

impl PartialEq<Option<Duration>> for MaybeGuardedDuration {
    fn eq(&self, other: &Option<Duration>) -> bool {
        match self {
            MaybeGuardedDuration::Exclusive(duration) => duration == other,
            MaybeGuardedDuration::Shared(shared_duration) => shared_duration == other,
        }
    }
}

impl AddAssign<Duration> for MaybeGuardedDuration {
    fn add_assign(&mut self, rhs: Duration) {
        match self {
            MaybeGuardedDuration::Exclusive(duration) => {
                *duration = Some(duration.unwrap_or_default() + rhs)
            }
            MaybeGuardedDuration::Shared(shared_duration) => *shared_duration += rhs,
        }
    }
}

/// The internal representation of [`MaybeGuardedDuration::Shared`]
#[derive(Debug, Clone)]
struct SharedDuration(Arc<Mutex<Option<Duration>>>);

impl PartialEq<Option<Duration>> for SharedDuration {
    fn eq(&self, other: &Option<Duration>) -> bool {
        *self
            .0
            .lock()
            .expect("owned timer guard panicked while holding lock")
            == *other
    }
}

impl AddAssign<Duration> for SharedDuration {
    fn add_assign(&mut self, rhs: Duration) {
        let mut guard = self
            .0
            .lock()
            .expect("owned timer guard panicked while holding lock");
        *guard = Some(guard.unwrap_or_default() + rhs);
    }
}

/// Stopwatches allow you to manually measure time
///
/// Stopwatches must be manually started by calling [`Stopwatch::start`]. This will return a guard
/// which you must hold until the measurement is complete.
///
/// A stopwatch MAY be started multiple times—the durations will add. It is impossible to run the stopwatch multiple times concurrently
/// as the `start` method uses `&mut self`.
#[derive(Debug)]
pub struct Stopwatch {
    time_source: TimeSource,
    start: Option<Instant>,
    duration: MaybeGuardedDuration,
}

impl Default for Stopwatch {
    fn default() -> Self {
        Self {
            time_source: time_source(),
            start: None,
            duration: MaybeGuardedDuration::default(),
        }
    }
}

impl Stopwatch {
    /// Creates a new stopwatch that is not yet started.
    ///
    /// The stopwatch must be started by calling `start()` or `start_owned()` before it will measure time.
    ///
    /// # Example
    /// ```
    /// use metrique::timers::Stopwatch;
    ///
    /// let mut stopwatch = Stopwatch::new();
    /// let guard = stopwatch.start(); // Start timing
    /// // Do some work...
    /// drop(guard); // Stop timing
    /// ```
    pub fn new() -> Self {
        Self::new_from_timesource(time_source())
    }

    /// Creates a new stopwatch from an explicit timesource
    ///
    /// The stopwatch must be started by calling `start()` before it will measure time.
    pub fn new_from_timesource(time_source: TimeSource) -> Self {
        Self {
            time_source,
            start: None,
            duration: MaybeGuardedDuration::default(),
        }
    }

    /// Starts the stopwatch and returns a guard that will stop it when dropped.
    ///
    /// The stopwatch will accumulate time until the guard is dropped. Multiple calls
    /// to `start()` or `start_owned()` will add to the total duration.
    ///
    /// Note that the guard will involve a mutable borrow of this [`Stopwatch`].
    /// If you encounter double mutable borrow errors or otherwise need an owned handle,
    /// use [`Stopwatch::start_owned()`] instead.
    ///
    /// # Example
    /// ```
    /// # use metrique::timers::Stopwatch;
    /// # use std::time::Duration;
    /// #
    /// let mut stopwatch = Stopwatch::new();
    ///
    /// // First timing session
    /// let guard = stopwatch.start();
    /// // Do some work...
    /// drop(guard);
    ///
    /// // Second timing session (adds to the first)
    /// let guard = stopwatch.start();
    /// // Do more work...
    /// drop(guard);
    /// ```
    pub fn start(&mut self) -> TimerGuard<'_> {
        let start = self.time_source.instant();
        TimerGuard {
            start: Some(start),
            self_time: None,
            timer: &mut self.duration,
        }
    }

    /// Starts the stopwatch and returns an shared guard that will stop it when dropped.
    ///
    /// The stopwatch will accumulate time until the guard is dropped. Multiple calls
    /// to `start()` or `start_owned()` will add to the total duration.
    ///
    /// The shared guard is backed by a (usually uncontended) `Arc<Mutex>` so this method slightly more overhead than
    /// a simple [`Stopwatch::start()`] call.
    ///
    /// Use this if you specifically are encountering double mutable borrows while passing around the guard.
    /// Or, if you want to stop the stopwatch in another task.
    ///
    /// # Example
    /// ```
    /// # use metrique::timers::Stopwatch;
    /// # use std::time::Duration;
    ///
    /// #[derive(Default, Debug)]
    /// struct DuckTracker {
    ///    total_latency: Stopwatch,
    ///    duck_count: usize
    /// }
    ///
    /// fn process_ducks(tracker: &mut DuckTracker) {
    ///     tracker.duck_count += 1;
    /// }
    ///
    /// let mut tracker = DuckTracker::default();
    /// loop {
    ///     let _timer = tracker.total_latency.start_owned();
    ///     process_ducks(&mut tracker);
    ///     if tracker.duck_count >= 5 {
    ///         break;
    ///     }
    ///     // timer drops, adding to total latency
    /// }
    /// println!("{tracker:#?}")
    /// ```
    pub fn start_owned(&mut self) -> OwnedTimerGuard {
        let shared_duration = self.duration.shared_cloned();
        let start = self.time_source.instant();

        OwnedTimerGuard {
            start: Some(start),
            self_time: None,
            timer: MaybeGuardedDuration::Shared(shared_duration),
        }
    }

    /// Clear all loaded duration in the [`Stopwatch`].
    ///
    /// This will result in no metrics being published on drop,
    /// unless further duration is loaded in.
    ///
    /// This only clears the [`Stopwatch`] for its immediate state.
    ///
    /// Any active [`TimerGuard`]/[`OwnedTimerGuard`]s will continue to tick and write
    /// to this `Stopwatch` when they drop or are stopped explicitly.
    ///
    /// See also: [`TimerGuard::discard()`]/[`OwnedTimerGuard::discard()`] to
    /// drop the guard's state but leave the stopwatch's loaded duration in place.
    ///
    /// And see:
    /// [`TimerGuard::overwrite()`]/[`OwnedTimerGuard::overwrite()`] to overwrite the stopwatch's loaded duration
    /// with the guard's contents.
    ///
    /// # Example
    /// ```
    /// use metrique::timers::Stopwatch;
    /// use std::thread::sleep;
    /// use std::time::Duration;
    ///
    /// let mut stopwatch = Stopwatch::new();
    /// let guard = stopwatch.start();
    /// sleep(Duration::from_millis(10));
    /// guard.stop(); // Explicitly stop timing
    ///
    /// stopwatch.clear();
    /// // stopwatch now contains no stored time
    ///
    /// // start fresh
    /// let guard = stopwatch.start();
    /// sleep(Duration::from_millis(10));
    /// let elapsed = guard.stop(); // Explicitly stop timing
    /// // duration will now be roughly 10 millis
    /// ```
    pub fn clear(&mut self) {
        self.duration.take();
        self.start.take();
    }
}

impl CloseValueRef for Stopwatch {
    type Closed = Option<Duration>;

    fn close_ref(&self) -> Self::Closed {
        match &self.duration {
            MaybeGuardedDuration::Exclusive(Some(duration)) => Some(*duration),
            MaybeGuardedDuration::Shared(mutex) => {
                let duration = mutex
                    .0
                    .lock()
                    .expect("owned timer guard panicked while holding lock");
                *duration
            }
            _ => self.start.as_ref().map(|start| start.elapsed()),
        }
    }
}

#[cfg(test)]
mod test {
    use std::time::{Duration, UNIX_EPOCH};

    use metrique_core::CloseValueRef;
    use metrique_timesource::{TimeSource, set_time_source};

    use crate::timers::Stopwatch;

    #[tokio::test(start_paused = true)]
    async fn stopwatch_can_start_multiple_times() {
        let _ts = set_time_source(TimeSource::tokio(UNIX_EPOCH));
        let mut stopwatch = Stopwatch::new();
        let guard = stopwatch.start();
        tokio::time::advance(Duration::from_secs(1)).await;
        drop(guard);
        assert_eq!(stopwatch.close_ref(), Some(Duration::from_secs(1)));
        let guard = stopwatch.start();
        tokio::time::advance(Duration::from_secs(3)).await;
        drop(guard);
        assert_eq!(stopwatch.close_ref(), Some(Duration::from_secs(4)));
    }

    #[tokio::test(start_paused = true)]
    async fn stopwatch_clear_works() {
        let _ts = set_time_source(TimeSource::tokio(UNIX_EPOCH));
        let mut stopwatch = Stopwatch::new();
        let guard = stopwatch.start();
        tokio::time::advance(Duration::from_secs(1)).await;
        drop(guard);
        assert_eq!(stopwatch.duration, Some(Duration::from_secs(1)));

        stopwatch.clear();
        assert_eq!(stopwatch.duration, None);
        assert!(stopwatch.start.is_none());

        let guard = stopwatch.start();
        tokio::time::advance(Duration::from_secs(1)).await;
        drop(guard);
        assert_eq!(stopwatch.close_ref(), Some(Duration::from_secs(1)));
    }

    #[tokio::test(start_paused = true)]
    async fn timer_guard_discard_works() {
        let _ts = set_time_source(TimeSource::tokio(UNIX_EPOCH));
        let mut stopwatch = Stopwatch::new();

        // load in one second
        let guard = stopwatch.start();
        tokio::time::advance(Duration::from_secs(1)).await;
        drop(guard);
        assert_eq!(stopwatch.duration, Some(Duration::from_secs(1)));

        let guard = stopwatch.start();
        tokio::time::advance(Duration::from_secs(2)).await;
        // discard the guard but leave existing data in place
        guard.discard();
        assert_eq!(stopwatch.duration, Some(Duration::from_secs(1)));
    }

    #[tokio::test(start_paused = true)]
    async fn timer_guard_overwrite_works() {
        let _ts = set_time_source(TimeSource::tokio(UNIX_EPOCH));
        let mut stopwatch = Stopwatch::new();

        // load in one second
        let guard = stopwatch.start();
        tokio::time::advance(Duration::from_secs(1)).await;
        drop(guard);
        assert_eq!(stopwatch.duration, Some(Duration::from_secs(1)));

        let guard = stopwatch.start();
        tokio::time::advance(Duration::from_secs(2)).await;
        // overwrite existing data iwth new data
        guard.overwrite();
        assert_eq!(stopwatch.duration, Some(Duration::from_secs(2)));
    }

    #[tokio::test(start_paused = true)]
    async fn owned_timer_guard_multiple_adds() {
        let _ts = set_time_source(TimeSource::tokio(UNIX_EPOCH));
        let mut stopwatch = Stopwatch::new();

        for _ in 0..3 {
            let _guard = stopwatch.start_owned();
            tokio::time::advance(Duration::from_secs(1)).await;
        }

        assert_eq!(stopwatch.duration, Some(Duration::from_secs(3)));
    }

    #[tokio::test(start_paused = true)]
    async fn owned_then_regular_timer_guard() {
        let _ts = set_time_source(TimeSource::tokio(UNIX_EPOCH));
        let mut stopwatch = Stopwatch::new();

        let owned_guard = stopwatch.start_owned();
        tokio::time::advance(Duration::from_secs(1)).await;
        owned_guard.stop();
        assert_eq!(stopwatch.duration, Some(Duration::from_secs(1)));

        let guard = stopwatch.start();
        tokio::time::advance(Duration::from_secs(2)).await;
        guard.stop();
        assert_eq!(stopwatch.duration, Some(Duration::from_secs(3)));
    }

    #[tokio::test(start_paused = true)]
    async fn owned_timer_guard_overwrite_works() {
        let _ts = set_time_source(TimeSource::tokio(UNIX_EPOCH));
        let mut stopwatch = Stopwatch::new();

        // load in one second
        let guard = stopwatch.start();
        tokio::time::advance(Duration::from_secs(1)).await;
        guard.stop();
        assert_eq!(stopwatch.duration, Some(Duration::from_secs(1)));

        let guard = stopwatch.start_owned();
        tokio::time::advance(Duration::from_secs(2)).await;
        // overwrite the loaded time with the guard contents
        guard.overwrite();
        assert_eq!(stopwatch.duration, Some(Duration::from_secs(2)));
    }

    #[tokio::test(start_paused = true)]
    async fn owned_timer_guard_discard_works() {
        let _ts = set_time_source(TimeSource::tokio(UNIX_EPOCH));
        let mut stopwatch = Stopwatch::new();

        // load in one second
        let guard = stopwatch.start();
        tokio::time::advance(Duration::from_secs(1)).await;
        drop(guard);
        assert_eq!(stopwatch.duration, Some(Duration::from_secs(1)));

        let guard = stopwatch.start_owned();
        tokio::time::advance(Duration::from_secs(2)).await;
        // discard the guard's contents but leave loaded time in place
        guard.discard();
        assert_eq!(stopwatch.duration, Some(Duration::from_secs(1)));
    }
}
