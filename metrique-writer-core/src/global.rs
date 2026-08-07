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
use std::sync::Arc;
#[cfg(feature = "test-util")]
use std::sync::Mutex;
use std::sync::Weak;

use crate::{
    EntrySink,
    entry::BoxEntry,
    sink::{AppendOnDrop, BoxEntrySink},
};

use super::Entry;

/// A global version of [`crate::EntrySink`] that can be referred to by any thread or component.
///
/// Services typically run many components, only some of which may be directly written by application authors.
/// Many shared libraries still need to emit metrics or audit logs on
/// behalf of the application. Configuring a global entry sink makes it easy for library authors to
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

    /// Register a function to be called when the attach handle is dropped.
    ///
    /// # Panics
    /// Panics if no sink has been attached, or if the [`AttachHandle`] was
    /// dropped or [`forgotten`](AttachHandle::forget).
    fn register_shutdown_fn(f: ShutdownFn);
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
    /// Registry of shutdown functions to call when the attach handle is dropped.
    /// `None` after `forget()` is called.
    shutdown_registry: Option<Arc<ShutdownRegistry>>,
}

/// A function to be called during shutdown when the [`AttachHandle`] is dropped.
pub struct ShutdownFn(Box<dyn FnOnce() + Send>);

impl ShutdownFn {
    /// Create a new [`ShutdownFn`] from a closure.
    pub fn new(f: impl FnOnce() + Send + 'static) -> Self {
        Self(Box::new(f))
    }

    fn call(self) {
        self.0();
    }
}

// `ShutdownRegistry`'s internal lock, swapped for a shuttle-native `Mutex`
// under `--cfg shuttle` (see `shuttle_tests` below). `RuntimeSinkMap` below
// stays on `std::sync::Mutex` regardless -- it's test-only plumbing with no
// interleaving-sensitive invariant worth Shuttle's cost.
//
// Gated on `feature = "_shuttle"` too, not `cfg(shuttle)` alone: `--cfg
// shuttle` is set process-wide via RUSTFLAGS, so it also reaches builds of
// this crate (e.g. as a dev-dependency with different requested features)
// that don't have `_shuttle` enabled and therefore don't have the optional
// `shuttle` crate linked at all.
#[cfg(not(all(shuttle, feature = "_shuttle")))]
type ShutdownRegistryMutex<T> = std::sync::Mutex<T>;
#[cfg(all(shuttle, feature = "_shuttle"))]
type ShutdownRegistryMutex<T> = shuttle::sync::Mutex<T>;

/// Lock type for the macro-generated `ATTACHED` static
#[doc(hidden)]
#[cfg(not(all(shuttle, feature = "_shuttle")))]
pub type AttachedLock<T> = std::sync::RwLock<T>;
#[doc(hidden)]
#[cfg(all(shuttle, feature = "_shuttle"))]
pub type AttachedLock<T> = shuttle::sync::RwLock<T>;

/// Storage for [`ShutdownFn`]s registered on an [`AttachHandle`], to be run when the [`AttachHandle`] is dropped.
///
/// This type is public for macro-generated code. You should not need to use it directly,
/// use [`AttachGlobalEntrySink::register_shutdown_fn`] instead.
///
/// `None` means the registry has been closed (drained).
pub struct ShutdownRegistry(ShutdownRegistryMutex<Option<Vec<ShutdownFn>>>);

impl ShutdownRegistry {
    fn new(initial: ShutdownFn) -> Self {
        Self(ShutdownRegistryMutex::new(Some(vec![initial])))
    }

    /// Add a shutdown function. Functions run in LIFO order when the
    /// [`AttachHandle`] is dropped.
    ///
    /// Returns `false` (and does not add `f`) if the registry has already been closed by a
    /// concurrent (or prior) [`ShutdownRegistry::drain`].
    #[doc(hidden)]
    pub fn push(&self, f: ShutdownFn) -> bool {
        match self.0.lock().unwrap().as_mut() {
            Some(fns) => {
                fns.push(f);
                true
            }
            None => false,
        }
    }

