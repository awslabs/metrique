// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use crate::DropAll;
use crate::Guard;
use core::ops::Deref;
use core::ops::DerefMut;
use core::unreachable;
use metrique_core::CloseValue;
use tokio::sync::oneshot;

fn make_slot<T: CloseValue>(initial_value: T) -> (SlotGuard<T>, Waiting<T::Closed>) {
    let (tx, rx) = oneshot::channel();
    (
        SlotGuard {
            slot: SlotI::Writable {
                value: initial_value,
                tx,
            },
            parent_drop_mode: OnParentDrop::Discard,
        },
        Waiting { rx },
    )
}

/// [`Slot`] lets you split off a section of your metrics to be handled by another task
///
/// If you need to initialize a [`Slot`] but don't have an initial value yet, use [`LazySlot`].
///
/// It is often cumbersome to maintain a reference to the root metrics entry if you're handling
/// work in a separate tokio Task or thread. `Slot` enables handling that work in the background.
///
/// When you are ready to split off work, call [`Slot::open`] which will return a [`SlotGuard`].
///
/// When the [`SlotGuard`] is dropped, the contained record is [`closed`](CloseValue::close) and sent back to the parent.
/// This is helpful for patterns where [`crate::timers::TimestampOnClose`] is used to record the time a unit of work took.
///
/// If you need to clone around the contained entry and write to it using &self,
/// and you know that all background usages will complete before the parent entry flushes,
/// you can instead use the slightly cheaper [`crate::SharedChild`].
///
/// # Example
///
/// ```
/// use std::time::Duration;
/// use metrique::{Counter, OnParentDrop, ServiceMetrics, Slot, SlotGuard};
/// use metrique::unit_of_work::metrics;
/// use metrique::writer::GlobalEntrySink;
///
/// #[metrics(rename_all = "PascalCase")]
/// struct RequestMetrics {
///     operation: &'static str,
///     #[metrics(flatten)]
///     background_metrics: Slot<BackgroundMetrics>,
/// }
///
/// #[metrics(subfield)]
/// #[derive(Default)]
/// struct BackgroundMetrics {
///     field_1: usize,
///     counter: Counter,
/// }
///
/// async fn handle_request() {
///     let mut metrics = RequestMetrics {
///         operation: "abc",
///         background_metrics: Default::default(),
///     }
///     .append_on_drop(ServiceMetrics::sink());
///
///     let flush_guard = metrics.flush_guard();
///     // the flush_guard will delay the metric emission until dropped
///     // use OnParentDrop::Wait to wait until the `SlotGuard` is flushed.
///     let background_metrics = metrics
///         .background_metrics
///         .open(OnParentDrop::Wait(flush_guard))
///         .unwrap();
///
///     tokio::task::spawn(do_background_work(background_metrics));
///     // metric will be emitted after `do_background_work` completes
/// }
///
/// async fn do_background_work(mut metrics: SlotGuard<BackgroundMetrics>) {
///     // do some slow operation
///     tokio::time::sleep(Duration::from_secs(1)).await;
///     // `SlotGuard` derefs to the slot contents
///     metrics.field_1 += 1;
/// }
/// ```
pub struct Slot<T: CloseValue> {
    tx: Option<SlotGuard<T>>,
    rx: Option<Waiting<T::Closed>>,
    data: Option<T::Closed>,
}

/// Counterpart to Slot that can be initialized without immediately providing data
///
/// [`LazySlot::open`] returns a [`SlotGuard`], the same type returned by [`Slot`].
///
/// This is useful when you want to precisely control when a metric is created (for example, when you want to delay creating the)
/// metric until the segment of work starts to ensure accurate timestamps.
pub struct LazySlot<T: CloseValue> {
    slot: Option<Slot<T>>,
}

impl<T: CloseValue> Default for LazySlot<T> {
    fn default() -> Self {
        Self { slot: None }
    }
}

impl<T: CloseValue> CloseValue for LazySlot<T> {
    type Closed = Option<T::Closed>;

    fn close(self) -> Self::Closed {
        self.slot.and_then(|s| s.close())
    }
}

