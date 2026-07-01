// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Entry descriptors: compile-time structural metadata for macro-derived entries.
//!
//! Sinks interact with [`DescriptorRef`], which provides resolved field names,
//! flags, shapes, and units. The underlying storage types ([`EntryDescriptor`],
//! [`FieldDescriptor`]) are public for macro construction but sinks should use
//! [`DescriptorRef`] and [`FieldView`] accessors.
//!
//! See the ["Recipe: a descriptor-aware sink"](https://docs.rs/metrique/latest/metrique/_guide/extending/index.html#recipe-a-descriptor-aware-sink)
//! section in the extending guide for usage patterns and best practices.

use std::any::TypeId;

use smallvec::SmallVec;

use crate::Unit;

/// A descriptor name style definition.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct Style {
    /// Index into per-style name arrays.
    pub index: u8,
    /// Human-readable name for this style (used in codegen identifiers).
    pub name: &'static str,
}

/// All supported descriptor name styles. Single source of truth for style ordering.
///
/// Adding a new style means adding an entry here and a corresponding method on
/// [`FieldDescriptorBuilder`].
#[non_exhaustive]
pub struct Styles;

impl Styles {
    /// No transformation (field name used as declared).
    pub const PRESERVE: Style = Style {
        index: 0,
        name: "preserve",
    };
    /// PascalCase.
    pub const PASCAL: Style = Style {
        index: 1,
        name: "PascalCase",
    };
    /// snake_case.
    pub const SNAKE: Style = Style {
        index: 2,
        name: "snake_case",
    };
    /// kebab-case.
    pub const KEBAB: Style = Style {
        index: 3,
        name: "kebab-case",
    };
    /// All styles in index order.
    pub const ALL: [Style; 4] = [Self::PRESERVE, Self::PASCAL, Self::SNAKE, Self::KEBAB];
    /// Number of styles.
    pub const COUNT: usize = Self::ALL.len();
}

/// Static descriptor storage for a macro-derived entry.
#[derive(Debug)]
pub struct EntryDescriptor {
    name: &'static str,
    fields: &'static [FieldDescriptor],
    timestamp: Option<TimestampDescriptor>,
}

impl EntryDescriptor {
    /// Create a builder for an [`EntryDescriptor`] with the given name and fields.
    pub const fn builder(
        name: &'static str,
        fields: &'static [FieldDescriptor],
    ) -> EntryDescriptorBuilder {
        EntryDescriptorBuilder {
            name,
            fields,
            timestamp: None,
        }
    }
}

/// Static field storage. Stores resolved names for all name styles.
#[derive(Debug)]
pub struct FieldDescriptor {
    names: [&'static str; Styles::COUNT],
    flags: &'static [FieldFlag],
    skipped_flags: &'static [FieldFlag],
    shape: FieldShape<'static>,
    unit: Option<Unit>,
}

impl FieldDescriptor {
    /// Create a builder for a [`FieldDescriptor`] with a fixed name used for all styles.
    ///
    /// The name is used verbatim regardless of the parent's `rename_all` style.
    /// Call `.pascal()`, `.snake()`, `.kebab()` on the builder to set
    /// style-specific names if needed.
    pub const fn builder(name: &'static str) -> FieldDescriptorBuilder {
        FieldDescriptorBuilder {
            names: [name; Styles::COUNT],
            flags: &[],
            skipped_flags: &[],
            shape: FieldShape::Opaque,
            unit: None,
        }
    }
}

/// Describes the timestamp field of an entry.
#[derive(Debug)]
pub struct TimestampDescriptor {
    name: &'static str,
}

impl TimestampDescriptor {
    /// Create a [`TimestampDescriptor`] with the given name.
    pub const fn new(name: &'static str) -> Self {
        Self { name }
    }

    /// Field name as emitted through `EntryWriter::timestamp`.
    pub fn name(&self) -> &str {
        self.name
    }
}

/// Result of calling `Entry::descriptors()`.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum Descriptors<'a> {
    /// Descriptors are available for this entry.
    Available(AvailableDescriptors<'a>),
    /// This entry has not implemented descriptor support.
    Unavailable,
}

/// Opaque container of available descriptor segments.
#[derive(Debug, Clone)]
pub struct AvailableDescriptors<'a>(SmallVec<[DescriptorRef<'a>; 2]>);

