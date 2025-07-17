// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Contains the [`global_entry_sink`] macro, which can be used to define [`GlobalEntrySink`]s
//! which are a rendezvous points between metric sources and metric sinks.
//!
//! Note that [`GlobalEntrySink`]s involve boxing, since the types of the [`Entry`]
//! and the [`EntrySink`] are kept separate until run-time. This is implemented in a fairly
//! high-performance manner.
//!
//! However, applications with a very high metric emission rate might prefer to have their
//! high-rate metrics go directly to an [`EntrySink`] without any boxing - and as high-rate
//! metrics are often the per-request metrics from the data plane of a service, and it is
//! often a good idea to separate these from other service metrics for many reasons, even
//! ignoring the boxing performance issue.

use std::any::Any;

use crate::{
    EntrySink,
    entry::BoxEntry,
    sink::{AppendOnDrop, BoxEntrySink},
};

use super::Entry;

/// A global version of [`crate::EntrySink`] that can be referred to by any thread or component.
///
/// Services typically run many components, only some of which may be owned by the service team.
/// Many components, like the AuthRuntimeClient (ARC), still need to emit metrics or audit logs on
/// behalf of the service. Configuring a global entry sink makes it easy for library authors to
/// emit metrics to the right log file without being explicitly passed a background queue.
///
/// Note that there be dangers with globals. They're more difficult to test, and they create
/// implicit interfaces. Library authors *should* offer both implicit and explicit metric emission
/// configuration, allowing service teams to choose how much they'd like to customize.
pub trait GlobalEntrySink {
    /// Return a clone of the [`BoxEntrySink`] attached to this global.
    ///
    /// # Panics
    /// May panic if no sink is yet attached. See [`AttachGlobalEntrySink`].
    fn sink() -> BoxEntrySink;

    /// Append the `entry` to the in-memory buffer. Unless this is explicitly a test sink, the `append()` call must
    /// never block and must never panic. Test sinks are encouraged to immediately panic on invalid entries. Production
    /// sinks should emit a `tracing` event when invalid entries are found.
    ///
    /// If the in-memory buffer is bounded and full, the oldest entries should be dropped. More recent entries are more
    /// valuable for monitoring service health.
    ///
    /// # Panics
    /// May panic if no sink is yet attached. See [`AttachGlobalEntrySink`].
    fn append(entry: impl Entry + Send + 'static);

    /// Wrap `entry` in a smart pointer that will automatically append it to this sink when dropped.
    ///
    /// This will help enforce that an entry is always appended even if it's used across branching business logic. Note
    /// that Rust can't guarantee that the entry is dropped (e.g. `forget(entry)`).
    ///
    /// # Usage
    /// ```
    /// # use metrique_writer::{
    /// #    Entry,
    /// #    GlobalEntrySink,
    /// #    sink::{AttachGlobalEntrySinkExt, global_entry_sink},
    /// #    format::{FormatExt as _},
    /// # };
    /// # use metrique_writer_format_emf::Emf;
    /// # let log_dir = tempfile::tempdir().unwrap();
    /// # use tracing_appender::rolling::{RollingFileAppender, Rotation};
    /// # global_entry_sink! { ServiceMetrics }
    ///
    /// #[derive(Entry)]
    /// struct MyMetrics {
    ///  field: usize
    /// }
    /// #
    /// # let _join = ServiceMetrics::attach_to_stream(Emf::all_validations("MyApp".into(), vec![vec![]])
    /// #     .output_to_makewriter(
    /// #          RollingFileAppender::new(Rotation::HOURLY, log_dir, "prefix.log")
    /// #     )
    /// # );
    ///
    /// let metric_base = MyMetrics { field: 0 };
    /// let mut metric = ServiceMetrics::append_on_drop(metric_base);
    ///
    /// metric.field += 1;
    ///
    /// // metric appends to sink as scope ends and variable drops
    ///
    /// ```
    fn append_on_drop<E: Entry + Send + 'static>(entry: E) -> AppendOnDrop<E, BoxEntrySink>
    where
        Self: Sized + Clone,
    {
        AppendOnDrop::new(entry, Self::sink())
    }

    /// See [`GlobalEntrySink::append_on_drop()`].
    ///
    /// # Usage
    /// ```
    /// # use metrique_writer::{
    /// #    Entry,
    /// #    GlobalEntrySink,
    /// #    sink::{AttachGlobalEntrySinkExt, global_entry_sink},
    /// #    format::{FormatExt as _},
    /// # };
    /// # use metrique_writer_format_emf::Emf;
    /// # let log_dir = tempfile::tempdir().unwrap();
    ///
    /// use tracing_appender::rolling::{RollingFileAppender, Rotation};
    ///
    /// #[derive(Entry, Default)]
    /// struct MyMetrics {
    ///  field: usize
    /// }
    ///
    /// global_entry_sink! {
    ///     /// A special metrics sink for my application
    ///     MyEntrySink
    /// }
    ///
    /// let _join = MyEntrySink::attach_to_stream(Emf::all_validations("MyApp".into(), vec![vec![]])
    ///     .output_to_makewriter(
    ///         RollingFileAppender::new(Rotation::HOURLY, log_dir, "prefix.log")
    ///     )
    /// );
    ///
    /// let mut metric = MyEntrySink::append_on_drop_default::<MyMetrics>();
    ///
    /// metric.field += 1;
    ///
    /// // metric appends to sink as scope ends and variable drops
    ///
    /// ```
    fn append_on_drop_default<E: Default + Entry + Send + 'static>() -> AppendOnDrop<E, BoxEntrySink>
    where
        Self: Sized + Clone,
    {
        Self::append_on_drop(E::default())
    }
}

/// A [`GlobalEntrySink`] that can do nothing until it is attached to an output stream or sink.
pub trait AttachGlobalEntrySink {
    /// Returns whether there's already a sink attached to this global entry sink
    fn is_attached() -> bool {
        Self::try_sink().is_some()
    }

