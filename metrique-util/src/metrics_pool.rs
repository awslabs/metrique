// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Collects independently-created metrics into a parent metric entry.
//!
//! [`MetricsPool`] is intended to be a `#[metrics(flatten)]` field. A
//! [`MetricsPoolHandle`] can be passed explicitly or installed while a future
//! is polled. Code without direct access to the parent can then use
//! [`MetricsPool::current`] to contribute metrics.

use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll};
use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap};
use std::fmt::Debug;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, SystemTime};

use inflector::Inflector;
use metrique::writer::rate_limit::rate_limited;
use metrique::writer::sink::FlushWait;
use metrique::writer::{
    BoxEntry, Entry, EntryConfig, EntrySink, EntryWriter, Value, core::Descriptors,
};
use metrique::{
    AppendAndCloseOnDrop, CloseValue, Identity, InflectableEntry, KebabCase, NameStyle, PascalCase,
    RootEntry, SnakeCase, append_and_close,
    concat::{EmptyConstStr, const_str_value},
};
use metrique_core::CloseEntry;

thread_local! {
    static CURRENT_POOL: RefCell<Vec<MetricsPoolHandle>> = const { RefCell::new(Vec::new()) };
}

fn intern_prefix(prefix: String) -> &'static str {
    static INTERNED: OnceLock<Mutex<HashMap<String, &'static str>>> = OnceLock::new();
    let interned = INTERNED.get_or_init(|| Mutex::new(HashMap::new()));
    let mut interned = interned.lock().unwrap();

    if let Some(existing) = interned.get(prefix.as_str()) {
        return existing;
    }

    let leaked = Box::leak(prefix.clone().into_boxed_str());
    interned.insert(prefix, leaked);
    leaked
}

#[derive(Clone, Copy, Debug)]
enum RuntimeStyle {
    Identity,
    PascalCase,
    SnakeCase,
    KebabCase,
}

impl RuntimeStyle {
    fn for_ns<NS: NameStyle>() -> Self {
        match NS::DESCRIPTOR_STYLE_INDEX {
            1 => Self::PascalCase,
            2 => Self::SnakeCase,
            3 => Self::KebabCase,
            _ => Self::Identity,
        }
    }

    fn prefix_segment(self, segment: &str) -> String {
        match self {
            Self::Identity => {
                let mut prefix = segment.to_string();
                if !prefix.ends_with('_') && !prefix.ends_with('-') {
                    prefix.push('_');
                }
                prefix
            }
            Self::PascalCase => segment.to_pascal_case(),
            Self::SnakeCase => {
                let mut prefix = segment.to_snake_case();
                if !prefix.ends_with('_') {
                    prefix.push('_');
                }
                prefix
            }
            Self::KebabCase => {
                let mut prefix = segment.to_kebab_case();
                if !prefix.ends_with('-') {
                    prefix.push('-');
                }
                prefix
            }
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct PrefixSet {
    identity: &'static str,
    pascal: &'static str,
    snake: &'static str,
    kebab: &'static str,
}

impl PrefixSet {
    fn empty() -> Self {
        Self {
            identity: "",
            pascal: "",
            snake: "",
            kebab: "",
        }
    }

    fn with_segments<I>(self, segments: I) -> Self
    where
        I: IntoIterator<Item = &'static str>,
    {
        let mut identity = self.identity.to_string();
        let mut pascal = self.pascal.to_string();
        let mut snake = self.snake.to_string();
        let mut kebab = self.kebab.to_string();

        for segment in segments {
            if segment.is_empty() {
                continue;
            }

            identity.push_str(&RuntimeStyle::Identity.prefix_segment(segment));
            pascal.push_str(&RuntimeStyle::PascalCase.prefix_segment(segment));
            snake.push_str(&RuntimeStyle::SnakeCase.prefix_segment(segment));
            kebab.push_str(&RuntimeStyle::KebabCase.prefix_segment(segment));
        }

        Self {
            identity: intern_prefix(identity),
            pascal: intern_prefix(pascal),
            snake: intern_prefix(snake),
            kebab: intern_prefix(kebab),
        }
    }

    fn with_exact_prefix(self, prefix: &'static str) -> Self {
        fn append(existing: &'static str, prefix: &'static str) -> &'static str {
            match (existing.is_empty(), prefix.is_empty()) {
                (_, true) => existing,
                (true, false) => prefix,
                (false, false) => intern_prefix(format!("{existing}{prefix}")),
            }
        }

        Self {
            identity: append(self.identity, prefix),
            pascal: append(self.pascal, prefix),
            snake: append(self.snake, prefix),
            kebab: append(self.kebab, prefix),
        }
    }

    fn for_style(self, style: RuntimeStyle) -> &'static str {
        match style {
            RuntimeStyle::Identity => self.identity,
            RuntimeStyle::PascalCase => self.pascal,
            RuntimeStyle::SnakeCase => self.snake,
            RuntimeStyle::KebabCase => self.kebab,
        }
    }
}

struct MetricsPoolInner {
    entries: Mutex<Option<Vec<BufferedEntry>>>,
}

impl MetricsPoolInner {
    fn new() -> Self {
        Self {
            entries: Mutex::new(Some(Vec::new())),
        }
    }