impl<'a> AvailableDescriptors<'a> {
    /// Iterate over the descriptor segments in write order.
    pub fn iter(&self) -> impl Iterator<Item = &DescriptorRef<'a>> {
        self.0.iter()
    }

    /// Number of descriptor segments.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether there are no descriptor segments.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl<'a> std::ops::Index<usize> for AvailableDescriptors<'a> {
    type Output = DescriptorRef<'a>;
    fn index(&self, index: usize) -> &Self::Output {
        &self.0[index]
    }
}

impl<'a> IntoIterator for AvailableDescriptors<'a> {
    type Item = DescriptorRef<'a>;
    type IntoIter = DescriptorIter<'a>;
    fn into_iter(self) -> Self::IntoIter {
        DescriptorIter(self.0.into_iter())
    }
}

/// Owned iterator over descriptor segments. Returned by `AvailableDescriptors::into_iter()`.
#[derive(Debug)]
pub struct DescriptorIter<'a>(smallvec::IntoIter<[DescriptorRef<'a>; 2]>);

impl<'a> Iterator for DescriptorIter<'a> {
    type Item = DescriptorRef<'a>;
    fn next(&mut self) -> Option<Self::Item> {
        self.0.next()
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.0.size_hint()
    }
}

impl<'a> ExactSizeIterator for DescriptorIter<'a> {}

impl<'a> Descriptors<'a> {
    /// Create an `Available` result from an iterator of descriptors.
    pub fn available(iter: impl IntoIterator<Item = DescriptorRef<'a>>) -> Self {
        Descriptors::Available(AvailableDescriptors(iter.into_iter().collect()))
    }

    /// Returns true if descriptors are available.
    pub fn is_available(&self) -> bool {
        matches!(self, Descriptors::Available(_))
    }

