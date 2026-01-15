// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! All default implementations of CloseValue, grouped for clarity

use core::time::Duration;
use std::marker::PhantomData;
use std::sync::{Arc, MutexGuard};
use std::time::SystemTime;
use std::{borrow::Cow, sync::Mutex};

use metrique_writer_core::EntryWriter;
use metrique_writer_core::value::WithDimensions;
use metrique_writer_core::value::{FlagConstructor, ForceFlag};

use crate::{CloseValue, CloseValueRef, InflectableEntry};

macro_rules! close_value_ref {
    ($($type:ty),+) => {
        $(
            impl $crate::CloseValue for &'_ $type {
                type Closed = $type;
                fn close(self) -> Self::Closed {
                    *self
                }
            }
            impl $crate::CloseValue for $type {
                type Closed = $type;
                fn close(self) -> Self::Closed {
                    self
                }
            }
        )+
    };
}

macro_rules! close_value {
    ($($type:ty),+) => {
        $(
            impl $crate::CloseValue for $type {
                type Closed = $type;
                fn close(self) -> Self::Closed {
                    self
                }
            }
        )+
    };
}

// We have all of these manual impls to avoid the coherence issues we would have from having a blanket impl.
// This allows us to have specific impls for things like `WithDimensions`

close_value_ref!(
    bool, Duration, f32, f64, u16, u32, u64, u8, usize, SystemTime
);

close_value!(String);

#[diagnostic::do_not_recommend]
impl<'a> CloseValue for &'a str {
    type Closed = &'a str;

    fn close(self) -> Self::Closed {
        self
    }
}

#[diagnostic::do_not_recommend]
impl<'a> CloseValue for &&'a str {
    type Closed = &'a str;

    fn close(self) -> Self::Closed {
        *self
    }
}

#[diagnostic::do_not_recommend]
impl CloseValue for &Arc<String> {
    type Closed = Arc<String>;

    fn close(self) -> Self::Closed {
        self.clone()
    }
}

impl CloseValue for Arc<String> {
    type Closed = Arc<String>;

    fn close(self) -> Self::Closed {
        self
    }
}

#[diagnostic::do_not_recommend]
impl<'a, T: ToOwned + ?Sized> CloseValue for Cow<'a, T> {
    type Closed = Cow<'a, T>;

    fn close(self) -> Self::Closed {
        self
    }
}

#[diagnostic::do_not_recommend]
impl<T, C> CloseValue for &'_ Arc<T>
where
    T: CloseValueRef<Closed = C>,
{
    type Closed = C;

    fn close(self) -> Self::Closed {
        T::close_ref(self)
    }
}

#[diagnostic::do_not_recommend]
impl<T, C> CloseValue for Arc<T>
where
    T: CloseValueRef<Closed = C>,
{
    type Closed = C;

    fn close(self) -> Self::Closed {
        T::close_ref(&self)
    }
}

#[diagnostic::do_not_recommend]
impl<T, C> CloseValue for &'_ MutexGuard<'_, T>
where
    T: CloseValueRef<Closed = C>,
{
    type Closed = C;

    fn close(self) -> Self::Closed {
        T::close_ref(self)
    }
}

#[diagnostic::do_not_recommend]
impl<T, C> CloseValue for MutexGuard<'_, T>
where
    T: CloseValueRef<Closed = C>,
{
    type Closed = C;

    fn close(self) -> Self::Closed {
        T::close_ref(&self)
    }
}

#[diagnostic::do_not_recommend]
impl<T, C> CloseValue for Mutex<T>
where
    T: CloseValueRef<Closed = C>,
{
    type Closed = Option<C>;

    fn close(self) -> Self::Closed {
        self.close_ref()
    }
}

#[diagnostic::do_not_recommend]
impl<T, C> CloseValue for &'_ Mutex<T>
where
    T: CloseValueRef<Closed = C>,
{
    type Closed = Option<C>;

    fn close(self) -> Self::Closed {
        Some(self.lock().ok()?.close())
    }
}

#[diagnostic::do_not_recommend]
impl<T: CloseValue> CloseValue for Option<T> {
    type Closed = Option<T::Closed>;

    fn close(self) -> Self::Closed {
        self.map(|v| v.close())
    }
}

