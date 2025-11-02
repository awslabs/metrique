use std::{borrow::Cow, fmt::Write};

use metrique_core::{
    CloseValue, CloseValueRef, InflectableEntry, NameStyle, NoPrefix,
    concat::{ConstStr, const_str_value},
};
use metrique_writer::{EntryWriter, Value};
use metrique_writer_core::entry::SampleGroupElement;
use smartstring::{LazyCompact, SmartString};

/// Trait for items that can be used in a [`MultiFlex`] collection.
///
/// Items must be able to generate a prefix for their metrics based on their position
/// in the collection. This prefix is used to namespace the metrics for each item.
pub trait FlexItem {
    /// Generate a prefix for this item's metrics based on its index in the collection.
    ///
    /// The prefix should include separators (like `.0.`, `.1.`) to properly namespace
    /// the metrics. For example, writing `.{idx}.` will create metrics like
    /// `devices.0.size`, `devices.1.size`, etc.
    ///
    /// # Arguments
    /// * `idx` - The zero-based index of this item in the MultiFlex collection
    /// * `buffer` - A string buffer to write the prefix into
    fn prefix_item(&self, idx: usize, buffer: impl Write);
}

/// A collection type for emitting metrics from a dynamic list of items.
///
/// `MultiFlex<T>` allows you to collect metrics from a variable number of similar items,
/// where each item's metrics are prefixed with its index in the collection. This is useful
/// for scenarios like:
/// - Multiple database connections with per-connection metrics
/// - API calls to different endpoints
/// - Processing stages with per-stage timing
/// - Device metrics from multiple devices
///
/// # Example
/// ```rust
/// use metrique::{multi_flex::{FlexItem, MultiFlex}, unit_of_work::metrics};
/// use std::borrow::Cow;
///
/// #[metrics]
/// struct MyMetrics {
///     #[metrics(flatten, prefix = "devices")]
///     devices: MultiFlex<Device>,
/// }
///
/// #[metrics(subfield)]
/// struct Device {
///     id: usize,
///     size: usize,
/// }
///
/// impl FlexItem for Device {
///     fn prefix_item(&self, idx: usize, buffer: &mut String) {
///         use std::fmt::Write;
///         write!(buffer, ".{idx}.").unwrap();
///     }
/// }
/// ```
///
/// This will emit metrics like `devices.0.size`, `devices.1.size`, etc.
#[derive(Clone, Debug)]
pub struct MultiFlex<T>(Vec<T>);

impl<T> Default for MultiFlex<T> {
    fn default() -> Self {
        Self(vec![])
    }
}

impl<T> MultiFlex<T> {
    /// Add an item to the MultiFlex collection.
    ///
    /// The item will be assigned the next available index and its metrics will be
    /// prefixed accordingly when the collection is emitted.
    ///
    /// # Arguments
    /// * `item` - The item to add to the collection
    pub fn push(&mut self, item: T) {
        self.0.push(item);
    }

    /// Creates a MulitFlex with a
    pub fn with_capacity(cap: usize) -> Self {
        Self(Vec::with_capacity(cap))
    }
}

/// The closed form of a [`MultiFlex`] collection, containing the processed entries
/// ready for metric emission.
///
/// This struct is created when a `MultiFlex<T>` is closed and contains each item
/// in both unclosed (for prefix computation) and closed (for writing) forms.
pub struct MultiFlexEntry<T, C> {
    entries: Vec<(usize, T, C)>,
}

impl<T: CloseValueRef + FlexItem> CloseValue for MultiFlex<T> {
    type Closed = MultiFlexEntry<T, T::Closed>;

    fn close(self) -> Self::Closed {
        MultiFlexEntry {
            entries: self
                .0
                .into_iter()
                .enumerate()
                .map(|(idx, item)| {
                    let closed = item.close_ref();
                    (idx, item, closed)
                })
                .collect(),
        }
    }
}

impl<T: CloseValueRef + FlexItem + Clone> CloseValue for &MultiFlex<T> {
    type Closed = MultiFlexEntry<T, T::Closed>;

    fn close(self) -> Self::Closed {
        MultiFlexEntry {
            entries: self
                .0
                .iter()
                .enumerate()
                .map(|(idx, item)| {
                    let closed = item.close_ref();
                    (idx, item.clone(), closed)
                })
                .collect(),
        }
    }
}

struct EmptyString;
impl ConstStr for EmptyString {
    const VAL: &'static str = "";
}

impl<T: FlexItem, C: InflectableEntry<NoPrefix<NS>>, NS: NameStyle> InflectableEntry<NS>
    for MultiFlexEntry<T, C>
{
    fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
        let external_prefix =
            const_str_value::<NS::Inflect<EmptyString, EmptyString, EmptyString, EmptyString>>();
        // Pre-allocate a string buffer for building prefixes
        let mut entry = SmartString::<LazyCompact>::new();

        for (idx, item, closed) in &self.entries {
            entry.truncate(0);
            item.prefix_item(*idx, &mut entry);

            let mut prefixer = DynPrefix {
                head: external_prefix.as_ref(),
                prefix: entry.as_str(),
                writer,
            };
            InflectableEntry::<NoPrefix<NS>>::write(closed, &mut prefixer);
        }
    }

    fn sample_group(&self) -> impl Iterator<Item = SampleGroupElement> {
        // sample groups are ignored inside of multi-groups
        vec![].into_iter()
    }
}

struct DynPrefix<'b, T> {
    head: &'b str,
    prefix: &'b str,
    writer: &'b mut T,
}

impl<'a, 'b, T: EntryWriter<'a>> EntryWriter<'a> for DynPrefix<'b, T> {
    fn timestamp(&mut self, timestamp: std::time::SystemTime) {
        self.writer.timestamp(timestamp);
    }

    fn value(&mut self, name: impl Into<Cow<'a, str>>, value: &(impl Value + ?Sized)) {
        let name = name.into();
        self.writer
            .value(format!("{}{}{}", self.head, self.prefix, name), value);
    }

    fn config(&mut self, config: &'a dyn metrique_writer::EntryConfig) {
        self.writer.config(config);
    }
}