    /// Returns the available descriptors, panicking if unavailable.
    ///
    /// # Panics
    /// Panics if `self` is `Unavailable`.
    pub fn unwrap(self) -> AvailableDescriptors<'a> {
        match self {
            Descriptors::Available(v) => v,
            Descriptors::Unavailable => panic!("called unwrap() on Descriptors::Unavailable"),
        }
    }

    /// Convert to `Option<AvailableDescriptors>`, returning `None` if unavailable.
    pub fn into_available(self) -> Option<AvailableDescriptors<'a>> {
        match self {
            Descriptors::Available(v) => Some(v),
            Descriptors::Unavailable => None,
        }
    }

    /// Apply a transformation to each descriptor ref. Preserves Unavailable.
    pub fn map_available(self, f: impl FnMut(DescriptorRef<'a>) -> DescriptorRef<'a>) -> Self {
        match self {
            Descriptors::Available(a) => {
                let mapped: SmallVec<[DescriptorRef<'a>; 2]> = a.0.into_iter().map(f).collect();
                Descriptors::Available(AvailableDescriptors(mapped))
            }
            Descriptors::Unavailable => Descriptors::Unavailable,
        }
    }

    /// Chain two descriptor results. If both are `Available`, their segments are
    /// concatenated in write order. If either is `Unavailable`, the result is
    /// `Unavailable`.
    pub fn chain(self, other: Descriptors<'a>) -> Self {
        match (self, other) {
            (Descriptors::Available(mut a), Descriptors::Available(b)) => {
                a.0.extend(b.0);
                Descriptors::Available(a)
            }
            _ => Descriptors::Unavailable,
        }
    }
}

/// A descriptor segment describing a contiguous group of fields in an entry's
/// write output. Provides resolved field names, flags, shapes, and units.
///
/// Sinks obtain these by calling [`Entry::descriptors()`](crate::Entry::descriptors).
/// Simple entries yield one segment; composed entries (aggregation results,
/// entries with flattened children) yield multiple segments in write order.
///
/// # Example
///
/// ```ignore
/// for desc in entry.descriptors() {
///     for field in desc.fields() {
///         let name_parts = field.name_parts(); // prefixes then base name
///         let base = field.base_name();        // just the field name
///         let flags = field.flags();           // resolved flags
///         let shape = field.shape();
///         let unit = field.unit();
///     }
/// }
/// ```
#[derive(Clone, Debug)]
pub struct DescriptorRef<'a> {
    descriptor: &'a EntryDescriptor,
    id: DescriptorId,
    prefixes: SmallVec<[&'static str; 1]>,
    style_index: u8,
    extra_flags: &'static [FieldFlag],
}

impl<'a> DescriptorRef<'a> {
    /// Create a `DescriptorRef` from a `&'static EntryDescriptor`.
    #[doc(hidden)]
    pub fn from_static(
        descriptor: &'static EntryDescriptor,
        style_index: u8,
    ) -> DescriptorRef<'static> {
        let id = DescriptorId::compute(descriptor, &[]);
        DescriptorRef {
            descriptor,
            id,
            prefixes: SmallVec::new(),
            style_index,
            extra_flags: &[],
        }
    }

    /// Add a prefix to be prepended to all field names in this segment.
    /// Multiple calls stack (outermost prefix first).
    #[doc(hidden)]
    pub fn with_prefix(mut self, prefix: &'static str) -> Self {
        self.prefixes.insert(0, prefix);
        self.id = DescriptorId::compute(self.descriptor, &self.prefixes);
        self
    }

    /// Add extra flags to all fields in this segment (from flatten-site `default_flags`).
    /// These are merged with each field's own flags at access time, respecting
    /// field-level skips (flags in a field's `skipped_flags` are never added).
    #[doc(hidden)]
    pub fn with_extra_flags(mut self, flags: &'static [FieldFlag]) -> Self {
        self.extra_flags = flags;
        self
    }

    /// Stable identity for caching. Incorporates the base descriptor and any modifiers.
    pub fn id(&self) -> DescriptorId {
        self.id
    }

    /// Canonical name of this entry type.
    pub fn name(&self) -> &str {
        self.descriptor.name
    }

    /// Number of fields in this descriptor segment.
    pub fn fields_len(&self) -> usize {
        self.descriptor.fields.len()
    }

    /// The canonical timestamp field, if the entry has one.
    pub fn timestamp(&self) -> Option<&TimestampDescriptor> {
        self.descriptor.timestamp.as_ref()
    }

    /// Iterate over fields as [`FieldView`]s with all modifiers applied.
    pub fn fields(&self) -> impl Iterator<Item = FieldView<'_>> {
        (0..self.descriptor.fields.len()).map(move |i| FieldView { desc: self, idx: i })
    }
}

/// A view of a single field with modifiers applied.
#[derive(Clone, Debug)]
pub struct FieldView<'a> {
    desc: &'a DescriptorRef<'a>,
    idx: usize,
}

impl<'a> FieldView<'a> {
    /// Name parts in order: prefixes (outermost first) then base name.
    /// Concatenate to get the full resolved field name.
    pub fn name_parts(&self) -> impl Iterator<Item = &str> {
        self.desc.prefixes.iter().copied().chain(std::iter::once(
            self.desc.descriptor.fields[self.idx].names[self.desc.style_index as usize],
        ))
    }

    /// Just the base field name without any prefixes.
    pub fn base_name(&self) -> &'static str {
        self.desc.descriptor.fields[self.idx].names[self.desc.style_index as usize]
    }
    /// Flags applied to this field, including any extra flags from flatten-site defaults.
    /// Field-level skips take precedence: flags in the field's `skipped_flags` are never included.
    pub fn flags(&self) -> impl Iterator<Item = &'a FieldFlag> {
        let field = &self.desc.descriptor.fields[self.idx];
        let skipped = field.skipped_flags;
        field.flags.iter().chain(
            self.desc
                .extra_flags
                .iter()
                .filter(move |ef| !skipped.iter().any(|s| s.type_id() == ef.type_id())),
        )
    }

    /// Shape of this field.
    pub fn shape(&self) -> FieldShape<'a> {
        self.desc.descriptor.fields[self.idx].shape
    }

    /// Unit of this field.
    pub fn unit(&self) -> Option<Unit> {
        self.desc.descriptor.fields[self.idx].unit
    }
}

/// Opaque identifier for a descriptor segment, stable within a process lifetime.
///
/// Intended for caching and deduplication by sinks. Two `DescriptorRef`s backed by
/// the same static with the same modifiers produce equal ids. Collisions are
/// theoretically possible (weak hash) but extremely unlikely in practice.
///
/// For a single cache key covering an entire entry (all segments), combine the
/// sequence of ids from `entry.descriptors()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DescriptorId(u64);