    /// Close the registry and return whatever functions were registered before closure, in
    /// registration order. Whichever of `push` or `drain` acquires the lock first wins outright.
    fn drain(&self) -> Vec<ShutdownFn> {
        self.0.lock().unwrap().take().unwrap_or_default()
    }
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
        if let Some(arc) = self.shutdown_registry.take() {
            let fns = arc.drain();
            let total = fns.len();
            let mut panics = Vec::new();
            for f in fns.into_iter().rev() {
                // wrap in catch_unwind so we can get to the detach fn even if a previous fn call panics
                if let Err(payload) =
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f.call()))
                {
                    panics.push(payload);
                }
            }
            // avoid a double-panic abort (don't panic if already panicking)
            if !panics.is_empty() && !std::thread::panicking() {
                let messages: Vec<&str> = panics
                    .iter()
                    .map(|p| {
                        p.downcast_ref::<&str>()
                            .copied()
                            .or_else(|| p.downcast_ref::<String>().map(String::as_str))
                            .unwrap_or("<non-string panic payload>")
                    })
                    .collect();
                panic!(
                    "{} of {total} shutdown fn(s) panicked during AttachHandle::drop \
                     (every shutdown fn still ran to completion, including cleanup): {messages:?}",
                    panics.len()
                );
            }
        }
    }
}

impl AttachHandle {
    // pub so it can be accessed through macro
    #[doc(hidden)]
    pub fn new(join: fn()) -> Self {
        Self {
            shutdown_registry: Some(Arc::new(ShutdownRegistry::new(ShutdownFn::new(join)))),
        }
    }

    /// Cause the attached global sink to remain attached forever.
    ///
    /// The sink and any subscribed background tasks (e.g. tokio runtime metrics) will
    /// continue running indefinitely. Registered shutdown functions will not run.
    /// Subsequent calls to [`register_shutdown_fn`](AttachGlobalEntrySink::register_shutdown_fn)
    /// will panic.
    ///
    /// Note that this will prevent the sink from guaranteeing metric entries are flushed during
    /// shutdown. You *must* have another mechanism to ensure metrics are flushed.
    pub fn forget(mut self) {
        self.shutdown_registry = None;
    }

    #[doc(hidden)]
    pub fn shutdown_registry_weak(&self) -> Weak<ShutdownRegistry> {
        self.shutdown_registry
            .as_ref()
            .map(Arc::downgrade)
            .unwrap_or_default()
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
            use ::std::{sync::Weak, boxed::Box, option::Option::{self, Some, None}, result::Result, any::Any, marker::{Send, Sync}};
            use $crate::{Entry, BoxEntry, BoxEntrySink, EntrySink, global::{AttachGlobalEntrySink, AttachHandle, ShutdownFn, ShutdownRegistry, AttachedLock}};

            const NAME: &'static str = ::std::stringify!($name);

            // Both fields are set together by `attach()` and cleared together by the
            // shutdown fn it registers, under one lock. This way, a reader can never observe
            // one half of "attached" without the other.
            struct AttachedState {
                sink: (BoxEntrySink, Box<dyn Send + Sync + 'static>),
                shutdown_registry: Weak<ShutdownRegistry>,
            }
            static ATTACHED: AttachedLock<Option<AttachedState>> = AttachedLock::new(None);

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
                    let mut write = ATTACHED.write().unwrap();
                    if write.is_some() {
                        drop(write); // don't poison
                        panic!("Already installed a global {NAME} sink, drop the attach handle first if intentionally attaching a new sink");
                    }
                    let sink = BoxEntrySink::new(sink);
                    // Constructing the handle (and reading its shutdown_registry weak ref)
                    // happens before the single write below, so a reader can never see
                    // `sink` set without `shutdown_registry` also being set.
                    let attach_handle = AttachHandle::new(|| { ATTACHED.write().unwrap().take(); });
                    *write = Some(AttachedState {
                        sink: (sink, Box::new(handle)),
                        shutdown_registry: attach_handle.shutdown_registry_weak(),
                    });
                    drop(write);

                    attach_handle
                }

                fn try_sink() -> Option<BoxEntrySink> {
                    $crate::__test_util! {
                        if let Some(test_sink) = get_test_sink() {
                            return Some(test_sink);
                        }
                    }

                    let read = ATTACHED.read().unwrap();
                    let attached = read.as_ref()?;
                    Some(attached.sink.0.clone())
                }

                fn try_append<E: Entry + Send + 'static>(entry: E) -> Result<(), E> {
                    $crate::__test_util! {
                        if let Some(test_sink) = get_test_sink() {
                            test_sink.append(entry);
                            return Ok(());
                        }
                    }

                    let read = ATTACHED.read().unwrap();
                    if let Some(attached) = read.as_ref() {
                        attached.sink.0.append(entry);
                        Ok(())
                    } else {
                        Err(entry)
                    }
                }