    fn push(&self, entry: BufferedEntry) {
        if let Some(entries) = self.entries.lock().unwrap().as_mut() {
            entries.push(entry);
        }
    }

    fn take(&self) -> Vec<BufferedEntry> {
        self.entries.lock().unwrap().take().unwrap_or_default()
    }
}

/// A metric field that collects child metrics and flattens them into its parent.
///
/// When child metrics produce the same fully-inflected field name, the last
/// appended value is retained and a warning is emitted at most once per minute.
///
/// Closing the pool takes the entries available at that point. Handles may
/// outlive the pool, but entries appended through them after close are discarded.
pub struct MetricsPool {
    inner: Arc<MetricsPoolInner>,
}

impl MetricsPool {
    /// Create an empty metrics pool.
    pub fn new() -> Self {
        Self {
            inner: Arc::new(MetricsPoolInner::new()),
        }
    }

    /// Return a cloneable handle to this pool.
    pub fn handle(&self) -> MetricsPoolHandle {
        MetricsPoolHandle {
            inner: Arc::clone(&self.inner),
            prefixes: PrefixSet::empty(),
        }
    }

    /// Install this pool while `future` is being polled.
    pub fn scope<F>(&self, future: F) -> MetricsPoolScope<F> {
        self.handle().scope(future)
    }

    /// Return the pool installed for the current future poll, if any.
    pub fn current() -> Option<MetricsPoolHandle> {
        CURRENT_POOL.with(|current| current.borrow().last().cloned())
    }
}

impl Default for MetricsPool {
    fn default() -> Self {
        Self::new()
    }
}

impl Debug for MetricsPool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let entry_count = self
            .inner
            .entries
            .lock()
            .unwrap()
            .as_ref()
            .map_or(0, Vec::len);

        f.debug_struct("MetricsPool")
            .field("entry_count", &entry_count)
            .finish()
    }
}

impl CloseValue for MetricsPool {
    type Closed = MetricsPoolEntry;

    fn close(self) -> Self::Closed {
        MetricsPoolEntry {
            entries: self.inner.take(),
        }
    }
}

/// Cloneable producer handle for a [`MetricsPool`].
#[derive(Clone)]
pub struct MetricsPoolHandle {
    inner: Arc<MetricsPoolInner>,
    prefixes: PrefixSet,
}

impl Debug for MetricsPoolHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MetricsPoolHandle")
            .field("identity_prefix", &self.prefixes.identity)
            .finish()
    }
}

impl MetricsPoolHandle {
    /// Return a handle with an additional inflected prefix path.
    ///
    /// Prefix segments are static because each distinct, fully inflected path
    /// is interned for use by entry descriptors.
    pub fn with_prefix<I>(&self, segments: I) -> Self
    where
        I: IntoIterator<Item = &'static str>,
    {
        Self {
            inner: Arc::clone(&self.inner),
            prefixes: self.prefixes.with_segments(segments),
        }
    }