impl<T: CloseValue> LazySlot<T> {
    /// Open the slot and provie an initial value
    pub fn open(&mut self, initial_value: T, mode: OnParentDrop) -> Option<SlotGuard<T>> {
        if self.slot.is_some() {
            return None;
        }
        let mut slot = Slot::new(initial_value);
        let guard = slot
            .open(mode)
            .expect("unreachable: the slot is not opened twice");
        self.slot = Some(slot);
        Some(guard)
    }
}

/// Controls behavior when a parent metric record is dropped before a slot is closed.
///
/// This doesn't actually change the behavior of the [`Slot`] itself in any way, it just
/// provides a convenient way to hold a [`FlushGuard`] until the slot is closed.
///
/// This enum determines what happens when a parent metric record containing a `Slot`
/// is dropped before the `SlotGuard` for that slot is dropped.
pub enum OnParentDrop {
    /// Delay flushing the parent record until this slot is closed
    ///
    /// NOTE: this does not actually cause dropping the parent to be delayed.
    Wait(FlushGuard),

    /// If the parent is dropped before the slot closes, discard any data in this slot
    ///
    /// You can use [`SlotGuard::parent_is_closed`] to determine if the parent has been closed already.
    Discard,
}

impl<T: CloseValue> Slot<T> {
    /// Create a new slot directly. Used mostly if your inner type T doesn't implement Default
    pub fn new(value: T) -> Self {
        let (tx, rx) = make_slot(value);
        Self {
            tx: Some(tx),
            rx: Some(rx),
            data: None,
        }
    }

    #[doc(hidden)]
    #[deprecated(note = "Use Slot::open instead to explicitly chose the on drop behavior.")]
    pub fn open_slot(&mut self) -> Option<SlotGuard<T>> {
        self.tx.take()
    }

    /// Open a slot, providing an owned [`SlotGuard`] that can be sent to a background task.
    ///
    /// When the [`SlotGuard`] is dropped, it will be written back into the parent entry.
    ///
    /// Depending on the provided [`mode`](OnParentDrop), if the parent has already been dropped it will either:
    /// - Delay flushing the record to the queue until this `SlotGuard` is dropped ([`OnParentDrop::Wait`])
    /// - Discard the contents of this slot ([`OnParentDrop::Discard`])
    ///
    /// If a `SlotGuard` has already been opened for this slot, this returns None.
    pub fn open(&mut self, mode: OnParentDrop) -> Option<SlotGuard<T>> {
        let mut slot = self.tx.take();
        if let Some(slot) = slot.as_mut() {
            slot.parent_drop_mode = mode;
        }

        slot
    }

    /// Wait until the child [`SlotGuard`] closes (or panics, in which case any contained fields are dropped from your entry).
    ///
    /// Returns a mutable reference to the inner data if its guard didn't panic, or else None
    pub async fn wait_for_data(&mut self) -> &mut Option<T::Closed> {
        if let Some(rx) = self.rx.take() {
            self.data = rx.wait_for_value().await;
        }
        &mut self.data
    }
}

impl<T: Default + CloseValue> Default for Slot<T> {
    fn default() -> Self {
        Self::new(T::default())
    }
}

#[diagnostic::do_not_recommend]
impl<T: CloseValue> CloseValue for Slot<T> {
    type Closed = Option<T::Closed>;

    fn close(self) -> Self::Closed {
        match (self.data, self.rx) {
            (Some(data), _) => Some(data),
            (_, Some(rx)) => rx.take_value(),
            // TODO: refactor to enum to avoid this branch
            _ => unreachable!("cannot enter this state"),
        }
    }
}

/// A container for waiting on a value from a `SlotGuard`.
///
/// This struct is used internally by `Slot` to wait for a value to be sent back
/// from a `SlotGuard` when it is dropped.
struct Waiting<T> {
    rx: oneshot::Receiver<T>,
}

impl<T> Waiting<T> {
    /// Attempts to take the value without waiting.
    ///
    /// Returns `Some(T)` if the value is available, or `None` if the sender
    /// has not yet sent a value or has been dropped.
    fn take_value(mut self) -> Option<T> {
        self.rx.try_recv().ok()
    }

    /// Waits asynchronously for the value to be available.
    ///
    /// Returns `Some(T)` if the value is received, or `None` if the sender
    /// was dropped without sending a value.
    async fn wait_for_value(self) -> Option<T> {
        self.rx.await.ok()
    }
}

