// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::{
    future::Future,
    marker::PhantomData,
    ops::{AddAssign, DerefMut},
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll},
    time::{Duration, UNIX_EPOCH},
};

use metrique_core::CloseValue;
use metrique_timesource::{Instant, SystemTime, TimeSource, time_source};
use metrique_writer_core::{
    Value,
    unit::{Millisecond, Second},
};
use metrique_writer_core::{unit::Microsecond, value::ValueFormatter};
use timestamp_to_str::TimestampToStr;

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

impl CloseValue for &'_ Timestamp {
    type Closed = TimestampValue;

    fn close(self) -> Self::Closed {
        TimestampValue::new(&self.time)
    }
}

impl CloseValue for Timestamp {
    type Closed = TimestampValue;

    fn close(self) -> Self::Closed {
        <&Self>::close(&self)
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
#[derive(Debug)]
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
#[derive(Copy, Clone, Debug)]
pub struct TimestampValue {
    duration_since_epoch: Duration,
}

impl Value for TimestampValue {
    const SHAPE: metrique_writer_core::FieldShape<'static> =
        metrique_writer_core::FieldShape::Known(metrique_writer_core::KnownShape::String);
    const UNIT: metrique_writer_core::Unit = metrique_writer_core::Unit::None;

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

#[doc(hidden)]
pub struct TimestampFormat<Unit> {
    u: PhantomData<Unit>,
}

impl<U: TimestampToStr> ValueFormatter<TimestampValue> for TimestampFormat<U> {
    const SHAPE: metrique_writer_core::descriptor::FieldShape<'static> =
        metrique_writer_core::descriptor::FieldShape::Known(
            metrique_writer_core::descriptor::KnownShape::String,
        );

    fn format_value(writer: impl metrique_writer_core::ValueWriter, value: &TimestampValue) {
        U::to_str(value.duration_since_epoch, |s| writer.string(s));
    }
}

/// Timestamps must be formatted as strings
mod timestamp_to_str {
    use std::time::Duration;

    use metrique_writer_core::unit::{Microsecond, Millisecond, Second};

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
}

/// Timers record the elapsed time of an operation within a wide event
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

    /// Creates a timer that is already stopped at exactly `duration`.
    ///
    /// Useful when you have measured a duration yourself (or via [`Timed`]) and want to
    /// record it in a `Timer` field.
    ///
    /// # Example
    /// ```
    /// # use metrique::timers::Timer;
    /// # use std::time::Duration;
    /// let mut timer = Timer::from_duration(Duration::from_millis(5));
    /// assert_eq!(timer.stop(), Duration::from_millis(5));
    /// ```
    pub fn from_duration(duration: Duration) -> Self {
        Self {
            start: time_source().instant(),
            duration: Some(duration),
        }
    }

    /// Stops the timer and returns the elapsed duration.
    ///
    /// After calling this method, the timer will no longer update and will report
    /// the same duration when closed.
    ///
    /// Calling `stop` on a stopped timer is idempotent, and returns the
    /// timer's stopped duration.
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
        if let Some(duration) = self.duration {
            return duration;
        }

        let time = self.start.elapsed();
        self.duration = Some(time);
        time
    }
}

impl CloseValue for &'_ Timer {
    type Closed = Duration;

    fn close(self) -> Self::Closed {
        self.duration.unwrap_or_else(|| self.start.elapsed())
    }
}

impl CloseValue for Timer {
    type Closed = Duration;