#[diagnostic::do_not_recommend]
impl<T> CloseValue for &'_ Option<T>
where
    T: CloseValueRef,
{
    type Closed = Option<T::Closed>;

    fn close(self) -> Self::Closed {
        self.as_ref().map(|v| v.close_ref())
    }
}

#[diagnostic::do_not_recommend]
impl<T: CloseValue, const N: usize> CloseValue for WithDimensions<T, N> {
    type Closed = WithDimensions<T::Closed, N>;

    fn close(self) -> Self::Closed {
        self.map_value(|v| v.close())
    }
}

// no by-ref impl for WithDimensions due to not wanting to implicitly clone the dimensions

#[diagnostic::do_not_recommend]
impl<T: CloseValue, F: FlagConstructor> CloseValue for ForceFlag<T, F> {
    type Closed = ForceFlag<T::Closed, F>;

    fn close(self) -> Self::Closed {
        self.map_value(|v| v.close())
    }
}

struct ForceFlagEntryWriter<'a, W, FLAGS: FlagConstructor> {
    writer: &'a mut W,
    phantom: PhantomData<FLAGS>,
}

impl<'a, W: EntryWriter<'a>, FLAGS: FlagConstructor> EntryWriter<'a>
    for ForceFlagEntryWriter<'_, W, FLAGS>
{
    fn timestamp(&mut self, timestamp: std::time::SystemTime) {
        self.writer.timestamp(timestamp)
    }

    fn value(
        &mut self,
        name: impl Into<std::borrow::Cow<'a, str>>,
        value: &(impl metrique_writer_core::Value + ?Sized),
    ) {
        self.writer.value(name, &ForceFlag::<_, FLAGS>::from(value))
    }

    fn config(&mut self, config: &'a dyn metrique_writer_core::EntryConfig) {
        self.writer.config(config);
    }
}

#[diagnostic::do_not_recommend]
impl<T: CloseValueRef, F: FlagConstructor> CloseValue for &'_ ForceFlag<T, F> {
    type Closed = ForceFlag<T::Closed, F>;

    fn close(self) -> Self::Closed {
        self.map_value_ref(|v| v.close_ref())
    }
}

#[diagnostic::do_not_recommend]
impl<NS: crate::NameStyle, T: InflectableEntry<NS>, F: FlagConstructor> InflectableEntry<NS>
    for ForceFlag<T, F>
{
    fn write<'a>(&'a self, writer: &mut impl metrique_writer_core::EntryWriter<'a>) {
        <T as InflectableEntry<NS>>::write(
            self,
            &mut ForceFlagEntryWriter {
                writer,
                phantom: PhantomData::<F>,
            },
        );
    }
}

#[diagnostic::do_not_recommend]
impl<NS: crate::NameStyle, T: InflectableEntry<NS>, const N: usize> InflectableEntry<NS>
    for WithDimensions<T, N>
{
    fn write<'a>(&'a self, writer: &mut impl metrique_writer_core::EntryWriter<'a>) {
        <T as InflectableEntry<NS>>::write(self, &mut self.entry_writer_wrapper(writer))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use metrique_writer_core::value::WithDimensions;

    use crate::CloseValue;

    struct Closeable;
    impl CloseValue for Closeable {
        type Closed = usize;

        fn close(self) -> Self::Closed {
            42
        }
    }

    impl CloseValue for &'_ Closeable {
        type Closed = usize;

        fn close(self) -> Self::Closed {
            42
        }
    }

    #[test]
    fn close_option() {
        let x = Some(Closeable);
        assert_eq!(x.close(), Some(42));
    }

    #[test]
    fn close_arc() {
        let x = Arc::new(Closeable);
        assert_eq!(x.close(), 42);
    }

    #[test]
    fn close_arc_mutex() {
        let x = Arc::new(Mutex::new(Closeable));
        assert_eq!(x.close(), Some(42));
    }

    #[test]
    fn close_arc_mutex_poisoned() {
        let x = Arc::new(Mutex::new(Closeable));
        let x_cloned = x.clone();
        let _ = std::thread::spawn(move || {
            let _guard = x_cloned.lock();
            panic!();
        })
        .join();
        assert_eq!(x.close(), None);
    }

    #[test]
    fn close_with_dimensions() {
        let v: WithDimensions<Closeable, 1> = WithDimensions::new(Closeable, "foo", "bar");
        let closed = v.close();
        assert_eq!(*closed, 42);
    }
}