                fn register_shutdown_fn(f: ShutdownFn) {
                    let read = ATTACHED.read().unwrap();
                    let attached = read.as_ref().expect("No sink attached — call attach() before subscribing");
                    if !attached.shutdown_registry.upgrade().is_some_and(|registry| registry.push(f)) {
                        panic!("AttachHandle was dropped or forgotten — cannot register shutdown functions");
                    }
                }
            }

            impl $name {
                /// Returns a lazily-resolved sink that looks up the attached sink each
                /// time an entry is appended.
                ///
                /// Unlike [`sink()`](crate::GlobalEntrySink::sink), this method will never
                /// panic. The returned [`BoxEntrySink`] defers resolution: when an entry
                /// is actually appended (e.g., on drop of an [`AppendOnDrop`] guard), it
                /// checks whether a sink is attached at *that* point. If a sink is
                /// available, the entry is forwarded to it; otherwise the entry is
                /// silently discarded.
                ///
                /// This is particularly useful for **libraries** that want to emit metrics
                /// when available but don't control when (or whether) the host application
                /// attaches a sink. It is safe to call before a sink has been
                /// [`attach()`](crate::global::AttachGlobalEntrySink::attach)ed -- entries
                /// will still reach the real sink as long as it is attached before the
                /// entries are emitted.
                ///
                /// # Example
                #[doc = $crate::__macro_doctest!()]
                /// # use metrique_writer::sink::global_entry_sink;
                /// # use metrique_writer::test_util::{test_entry_sink, TestEntrySink};
                /// # global_entry_sink! { ServiceMetrics }
                /// #[test]
                /// fn test_metrics() {
                ///     #[metrics(rename_all = "PascalCase")]
                ///     struct MyMetrics {
                ///         operation: &'static str,
                ///     }
                ///
                ///     // On drop: no sink is attached, so the entry is silently discarded
                ///     let _my_metrics =
                ///         MyMetrics { operation: "test" }.append_on_drop(ServiceMetrics::sink_or_discard());
                ///
                /// }
                /// ```
                ///
                /// When a sink *is* attached, entries are captured:
                #[doc = $crate::__macro_doctest!()]
                /// # use metrique_writer::sink::global_entry_sink;
                /// # use metrique_writer::test_util::{test_entry_sink, TestEntrySink};
                /// # global_entry_sink! { ServiceMetrics }
                /// #[test]
                /// fn test_metrics_with_sink() {
                ///     #[metrics(rename_all = "PascalCase")]
                ///     struct MyMetrics {
                ///         operation: &'static str,
                ///     }
                ///
                ///     let TestEntrySink { inspector, sink } = test_entry_sink();
                ///     let _guard = ServiceMetrics::set_test_sink(sink);
                ///
                ///     let _my_metrics =
                ///         MyMetrics { operation: "test" }.append_on_drop(ServiceMetrics::sink_or_discard());
                ///     drop(_my_metrics);
                ///
                ///     assert_eq!(inspector.entries()[0].values["Operation"], "test");
                /// }
                /// ```
                pub fn sink_or_discard() -> BoxEntrySink {
                    BoxEntrySink::lazy(<Self as $crate::global::AttachGlobalEntrySink>::try_sink)
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
        AnyEntrySink, AttachGlobalEntrySink, AttachGlobalEntrySinkExt as _, Entry, EntrySink,
        EntryWriter, GlobalEntrySink, format::FormatExt as _, sink::FlushImmediately,
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

    #[test]
    fn sink_or_discard_without_attached_sink() {
        let sink = ServiceMetrics::sink_or_discard();
        sink.append(TestEntry);
    }

    #[test]
    fn sink_or_discard_with_test_sink() {
        let TestEntrySink { inspector, sink } = test_entry_sink();
        let _guard = ServiceMetrics::set_test_sink(sink);

        ServiceMetrics::sink_or_discard().append(TestEntry);
        assert_eq!(inspector.entries().len(), 1);
        assert_eq!(inspector.entries()[0].metrics["BasicIntCount"], 1234);
    }

    #[test]
    fn sink_or_discard_append_on_drop_without_sink() {
        let _metric = ServiceMetrics::sink_or_discard().append_on_drop(TestEntry);
    }

    #[test]
    fn sink_or_discard_append_on_drop_with_test_sink() {
        let TestEntrySink { inspector, sink } = test_entry_sink();
        let _guard = ServiceMetrics::set_test_sink(sink);

        {
            let _metric = ServiceMetrics::sink_or_discard().append_on_drop(TestEntry);
        }
        assert_eq!(inspector.entries().len(), 1);
    }

    #[test]
    fn sink_or_discard_resolves_lazily() {
        let lazy_sink = ServiceMetrics::sink_or_discard();

        let TestEntrySink { inspector, sink } = test_entry_sink();
        let _guard = ServiceMetrics::set_test_sink(sink);

        lazy_sink.append(TestEntry);
        assert_eq!(inspector.entries().len(), 1);
        assert_eq!(inspector.entries()[0].metrics["BasicIntCount"], 1234);
    }

    #[test]
    fn sink_or_discard_append_on_drop_resolves_lazily() {
        let lazy_sink = ServiceMetrics::sink_or_discard();
        let metric = lazy_sink.append_on_drop(TestEntry);

        let TestEntrySink { inspector, sink } = test_entry_sink();
        let _guard = ServiceMetrics::set_test_sink(sink);

        drop(metric);
        assert_eq!(inspector.entries().len(), 1);
        assert_eq!(inspector.entries()[0].metrics["BasicIntCount"], 1234);
    }

    #[test]
    fn sink_or_discard_flush_without_sink() {
        let lazy_sink = ServiceMetrics::sink_or_discard();
        let mut flush = std::pin::pin!(AnyEntrySink::flush_async(&lazy_sink));
        let waker = std::task::Waker::noop();
        let mut cx = std::task::Context::from_waker(&waker);
        assert!(flush.as_mut().poll(&mut cx).is_ready());
    }

    #[test]
    fn sink_or_discard_discards_then_forwards() {
        let lazy_sink = ServiceMetrics::sink_or_discard();

        lazy_sink.append(TestEntry);

        let TestEntrySink { inspector, sink } = test_entry_sink();
        let _guard = ServiceMetrics::set_test_sink(sink);

        lazy_sink.append(TestEntry);
        assert_eq!(inspector.entries().len(), 1);
    }

    #[test]
    fn sink_or_discard_detach_stops_forwarding() {
        let lazy_sink = ServiceMetrics::sink_or_discard();

        let TestEntrySink { inspector, sink } = test_entry_sink();
        let guard = ServiceMetrics::set_test_sink(sink);

        lazy_sink.append(TestEntry);
        assert_eq!(inspector.entries().len(), 1);

        drop(guard);

        lazy_sink.append(TestEntry);
        assert_eq!(inspector.entries().len(), 1);
    }
}

#[cfg(test)]
mod shutdown_registry_tests {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use metrique_writer::sink::AttachGlobalEntrySink;
    use metrique_writer::test_util::{TestEntrySink, test_entry_sink};

    use metrique_writer::ShutdownFn;

    #[test]
    fn shutdown_fn_runs_on_drop() {
        metrique_writer::sink::global_entry_sink! { Sink }
        let TestEntrySink { sink, .. } = test_entry_sink();
        let called = Arc::new(AtomicBool::new(false));
        let called2 = called.clone();

        let handle = Sink::attach((sink, ()));
        Sink::register_shutdown_fn(ShutdownFn::new(move || {
            called2.store(true, Ordering::SeqCst);
        }));

        assert!(!called.load(Ordering::SeqCst));
        drop(handle);
        assert!(called.load(Ordering::SeqCst));
    }

    #[test]
    fn shutdown_fns_run_before_sink_detach() {
        metrique_writer::sink::global_entry_sink! { Sink }
        let TestEntrySink { sink, .. } = test_entry_sink();

        let sink_was_attached_during_shutdown = Arc::new(AtomicBool::new(false));
        let flag = sink_was_attached_during_shutdown.clone();

        let handle = Sink::attach((sink, ()));

        // The sink detach fn was registered first. Since shutdown runs in LIFO order,
        // this subscriber fn runs before the sink detaches.
        Sink::register_shutdown_fn(ShutdownFn::new(move || {
            flag.store(Sink::try_sink().is_some(), Ordering::SeqCst);
        }));

        drop(handle);

        assert!(
            sink_was_attached_during_shutdown.load(Ordering::SeqCst),
            "subscriber shutdown fn should run while sink is still attached"
        );
        // And after drop completes, the sink should be detached.
        assert!(Sink::try_sink().is_none());
    }

    #[test]
    fn forget_prevents_shutdown_fns_from_running() {
        metrique_writer::sink::global_entry_sink! { Sink }
        let TestEntrySink { sink, .. } = test_entry_sink();
        let called = Arc::new(AtomicBool::new(false));
        let called2 = called.clone();

        let handle = Sink::attach((sink, ()));
        Sink::register_shutdown_fn(ShutdownFn::new(move || {
            called2.store(true, Ordering::SeqCst);
        }));

        handle.forget();
        assert!(!called.load(Ordering::SeqCst));
    }

    #[test]
    fn forget_keeps_sink_attached() {
        metrique_writer::sink::global_entry_sink! { Sink }
        let TestEntrySink { sink, .. } = test_entry_sink();

        let handle = Sink::attach((sink, ()));
        handle.forget();

        // Sink should still be usable
        assert!(Sink::try_sink().is_some());
    }

    #[test]
    #[should_panic(expected = "No sink attached")]
    fn register_without_attach_panics() {
        metrique_writer::sink::global_entry_sink! { Sink }
        Sink::register_shutdown_fn(ShutdownFn::new(|| {}));
    }

    #[test]
    #[should_panic(expected = "dropped or forgotten")]
    fn register_after_forget_panics() {
        metrique_writer::sink::global_entry_sink! { Sink }
        let TestEntrySink { sink, .. } = test_entry_sink();
        let handle = Sink::attach((sink, ()));
        handle.forget();
        Sink::register_shutdown_fn(ShutdownFn::new(|| {}));
    }

    #[test]
    fn can_reattach_after_drop() {
        metrique_writer::sink::global_entry_sink! { Sink }
        let TestEntrySink { sink, .. } = test_entry_sink();
        let called = Arc::new(AtomicUsize::new(0));

        // First attach + drop
        {
            let handle = Sink::attach((sink, ()));
            let called2 = called.clone();
            Sink::register_shutdown_fn(ShutdownFn::new(move || {
                called2.fetch_add(1, Ordering::SeqCst);
            }));
            drop(handle);
        }
        assert_eq!(called.load(Ordering::SeqCst), 1);

        // Second attach + drop — new registry, new shutdown fns
        let TestEntrySink { sink, .. } = test_entry_sink();
        {
            let handle = Sink::attach((sink, ()));
            let called2 = called.clone();
            Sink::register_shutdown_fn(ShutdownFn::new(move || {
                called2.fetch_add(1, Ordering::SeqCst);
            }));
            drop(handle);
        }
        assert_eq!(called.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn shutdown_fns_run_in_lifo_order() {
        metrique_writer::sink::global_entry_sink! { Sink }
        let TestEntrySink { sink, .. } = test_entry_sink();

        let order = Arc::new(Mutex::new(Vec::new()));

        let handle = Sink::attach((sink, ()));

        // The attach call registers the sink detach fn first
        // Register three more in order:
        for i in 1..=3 {
            let order = order.clone();
            Sink::register_shutdown_fn(ShutdownFn::new(move || {
                order.lock().unwrap().push(i);
            }));
        }

        drop(handle);

        assert_eq!(*order.lock().unwrap(), vec![3, 2, 1]);
    }

    #[test]
    fn drop_does_not_panic_with_outstanding_strong_ref() {
        let handle = super::AttachHandle::new(|| {});
        let extra_strong_ref = handle.shutdown_registry_weak().upgrade().unwrap();
        drop(handle); // must not panic even though `extra_strong_ref` is still alive
        drop(extra_strong_ref);
    }

    #[test]
    fn push_after_drain_starts_is_rejected_and_never_runs() {
        // A push after drain has started must be rejected, never enqueued.
        let ran = Arc::new(AtomicBool::new(false));
        let registry = super::ShutdownRegistry::new(super::ShutdownFn::new(|| {}));

        let drained = registry.drain();
        assert_eq!(drained.len(), 1, "initial fn should be drained");

        let ran2 = ran.clone();
        let accepted = registry.push(super::ShutdownFn::new(move || {
            ran2.store(true, Ordering::SeqCst);
        }));

        assert!(!accepted, "push after drain has started must be rejected");
        assert!(
            registry.drain().is_empty(),
            "a rejected push must never be enqueued"
        );
        assert!(!ran.load(Ordering::SeqCst));
    }

    #[test]
    #[should_panic(
        expected = "AttachHandle was dropped or forgotten — cannot register shutdown functions"
    )]
    fn push_during_live_drain_produces_dropped_or_forgotten_panic() {
        // Losing a race against a concurrent, in-progress drain.
        let handle = super::AttachHandle::new(|| {});
        let weak = handle.shutdown_registry_weak();

        weak.upgrade().unwrap().drain();

        let registry = weak
            .upgrade()
            .expect("AttachHandle was dropped or forgotten — cannot register shutdown functions");
        assert!(
            registry.push(super::ShutdownFn::new(|| {})),
            "AttachHandle was dropped or forgotten — cannot register shutdown functions"
        );
    }

    #[test]
    fn attach_never_observed_with_sink_set_but_registry_unset() {
        // Verifies try_sink() and register_shutdown_fn() never disagree about
        // whether a sink is attached -- register_shutdown_fn() must not panic
        // with "No sink attached" right after try_sink() just reported one is.
        metrique_writer::sink::global_entry_sink! { Sink }
        use std::sync::Barrier;

        const ROUNDS: usize = 200_000;
        const MAX_PROBES_PER_ROUND: usize = 1_000;

        let bad_panic = Arc::new(AtomicBool::new(false));
        let start_barrier = Arc::new(Barrier::new(2));
        let done_barrier = Arc::new(Barrier::new(2));

        std::thread::scope(|scope| {
            {
                let start_barrier = start_barrier.clone();
                let done_barrier = done_barrier.clone();
                scope.spawn(move || {
                    for _ in 0..ROUNDS {
                        start_barrier.wait();
                        let handle = Sink::attach((metrique_writer::sink::DevNullSink::new(), ()));
                        // Hold the sink attached until the racer has finished
                        // probing this round -- no detach can happen mid-round.
                        done_barrier.wait();
                        drop(handle);
                    }
                });
            }

            let bad_panic = bad_panic.clone();
            scope.spawn(move || {
                for _ in 0..ROUNDS {
                    start_barrier.wait();
                    for _ in 0..MAX_PROBES_PER_ROUND {
                        if Sink::try_sink().is_none() {
                            continue;
                        }
                        // try_sink() just reported "attached" -- register_shutdown_fn()
                        // must not disagree by claiming no sink was ever attached.
                        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            Sink::register_shutdown_fn(ShutdownFn::new(|| {}));
                        }));
                        if let Err(payload) = result {
                            let msg = payload
                                .downcast_ref::<&str>()
                                .copied()
                                .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
                                .unwrap_or_default();
                            if msg.contains("No sink attached") {
                                bad_panic.store(true, Ordering::SeqCst);
                            }
                        }
                        break;
                    }
                    done_barrier.wait();
                }
            });
        });

        assert!(
            !bad_panic.load(Ordering::SeqCst),
            "register_shutdown_fn() panicked with \"No sink attached\" even though \
             try_sink() just reported a sink was attached -- attach() must set the \
             sink and the shutdown registry together, not as two separately \
             observable writes"
        );
    }

    #[test]
    #[should_panic(expected = "No sink attached")]
    fn register_after_full_drop_panics() {
        metrique_writer::sink::global_entry_sink! { Sink }
        let TestEntrySink { sink, .. } = test_entry_sink();
        let handle = Sink::attach((sink, ()));
        drop(handle);
        Sink::register_shutdown_fn(ShutdownFn::new(|| {}));
    }

    #[test]
    fn sink_still_detaches_after_a_shutdown_fn_panics() {
        metrique_writer::sink::global_entry_sink! { Sink }
        let TestEntrySink { sink, .. } = test_entry_sink();

        let handle = Sink::attach((sink, ()));
        Sink::register_shutdown_fn(ShutdownFn::new(|| {
            panic!("boom");
        }));

        // drop() is expected to re-panic, but the sink must still be detached
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(handle)));

        assert!(
            Sink::try_sink().is_none(),
            "sink must be detached even though a shutdown fn panicked"
        );

        // A fresh attach() must succeed
        let TestEntrySink { sink, .. } = test_entry_sink();
        let _handle2 = Sink::attach((sink, ()));
    }

    #[test]
    #[should_panic(expected = "shutdown fn(s) panicked during AttachHandle::drop")]
    fn panicking_shutdown_fn_does_not_block_sink_detach() {
        metrique_writer::sink::global_entry_sink! { Sink }
        let TestEntrySink { sink, .. } = test_entry_sink();

        let handle = Sink::attach((sink, ()));
        Sink::register_shutdown_fn(ShutdownFn::new(|| {
            panic!("boom");
        }));

        drop(handle);
    }

    #[test]
    fn drop_during_existing_panic_does_not_abort() {
        // Check that dropping the handle while already unwinding from an unrelated panic
        // doesn't double-panic and abort the whole process.
        let result = std::thread::spawn(|| {
            metrique_writer::sink::global_entry_sink! { Sink }
            let TestEntrySink { sink, .. } = test_entry_sink();
            let handle = Sink::attach((sink, ()));
            Sink::register_shutdown_fn(ShutdownFn::new(|| {
                panic!("shutdown fn panic");
            }));

            struct DropsHandleOnUnwind(Option<metrique_writer::sink::AttachHandle>);
            impl Drop for DropsHandleOnUnwind {
                fn drop(&mut self) {
                    drop(self.0.take()); // AttachHandle::drop runs here, already unwinding
                }
            }
            let _guard = DropsHandleOnUnwind(Some(handle));
            panic!("original, unrelated panic");
        })
        .join();

        assert!(
            result.is_err(),
            "the spawned thread should have panicked exactly once, not aborted"
        );
    }
}

