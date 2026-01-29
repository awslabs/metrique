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
#[cfg(feature = "test-util")]
use std::collections::HashMap;
#[cfg(feature = "test-util")]
use std::marker::PhantomData;
#[cfg(feature = "test-util")]
use std::sync::{Arc, Mutex};

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
    #[track_caller]
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
    #[track_caller]
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
    ///
    /// [`BackgroundQueue`]: https://docs.rs/metrique-writer/0.1/metrique_writer/sink/struct.BackgroundQueue.html
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
///
/// ## Examples
///
/// After detaching, it is possible to attach a new sink:
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
/// use tracing_appender::rolling::{RollingFileAppender, Rotation};
///
/// global_entry_sink! {
///     /// A special metrics sink for my application
///     MyEntrySink
/// }
///
/// let join = MyEntrySink::attach_to_stream(Emf::all_validations("MyApp".into(), vec![vec![]])
///     .output_to_makewriter(
///         RollingFileAppender::new(Rotation::HOURLY, &log_dir, "prefix.log")
///     )
/// );
///
/// // Can use from any thread
/// MyEntrySink::append(MyMetrics { });
///
/// // When dropped, `join` will flush all appended metrics and detach the output stream.
/// drop(join);
///
/// // Most users don't need to do any of the below:
///
/// // This is normally not needed, but after a sink is detached, it is possible to attach
/// // a new one. Currently there is no way to do an "atomic detach and attach", please file
/// // an issue if you have a use-case for atomic detach-and-attach.
/// let join = MyEntrySink::attach_to_stream(Emf::all_validations("MyApp2".into(), vec![vec![]])
///     .output_to_makewriter(
///         RollingFileAppender::new(Rotation::HOURLY, log_dir, "prefix2.log")
///     )
/// );
///
/// // Will go to the new sink
/// MyEntrySink::append(MyMetrics { });
///
/// // It is also possible to call `AttachHandle::forget` on an `AttachHandle`, which will keep the
/// // stream running. However, in that case, if an asynchronous background queue is used, some other
/// // synchronization mechanism will be needed to avoid dropping metrics during shutdown.
/// join.forget();
/// ```
#[must_use = "if unused the global sink will be immediately detached and shut down"]
pub struct AttachHandle {
    join: Option<fn()>,
}

/// Guard that manages the lifecycle of a thread-local test sink override.
///
/// When created, this guard installs a thread-local test sink that takes precedence
/// over the global sink for the current thread. When dropped, it automatically
/// restores the previous sink state.
///
/// This functionality is only available when the `test-util` feature is enabled
/// and enables isolated testing of metrics without affecting other tests or global state.
#[cfg(feature = "test-util")]
#[must_use = "if unused the thread-local test sink will be immediately restored"]
pub struct ThreadLocalTestSinkGuard {
    // Function pointer to clear the guard when dropped
    // This is set by the macro-generated code
    clear_fn: fn(),
    // ThreadLocalTestSinkGuard touches thread-local data and is therefore !Send/!Sync
    _marker: PhantomData<*const ()>,
}

#[cfg(feature = "test-util")]
impl ThreadLocalTestSinkGuard {
    /// Create a new guard with the previous sink state and restore function.
    ///
    /// This is intended to be called by the macro-generated code after
    /// installing the thread-local sink override.
    #[doc(hidden)]
    pub fn new(clear_fn: fn()) -> Self {
        Self {
            clear_fn,
            _marker: PhantomData,
        }
    }
}

#[cfg(feature = "test-util")]
impl Drop for ThreadLocalTestSinkGuard {
    fn drop(&mut self) {
        (self.clear_fn)();
    }
}

#[cfg(feature = "test-util")]
type RuntimeSinkMap = Arc<Mutex<HashMap<tokio::runtime::Id, BoxEntrySink>>>;

/// Guard for runtime-scoped test sinks.
///
/// This guard is Send + Sync and can be used across threads within a tokio runtime.
/// When dropped, it removes the test sink from the runtime's sink map.
#[cfg(feature = "test-util")]
#[must_use = "if unused the runtime test sink will be immediately removed"]
#[derive(Debug)]
pub struct TokioRuntimeTestSinkGuard {
    runtime_id: tokio::runtime::Id,
    map: RuntimeSinkMap,
}

#[cfg(feature = "test-util")]
impl TokioRuntimeTestSinkGuard {
    #[doc(hidden)]
    pub fn new(runtime_id: tokio::runtime::Id, map: RuntimeSinkMap) -> Self {
        Self { runtime_id, map }
    }
}

