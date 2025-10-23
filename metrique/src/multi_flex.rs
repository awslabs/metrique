use std::borrow::Cow;

use metrique_core::{
    CloseValue, InflectableEntry, NameStyle, NoPrefix,
    concat::{ConstStr, MaybeConstStr},
};
use metrique_writer::{EntryWriter, Value};
use metrique_writer_core::entry::SampleGroupElement;

/// Trait for items that can be used in a [`MultiFlex`] collection.
///
/// Items must be able to generate a prefix for their metrics based on their position
/// in the collection. This prefix is used to namespace the metrics for each item.
pub trait FlexItem {
    /// Generate a prefix for this item's metrics based on its index in the collection.
    ///
    /// The prefix should include separators (like `.0.`, `.1.`) to properly namespace
    /// the metrics. For example, returning `.{idx}.` will create metrics like
    /// `devices.0.size`, `devices.1.size`, etc.
    ///
    /// # Arguments
    /// * `idx` - The zero-based index of this item in the MultiFlex collection
    ///
    /// # Returns
    /// A string that will be used as a prefix for all metrics from this item
    fn prefix_item(&self, idx: usize) -> Cow<'static, str>;
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
/// #[metrics]
/// struct Device {
///     id: String,
///     size: usize,
/// }
///
/// impl FlexItem for Device {
///     fn prefix_item(&self, idx: usize) -> Cow<'static, str> {
///         Cow::Owned(format!(".{idx}."))
///     }
/// }
/// ```
///
/// This will emit metrics like `devices.0.size`, `devices.1.size`, etc.
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
}

/// The closed form of a [`MultiFlex`] collection, containing the processed entries
/// ready for metric emission.
///
/// This struct is created when a `MultiFlex<T>` is closed and contains each item
/// paired with its computed prefix. You typically don't interact with this type
/// directly - it's used internally by the metrics system.
pub struct MultiFlexEntry<T> {
    entries: Vec<(Cow<'static, str>, T)>,
}

impl<T: CloseValue + FlexItem> CloseValue for MultiFlex<T> {
    type Closed = MultiFlexEntry<T::Closed>;

    fn close(self) -> Self::Closed {
        MultiFlexEntry {
            entries: self
                .0
                .into_iter()
                .enumerate()
                .map(|(idx, entry)| (entry.prefix_item(idx), entry.close()))
                .collect(),
        }
    }
}

struct EmptyString;
impl ConstStr for EmptyString {
    const VAL: &'static str = "";
}

impl<T: InflectableEntry<NoPrefix<NS>>, NS: NameStyle> InflectableEntry<NS> for MultiFlexEntry<T> {
    fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
        //writer.value(Cow::Borrowed(self.key.as_ref()), &self.value);
        for (prefix, item) in &self.entries {
            let external_prefix = <NS::Inflect<EmptyString, EmptyString, EmptyString, EmptyString> as MaybeConstStr>::MAYBE_VAL;
            let prefix = format!("{external_prefix}{prefix}");

            let prefix = Cow::Owned(prefix);
            let mut prefixer = DynPrefix { prefix, writer };
            InflectableEntry::<NoPrefix<NS>>::write(item, &mut prefixer);
            item.write(&mut prefixer);
        }
    }

    fn sample_group(&self) -> impl Iterator<Item = SampleGroupElement> {
        // sample groups are ignored inside of multi-groups
        vec![].into_iter()
    }
}

struct DynPrefix<'b, T> {
    prefix: Cow<'static, str>,
    writer: &'b mut T,
}

impl<'a, 'b, T: EntryWriter<'a>> EntryWriter<'a> for DynPrefix<'b, T> {
    fn timestamp(&mut self, timestamp: std::time::SystemTime) {
        self.writer.timestamp(timestamp);
    }

    fn value(&mut self, name: impl Into<Cow<'a, str>>, value: &(impl Value + ?Sized)) {
        let prefix = &self.prefix;
        let name = name.into();
        let name = format!("{prefix}{name}");
        self.writer.value(name, value);
    }

    fn config(&mut self, config: &'a dyn metrique_writer::EntryConfig) {
        self.writer.config(config);
    }
}