// Shuttle test for the `AttachHandle`/`ShutdownRegistry` `Arc`/`Weak`
// handshake. Constructs both directly instead of going through
// `global_entry_sink!` like the tests above: that macro's `SINK`/
// `SHUTDOWN_REGISTRY` slots are real `static`s, and Shuttle re-runs the same
// test body many times in one process, so a `static`'s state would leak
// across iterations and invalidate the exploration.
#[cfg(all(test, shuttle, feature = "_shuttle"))]
mod shuttle_tests {
    use shuttle::sync::Mutex;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::{AttachHandle, ShutdownFn};
    use crate::shuttle_test;

    shuttle_test! {
        num_iters = 2_000, depth = 3;
        /// Registering a shutdown fn concurrently with `AttachHandle::drop` must
        /// never panic, and every racing fn must either run exactly once or be
        /// cleanly rejected.
        fn concurrent_register_and_drop() {
            const REGISTRARS: usize = 2;

            let ran = Arc::new(AtomicUsize::new(0));
            let rejected = Arc::new(AtomicUsize::new(0));
            let handle = AttachHandle::new(|| {});
            let weak = handle.shutdown_registry_weak();

            let registrars: Vec<_> = (0..REGISTRARS)
                .map(|_| {
                    let weak = weak.clone();
                    let ran = ran.clone();
                    let rejected = rejected.clone();
                    shuttle::thread::spawn(move || {
                        let accepted = weak.upgrade().is_some_and(|registry| {
                            let ran = ran.clone();
                            registry.push(ShutdownFn::new(move || {
                                ran.fetch_add(1, Ordering::SeqCst);
                            }))
                        });
                        if !accepted {
                            rejected.fetch_add(1, Ordering::SeqCst);
                        }
                    })
                })
                .collect();

            drop(handle); // must not panic

            for registrar in registrars {
                registrar.join().unwrap();
            }

            assert_eq!(
                ran.load(Ordering::SeqCst) + rejected.load(Ordering::SeqCst),
                REGISTRARS,
                "every racing registration must be accounted for exactly once (ran xor rejected)"
            );
        }
    }