    /// Return a handle with an additional literal, non-inflected prefix.
    ///
    /// Use this for punctuation-delimited names such as `sdk.request.`.
    pub fn with_exact_prefix(&self, prefix: &'static str) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
            prefixes: self.prefixes.with_exact_prefix(prefix),
        }
    }

    /// Install this pool while `future` is being polled.
    pub fn scope<F>(&self, future: F) -> MetricsPoolScope<F> {
        MetricsPoolScope {
            pool: self.clone(),
            future,
        }
    }

    /// Close and append `metric` immediately.
    pub fn append<E>(&self, metric: E)
    where
        E: CloseEntry + Send + Sync + 'static,
        E::Closed: InflectableEntry<Identity>
            + InflectableEntry<PascalCase>
            + InflectableEntry<SnakeCase>
            + InflectableEntry<KebabCase>
            + Send
            + Sync
            + 'static,
    {
        self.push_closed(metric.close());
    }

    /// Return an append-on-drop guard targeting this pool.
    pub fn append_on_drop<E>(&self, metric: E) -> AppendAndCloseOnDrop<E, Self>
    where
        E: CloseEntry + Send + Sync + 'static,
        E::Closed: InflectableEntry<Identity>
            + InflectableEntry<PascalCase>
            + InflectableEntry<SnakeCase>
            + InflectableEntry<KebabCase>
            + Send
            + Sync
            + 'static,
    {
        append_and_close(metric, self.clone())
    }

    /// Like [`MetricsPoolHandle::append_on_drop`], starting from `E::default()`.
    pub fn append_on_drop_default<E>(&self) -> AppendAndCloseOnDrop<E, Self>
    where
        E: Default + CloseEntry + Send + Sync + 'static,
        E::Closed: InflectableEntry<Identity>
            + InflectableEntry<PascalCase>
            + InflectableEntry<SnakeCase>
            + InflectableEntry<KebabCase>
            + Send
            + Sync
            + 'static,
    {
        self.append_on_drop(E::default())
    }

    fn push_closed<M>(&self, metric: M)
    where
        M: InflectableEntry<Identity>
            + InflectableEntry<PascalCase>
            + InflectableEntry<SnakeCase>
            + InflectableEntry<KebabCase>
            + Send
            + Sync
            + 'static,
    {
        self.inner.push(BufferedEntry::new(metric, self.prefixes));
    }
}

impl<M> EntrySink<RootEntry<M>> for MetricsPoolHandle
where
    M: InflectableEntry<Identity>
        + InflectableEntry<PascalCase>
        + InflectableEntry<SnakeCase>
        + InflectableEntry<KebabCase>
        + Send
        + Sync
        + 'static,
{
    fn append(&self, entry: RootEntry<M>) {
        self.push_closed(entry.into_inner());
    }

    fn flush_async(&self) -> FlushWait {
        FlushWait::ready()
    }
}

struct ScopeGuard;

impl ScopeGuard {
    fn install(pool: MetricsPoolHandle) -> Self {
        CURRENT_POOL.with(|current| current.borrow_mut().push(pool));
        Self
    }
}

impl Drop for ScopeGuard {
    fn drop(&mut self) {
        CURRENT_POOL.with(|current| {
            current.borrow_mut().pop();
        });
    }
}

/// Future wrapper returned by [`MetricsPool::scope`] and [`MetricsPoolHandle::scope`].
#[must_use = "futures do nothing unless polled"]
pub struct MetricsPoolScope<F> {
    pool: MetricsPoolHandle,
    future: F,
}

impl<F: Future> Future for MetricsPoolScope<F> {
    type Output = F::Output;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // Safety: once `self` is pinned, `future` is never moved again.
        let this = unsafe { self.get_unchecked_mut() };
        let _scope = ScopeGuard::install(this.pool.clone());
        // Safety: `future` stays pinned for the lifetime of `self`.
        unsafe { Pin::new_unchecked(&mut this.future) }.poll(cx)
    }
}

/// Closed representation of [`MetricsPool`].
#[doc(hidden)]
pub struct MetricsPoolEntry {
    entries: Vec<BufferedEntry>,
}

impl<NS: NameStyle> InflectableEntry<NS> for MetricsPoolEntry {
    fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
        let scan = self.scan_fields::<NS>();
        if !scan.collisions.is_empty() {
            rate_limited!(
                Duration::from_secs(60),
                tracing::warn!(
                    fields = ?scan.collisions,
                    "MetricsPool overwrote duplicate fields; later entries win"
                )
            );
        }

