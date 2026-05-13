// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Entry descriptors: compile-time structural metadata for macro-derived entries.
//!
//! Sinks interact with [`DescriptorRef`], which provides resolved field names,
//! tags, shapes, and units. The underlying storage types ([`EntryDescriptor`],
//! [`FieldDescriptor`]) are public for macro construction but sinks should use
//! [`DescriptorRef`] and [`FieldView`] accessors.

use std::any::TypeId;

use smallvec::SmallVec;

use crate::Unit;

/// Static descriptor storage for a macro-derived entry.
#[derive(Debug)]
pub struct EntryDescriptor {
    name: &'static str,
    fields: &'static [FieldDescriptor],
    timestamp: Option<TimestampDescriptor>,
}

impl EntryDescriptor {
    /// Hidden constructor for use by the metrique macro only.
    #[doc(hidden)]
    pub const fn __metrique_private_new(
        name: &'static str,
        fields: &'static [FieldDescriptor],
        timestamp: Option<TimestampDescriptor>,
    ) -> Self {
        Self {
            name,
            fields,
            timestamp,
        }
    }
}

/// Static field storage. Stores a single resolved name for one name style.
#[derive(Debug)]
pub struct FieldDescriptor {
    name: &'static str,
    flags: &'static [FieldFlag],
    shape: FieldShape<'static>,
    unit: Option<Unit>,
}

impl FieldDescriptor {
    /// Hidden constructor for use by the metrique macro only.
    #[doc(hidden)]
    pub const fn __metrique_private_new(
        name: &'static str,
        flags: &'static [FieldFlag],
        shape: FieldShape<'static>,
        unit: Option<Unit>,
    ) -> Self {
        Self {
            name,
            flags,
            shape,
            unit,
        }
    }
}

/// Describes the timestamp field of an entry.
#[derive(Debug)]
pub struct TimestampDescriptor {
    name: &'static str,
}

impl TimestampDescriptor {
    /// Field name as emitted through `EntryWriter::timestamp`.
    pub fn name(&self) -> &str {
        self.name
    }

    /// Hidden constructor for use by the metrique macro only.
    #[doc(hidden)]
    pub const fn __metrique_private_new(name: &'static str) -> Self {
        Self { name }
    }
}

/// Result of calling `Entry::descriptors()`.
#[derive(Debug)]
pub enum Descriptors<'a> {
    /// Descriptors are available for this entry.
    Available(AvailableDescriptors<'a>),
    /// This entry has not implemented descriptor support.
    Unavailable,
}

/// Opaque container of available descriptor segments.
#[derive(Debug)]
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
///         let flags = field.flags();             // resolved flags
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
}

impl<'a> DescriptorRef<'a> {
    /// Create a `DescriptorRef` from a `&'static EntryDescriptor`.
    #[doc(hidden)]
    pub fn from_static(descriptor: &'static EntryDescriptor) -> DescriptorRef<'static> {
        let id = DescriptorId::compute(descriptor, &[]);
        DescriptorRef {
            descriptor,
            id,
            prefixes: SmallVec::new(),
        }
    }

    /// Add a prefix to be prepended to all field names in this segment.
    /// Multiple calls stack (outermost prefix first).
    #[doc(hidden)]
    pub fn with_prefix(mut self, prefix: &'static str) -> Self {
        self.prefixes.push(prefix);
        self.id = DescriptorId::compute(self.descriptor, &self.prefixes);
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
        self.desc
            .prefixes
            .iter()
            .copied()
            .chain(std::iter::once(self.desc.descriptor.fields[self.idx].name))
    }

    /// Just the base field name without any prefixes.
    pub fn base_name(&self) -> &'static str {
        self.desc.descriptor.fields[self.idx].name
    }
    /// Flags applied to this field.
    pub fn flags(&self) -> impl Iterator<Item = &'a FieldFlag> {
        self.desc.descriptor.fields[self.idx].flags.iter()
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

    /// Hidden constructor for use by the metrique macro only.
    #[doc(hidden)]
    pub const fn __metrique_private_new(inner: &'a FieldShape<'a>) -> Self {
        Self { inner }
    }
}

/// A resolved field flag representing a `FlagConstructor` applied to a field.
///
/// Stores the `TypeId` of the `FlagConstructor` type for identity comparison.
/// Sinks that need the `MetricFlags` value call `FlagConstructor::construct()`
/// on the type they're checking for.
#[derive(Debug)]
pub struct FieldFlag {
    type_id: TypeId,
}

impl FieldFlag {
    /// The [`TypeId`] of the `FlagConstructor` type.
    pub fn type_id(&self) -> TypeId {
        self.type_id
    }

    /// Check if this flag matches a specific `FlagConstructor` type.
    pub fn is<T: 'static>(&self) -> bool {
        self.type_id == TypeId::of::<T>()
    }

    /// Hidden constructor for use by the metrique macro only.
    #[doc(hidden)]
    pub const fn __metrique_private_new(type_id: TypeId) -> Self {
        Self { type_id }
    }
}

// ─── Builders ────────────────────────────────────────────────────────────────