    shuttle_test! {
        num_iters = 2_000, depth = 3;
        fn concurrent_register_and_forget() {
            let ran = Arc::new(AtomicUsize::new(0));
            let handle = AttachHandle::new(|| {});
            let weak = handle.shutdown_registry_weak();

            let ran2 = ran.clone();
            let registrar = shuttle::thread::spawn(move || {
                if let Some(registry) = weak.upgrade() {
                    registry.push(ShutdownFn::new(move || {
                        ran2.fetch_add(1, Ordering::SeqCst);
                    }));
                }
            });

            // required for shuttle to schedule `registrar` *during* `forget()`
            shuttle::thread::yield_now();

            handle.forget();
            registrar.join().unwrap();

            assert_eq!(
                ran.load(Ordering::SeqCst),
                0,
                "a fn registered around forget() must never run"
            );
        }
    }

    shuttle_test! {
        num_iters = 2_000, depth = 3;
        /// `ShutdownRegistry::push` racing itself
        fn concurrent_registrars_race_push() {
            const REGISTRARS: usize = 2;

            let ran = Arc::new(AtomicUsize::new(0));
            let handle = AttachHandle::new(|| {});
            let weak = handle.shutdown_registry_weak();

            let registrars: Vec<_> = (0..REGISTRARS)
                .map(|_| {
                    let weak = weak.clone();
                    let ran = ran.clone();
                    shuttle::thread::spawn(move || {
                        weak.upgrade()
                            .expect("handle still alive, upgrade must succeed")
                            .push(ShutdownFn::new(move || {
                                ran.fetch_add(1, Ordering::SeqCst);
                            }));
                    })
                })
                .collect();

            for registrar in registrars {
                registrar.join().unwrap();
            }

            drop(handle);

            assert_eq!(
                ran.load(Ordering::SeqCst),
                REGISTRARS,
                "every concurrently-registered fn must run exactly once"
            );
        }
    }