#[cfg(feature = "test-util")]
impl Drop for TokioRuntimeTestSinkGuard {
    fn drop(&mut self) {
        self.map.lock().unwrap().remove(&self.runtime_id);
    }
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
    #[track_caller]
    fn sink() -> BoxEntrySink {
        Q::try_sink().expect("sink must be `attach()`ed before use")
    }

    #[track_caller]
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
/// ## Examples
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
///
/// ### Testing
///
/// Global entry sinks support thread-local test overrides for isolated testing.
/// This functionality is only available when the `test-util` feature is enabled
/// and is compiled out when the feature is not enabled.
///
/// ```rust,ignore
/// # use metrique_writer::sink::global_entry_sink;
/// # use metrique_writer::test_util::{test_entry_sink, TestEntrySink};
/// # use metrique_writer::GlobalEntrySink;
/// global_entry_sink! { MyMetrics }
///
/// #[test]
/// fn test_metrics() {
///     let TestEntrySink { inspector, sink } = test_entry_sink();
///     let _guard = MyMetrics::set_test_sink(sink);
///
///     // Code that uses MyMetrics::append() will now go to test sink
///     // Guard automatically restores when dropped
///
///     let entries = inspector.entries();
///     // Assert on captured metrics...
/// }
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

            $crate::__test_util! {
                use ::std::cell::RefCell;
                use ::std::sync::{Arc, Mutex};
                use ::std::collections::HashMap;

                thread_local! {
                    static THREAD_LOCAL_TEST_SINK: RefCell<Option<BoxEntrySink>> = const { RefCell::new(None) };
                }

                static RUNTIME_TEST_SINKS: ::std::sync::OnceLock<Arc<Mutex<HashMap<$crate::__tokio::runtime::Id, BoxEntrySink>>>> = ::std::sync::OnceLock::new();

                fn runtime_sinks() -> &'static Arc<Mutex<HashMap<$crate::__tokio::runtime::Id, BoxEntrySink>>> {
                    RUNTIME_TEST_SINKS.get_or_init(|| Arc::new(Mutex::new(HashMap::new())))
                }

                fn get_test_sink() -> Option<BoxEntrySink> {
                    // Check thread-local first for backwards compatibility
                    if let Some(sink) = THREAD_LOCAL_TEST_SINK.with(|cell| cell.borrow().clone()) {
                        return Some(sink);
                    }
                    // Then check runtime-based
                    if let Ok(handle) = $crate::__tokio::runtime::Handle::try_current() {
                        let map = runtime_sinks().lock().unwrap();
                        return map.get(&handle.id()).cloned();
                    }
                    None
                }