impl DescriptorId {
    // TODO: consider using fxhash instead to be a bit more collision resistant
    fn compute(descriptor: &EntryDescriptor, prefixes: &[&'static str]) -> Self {
        let mut id = descriptor as *const EntryDescriptor as u64;
        for p in prefixes {
            id = id.wrapping_mul(31).wrapping_add(p.as_ptr() as u64);
        }
        DescriptorId(id)
    }
}

/// The closed/emitted shape of a field.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldShape<'a> {
    /// A known scalar shape.
    Known(KnownShape),
    /// An optional wrapper around an inner shape.
    Optional(ShapeRef<'a>),
    /// A dynamic-key map.
    Flex {
        /// The key shape.
        key: StringShape,
        /// The value shape.
        value: ShapeRef<'a>,
    },
    /// A list/sequence.
    List(ShapeRef<'a>),
    /// Shape not statically known.
    Opaque,
}

/// Known scalar shapes.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KnownShape {
    /// Boolean
    Bool,
    /// Unsigned 8-bit integer
    U8,
    /// Unsigned 16-bit integer
    U16,
    /// Unsigned 32-bit integer
    U32,
    /// Unsigned 64-bit integer
    U64,
    /// Signed 8-bit integer
    I8,
    /// Signed 16-bit integer
    I16,
    /// Signed 32-bit integer
    I32,
    /// Signed 64-bit integer
    I64,
    /// 32-bit floating point
    F32,
    /// 64-bit floating point
    F64,
    /// String
    String,
    /// Byte slice
    Bytes,
}

/// String shape variants for map keys.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StringShape {
    /// Standard string.
    String,
}

/// Opaque handle to a nested [`FieldShape`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShapeRef<'a> {
    inner: &'a FieldShape<'a>,
}

impl<'a> ShapeRef<'a> {
    /// Borrow the underlying shape.
    pub fn get(&self) -> &FieldShape<'a> {
        self.inner
    }

    /// Create a ShapeRef wrapping an inner FieldShape.
    pub const fn new(inner: &'a FieldShape<'a>) -> Self {
        Self { inner }
    }
}

/// A resolved field flag representing a `FlagConstructor` applied to a field.
///
/// Identifies a flag applied to a field, storing both the `TypeId` for identity
/// comparison and a constructor for obtaining the flag's runtime value.
///
/// # Dylib caveat
///
/// `TypeId` is not guaranteed stable across separately compiled shared libraries.
/// If your application loads metrique-using code via `dlopen`, flag identity checks
/// may not work across the dylib boundary. This is a Rust language limitation, not
/// specific to metrique.
///
/// # Examples
///
/// Sinks check for specific flags using [`is`](Self::is):
///
/// ```ignore
/// use my_format::flags::HighStorageResolution;
///
/// for field in descriptor.fields() {
///     if field.flags().any(|f| f.is::<HighStorageResolution>()) {
///         // this field has high storage resolution
///     }
/// }
/// ```
///
/// Sinks that need the flag's runtime data can call [`construct`](Self::construct):
///
/// ```ignore
/// for flag in field.flags() {
///     let metric_flags = flag.construct();
///     // use metric_flags for format-specific behavior
/// }
/// ```
#[derive(Debug)]
pub struct FieldFlag {
    type_id: TypeId,
    construct: fn() -> crate::value::MetricFlags<'static>,
}

impl FieldFlag {
    /// Create a flag from a `FlagConstructor` type.
    pub const fn new<T: crate::value::FlagConstructor + 'static>() -> Self {
        Self {
            type_id: TypeId::of::<T>(),
            construct: T::construct,
        }
    }

    /// The [`TypeId`] of the `FlagConstructor` type.
    pub fn type_id(&self) -> TypeId {
        self.type_id
    }

    /// Check if this flag matches a specific `FlagConstructor` type.
    pub fn is<T: 'static>(&self) -> bool {
        self.type_id == TypeId::of::<T>()
    }

    /// Construct the [`MetricFlags`](crate::value::MetricFlags) value for this flag.
    ///
    /// This calls the `FlagConstructor::construct()` method of the original flag type,
    /// giving sinks access to the flag's runtime data directly from the descriptor
    /// without requiring the write path.
    pub fn construct(&self) -> crate::value::MetricFlags<'static> {
        (self.construct)()
    }
}