/// A guard for a slot that can be sent to another task.
///
/// This struct holds a value that can be modified and will be sent back to the
/// parent `Slot` when dropped. It is typically created by calling `Slot::open`.
///
/// The guard can be sent to another task, allowing that task to modify the value
/// and have those modifications reflected in the parent metric record when the
/// guard is dropped.
pub struct SlotGuard<T: CloseValue> {
    slot: SlotI<T>,
    parent_drop_mode: OnParentDrop,
}

impl<T: CloseValue> SlotGuard<T> {
    /// Check if the `Slot` is still open
    ///
    /// If the parent side of the `Slot` has already been dropped, this function will return false
    pub fn parent_is_closed(&self) -> bool {
        match &self.slot {
            SlotI::Writable { tx, .. } => tx.is_closed(),
            SlotI::Dropped => unreachable!("this state is only entered after drop"),
        }
    }

    /// Pass the parent's flush guard in to instruct the parent entry to wait to close
    /// until this slot drops.
    pub fn delay_flush(&mut self, flush_guard: FlushGuard) {
        self.parent_drop_mode = OnParentDrop::Wait(flush_guard);
    }
}

/// A `FlushGuard` allows delaying flushing a metrics entry until a future point when this is dropped
///
/// A `FlushGuard` is obtained by calling `flush_guard` on `AppendAndCloseOnDrop`
pub struct FlushGuard {
    pub(crate) _drop_guard: Guard,
}

/// The counterpart to `FlushGuard`:
///
/// If you create a `ForceFlushGuard` and drop it, all existing `FlushGuard`s are ignored and the entry
/// is flushed (provided the root entry has already been dropped).
pub struct ForceFlushGuard {
    pub(crate) _drop_guard: DropAll,
}

impl<T: CloseValue> Deref for SlotGuard<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        match &self.slot {
            SlotI::Writable { value, .. } => value,
            SlotI::Dropped => unreachable!("only occurs after drop"),
        }
    }
}

impl<T: CloseValue> DerefMut for SlotGuard<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        match &mut self.slot {
            SlotI::Writable { value, .. } => value,
            SlotI::Dropped => unreachable!("only set after drop"),
        }
    }
}

impl<T: CloseValue> Drop for SlotGuard<T> {
    fn drop(&mut self) {
        if let SlotI::Writable { value, tx } = std::mem::replace(&mut self.slot, SlotI::Dropped) {
            // send the value back to the parent
            let _ = tx.send(value.close());
        } else {
            unreachable!("move out of slot must only occur during drop")
        }
    }
}

enum SlotI<T: CloseValue> {
    Writable {
        value: T,
        tx: oneshot::Sender<T::Closed>,
    },
    Dropped,
}

#[cfg(test)]
mod test {
    use metrique_core::CloseValue;

    use crate::Slot;

    use super::{LazySlot, OnParentDrop};

    #[derive(Default)]
    struct TestCloseable;
    impl CloseValue for TestCloseable {
        type Closed = usize;

        fn close(self) -> Self::Closed {
            42
        }
    }

    #[test]
    fn test_double_open_lazy() {
        let mut slot: LazySlot<TestCloseable> = LazySlot::default();
        let _guard = slot
            .open(TestCloseable, OnParentDrop::Discard)
            .expect("open once");
        assert!(slot.open(TestCloseable, OnParentDrop::Discard).is_none());
    }

    #[test]
    fn test_double_open() {
        let mut slot: Slot<TestCloseable> = Slot::default();
        let _guard = slot.open(OnParentDrop::Discard).expect("open once");
        assert!(slot.open(OnParentDrop::Discard).is_none());
    }

    #[tokio::test]
    async fn test_wait_for_data() {
        let mut slot: Slot<TestCloseable> = Slot::default();
        drop(slot.open(OnParentDrop::Discard));
        assert_eq!(slot.wait_for_data().await, &Some(42));
    }

    #[test]
    fn test_parent_is_closed() {
        let mut slot: Slot<TestCloseable> = Slot::default();
        let guard = slot.open(OnParentDrop::Discard).unwrap();
        assert_eq!(guard.parent_is_closed(), false);
        drop(slot);
        assert_eq!(guard.parent_is_closed(), true);
    }
}