                #[track_caller]
                fn set_test_sink(sink: Option<BoxEntrySink>) {
                    let should_panic = THREAD_LOCAL_TEST_SINK.with(|cell| {
                        let mut borrowed = cell.borrow_mut();
                        let should_panic = borrowed.is_some() && sink.is_some();
                        if !should_panic {
                            *borrowed = sink;
                        }
                        should_panic
                    });

                    if should_panic {
                        panic!("A test sink was previously installed. You can only install one test sink at a time.");
                    }
                }
            }

            impl AttachGlobalEntrySink for $name {
                fn attach(
                    (sink, handle): (impl EntrySink<BoxEntry> + Send + Sync + 'static, impl Any + Send + Sync),
                ) -> AttachHandle {
                    let mut write = SINK.write().unwrap();
                    if write.is_some() {
                        drop(write); // don't poison
                        panic!("Already installed a global {NAME} sink, drop the attach handle first if intentionally attaching a new sink");
                    } else {
                        *write = Some((BoxEntrySink::new(sink), Box::new(handle)));
                    }
                    AttachHandle::new(|| { SINK.write().unwrap().take(); })
                }

                fn try_sink() -> Option<BoxEntrySink> {
                    $crate::__test_util! {
                        if let Some(test_sink) = get_test_sink() {
                            return Some(test_sink);
                        }
                    }

                    let read = SINK.read().unwrap();
                    let (sink, _handle) = read.as_ref()?;
                    Some(sink.clone())
                }

                fn try_append<E: Entry + Send + 'static>(entry: E) -> Result<(), E> {
                    $crate::__test_util! {
                        if let Some(test_sink) = get_test_sink() {
                            test_sink.append(entry);
                            return Ok(());
                        }
                    }

                    let read = SINK.read().unwrap();
                    if let Some((sink, _handle)) = read.as_ref() {
                        sink.append(entry);
                        Ok(())
                    } else {
                        Err(entry)
                    }
                }
            }

            // Test-only methods for thread-local sink management
            $crate::__test_util! {
                const _: () = {
                    impl $name {
                        /// Install a thread-local test sink that takes precedence over the global sink.
                        ///
                        /// Returns a guard that will automatically restore the previous sink state when dropped.
                        /// Only available when the `test-util` feature is enabled.
                        ///
                        /// **Note:** This guard is ONLY applies to the current thread meaning that it will
                        /// not work across threads (e.g. on a multithreaded Tokio runtime). For multi-threaded
                        /// tokio runtimes, use [`set_test_sink_on_current_tokio_runtime`](Self::set_test_sink_on_current_tokio_runtime) instead.
                        ///
                        /// # Example
                        #[doc = $crate::__macro_doctest!()]
                        /// # use metrique_writer::sink::global_entry_sink;
                        /// # use metrique_writer::test_util::{test_entry_sink, TestEntrySink};
                        /// # global_entry_sink! { TestSink }
                        /// let TestEntrySink { inspector, sink } = test_entry_sink();
                        /// let _guard = TestSink::set_test_sink(sink);
                        ///
                        /// // All appends now go to the thread-local test sink
                        /// // Guard automatically restores previous state when dropped
                        /// ```
                        ///
                        /// If you want to ignore metrics, you can attach a thread-local DevNullSink:
                        #[doc = $crate::__macro_doctest!()]
                        /// # use metrique_writer::sink::{DevNullSink, global_entry_sink};
                        /// # use metrique_writer::GlobalEntrySink;
                        /// global_entry_sink! { TestSink }
                        ///
                        /// #[test]
                        /// fn test_metrics() {
                        ///     let _guard = TestSink::set_test_sink(DevNullSink::boxed());
                        ///
                        ///     // Code that uses TestSink::append() will drop entries
                        ///     // Guard automatically restores when dropped
                        /// }
                        /// ```
                        #[track_caller]
                        pub fn set_test_sink(sink: BoxEntrySink) -> $crate::global::ThreadLocalTestSinkGuard {
                            set_test_sink(Some(sink));
                            $crate::global::ThreadLocalTestSinkGuard::new(|| {
                                set_test_sink(None);
                            })
                        }

                        /// Temporarily install a thread-local test sink for the duration of the closure.
                        ///
                        /// This is a convenience method that automatically manages the guard lifecycle.
                        /// Only available when the `test-util` feature is enabled.
                        ///
                        /// # Example
                        #[doc = $crate::__macro_doctest!()]
                        /// # use metrique_writer::sink::global_entry_sink;
                        /// # use metrique_writer::test_util::{test_entry_sink, TestEntrySink};
                        /// # global_entry_sink! { TestSink }
                        /// let TestEntrySink { inspector, sink } = test_entry_sink();
                        ///
                        /// let result = TestSink::with_test_sink(sink, || {
                        ///     // All appends in this closure go to the thread-local test sink
                        ///     42
                        /// });
                        ///
                        /// assert_eq!(result, 42);
                        /// // Thread-local sink is automatically restored
                        /// ```
                        pub fn with_test_sink<F, R>(sink: BoxEntrySink, f: F) -> R
                        where
                            F: FnOnce() -> R,
                        {
                            let _guard = Self::set_test_sink(sink);
                            f()
                        }

                        /// Install a runtime-scoped test sink for a specific tokio runtime.
                        ///
                        /// This allows installing a test sink on a runtime from outside that runtime's context.
                        /// The sink will be used by all tasks running on the specified runtime.
                        ///
                        /// **Note:** Most users should use [`set_test_sink_on_current_tokio_runtime`](Self::set_test_sink_on_current_tokio_runtime)
                        /// instead, which automatically uses the current runtime.
                        ///
                        /// Returns a guard that will automatically remove the sink when dropped.
                        /// Only available when the `test-util` feature is enabled.
                        ///
                        /// # Panics
                        /// If this runtime already has a test sink installed.
                        ///
                        /// # Example
                        #[doc = $crate::__macro_doctest!()]
                        /// # use metrique_writer::sink::global_entry_sink;
                        /// # use metrique_writer::test_util::{test_entry_sink, TestEntrySink};
                        /// # global_entry_sink! { TestSink }
                        /// #[test]
                        /// fn test_metrics() {
                        ///     let rt = tokio::runtime::Runtime::new().unwrap();
                        ///     let TestEntrySink { inspector, sink } = test_entry_sink();
                        ///     let _guard = TestSink::set_test_sink_for_tokio_runtime(&rt.handle(), sink);
                        ///
                        ///     rt.block_on(async {
                        ///         // All appends on this runtime now go to the test sink
                        ///     });
                        ///    // When the _guard is dropped, the sink is now detached and can be reattached again.
                        /// }
                        /// ```
                        #[track_caller]
                        pub fn set_test_sink_for_tokio_runtime(handle: &$crate::__tokio::runtime::Handle, sink: BoxEntrySink) -> $crate::global::TokioRuntimeTestSinkGuard {
                            let runtime_id = handle.id();
                            let map = runtime_sinks();

                            let already_installed = {
                                let mut guard = map.lock().unwrap();
                                if !guard.contains_key(&runtime_id) {
                                    guard.insert(runtime_id, sink);
                                    false
                                } else {
                                    true
                                }
                            };

                            if already_installed {
                                panic!("A test sink was already installed for this runtime. You can only install one test sink per runtime at a time.");
                            }

                            $crate::global::TokioRuntimeTestSinkGuard::new(runtime_id, map.clone())
                        }

                        /// Install a runtime-scoped test sink for the current tokio runtime.
                        ///
                        /// Unlike `set_test_sink`, this guard is not a thread local override and
                        /// instead overrides all usages of of the sink across the runtime.
                        ///
                        /// Returns a guard that will automatically remove the sink when dropped.
                        /// Only available when the `test-util` feature is enabled.
                        ///
                        /// # Panics
                        /// - If called outside a tokio runtime context.
                        /// - If a test sink is already installed on this runtime
                        ///
                        /// # Example
                        #[doc = $crate::__macro_doctest!()]
                        /// # use metrique_writer::sink::global_entry_sink;
                        /// # use metrique_writer::test_util::{test_entry_sink, TestEntrySink};
                        /// # global_entry_sink! { TestSink }
                        /// #[cfg(feature = "test-util")]
                        /// #[tokio::test(flavor = "multi_thread")]
                        /// async fn test_metrics() {
                        ///     let TestEntrySink { inspector, sink } = test_entry_sink();
                        ///     let _guard = TestSink::set_test_sink_on_current_tokio_runtime(sink);
                        ///
                        ///     // `TestSink::sink()` will now always refer to the test sink on any thread on this runtime.
                        ///     // NOTE: that threads _outside_ this runtime (e.g. a background thread) will still NOT have this sink
                        ///     // installed.
                        ///
                        ///     // When _guard is dropped, the sink will be detached.
                        /// }
                        /// ```
                        #[track_caller]
                        pub fn set_test_sink_on_current_tokio_runtime(sink: BoxEntrySink) -> $crate::global::TokioRuntimeTestSinkGuard {
                            let handle = $crate::__tokio::runtime::Handle::current();
                            Self::set_test_sink_for_tokio_runtime(&handle, sink)
                        }
                    }
                };
            }
        };
    };
}
pub use global_entry_sink;