    /// Attach the given sink and join handle to this global sink reference.
    ///
    /// Note that the input type matches the result of [`BackgroundQueue`] build fns.
    ///
    /// # Panics
    /// Panics if a sink is already attached.
    fn attach(
        queue_and_handle: (
            impl EntrySink<BoxEntry> + Send + Sync + 'static,
            impl Any + Send + Sync,
        ),
    ) -> AttachHandle;

    /// Return a cloned reference to the underlying sink attached to the global reference (if
    /// any).
    fn try_sink() -> Option<BoxEntrySink>;

    /// Try to append the entry to the global sink, returning it an [`Err`] case if no sink
    /// is currently attached.
    fn try_append<E: Entry + Send + 'static>(entry: E) -> Result<(), E>;
}

/// Handle that, when dropped, will cause the attached global sink to flush remaining entries and
/// then detach.
#[must_use = "if unused the global sink will be immediately detached and shut down"]
pub struct AttachHandle {
    join: Option<fn()>,
}

impl Drop for AttachHandle {
    fn drop(&mut self) {
        if let Some(join) = self.join.take() {
            join();
        }
    }
}

impl AttachHandle {
    // pub so it can be accessed through macro
    #[doc(hidden)]
    pub fn new(join: fn()) -> Self {
        Self { join: Some(join) }
    }

    /// Cause the attached global sink to remain attached forever.
    ///
    /// Note that this will prevent the sink from guaranteeing metric entries are flushed during
    /// shutdown. You *must* have another mechanism to ensure metrics are flushed.
    pub fn forget(mut self) {
        self.join = None;
    }
}

impl<Q: AttachGlobalEntrySink> GlobalEntrySink for Q {
    fn sink() -> BoxEntrySink {
        Q::try_sink().expect("sink must be `attach()`ed before use")
    }