        let mut writer = OverwriteEntryWriter {
            inner: writer,
            winners: &scan.winners,
            next_occurrence: 0,
        };
        for entry in &self.entries {
            entry.write::<NS>(&mut writer);
        }
        debug_assert_eq!(writer.next_occurrence, scan.next_occurrence);
    }

    // Contributed entries must not implicitly influence the parent sampling policy.
    fn sample_group(
        &self,
    ) -> impl Iterator<Item = metrique::writer::core::entry::SampleGroupElement> {
        std::iter::empty()
    }

    fn descriptors(&self) -> Descriptors<'_> {
        if !self.scan_fields::<NS>().collisions.is_empty() {
            return Descriptors::Unavailable;
        }

        self.entries
            .iter()
            .fold(Descriptors::available(std::iter::empty()), |acc, entry| {
                acc.chain(entry.descriptors::<NS>())
            })
    }
}

impl MetricsPoolEntry {
    fn scan_fields<NS: NameStyle>(&self) -> FieldScan {
        let mut scan = FieldScan::default();
        for entry in &self.entries {
            entry.write::<NS>(&mut scan);
        }
        scan
    }
}

type NamespacePrefix<NS> =
    <NS as NameStyle>::Inflect<EmptyConstStr, EmptyConstStr, EmptyConstStr, EmptyConstStr>;

fn namespace_prefix<NS: NameStyle>() -> &'static str {
    match const_str_value::<NamespacePrefix<NS>>() {
        Cow::Borrowed(prefix) => prefix,
        Cow::Owned(prefix) => intern_prefix(prefix),
    }
}

#[derive(Default)]
struct FieldScan {
    winners: HashMap<String, usize>,
    collisions: BTreeSet<String>,
    next_occurrence: usize,
}

impl<'a> EntryWriter<'a> for FieldScan {
    fn timestamp(&mut self, _timestamp: SystemTime) {}

    fn value(&mut self, name: impl Into<Cow<'a, str>>, _value: &(impl Value + ?Sized)) {
        let name = name.into().into_owned();
        if self
            .winners
            .insert(name.clone(), self.next_occurrence)
            .is_some()
        {
            self.collisions.insert(name);
        }
        self.next_occurrence += 1;
    }

    fn config(&mut self, _config: &'a dyn EntryConfig) {}
}

struct OverwriteEntryWriter<'a, W> {
    inner: &'a mut W,
    winners: &'a HashMap<String, usize>,
    next_occurrence: usize,
}

impl<'a, W: EntryWriter<'a>> EntryWriter<'a> for OverwriteEntryWriter<'_, W> {
    fn timestamp(&mut self, timestamp: SystemTime) {
        self.inner.timestamp(timestamp);
    }

    fn value(&mut self, name: impl Into<Cow<'a, str>>, value: &(impl Value + ?Sized)) {
        let name = name.into();
        let occurrence = self.next_occurrence;
        self.next_occurrence += 1;

        if self.winners.get(name.as_ref()) == Some(&occurrence) {
            self.inner.value(name, value);
        }
    }

    fn config(&mut self, config: &'a dyn EntryConfig) {
        self.inner.config(config);
    }
}

struct BufferedEntry {
    prefixes: PrefixSet,
    entry: StyledEntries,
}

impl BufferedEntry {
    fn new<M>(metric: M, prefixes: PrefixSet) -> Self
    where
        M: InflectableEntry<Identity>
            + InflectableEntry<PascalCase>
            + InflectableEntry<SnakeCase>
            + InflectableEntry<KebabCase>
            + Send
            + Sync
            + 'static,
    {
        let metric = Arc::new(metric);
        Self {
            prefixes,
            entry: StyledEntries {
                identity: BoxEntry::new(RuntimeInflectedEntry::new(
                    Arc::clone(&metric),
                    RuntimeStyle::Identity,
                )),
                pascal: BoxEntry::new(RuntimeInflectedEntry::new(
                    Arc::clone(&metric),
                    RuntimeStyle::PascalCase,
                )),
                snake: BoxEntry::new(RuntimeInflectedEntry::new(
                    Arc::clone(&metric),
                    RuntimeStyle::SnakeCase,
                )),
                kebab: BoxEntry::new(RuntimeInflectedEntry::new(metric, RuntimeStyle::KebabCase)),
            },
        }
    }

    fn write<'a, NS: NameStyle>(&'a self, writer: &mut impl EntryWriter<'a>) {
        let style = RuntimeStyle::for_ns::<NS>();
        let prefix = self.prefix::<NS>(style);
        let entry = self.entry.for_style(style);