#[cfg(test)]
mod tests {
    use crate::test_stream::TestSink;
    use metrique_writer::test_util::{TestEntrySink, test_entry_sink};
    use metrique_writer::{
        AttachGlobalEntrySink, AttachGlobalEntrySinkExt as _, Entry, EntryWriter, GlobalEntrySink,
        format::FormatExt as _, sink::FlushImmediately,
    };
    use metrique_writer_format_emf::{Emf, EntryDimensions};
    use std::{
        borrow::Cow,
        time::{Duration, SystemTime},
    };

    metrique_writer::sink::global_entry_sink! { ServiceMetrics }

    struct TestEntry;
    impl Entry for TestEntry {
        fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
            writer.timestamp(SystemTime::UNIX_EPOCH + Duration::from_secs_f64(1749475336.0157819));
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

    #[test]
    fn dummy() {
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

    #[test]
    fn thread_local_sink_capture_raw_data() {
        use crate::test_stream::TestSink;

        // Set up thread-local test sink
        let thread_local_output = TestSink::default();
        let formatter = Emf::all_validations("ThreadLocalApp".into(), vec![vec![]])
            .output_to(thread_local_output.clone());
        let sink = FlushImmediately::new_boxed(formatter);

        let content = {
            let _guard = ServiceMetrics::set_test_sink(sink);

            // This should go to the thread-local sink
            ServiceMetrics::append(TestEntry);

            // Verify thread-local sink received the entry
            let content = thread_local_output.dump();
            assert!(content.contains("Time"));
            assert!(content.contains("42"));
            assert!(content.contains("ThreadLocalApp")); // Verify it went to the right namespace
            content
        };
        assert_eq!(
            content,
            r#"{"_aws":{"CloudWatchMetrics":[{"Namespace":"ThreadLocalApp","Dimensions":[["Operation"]],"Metrics":[{"Name":"Time","Unit":"Milliseconds"},{"Name":"BasicIntCount"}]}],"Timestamp":1749475336015},"Time":42,"BasicIntCount":1234,"Operation":"MyOperation","StringProp":"some string value"}
"#
        );
    }

    #[test]
    fn thread_local_sink_capture_entry() {
        use metrique_writer::test_util::{TestEntrySink, test_entry_sink};
        let TestEntrySink { inspector, sink } = test_entry_sink();

        let _guard = ServiceMetrics::set_test_sink(sink);

        // This should go to the thread-local sink
        ServiceMetrics::append(TestEntry);
        assert_eq!(inspector.entries()[0].metrics["BasicIntCount"], 1234);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn runtime_sink_works_across_threads() {
        use metrique_writer::test_util::{TestEntrySink, test_entry_sink};
        let TestEntrySink { inspector, sink } = test_entry_sink();

        let _guard = ServiceMetrics::set_test_sink_on_current_tokio_runtime(sink);

        // Spawn tasks on different threads
        let handles: Vec<_> = (0..4)
            .map(|_| {
                tokio::spawn(async move {
                    ServiceMetrics::append(TestEntry);
                })
            })
            .collect();

        for handle in handles {
            handle.await.unwrap();
        }

        let entries = inspector.entries();
        assert_eq!(entries.len(), 4);
        for entry in entries {
            assert_eq!(entry.metrics["BasicIntCount"], 1234);
        }
    }

    #[tokio::test]
    async fn runtime_sink_guard_is_send_sync() {
        use metrique_writer::test_util::{TestEntrySink, test_entry_sink};
        let TestEntrySink { inspector, sink } = test_entry_sink();

        let guard = ServiceMetrics::set_test_sink_on_current_tokio_runtime(sink);

        // Verify the guard can be sent across threads
        tokio::spawn(async move {
            ServiceMetrics::append(TestEntry);
            drop(guard); // Guard can be dropped on a different thread
        })
        .await
        .unwrap();

        assert_eq!(inspector.entries().len(), 1);
    }

    #[tokio::test]
    async fn runtime_sink_cleanup_on_drop() {
        use metrique_writer::test_util::{TestEntrySink, test_entry_sink};

        let TestEntrySink {
            inspector: inspector1,
            sink: sink1,
        } = test_entry_sink();

        {
            let _guard = ServiceMetrics::set_test_sink_on_current_tokio_runtime(sink1);
            ServiceMetrics::append(TestEntry);
        } // Guard dropped here

        assert_eq!(inspector1.entries().len(), 1);

        // After guard is dropped, we should be able to install a new sink
        let TestEntrySink {
            inspector: inspector2,
            sink: sink2,
        } = test_entry_sink();
        let _guard2 = ServiceMetrics::set_test_sink_on_current_tokio_runtime(sink2);
        ServiceMetrics::append(TestEntry);

        // First inspector should still have 1 entry
        assert_eq!(inspector1.entries().len(), 1);
        // Second inspector should have 1 entry
        assert_eq!(inspector2.entries().len(), 1);
    }

    #[test]
    #[should_panic(expected = "no reactor running")]
    fn runtime_sink_panics_outside_tokio() {
        use metrique_writer::test_util::{TestEntrySink, test_entry_sink};

        let TestEntrySink { inspector: _, sink } = test_entry_sink();
        let _guard = ServiceMetrics::set_test_sink_on_current_tokio_runtime(sink);
    }

    #[test]
    fn runtime_sink_for_runtime_works() {
        use metrique_writer::test_util::{TestEntrySink, test_entry_sink};

        let rt = tokio::runtime::Runtime::new().unwrap();
        let TestEntrySink { inspector, sink } = test_entry_sink();
        let _guard = ServiceMetrics::set_test_sink_for_tokio_runtime(&rt.handle(), sink);

        rt.block_on(async {
            ServiceMetrics::append(TestEntry);
        });

        assert_eq!(inspector.entries().len(), 1);
        assert_eq!(inspector.entries()[0].metrics["BasicIntCount"], 1234);
    }

    #[tokio::test]
    async fn runtime_sink_panics_on_double_install() {
        use metrique_writer::test_util::{TestEntrySink, test_entry_sink};
        use std::panic::AssertUnwindSafe;

        let TestEntrySink {
            inspector: _,
            sink: sink1,
        } = test_entry_sink();
        let _guard1 = ServiceMetrics::set_test_sink_on_current_tokio_runtime(sink1);

        let TestEntrySink {
            inspector: _,
            sink: sink2,
        } = test_entry_sink();
        let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
            ServiceMetrics::set_test_sink_on_current_tokio_runtime(sink2)
        }));

