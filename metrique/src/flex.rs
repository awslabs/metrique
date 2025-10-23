//! Utilities for dynamic metric fields
//!
//! This module contains `Flex`, a struct that allows dynamically controlling the key field for a given metric.
//!
//! `Flex` is useful when you need to create metric fields with names that are only known at runtime,
//! such as:
//! - User-provided tags or labels
//! - Configuration-driven field names
//! - Computed field names based on business logic
//! - Dynamic dimensions from external systems
//! - Runtime-determined metric names based on request context
//!
//! # Example
//!
//! ```rust
//! use metrique::{flex::Flex, unit_of_work::metrics};
//! use std::time::SystemTime;
//!
//! #[metrics]
//! struct DynamicMetrics {
//!     #[metrics(timestamp)]
//!     timestamp: SystemTime,
//!     operation: &'static str,
//!
//!     // Dynamic field - key name determined at runtime
//!     #[metrics(flatten)]
//!     dynamic_count: Flex<usize>,
//! }
//!
//! // Usage - flexible builder API
//! let field_name = format!("{}_requests", "api"); // "api_requests"
//! let metrics = DynamicMetrics {
//!     timestamp: SystemTime::now(),
//!     operation: "ProcessRequest",
//!     dynamic_count: Flex::new(field_name).with_value(42),
//! };
//! ```
use std::borrow::Cow;

use metrique_core::{CloseValue, InflectableEntry, NameStyle};
use metrique_writer::{Entry, EntryWriter, Value};
use metrique_writer_core::entry::SampleGroupElement;

/// A struct that allows dynamic specification of keys
pub struct Flex<T> {
    key: Cow<'static, str>,
    value: Option<T>,
}

impl<T> Flex<T> {
    /// Create a new `Flex` with the given key and no value initially.
    /// Use `with_value()` to set the value.
    ///
    /// # Example
    /// ```rust
    /// use metrique::flex::Flex;
    ///
    /// let metric = Flex::<usize>::new("dynamic_field").with_value(42);
    /// ```
    pub fn new(key: impl Into<Cow<'static, str>>) -> Self {
        Self {
            key: key.into(),
            value: None,
        }
    }

    /// Set the value for this `Flex` field.
    /// This is useful for builder-style construction.
    ///
    /// # Example
    /// ```rust
    /// use metrique::flex::Flex;
    ///
    /// let metric = Flex::new("field_name").with_value(42usize);
    /// ```
    pub fn with_value(mut self, value: T) -> Self {
        self.value = Some(value);
        self
    }

    /// Set an optional value for this `Flex` field.
    ///
    /// If the value is `None`, the field will not be included in the output.
    ///
    /// # Example
    /// ```rust
    /// use metrique::flex::Flex;
    ///
    /// let metric = Flex::new("field_name").with_optional_value(Some(42usize));
    /// let empty_metric = Flex::new("missing_field").with_optional_value(None::<usize>);
    /// ```
    pub fn with_optional_value(mut self, value: Option<T>) -> Self {
        self.value = value;
        self
    }

    /// Get a reference to the key name.
    pub fn key(&self) -> &str {
        &self.key
    }

    /// Get a reference to the value, if present.
    pub fn value(&self) -> Option<&T> {
        self.value.as_ref()
    }

    /// Update the value.
    pub fn set_value(&mut self, value: T) {
        self.value = Some(value);
    }

    /// Clear the value (set to `None`).
    pub fn clear_value(&mut self) {
        self.value = None;
    }
}

impl<T: Default> Flex<T> {
    /// Create a new `Flex` with the given key and a default value.
    ///
    /// # Example
    /// ```rust
    /// use metrique::flex::Flex;
    ///
    /// let metric = Flex::<usize>::new("count").with_default_value();
    /// // Equivalent to: Flex::new("count").with_value(0usize) for usize
    /// ```
    pub fn with_default_value(mut self) -> Self {
        self.value = Some(T::default());
        self
    }
}

/// The Entry type for [`Flex`]
pub struct FlexEntry<T> {
    key: Cow<'static, str>,
    value: Option<T>,
}

impl<T: Value> Entry for FlexEntry<T> {
    fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
        writer.value(Cow::Borrowed(self.key.as_ref()), &self.value);
    }
}

impl<T: Value, NS: NameStyle> InflectableEntry<NS> for FlexEntry<T> {
    fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
        writer.value(Cow::Borrowed(self.key.as_ref()), &self.value);
    }

    fn sample_group(&self) -> impl Iterator<Item = SampleGroupElement> {
        vec![].into_iter()
    }
}

impl<T: CloseValue> CloseValue for Flex<T> {
    type Closed = FlexEntry<T::Closed>;

    fn close(self) -> Self::Closed {
        FlexEntry {
            key: self.key,
            value: self.value.close(),
        }
    }
}