    fn append(entry: impl Entry + Send + 'static) {
        if Q::try_append(entry).is_err() {
            panic!("sink must be `attach()`ed before appending")
        }
    }
}

/// Define a new global [`AttachGlobalEntrySink`] that can be referenced by type name in all threads.
///
/// # Usage
///
/// To use it, you can attach an [`EntrySink`] (or a [`EntryIoStream`] by using
/// `attach_to_stream`, which uses a `BackgroundQueue`) to the global entry sink,
/// and then you can append metrics into it.
///
/// [`EntryIoStream`]: crate::stream::EntryIoStream
///
/// ## Example
///
/// ```
/// # use metrique_writer::{
/// #    AttachGlobalEntrySinkExt,
/// #    Entry,
/// #    GlobalEntrySink,
/// #    sink::{global_entry_sink, AttachGlobalEntrySink},
/// #    format::{FormatExt as _},
/// # };
/// # use metrique_writer_format_emf::Emf;
/// # let log_dir = tempfile::tempdir().unwrap();
/// # #[derive(Entry)]
/// # struct MyMetrics { }
///
/// use tracing_appender::rolling::{RollingFileAppender, Rotation};
///
/// global_entry_sink! {
///     /// A special metrics sink for my application
///     MyEntrySink
/// }
///
/// let _join = MyEntrySink::attach_to_stream(Emf::all_validations("MyApp".into(), vec![vec![]])
///     .output_to_makewriter(
///         RollingFileAppender::new(Rotation::HOURLY, log_dir, "prefix.log")
///     )
/// );
///
/// // Can use from any thread
/// MyEntrySink::append(MyMetrics { });
///
/// // When dropped, _join will flush all appended metrics and detach the output stream.
/// ```
#[macro_export]
macro_rules! global_entry_sink {
    ($(#[$attr:meta])* $name:ident) => {
        $(#[$attr])*
        #[derive(Debug, Clone)]
        pub struct $name;

        const _: () = {
            use ::std::{sync::RwLock, boxed::Box, option::Option::{self, Some, None}, result::Result, any::Any, marker::{Send, Sync}};
            use $crate::{Entry, BoxEntry, BoxEntrySink, EntrySink, global::{AttachGlobalEntrySink, AttachHandle}};

            const NAME: &'static str = ::std::stringify!($name);
            static SINK: RwLock<Option<(BoxEntrySink, Box<dyn Send + Sync + 'static>)>> = RwLock::new(None);

            impl AttachGlobalEntrySink for $name {
                fn attach(
                    (sink, handle): (impl EntrySink<BoxEntry> + Send + Sync + 'static, impl Any + Send + Sync),
                ) -> AttachHandle {
                    let mut write = SINK.write().unwrap();
                    if write.is_some() {
                        drop(write); // don't poison
                        panic!("Already installed a global {NAME} sink, call `join()` first if intentionally attaching a new sink");
                    } else {
                        *write = Some((BoxEntrySink::new(sink), Box::new(handle)));
                    }
                    AttachHandle::new(|| { SINK.write().unwrap().take(); })
                }

                fn try_sink() -> Option<BoxEntrySink> {
                    let read = SINK.read().unwrap();
                    let (sink, _handle) = read.as_ref()?;
                    Some(sink.clone())
                }

                fn try_append<E: Entry + Send + 'static>(entry: E) -> Result<(), E> {
                    let read = SINK.read().unwrap();
                    if let Some((sink, _handle)) = read.as_ref() {
                        sink.append(entry);
                        Ok(())
                    } else {
                        Err(entry)
                    }
                }
            }
        };
    };
}
pub use global_entry_sink;

#[cfg(test)]
mod tests {
    use crate::test_stream::TestSink;
    use metrique_writer::{
        AttachGlobalEntrySinkExt as _, Entry, EntryWriter, GlobalEntrySink, format::FormatExt as _,
    };
    use metrique_writer_format_emf::{Emf, EntryDimensions};
    use std::{
        borrow::Cow,
        time::{Duration, SystemTime},
    };

    metrique_writer::sink::global_entry_sink! { ServiceMetrics }

    #[test]
    fn dummy() {
        struct TestEntry;
        impl Entry for TestEntry {
            fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
                writer.timestamp(
                    SystemTime::UNIX_EPOCH + Duration::from_secs_f64(1749475336.0157819),
                );
                writer.config(
                    const {
                        &EntryDimensions::new_static(&[Cow::Borrowed(&[Cow::Borrowed(
                            "Operation",
                        )])])
                    },
                );
                writer.value("Time", &Duration::from_millis(42));
                writer.value("Operation", "MyOperation");
                writer.value("StringProp", "some string value");
                writer.value("BasicIntCount", &1234u64);
            }
        }

        let output = TestSink::default();
        {
            let _attached = ServiceMetrics::attach_to_stream(
                Emf::all_validations("MyApp".into(), vec![vec![]]).output_to(output.clone()),
            );
            ServiceMetrics::append(TestEntry);
        }
        assert_json_diff::assert_json_eq!(
            serde_json::from_str::<serde_json::Value>(&output.dump()).unwrap(),
            serde_json::json!({
                "_aws":{
                    "CloudWatchMetrics": [
                        {
                            "Namespace": "MyApp",
                            "Dimensions": [["Operation"]],
                            "Metrics": [
                                {"Name":"Time", "Unit":"Milliseconds"},
                                {"Name":"BasicIntCount"}
                            ]
                        }
                    ],
                    "Timestamp": 1749475336015u64,
                },
                "Time":42,
                "BasicIntCount":1234,
                "Operation":"MyOperation",
                "StringProp":"some string value"
            })
        )
    }
}