        assert!(result.is_err());
        let panic_msg = result.unwrap_err();
        if let Some(s) = panic_msg.downcast_ref::<String>() {
            assert!(s.contains("A test sink was already installed for this runtime"));
        } else if let Some(s) = panic_msg.downcast_ref::<&str>() {
            assert!(s.contains("A test sink was already installed for this runtime"));
        } else {
            panic!("Unexpected panic type");
        }
    }

    #[test]
    fn with_test_sink() {
        let TestEntrySink { inspector, sink } = test_entry_sink();

        ServiceMetrics::with_test_sink(sink, || {
            // This should go to the thread-local sink
            ServiceMetrics::append(TestEntry);
            assert_eq!(inspector.entries()[0].metrics["BasicIntCount"], 1234);
        });
    }

    #[test]
    #[should_panic]
    fn duplicate_install_panics() {
        let TestEntrySink {
            inspector: _outer_inspector,
            sink,
        } = test_entry_sink();
        let _outer_guard = ServiceMetrics::set_test_sink(sink);
        ServiceMetrics::append(TestEntry);
        let TestEntrySink {
            inspector: _inner_inspector,
            sink,
        } = test_entry_sink();
        ServiceMetrics::append(TestEntry);
        let _inner_guard = ServiceMetrics::set_test_sink(sink);
    }