    shuttle_test! {
        num_iters = 2_000, depth = 3;
        fn concurrent_registrars_preserve_lifo_order() {
            const REGISTRARS: u32 = 2;

            let push_order = Arc::new(Mutex::new(Vec::new()));
            let run_order = Arc::new(Mutex::new(Vec::new()));
            let handle = AttachHandle::new(|| {});
            let weak = handle.shutdown_registry_weak();

            let registrars: Vec<_> = (0..REGISTRARS)
                .map(|i| {
                    let weak = weak.clone();
                    let push_order = push_order.clone();
                    let run_order = run_order.clone();
                    shuttle::thread::spawn(move || {
                        let registry = weak.upgrade().expect("handle still alive");
                        let mut push_order = push_order.lock().unwrap();
                        registry.push(ShutdownFn::new(move || {
                            run_order.lock().unwrap().push(i);
                        }));
                        push_order.push(i);
                    })
                })
                .collect();

            for registrar in registrars {
                registrar.join().unwrap();
            }

            drop(handle);

            let expected_run_order: Vec<_> = push_order.lock().unwrap().iter().rev().copied().collect();
            assert_eq!(
                *run_order.lock().unwrap(),
                expected_run_order,
                "shutdown fns must run in exact reverse of push order, regardless of how concurrent registration interleaves"
            );
        }
    }