/// Create an [`EntryDescriptor`] with the given name and fields.
///
/// ```ignore
/// use metrique_writer_core::descriptor::*;
///
/// static FIELDS: [FieldDescriptor; 1] = [
///     field("request_count").unit(Unit::Count).build(),
/// ];
/// static DESC: EntryDescriptor = entry("MyMetrics", &FIELDS).build();
/// ```
pub const fn entry(
    name: &'static str,
    fields: &'static [FieldDescriptor],
) -> EntryDescriptorBuilder {
    EntryDescriptorBuilder {
        name,
        fields,
        timestamp: None,
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

    /// Build the [`EntryDescriptor`].
    pub const fn build(self) -> EntryDescriptor {
        EntryDescriptor {
            name: self.name,
            fields: self.fields,
            timestamp: self.timestamp,
        }
    }
}

/// Create a [`FieldDescriptor`] with the given name.
///
/// ```ignore
/// static FIELD: FieldDescriptor = field("latency")
///     .flags(&MY_FLAGS)
///     .unit(Unit::Milliseconds)
///     .build();
/// ```
pub const fn field(name: &'static str) -> FieldDescriptorBuilder {
    FieldDescriptorBuilder {
        name,
        flags: &[],
        shape: FieldShape::Opaque,
        unit: None,
    }
}

/// Builder for [`FieldDescriptor`].
pub struct FieldDescriptorBuilder {
    name: &'static str,
    flags: &'static [FieldFlag],
    shape: FieldShape<'static>,
    unit: Option<Unit>,
}

impl FieldDescriptorBuilder {
    /// Set the flags for this field.
    pub const fn flags(mut self, flags: &'static [FieldFlag]) -> Self {
        self.flags = flags;
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

    /// Build the [`FieldDescriptor`].
    pub const fn build(self) -> FieldDescriptor {
        FieldDescriptor {
            name: self.name,
            flags: self.flags,
            shape: self.shape,
            unit: self.unit,
        }
    }
}

/// Create a [`TimestampDescriptor`] with the given name.
pub const fn timestamp(name: &'static str) -> TimestampDescriptor {
    TimestampDescriptor { name }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptor_ref_stable_id() {
        static DESC: EntryDescriptor = entry("Test", &[]).build();
        let r1 = DescriptorRef::from_static(&DESC);
        let r2 = DescriptorRef::from_static(&DESC);
        assert_eq!(r1.id(), r2.id());
        assert_eq!(r1.name(), "Test");
    }

    #[test]
    fn different_descriptors_different_ids() {
        static A: EntryDescriptor = entry("A", &[]).build();
        static B: EntryDescriptor = entry("B", &[]).build();
        assert_ne!(
            DescriptorRef::from_static(&A).id(),
            DescriptorRef::from_static(&B).id()
        );
    }

    #[test]
    fn prefix_changes_id() {
        static DESC: EntryDescriptor = entry("T", &[]).build();
        let plain = DescriptorRef::from_static(&DESC);
        let prefixed = DescriptorRef::from_static(&DESC).with_prefix("Api");
        assert_ne!(plain.id(), prefixed.id());
    }

    #[test]
    fn field_name_no_prefix() {
        static FIELDS: [FieldDescriptor; 1] = [field("MyField").build()];
        static DESC: EntryDescriptor = entry("T", &FIELDS).build();

        let d = DescriptorRef::from_static(&DESC);
        assert_eq!(d.fields().next().unwrap().base_name(), "MyField");
    }

    #[test]
    fn field_name_with_prefix() {
        static FIELDS: [FieldDescriptor; 1] = [field("Latency").build()];
        static DESC: EntryDescriptor = entry("T", &FIELDS).build();

        let d = DescriptorRef::from_static(&DESC).with_prefix("Api");
        let fields: Vec<_> = d.fields().collect();
        let parts: Vec<&str> = fields[0].name_parts().collect();
        assert_eq!(parts, vec!["Api", "Latency"]);
    }

    #[test]
    fn field_name_with_nested_prefixes() {
        static FIELDS: [FieldDescriptor; 1] = [field("Latency").build()];
        static DESC: EntryDescriptor = entry("T", &FIELDS).build();

        let d = DescriptorRef::from_static(&DESC)
            .with_prefix("Http")
            .with_prefix("Api");
        let fields: Vec<_> = d.fields().collect();
        let parts: Vec<&str> = fields[0].name_parts().collect();
        assert_eq!(parts, vec!["Http", "Api", "Latency"]);
    }

    #[test]
    fn field_view_iteration() {
        static FIELDS: [FieldDescriptor; 2] = [
            field("Alpha").build(),
            field("Beta").unit(Unit::Count).build(),
        ];
        static DESC: EntryDescriptor = entry("T", &FIELDS).build();

        let d = DescriptorRef::from_static(&DESC);
        let fields: Vec<_> = d.fields().collect();
        assert_eq!(fields.len(), 2);
        assert_eq!(fields[0].base_name(), "Alpha");
        assert_eq!(fields[1].base_name(), "Beta");
        assert_eq!(fields[1].unit(), Some(Unit::Count));
    }

    #[test]
    fn timestamp() {
        static DESC: EntryDescriptor = EntryDescriptor::__metrique_private_new(
            "E",
            &[],
            Some(TimestampDescriptor::__metrique_private_new("ts")),
        );
        let d = DescriptorRef::from_static(&DESC);
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
        static DESC: EntryDescriptor = entry("X", &[]).build();
        struct WithDesc;
        impl Entry for WithDesc {
            fn write<'a>(&'a self, _w: &mut impl EntryWriter<'a>) {}
            fn descriptors(&self) -> Descriptors<'_> {
                Descriptors::available(std::iter::once(DescriptorRef::from_static(&DESC)))
            }
        }
        let boxed = BoxEntry::new(WithDesc);
        let descs = boxed.descriptors().unwrap();
        assert_eq!(descs.len(), 1);
        assert_eq!(descs[0].name(), "X");
    }
}