    #[test]
    fn after_guard_dropped_use_global_queue() {
        let TestEntrySink {
            inspector: global_inspector,
            sink,
        } = test_entry_sink();
        let _handle = ();
        let _handle = ServiceMetrics::attach((sink, _handle));
        // this goes global
        ServiceMetrics::append(TestEntry);
        let TestEntrySink {
            inspector: thread_local_inspector,
            sink,
        } = test_entry_sink();

        {
            let _tl = ServiceMetrics::set_test_sink(sink);
            // local
            ServiceMetrics::append(TestEntry);
        }

        assert_eq!(global_inspector.entries().len(), 1);
        // one more back to global
        ServiceMetrics::append(TestEntry);
        assert_eq!(global_inspector.entries().len(), 2);
        assert_eq!(thread_local_inspector.entries().len(), 1);
    }
}
// Helper macro that conditionally expands based on the test-util feature
// This is checked at macro expansion time in the metrique-writer-core crate

/// Expands the given block of code when `metrique-writer-core` is compiled with the `test-util` feature.
#[doc(hidden)]
#[macro_export]
#[cfg(feature = "test-util")]
macro_rules! __test_util {
    ($($tt:tt)*) => { $($tt)* };
}

/// Does not expand the given block of code when `metrique-writer-core` is compiled without the `test-util` feature.
#[doc(hidden)]
#[macro_export]
#[cfg(not(feature = "test-util"))]
macro_rules! __test_util {
    ($($tt:tt)*) => {};
}

// the __macro_doctest attribute is used to make sure our doctests are not compiled
// in customer crates, since customer crates getting compilation errors on our doctests is
// very annoying.

/// Expands to ```rust to run doctests the given block of code when `metrique-writer-core`
/// is compiled with the `private-test-util` feature, for our internal testing
#[doc(hidden)]
#[macro_export]
#[cfg(feature = "private-test-util")]
macro_rules! __macro_doctest {
    () => {
        "```rust"
    };
}

/// Does not expand the given block of code when `metrique-writer-core` is compiled without the `test-util` feature.
#[doc(hidden)]
#[macro_export]
#[cfg(not(feature = "private-test-util"))]
macro_rules! __macro_doctest {
    () => {
        "```rust,ignore"
    };
}