    // Unlike other shuttle tests here, this one's `static ATTACHED` is real,
    // process-wide state, not fresh per call. Two sessions (pct/determinism)
    // touching that same static's lock at once corrupts Shuttle's own bookkeeping.
    // Serialize the two test fns instead.
    static SERIALIZE_PCT_AND_DETERMINISM: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Shuttle counterpart of `attach_never_observed_with_sink_set_but_registry_unset`,
    /// exhaustive instead of probabilistic.
    fn attach_race_never_observes_sink_without_registry() {
        metrique_writer::sink::global_entry_sink! { Sink }
        use metrique_writer::{AttachGlobalEntrySink, ShutdownFn as WriterShutdownFn};

        let attacher = shuttle::thread::spawn(|| {
            Sink::attach((metrique_writer::sink::DevNullSink::new(), ()))
        });

        let racer = shuttle::thread::spawn(|| {
            if Sink::try_sink().is_none() {
                // Not this schedule's interleaving.
                return None;
            }
            Some(std::panic::catch_unwind(std::panic::AssertUnwindSafe(
                || {
                    Sink::register_shutdown_fn(WriterShutdownFn::new(|| {}));
                },
            )))
        });

        let handle = attacher.join().unwrap();
        let racer_result = racer.join().unwrap();
        // Detach first so state doesn't leak into the next call.
        drop(handle);

        if let Some(Err(payload)) = racer_result {
            let msg = payload
                .downcast_ref::<&str>()
                .copied()
                .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
                .unwrap_or_default();
            assert!(
                !msg.contains("No sink attached"),
                "register_shutdown_fn() panicked with \"No sink attached\" even \
                 though try_sink() just observed a sink was attached"
            );
        }
    }

    // Recover from poisoning instead of propagating a confusing PoisonError.
    fn run_serialized(f: impl FnOnce()) {
        let _guard = SERIALIZE_PCT_AND_DETERMINISM
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        f();
    }

    #[test]
    fn attach_race_never_observes_sink_without_registry_pct() {
        run_serialized(|| {
            shuttle::check_pct(attach_race_never_observes_sink_without_registry, 5_000, 3)
        });
    }

    #[test]
    fn attach_race_never_observes_sink_without_registry_determinism() {
        run_serialized(|| {
            shuttle::check_uncontrolled_nondeterminism(
                attach_race_never_observes_sink_without_registry,
                5_000,
            )
        });
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