/// Builder for [`EntryDescriptor`].
pub struct EntryDescriptorBuilder {
    name: &'static str,
    fields: &'static [FieldDescriptor],
    timestamp: Option<TimestampDescriptor>,
}

impl EntryDescriptorBuilder {
    /// Set the timestamp descriptor.
    pub const fn timestamp(mut self, ts: TimestampDescriptor) -> Self {
        self.timestamp = Some(ts);
        self
    }

    /// Set the timestamp from an Option
    pub const fn maybe_timestamp(mut self, ts: Option<TimestampDescriptor>) -> Self {
        self.timestamp = ts;
        self
    }

    /// Build the [`EntryDescriptor`].
    pub const fn build(self) -> EntryDescriptor {
        EntryDescriptor {
            name: self.name,
            fields: self.fields,
            timestamp: self.timestamp,
        }
    }
}

/// Builder for [`FieldDescriptor`].
pub struct FieldDescriptorBuilder {
    names: [&'static str; Styles::COUNT],
    flags: &'static [FieldFlag],
    skipped_flags: &'static [FieldFlag],
    shape: FieldShape<'static>,
    unit: Option<Unit>,
}

impl FieldDescriptorBuilder {
    /// Set the PascalCase name for this field.
    pub const fn pascal(mut self, name: &'static str) -> Self {
        self.names[Styles::PASCAL.index as usize] = name;
        self
    }

    /// Set the snake_case name for this field.
    pub const fn snake(mut self, name: &'static str) -> Self {
        self.names[Styles::SNAKE.index as usize] = name;
        self
    }

    /// Set the kebab-case name for this field.
    pub const fn kebab(mut self, name: &'static str) -> Self {
        self.names[Styles::KEBAB.index as usize] = name;
        self
    }

    /// Set the flags for this field.
    pub const fn flags(mut self, flags: &'static [FieldFlag]) -> Self {
        self.flags = flags;
        self
    }

    /// Set the skipped flags for this field. These flags were explicitly opted out
    /// at field level and will not be added by flatten-site `default_flags`.
    pub const fn skipped_flags(mut self, flags: &'static [FieldFlag]) -> Self {
        self.skipped_flags = flags;
        self
    }

    /// Set the shape for this field.
    pub const fn shape(mut self, shape: FieldShape<'static>) -> Self {
        self.shape = shape;
        self
    }

    /// Set the unit for this field.
    pub const fn unit(mut self, unit: Unit) -> Self {
        self.unit = Some(unit);
        self
    }

    /// Set the unit from an Option
    pub const fn maybe_unit(mut self, unit: Option<Unit>) -> Self {
        self.unit = unit;
        self
    }

    /// Build the [`FieldDescriptor`].
    pub const fn build(self) -> FieldDescriptor {
        FieldDescriptor {
            names: self.names,
            flags: self.flags,
            skipped_flags: self.skipped_flags,
            shape: self.shape,
            unit: self.unit,
        }
    }
}

// Static assert: the number of named style setters on FieldDescriptorBuilder
// (pascal, snake, kebab = 3, plus the base preserve slot = 4) must match Styles::COUNT.
// If you add a new Style to Styles::ALL, add a corresponding builder method.
const _: () = assert!(
    Styles::COUNT == 4,
    "Styles::COUNT changed; update FieldDescriptorBuilder with a new style method"
);
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptor_ref_stable_id() {
        static DESC: EntryDescriptor = EntryDescriptor::builder("Test", &[]).build();
        let r1 = DescriptorRef::from_static(&DESC, 0);
        let r2 = DescriptorRef::from_static(&DESC, 0);
        assert_eq!(r1.id(), r2.id());
        assert_eq!(r1.name(), "Test");
    }

    #[test]
    fn different_descriptors_different_ids() {
        static A: EntryDescriptor = EntryDescriptor::builder("A", &[]).build();
        static B: EntryDescriptor = EntryDescriptor::builder("B", &[]).build();
        assert_ne!(
            DescriptorRef::from_static(&A, 0).id(),
            DescriptorRef::from_static(&B, 0).id()
        );
    }

