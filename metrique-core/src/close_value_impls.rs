// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! All default implementations of CloseValue, grouped for clarity

use core::time::Duration;
use std::sync::{Arc, MutexGuard};
use std::time::SystemTime;
use std::{borrow::Cow, sync::Mutex};

use metrique_writer_core::value::WithDimensions;
use metrique_writer_core::value::{FlagConstructor, ForceFlag};

use crate::{CloseValue, CloseValueRef};

macro_rules! close_value_ref {
    ($($type:ty),+) => {
        $(
            impl $crate::CloseValueRef for $type {
                type Closed = $type;
                fn close_ref(&self) -> Self::Closed {
                    *self
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
    bool,
    Duration,
    f32,
    f64,
    u16,
    u32,
    u64,
    u8,
    usize,
    &'static str,
    SystemTime
);

close_value!(
    String,
    // I believe a generic implementation for Cow is possible but the trait bounds are nontrivial. I imagine this is the most common use case.
    Cow<'static, str>
);

#[diagnostic::do_not_recommend]
impl<T: CloseValueRef> CloseValueRef for Arc<T> {
    type Closed = T::Closed;

    fn close_ref(&self) -> Self::Closed {
        self.as_ref().close_ref()
    }
}

#[diagnostic::do_not_recommend]
impl<T: CloseValueRef> CloseValueRef for MutexGuard<'_, T> {
    type Closed = T::Closed;

    fn close_ref(&self) -> Self::Closed {
        T::close_ref(self)
    }
}

#[diagnostic::do_not_recommend]
impl<T: CloseValueRef> CloseValueRef for Arc<Mutex<Option<T>>> {
    type Closed = Option<T::Closed>;

    fn close_ref(&self) -> Self::Closed {
        self.as_ref()
            .lock()
            .ok()?
            .as_ref()
            .map(CloseValueRef::close_ref)
    }
}

#[diagnostic::do_not_recommend]
impl<T: CloseValueRef> CloseValueRef for MutexGuard<'_, Option<T>> {
    type Closed = Option<T::Closed>;

    fn close_ref(&self) -> Self::Closed {
        self.as_ref().map(CloseValueRef::close_ref)
    }
}

#[diagnostic::do_not_recommend]
impl<T: CloseValueRef> CloseValueRef for Arc<Mutex<T>> {
    type Closed = Option<T::Closed>;

    fn close_ref(&self) -> Self::Closed {
        Some(self.as_ref().lock().ok()?.close())
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
impl<T: CloseValue, const N: usize> CloseValue for WithDimensions<T, N> {
    type Closed = WithDimensions<T::Closed, N>;

    fn close(self) -> Self::Closed {
        self.map_value(|v| v.close())
    }
}

#[diagnostic::do_not_recommend]
impl<T: CloseValue, F: FlagConstructor> CloseValue for ForceFlag<T, F> {
    type Closed = ForceFlag<T::Closed, F>;

    fn close(self) -> Self::Closed {
        self.map_value(|v| v.close())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use metrique_writer_core::value::WithDimensions;

    use crate::{CloseValue, CloseValueRef};

    struct Closeable;
    impl CloseValueRef for Closeable {
        type Closed = usize;

        fn close_ref(&self) -> Self::Closed {
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