    fn close(self) -> Self::Closed {
        <&Self>::close(&self)
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

    /// Adds an already-measured `duration` to this stopwatch.
    ///
    /// # Example
    /// ```
    /// # use metrique::timers::Stopwatch;
    /// # use std::time::Duration;
    /// let mut stopwatch = Stopwatch::new();
    /// stopwatch.record(Duration::from_millis(5));
    /// stopwatch.record(Duration::from_millis(5));
    /// // stopwatch now contains 10ms
    /// ```
    pub fn record(&mut self, duration: Duration) {
        self.duration += duration;
    }

    /// Runs `f`, adding the time it took to this stopwatch, and returns its result.
    ///
    /// The time is recorded even if `f` panics.
    ///
    /// # Example
    /// ```
    /// # use metrique::timers::Stopwatch;
    /// let mut stopwatch = Stopwatch::new();
    /// let answer = stopwatch.time(|| 40 + 2);
    /// assert_eq!(answer, 42);
    /// ```
    pub fn time<T>(&mut self, f: impl FnOnce() -> T) -> T {
        let _guard = self.start();
        f()
    }

    /// Awaits `fut`, adding the time it took to this stopwatch, and returns its output.
    ///
    /// Timing starts when this future is first polled. If the future is dropped before
    /// completing (e.g. cancelled), the time elapsed so far is still recorded.
    ///
    /// For timing a future you don't want to borrow the stopwatch for, see [`TimedFutureExt::timed`].
    ///
    /// # Example
    /// ```
    /// # use metrique::timers::Stopwatch;
    /// # async fn do_io() -> u32 { 1 }
    /// # async fn example() {
    /// let mut stopwatch = Stopwatch::new();
    /// let value = stopwatch.time_async(do_io()).await;
    /// # }
    /// ```
    pub async fn time_async<F: Future>(&mut self, fut: F) -> F::Output {
        let _guard = self.start();
        fut.await
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

impl CloseValue for &'_ Stopwatch {
    type Closed = Option<Duration>;

    fn close(self) -> Self::Closed {
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
impl CloseValue for Stopwatch {
    type Closed = Option<Duration>;

    fn close(self) -> Self::Closed {
        <&Self>::close(&self)
    }
}

/// The output of a completed operation, paired with how long it took.
///
/// Produced by [`TimedFutureExt::timed`] and [`Timed::blocking`]. The duration is frozen at the
/// moment the operation finished, not when the parent metric is closed. Use
/// [`Timed::split_time_to`] / [`Timed::record_to`] to park the duration in a metric field
/// and get the operation's output back, or [`Timed::into_parts`] when the same duration should
/// feed more than one field.
///
/// # Examples
/// ```
/// use metrique::timers::{Timed, Timer};
/// use metrique::unit_of_work::metrics;
///
/// #[metrics]
/// #[derive(Default)]
/// struct MyMetric {
///     expensive_op: Option<Timer>,
/// }
///
/// let mut m = MyMetric::default();
/// let value = Timed::blocking(|| 40 + 2).split_time_to(&mut m.expensive_op);
/// assert_eq!(value, 42);
/// assert!(m.expensive_op.is_some());
/// ```
///
/// Timing a future:
/// ```
/// use metrique::timers::{Timer, TimedFutureExt};
/// use metrique::unit_of_work::metrics;
///
/// #[metrics]
/// #[derive(Default)]
/// struct MyMetric {
///     expensive_op: Option<Timer>,
/// }
///
/// # async fn do_some_expensive_op() -> u32 { 42 }
/// # async fn example() {
/// let mut m = MyMetric::default();
/// let value = do_some_expensive_op().timed().await.split_time_to(&mut m.expensive_op);
/// # }
/// ```
#[derive(Debug)]
#[must_use = "the measured duration is lost unless it is recorded with `split_time_to`, `record_to` or `into_parts`"]
pub struct Timed<T> {
    result: T,
    duration: Duration,
}

impl<T> Timed<T> {
    /// Runs `f` and returns its result together with the time it took.
    ///
    /// This is the synchronous counterpart to [`TimedFutureExt::timed`].
    pub fn blocking(f: impl FnOnce() -> T) -> Self {
        let start = time_source().instant();
        let result = f();
        Self {
            result,
            duration: start.elapsed(),
        }
    }

    /// Like [`Timed::blocking`], but measures with an explicit [`TimeSource`].
    pub fn blocking_with_timesource(time_source: TimeSource, f: impl FnOnce() -> T) -> Self {
        let start = time_source.instant();
        let result = f();
        Self {
            result,
            duration: start.elapsed(),
        }
    }

    /// The measured duration.
    pub fn duration(&self) -> Duration {
        self.duration
    }

    /// Borrow the operation's output.
    pub fn result(&self) -> &T {
        &self.result
    }

    /// Return the operation's output alongside the elapsed duration, for when one
    /// interval feeds more than one metric field.
    pub fn into_parts(self) -> (T, Duration) {
        (self.result, self.duration)
    }

    /// Park the (stopped) duration in an `Option<Timer>` metric field, replacing any previous
    /// value, and return the operation's output.
    pub fn split_time_to(self, mut timer_field: impl DerefMut<Target = Option<Timer>>) -> T {
        timer_field.replace(Timer::from_duration(self.duration));
        self.result
    }

    /// Add the duration to a [`Stopwatch`] and return the operation's output.
    pub fn record_to(self, stopwatch: &mut Stopwatch) -> T {
        stopwatch.record(self.duration);
        self.result
    }
}

impl<T, E> Timed<Result<T, E>> {
    /// Park the duration in `field` only if the operation succeeded, and return the `Result`.
    ///
    /// On `Err` the field is left untouched.
    ///
    /// # Example
    /// ```
    /// use metrique::timers::{Timed, Timer};
    /// let mut ok_latency: Option<Timer> = None;
    /// let r: Result<u32, ()> = Timed::blocking(|| Ok(1)).split_time_to_if_ok(&mut ok_latency);
    /// assert!(ok_latency.is_some());
    /// ```
    pub fn split_time_to_if_ok(
        self,
        mut field: impl DerefMut<Target = Option<Timer>>,
    ) -> Result<T, E> {
        if self.result.is_ok() {
            field.replace(Timer::from_duration(self.duration));
        }
        self.result
    }

    /// Park the duration in `ok_field` if the operation succeeded, or `err_field` if it failed,
    /// and return the `Result`. The other field is left untouched.
    ///
    /// # Example
    /// ```
    /// use metrique::timers::{Timed, Timer};
    /// let mut ok_latency: Option<Timer> = None;
    /// let mut err_latency: Option<Timer> = None;
    /// let r: Result<(), &str> = Timed::blocking(|| Err("boom"))
    ///     .split_time_to_result(&mut ok_latency, &mut err_latency);
    /// assert!(ok_latency.is_none());
    /// assert!(err_latency.is_some());
    /// ```
    pub fn split_time_to_result(
        self,
        mut ok_field: impl DerefMut<Target = Option<Timer>>,
        mut err_field: impl DerefMut<Target = Option<Timer>>,
    ) -> Result<T, E> {
        let timer = Timer::from_duration(self.duration);
        match self.result {
            Ok(_) => ok_field.replace(timer),
            Err(_) => err_field.replace(timer),
        };
        self.result
    }

    /// Add the duration to `stopwatch` only if the operation succeeded, and return the `Result`.
    pub fn record_to_if_ok(self, stopwatch: &mut Stopwatch) -> Result<T, E> {
        if self.result.is_ok() {
            stopwatch.record(self.duration);
        }
        self.result
    }
}

pin_project_lite::pin_project! {
    /// Future returned by [`TimedFutureExt::timed`].
    ///
    /// Timing starts when the future is first polled.
    #[must_use = "futures do nothing unless you `.await` or poll them"]
    pub struct TimedFuture<F> {
        #[pin]
        inner: F,
        time_source: Option<TimeSource>,
        start: Option<Instant>,
    }
}

impl<F: Future> Future for TimedFuture<F> {
    type Output = Timed<F::Output>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        let start = this.start.get_or_insert_with(|| match this.time_source {
            Some(ts) => ts.instant(),
            None => time_source().instant(),
        });
        match this.inner.poll(cx) {
            Poll::Ready(result) => Poll::Ready(Timed {
                result,
                duration: start.elapsed(),
            }),
            Poll::Pending => Poll::Pending,
        }
    }
}

/// Extension trait for timing a future.
///
/// Implemented for every [`Future`].
pub trait TimedFutureExt: Future + Sized {
    /// Time this future. Timing starts when it is first polled, and the resulting duration is
    /// frozen when the future resolves. See [`Timed`] for how to record it.
    fn timed(self) -> TimedFuture<Self> {
        TimedFuture {
            inner: self,
            time_source: None,
            start: None,
        }
    }

    /// Like [`TimedFutureExt::timed`], but measures with an explicit [`TimeSource`] instead of the
    /// global one. Useful for tests and for code that carries its own clock.
    ///
    /// # Example
    /// ```
    /// # use metrique::timers::TimedFutureExt;
    /// # use metrique_timesource::time_source;
    /// # async fn example() {
    /// let timed = async { 1 }.timed_with_timesource(time_source()).await;
    /// # }
    /// ```
    fn timed_with_timesource(self, time_source: TimeSource) -> TimedFuture<Self> {
        TimedFuture {
            inner: self,
            time_source: Some(time_source),
            start: None,
        }
    }
}

impl<F: Future> TimedFutureExt for F {}

#[cfg(test)]
mod test {
    use std::time::{Duration, UNIX_EPOCH};

    use metrique_core::CloseValue;
    use metrique_timesource::{TimeSource, set_time_source};

    use crate::timers::{Stopwatch, Timed, TimedFutureExt, Timer};

    #[tokio::test(start_paused = true)]
    async fn timer_stop_is_idempotent() {
        let _ts = set_time_source(TimeSource::tokio(UNIX_EPOCH));
        let mut timer = Timer::start_now();

        tokio::time::sleep(Duration::from_millis(1)).await;
        let first_stop = timer.stop();
        tokio::time::sleep(Duration::from_millis(1)).await;
        let second_stop = timer.stop();

        assert_eq!(first_stop, second_stop);
    }

    #[tokio::test(start_paused = true)]
    async fn stopwatch_can_start_multiple_times() {
        let _ts = set_time_source(TimeSource::tokio(UNIX_EPOCH));
        let mut stopwatch = Stopwatch::new();
        let guard = stopwatch.start();
        tokio::time::advance(Duration::from_secs(1)).await;
        drop(guard);
        assert_eq!((&stopwatch).close(), Some(Duration::from_secs(1)));
        let guard = stopwatch.start();
        tokio::time::advance(Duration::from_secs(3)).await;
        drop(guard);
        assert_eq!((&stopwatch).close(), Some(Duration::from_secs(4)));
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
        assert_eq!((&stopwatch).close(), Some(Duration::from_secs(1)));
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

    #[tokio::test(start_paused = true)]
    async fn stopwatch_time_and_record() {
        let _ts = set_time_source(TimeSource::tokio(UNIX_EPOCH));
        let mut stopwatch = Stopwatch::new();
        let out = stopwatch.time(|| {
            std::thread::sleep(Duration::from_millis(1));
            7
        });
        assert_eq!(out, 7);
        assert!((&stopwatch).close().is_some());
        stopwatch.clear();
        stopwatch.record(Duration::from_secs(1));
        stopwatch.record(Duration::from_secs(2));
        assert_eq!((&stopwatch).close(), Some(Duration::from_secs(3)));
    }

    #[tokio::test(start_paused = true)]
    async fn stopwatch_time_async_accumulates() {
        let _ts = set_time_source(TimeSource::tokio(UNIX_EPOCH));
        let mut stopwatch = Stopwatch::new();
        let v = stopwatch
            .time_async(async {
                tokio::time::sleep(Duration::from_secs(1)).await;
                5
            })
            .await;
        assert_eq!(v, 5);
        stopwatch
            .time_async(tokio::time::sleep(Duration::from_secs(2)))
            .await;
        assert_eq!((&stopwatch).close(), Some(Duration::from_secs(3)));
    }

    #[tokio::test(start_paused = true)]
    async fn stopwatch_time_async_records_on_cancel() {
        let _ts = set_time_source(TimeSource::tokio(UNIX_EPOCH));
        let mut stopwatch = Stopwatch::new();
        let _ = tokio::time::timeout(
            Duration::from_secs(1),
            stopwatch.time_async(tokio::time::sleep(Duration::from_secs(10))),
        )
        .await;
        assert_eq!((&stopwatch).close(), Some(Duration::from_secs(1)));
    }

    #[tokio::test(start_paused = true)]
    async fn timed_future_split_time_to_option_timer() {
        let _ts = set_time_source(TimeSource::tokio(UNIX_EPOCH));
        let mut field: Option<Timer> = None;
        let v = async {
            tokio::time::sleep(Duration::from_secs(2)).await;
            "done"
        }
        .timed()
        .await
        .split_time_to(&mut field);
        assert_eq!(v, "done");
        // time passing after the future resolved must not affect the recorded value
        tokio::time::advance(Duration::from_secs(5)).await;
        assert_eq!(field.close(), Some(Duration::from_secs(2)));
    }

    #[tokio::test(start_paused = true)]
    async fn timed_future_starts_on_first_poll() {
        let _ts = set_time_source(TimeSource::tokio(UNIX_EPOCH));
        // the sleep is created lazily on first poll, so its deadline is relative to the first poll
        let fut = async { tokio::time::sleep(Duration::from_secs(1)).await }.timed();
        tokio::time::advance(Duration::from_secs(10)).await;
        let (_, d) = fut.await.into_parts();
        assert_eq!(d, Duration::from_secs(1));
    }

    #[tokio::test(start_paused = true)]
    async fn timed_record_to_stopwatch() {
        let _ts = set_time_source(TimeSource::tokio(UNIX_EPOCH));
        let mut stopwatch = Stopwatch::new();
        for _ in 0..2 {
            tokio::time::sleep(Duration::from_secs(1))
                .timed()
                .await
                .record_to(&mut stopwatch);
        }
        assert_eq!((&stopwatch).close(), Some(Duration::from_secs(2)));
    }

    #[test]
    fn timed_blocking_returns_result_and_duration() {
        let t = Timed::blocking(|| 3);
        assert_eq!(*t.result(), 3);
        let (v, _d) = t.into_parts();
        assert_eq!(v, 3);
    }

    #[tokio::test(start_paused = true)]
    async fn timed_with_explicit_timesource() {
        // no global time source set: the explicit one must be used
        let ts = TimeSource::tokio(UNIX_EPOCH);
        let t = async { tokio::time::sleep(Duration::from_secs(3)).await }
            .timed_with_timesource(ts.clone())
            .await;
        assert_eq!(t.duration(), Duration::from_secs(3));
        let t = Timed::blocking_with_timesource(ts, || ());
        assert_eq!(t.duration(), Duration::ZERO);
    }

    #[tokio::test(start_paused = true)]
    async fn timed_result_routing() {
        let _ts = set_time_source(TimeSource::tokio(UNIX_EPOCH));
        let (mut ok, mut err): (Option<Timer>, Option<Timer>) = (None, None);

        let r = async {
            tokio::time::sleep(Duration::from_secs(1)).await;
            Ok::<_, &str>(1)
        }
        .timed()
        .await
        .split_time_to_result(&mut ok, &mut err);
        assert_eq!(r, Ok(1));
        assert_eq!(ok.take().close(), Some(Duration::from_secs(1)));
        assert!(err.is_none());

        let r = async {
            tokio::time::sleep(Duration::from_secs(2)).await;
            Err::<u32, _>("boom")
        }
        .timed()
        .await
        .split_time_to_result(&mut ok, &mut err);
        assert_eq!(r, Err("boom"));
        assert!(ok.is_none());
        assert_eq!(err.close(), Some(Duration::from_secs(2)));
    }

    #[tokio::test(start_paused = true)]
    async fn timed_if_ok_skips_errors() {
        let _ts = set_time_source(TimeSource::tokio(UNIX_EPOCH));
        let mut field: Option<Timer> = None;
        let mut sw = Stopwatch::new();

        let _ = async { Err::<(), _>("x") }
            .timed()
            .await
            .split_time_to_if_ok(&mut field);
        let _ = async { Err::<(), _>("x") }
            .timed()
            .await
            .record_to_if_ok(&mut sw);
        assert!(field.is_none());
        assert_eq!((&sw).close(), None);

        let _ = async {
            tokio::time::sleep(Duration::from_secs(1)).await;
            Ok::<_, ()>(())
        }
        .timed()
        .await
        .record_to_if_ok(&mut sw);
        assert_eq!((&sw).close(), Some(Duration::from_secs(1)));
    }
}
