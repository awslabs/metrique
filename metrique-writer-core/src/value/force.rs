// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::{
    io,
    marker::PhantomData,
    ops::{Deref, DerefMut},
};

use derive_where::derive_where;

use crate::{
    Entry, EntryIoStream, EntryWriter, IoStreamError, Observation, Unit, ValidationError,
    ValueWriter,
};

use super::{MetricFlags, MetricValue, Value};

/// A trait for functions that return a [`MetricFlags<'static>`][MetricFlags]
///
/// <div id="doc-warning-1" class="warning">
/// The API for defining new flags is currently not covered by semver,
/// and might break in new versions of this library.
/// </div>
///
/// If you want to implement your own metric flag, and you want to
/// be able to use it with [`ForceFlag`], you can create a [`FlagConstructor`]
/// for your flag:
///
/// ```
/// # use metrique_writer::MetricFlags;
/// # use metrique_writer::value::{FlagConstructor, ForceFlag};
///
/// #[derive(Debug)]
/// pub struct MyFlagOpt;
///
/// pub struct MyFlagCtor;
///
/// impl FlagConstructor for MyFlagCtor {
///     fn construct() -> MetricFlags<'static> {
///         MetricFlags::upcast(&MyFlagOpt)
///     }
/// }
///
/// impl metrique_writer::value::MetricOptions for MyFlagOpt {}
///
/// pub type MyFlag<T> = ForceFlag<T, MyFlagCtor>;
/// ```
pub trait FlagConstructor {
    /// Return the desired flag
    fn construct() -> MetricFlags<'static>;
}

/// Helper to force enable metric flags on a value
///
/// This is intentionally "punned" to work with [Entry], [Value], and [EntryIoStream] to
/// avoid duplication of the format-specific flag types like `HighStorageResolution`.
#[derive_where(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash; T)]
pub struct ForceFlag<T, FLAGS: FlagConstructor>(T, PhantomData<FLAGS>);

impl<V, FLAGS: FlagConstructor> ForceFlag<V, FLAGS> {
    /// Map the value within this [ForceFlag]
    pub fn map_value<U>(self, f: impl Fn(V) -> U) -> ForceFlag<U, FLAGS> {
        ForceFlag(f(self.0), PhantomData)
    }

    /// Map the value within this [ForceFlag] by reference
    pub fn map_value_ref<U>(&self, f: impl Fn(&V) -> U) -> ForceFlag<U, FLAGS> {
        ForceFlag(f(&self.0), PhantomData)
    }
}

impl<T, FLAGS: FlagConstructor> From<T> for ForceFlag<T, FLAGS> {
    fn from(value: T) -> Self {
        Self(value, PhantomData)
    }
}

impl<T, FLAGS: FlagConstructor> Deref for ForceFlag<T, FLAGS> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T, FLAGS: FlagConstructor> DerefMut for ForceFlag<T, FLAGS> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl<T, FLAGS: FlagConstructor> ForceFlag<T, FLAGS> {
    /// Return the value contained within this [ForceFlag]
    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T: Value, FLAGS: FlagConstructor> Value for ForceFlag<T, FLAGS> {
    fn write(&self, writer: impl ValueWriter) {
        struct Wrapper<W, FLAGS: FlagConstructor>(W, PhantomData<FLAGS>);

        impl<W: ValueWriter, FLAGS: FlagConstructor> ValueWriter for Wrapper<W, FLAGS> {
            fn string(self, value: &str) {
                self.0.string(value)
            }

            fn metric<'a>(
                self,
                distribution: impl IntoIterator<Item = Observation>,
                unit: Unit,
                dimensions: impl IntoIterator<Item = (&'a str, &'a str)>,
                flags: MetricFlags<'_>,
            ) {
                self.0.metric(
                    distribution,
                    unit,
                    dimensions,
                    flags.try_merge(FLAGS::construct()),
                );
            }

            fn error(self, error: ValidationError) {
                self.0.error(error)
            }
        }

        self.0.write(Wrapper::<_, FLAGS>(writer, PhantomData))
    }
}

impl<T: MetricValue, FLAGS: FlagConstructor> MetricValue for ForceFlag<T, FLAGS> {
    type Unit = T::Unit;
}

// This one is private for now since there is no obvious use for it.
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
        value: &(impl crate::Value + ?Sized),
    ) {
        self.writer.value(name, &ForceFlag::<_, FLAGS>::from(value))
    }

    fn config(&mut self, config: &'a dyn crate::EntryConfig) {
        self.writer.config(config);
    }
}

impl<E: Entry, FLAGS: FlagConstructor> Entry for ForceFlag<E, FLAGS> {
    fn write<'a>(&'a self, writer: &mut impl crate::EntryWriter<'a>) {
        self.0.write(&mut ForceFlagEntryWriter {
            writer,
            phantom: self.1,
        })
    }
}

impl<S: EntryIoStream, FLAGS: FlagConstructor> EntryIoStream for ForceFlag<S, FLAGS> {
    fn next(&mut self, entry: &impl Entry) -> Result<(), IoStreamError> {
        self.0.next(&ForceFlag(entry, self.1))
    }

    fn next_basic_error(&mut self, message: &str) -> Result<(), IoStreamError> {
        self.0.next_basic_error(message)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}