        if prefix.is_empty() {
            entry.write(writer);
            return;
        }

        let mut prefixed = PrefixedEntryWriter {
            inner: writer,
            prefix,
        };
        entry.write(&mut prefixed);
    }

    fn descriptors<NS: NameStyle>(&self) -> Descriptors<'_> {
        let style = RuntimeStyle::for_ns::<NS>();
        let prefix = self.prefix::<NS>(style);
        let descriptors = self.entry.for_style(style).descriptors();

        if prefix.is_empty() {
            descriptors
        } else {
            descriptors.map_available(|descriptor| descriptor.with_prefix(prefix))
        }
    }

    fn prefix<NS: NameStyle>(&self, style: RuntimeStyle) -> &'static str {
        let namespace = namespace_prefix::<NS>();
        let handle = self.prefixes.for_style(style);
        match (namespace.is_empty(), handle.is_empty()) {
            (_, true) => namespace,
            (true, false) => handle,
            (false, false) => intern_prefix(format!("{namespace}{handle}")),
        }
    }
}

struct StyledEntries {
    identity: BoxEntry,
    pascal: BoxEntry,
    snake: BoxEntry,
    kebab: BoxEntry,
}

impl StyledEntries {
    fn for_style(&self, style: RuntimeStyle) -> &BoxEntry {
        match style {
            RuntimeStyle::Identity => &self.identity,
            RuntimeStyle::PascalCase => &self.pascal,
            RuntimeStyle::SnakeCase => &self.snake,
            RuntimeStyle::KebabCase => &self.kebab,
        }
    }
}

struct RuntimeInflectedEntry<M> {
    entry: Arc<M>,
    style: RuntimeStyle,
}

impl<M> RuntimeInflectedEntry<M> {
    fn new(entry: Arc<M>, style: RuntimeStyle) -> Self {
        Self { entry, style }
    }
}

impl<M> Entry for RuntimeInflectedEntry<M>
where
    M: InflectableEntry<Identity>
        + InflectableEntry<PascalCase>
        + InflectableEntry<SnakeCase>
        + InflectableEntry<KebabCase>,
{
    fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
        match self.style {
            RuntimeStyle::Identity => InflectableEntry::<Identity>::write(&self.entry, writer),
            RuntimeStyle::PascalCase => InflectableEntry::<PascalCase>::write(&self.entry, writer),
            RuntimeStyle::SnakeCase => InflectableEntry::<SnakeCase>::write(&self.entry, writer),
            RuntimeStyle::KebabCase => InflectableEntry::<KebabCase>::write(&self.entry, writer),
        }
    }

    fn descriptors(&self) -> Descriptors<'_> {
        match self.style {
            RuntimeStyle::Identity => InflectableEntry::<Identity>::descriptors(&self.entry),
            RuntimeStyle::PascalCase => InflectableEntry::<PascalCase>::descriptors(&self.entry),
            RuntimeStyle::SnakeCase => InflectableEntry::<SnakeCase>::descriptors(&self.entry),
            RuntimeStyle::KebabCase => InflectableEntry::<KebabCase>::descriptors(&self.entry),
        }
    }
}

struct PrefixedEntryWriter<'a, W> {
    inner: &'a mut W,
    prefix: &'static str,
}

impl<'a, W: EntryWriter<'a>> EntryWriter<'a> for PrefixedEntryWriter<'_, W> {
    fn timestamp(&mut self, timestamp: std::time::SystemTime) {
        self.inner.timestamp(timestamp);
    }

    fn value(&mut self, name: impl Into<Cow<'a, str>>, value: &(impl Value + ?Sized)) {
        let name = name.into();
        let mut prefixed = String::with_capacity(self.prefix.len() + name.len());
        prefixed.push_str(self.prefix);
        prefixed.push_str(name.as_ref());
        self.inner.value(Cow::Owned(prefixed), value);
    }

    fn config(&mut self, config: &'a dyn EntryConfig) {
        self.inner.config(config);
    }
}

/// Install `pool` while `future` is being polled.
pub fn with_metrics_pool<F>(pool: MetricsPoolHandle, future: F) -> MetricsPoolScope<F> {
    pool.scope(future)
}