    #[test]
    fn prefix_changes_id() {
        static DESC: EntryDescriptor = EntryDescriptor::builder("T", &[]).build();
        let plain = DescriptorRef::from_static(&DESC, 0);
        let prefixed = DescriptorRef::from_static(&DESC, 0).with_prefix("Api");
        assert_ne!(plain.id(), prefixed.id());
    }

    #[test]
    fn field_name_no_prefix() {
        static FIELDS: [FieldDescriptor; 1] = [FieldDescriptor::builder("MyField").build()];
        static DESC: EntryDescriptor = EntryDescriptor::builder("T", &FIELDS).build();

        let d = DescriptorRef::from_static(&DESC, 0);
        assert_eq!(d.fields().next().unwrap().base_name(), "MyField");
    }

    #[test]
    fn field_name_with_prefix() {
        static FIELDS: [FieldDescriptor; 1] = [FieldDescriptor::builder("Latency").build()];
        static DESC: EntryDescriptor = EntryDescriptor::builder("T", &FIELDS).build();

        let d = DescriptorRef::from_static(&DESC, 0).with_prefix("Api");
        let fields: Vec<_> = d.fields().collect();
        let parts: Vec<&str> = fields[0].name_parts().collect();
        assert_eq!(parts, vec!["Api", "Latency"]);
    }

    #[test]
    fn field_name_with_nested_prefixes() {
        static FIELDS: [FieldDescriptor; 1] = [FieldDescriptor::builder("Latency").build()];
        static DESC: EntryDescriptor = EntryDescriptor::builder("T", &FIELDS).build();

        // Simulates: inner child applies "Api", then outer parent applies "Http"
        let d = DescriptorRef::from_static(&DESC, 0)
            .with_prefix("Api")
            .with_prefix("Http");
        let fields: Vec<_> = d.fields().collect();
        let parts: Vec<&str> = fields[0].name_parts().collect();
        assert_eq!(parts, vec!["Http", "Api", "Latency"]);
    }

    #[test]
    fn field_view_iteration() {
        static FIELDS: [FieldDescriptor; 2] = [
            FieldDescriptor::builder("Alpha").build(),
            FieldDescriptor::builder("Beta").unit(Unit::Count).build(),
        ];
        static DESC: EntryDescriptor = EntryDescriptor::builder("T", &FIELDS).build();

        let d = DescriptorRef::from_static(&DESC, 0);
        let fields: Vec<_> = d.fields().collect();
        assert_eq!(fields.len(), 2);
        assert_eq!(fields[0].base_name(), "Alpha");
        assert_eq!(fields[1].base_name(), "Beta");
        assert_eq!(fields[1].unit(), Some(Unit::Count));
    }

    #[test]
    fn timestamp() {
        static DESC: EntryDescriptor = EntryDescriptor::builder("E", &[])
            .timestamp(TimestampDescriptor::new("ts"))
            .build();
        let d = DescriptorRef::from_static(&DESC, 0);
        assert_eq!(d.timestamp().unwrap().name(), "ts");
    }

    #[test]
    fn hand_written_entry_empty() {
        use crate::{Entry, EntryWriter};
        struct HandWritten;
        impl Entry for HandWritten {
            fn write<'a>(&'a self, _w: &mut impl EntryWriter<'a>) {}
        }
        assert_eq!(HandWritten.descriptors().is_available(), false);
    }

    #[test]
    fn boxentry_forwards() {
        use crate::{BoxEntry, Entry, EntryWriter};
        static DESC: EntryDescriptor = EntryDescriptor::builder("X", &[]).build();
        struct WithDesc;
        impl Entry for WithDesc {
            fn write<'a>(&'a self, _w: &mut impl EntryWriter<'a>) {}
            fn descriptors(&self) -> Descriptors<'_> {
                Descriptors::available(std::iter::once(DescriptorRef::from_static(&DESC, 0)))
            }
        }
        let boxed = BoxEntry::new(WithDesc);
        let descs = boxed.descriptors().unwrap();
        assert_eq!(descs.len(), 1);
        assert_eq!(descs[0].name(), "X");
    }
}
