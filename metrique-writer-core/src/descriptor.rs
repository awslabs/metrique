// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Entry descriptors: compile-time structural metadata for macro-derived entries.
//!
//! Sinks interact with [`DescriptorRef`], which provides resolved field names,
//! tags, shapes, and units. The underlying storage types ([`EntryDescriptor`],
//! [`FieldDescriptor`]) are public for macro construction but sinks should use
//! [`DescriptorRef`] and [`FieldView`] accessors.

use std::any::TypeId;
use std::borrow::Cow;
use std::hash::{Hash, Hasher};

use crate::Unit;

// ─── Internal storage types (pub for macro, sinks use DescriptorRef) ────────

/// Static descriptor storage for a macro-derived entry.
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
pub struct FieldDescriptor {
    name: &'static str,
    tags: &'static [ResolvedFieldTag],
    shape: FieldShape<'static>,
    unit: Option<Unit>,
}

impl FieldDescriptor {
    /// Hidden constructor for use by the metrique macro only.
    #[doc(hidden)]
    pub const fn __metrique_private_new(
        name: &'static str,
        tags: &'static [ResolvedFieldTag],
        shape: FieldShape<'static>,
        unit: Option<Unit>,
    ) -> Self {
        Self {
            name,
            tags,
            shape,
            unit,
        }
    }
}

/// Describes the timestamp field of an entry.
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

// ─── Public API ─────────────────────────────────────────────────────────────

/// The primary interface for sinks to read descriptor metadata.
///
/// Carries a reference to the underlying static descriptor plus optional
/// modifiers (prefix, default tags) applied transparently when reading.
#[derive(Clone)]
pub struct DescriptorRef<'a> {
    descriptor: &'a EntryDescriptor,
    id: DescriptorId,
    prefix: Option<&'static str>,
    default_tags: &'static [ResolvedFieldTag],
}

impl<'a> DescriptorRef<'a> {
    /// Create a `DescriptorRef` from a `&'static EntryDescriptor`.
    pub fn from_static(descriptor: &'static EntryDescriptor) -> DescriptorRef<'static> {
        static EMPTY_TAGS: [ResolvedFieldTag; 0] = [];
        let id = DescriptorId::compute(descriptor, None, &EMPTY_TAGS);
        DescriptorRef {
            descriptor,
            id,
            prefix: None,
            default_tags: &EMPTY_TAGS,
        }
    }

    /// Add a prefix prepended to all field names (already inflected).
    pub fn with_prefix(mut self, prefix: &'static str) -> Self {
        self.prefix = Some(prefix);
        self.id = DescriptorId::compute(self.descriptor, self.prefix, self.default_tags);
        self
    }

    /// Add default tags applied to fields that don't already have them.
    pub fn with_default_tags(mut self, tags: &'static [ResolvedFieldTag]) -> Self {
        self.default_tags = tags;
        self.id = DescriptorId::compute(self.descriptor, self.prefix, self.default_tags);
        self
    }

    /// Stable identity for caching (incorporates base descriptor + modifiers).
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

    /// Resolved field name at the given index (with prefix applied).
    pub fn field_name(&self, idx: usize) -> Cow<'static, str> {
        let base = self.descriptor.fields[idx].name;
        match self.prefix {
            None => Cow::Borrowed(base),
            Some(prefix) => Cow::Owned(format!("{}{}", prefix, base)),
        }
    }

    /// Resolved tags for the field at the given index.
    /// Field-level tags win; default tags fill in for tag ids not already present.
    pub fn field_tags(&self, idx: usize) -> impl Iterator<Item = &ResolvedFieldTag> {
        let field_tags = self.descriptor.fields[idx].tags;
        let defaults = self.default_tags;
        field_tags.iter().chain(
            defaults
                .iter()
                .filter(move |dt| !field_tags.iter().any(|ft| ft.tag_id == dt.tag_id)),
        )
    }

    /// Shape of the field at the given index.
    pub fn field_shape(&self, idx: usize) -> FieldShape<'_> {
        self.descriptor.fields[idx].shape
    }

    /// Unit of the field at the given index.
    pub fn field_unit(&self, idx: usize) -> Option<Unit> {
        self.descriptor.fields[idx].unit
    }

    /// The canonical timestamp field, if the entry has one.
    pub fn timestamp(&self) -> Option<&TimestampDescriptor> {
        self.descriptor.timestamp.as_ref()
    }

    /// Iterate over fields as [`FieldView`]s.
    pub fn fields(&self) -> impl Iterator<Item = FieldView<'_>> {
        (0..self.descriptor.fields.len()).map(move |i| FieldView { desc: self, idx: i })
    }
}

/// A view of a single field with modifiers applied.
#[derive(Clone)]
pub struct FieldView<'a> {
    desc: &'a DescriptorRef<'a>,
    idx: usize,
}

impl<'a> FieldView<'a> {
    /// Resolved field name (with prefix applied).
    pub fn name(&self) -> Cow<'static, str> {
        self.desc.field_name(self.idx)
    }

    /// Resolved tags (with defaults applied).
    pub fn tags(&self) -> impl Iterator<Item = &'a ResolvedFieldTag> {
        self.desc.field_tags(self.idx)
    }

    /// Shape of this field.
    pub fn shape(&self) -> FieldShape<'a> {
        self.desc.field_shape(self.idx)
    }

    /// Unit of this field.
    pub fn unit(&self) -> Option<Unit> {
        self.desc.field_unit(self.idx)
    }
}

/// Opaque identifier for a descriptor (including modifiers).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DescriptorId(u64);

impl DescriptorId {
    fn compute(
        descriptor: &EntryDescriptor,
        prefix: Option<&'static str>,
        default_tags: &'static [ResolvedFieldTag],
    ) -> Self {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        (descriptor as *const EntryDescriptor as usize).hash(&mut hasher);
        prefix
            .map_or(0usize, |p| p.as_ptr() as usize)
            .hash(&mut hasher);
        (default_tags.as_ptr() as usize).hash(&mut hasher);
        DescriptorId(hasher.finish())
    }
}

// ─── Shape types ────────────────────────────────────────────────────────────

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

// ─── Tag types ──────────────────────────────────────────────────────────────

/// A resolved field tag.
pub struct ResolvedFieldTag {
    tag_id: TypeId,
    state: FieldTagState,
}

impl ResolvedFieldTag {
    /// The [`TypeId`] of the tag marker type.
    pub fn tag_id(&self) -> TypeId {
        self.tag_id
    }

    /// Whether this tag is present or explicitly absent.
    pub fn state(&self) -> FieldTagState {
        self.state
    }

    /// Hidden constructor for use by the metrique macro only.
    #[doc(hidden)]
    pub const fn __metrique_private_new(tag_id: TypeId, state: FieldTagState) -> Self {
        Self { tag_id, state }
    }
}

/// Whether a field tag is present or explicitly absent.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FieldTagState {
    /// The tag is present on this field.
    Present,
    /// The tag is explicitly absent (via `skip(T)`).
    Absent,
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptor_ref_stable_id() {
        static DESC: EntryDescriptor = EntryDescriptor::__metrique_private_new("Test", &[], None);
        let r1 = DescriptorRef::from_static(&DESC);
        let r2 = DescriptorRef::from_static(&DESC);
        assert_eq!(r1.id(), r2.id());
        assert_eq!(r1.name(), "Test");
    }

    #[test]
    fn different_descriptors_different_ids() {
        static A: EntryDescriptor = EntryDescriptor::__metrique_private_new("A", &[], None);
        static B: EntryDescriptor = EntryDescriptor::__metrique_private_new("B", &[], None);
        assert_ne!(
            DescriptorRef::from_static(&A).id(),
            DescriptorRef::from_static(&B).id()
        );
    }

    #[test]
    fn prefix_changes_id() {
        static DESC: EntryDescriptor = EntryDescriptor::__metrique_private_new("T", &[], None);
        let plain = DescriptorRef::from_static(&DESC);
        let prefixed = DescriptorRef::from_static(&DESC).with_prefix("Api");
        assert_ne!(plain.id(), prefixed.id());
    }

    #[test]
    fn field_name_no_prefix() {
        static TAGS: [ResolvedFieldTag; 0] = [];
        static FIELDS: [FieldDescriptor; 1] = [FieldDescriptor::__metrique_private_new(
            "MyField",
            &TAGS,
            FieldShape::Opaque,
            None,
        )];
        static DESC: EntryDescriptor = EntryDescriptor::__metrique_private_new("T", &FIELDS, None);

        let d = DescriptorRef::from_static(&DESC);
        assert_eq!(d.field_name(0), "MyField");
    }

    #[test]
    fn field_name_with_prefix() {
        static TAGS: [ResolvedFieldTag; 0] = [];
        static FIELDS: [FieldDescriptor; 1] = [FieldDescriptor::__metrique_private_new(
            "Latency",
            &TAGS,
            FieldShape::Opaque,
            None,
        )];
        static DESC: EntryDescriptor = EntryDescriptor::__metrique_private_new("T", &FIELDS, None);

        let d = DescriptorRef::from_static(&DESC).with_prefix("Api");
        assert_eq!(d.field_name(0), "ApiLatency");
    }

    #[test]
    fn field_tags_with_defaults() {
        static FIELD_TAGS: [ResolvedFieldTag; 0] = [];
        static DEFAULT_TAGS: [ResolvedFieldTag; 1] = [ResolvedFieldTag::__metrique_private_new(
            TypeId::of::<u8>(),
            FieldTagState::Present,
        )];
        static FIELDS: [FieldDescriptor; 1] = [FieldDescriptor::__metrique_private_new(
            "f",
            &FIELD_TAGS,
            FieldShape::Opaque,
            None,
        )];
        static DESC: EntryDescriptor = EntryDescriptor::__metrique_private_new("T", &FIELDS, None);

        // Without defaults: no tags
        let d = DescriptorRef::from_static(&DESC);
        assert_eq!(d.field_tags(0).count(), 0);

        // With defaults: one tag
        let d = DescriptorRef::from_static(&DESC).with_default_tags(&DEFAULT_TAGS);
        assert_eq!(d.field_tags(0).count(), 1);
    }

    #[test]
    fn field_tags_field_level_wins() {
        static FIELD_TAGS: [ResolvedFieldTag; 1] = [ResolvedFieldTag::__metrique_private_new(
            TypeId::of::<u8>(),
            FieldTagState::Absent,
        )];
        static DEFAULT_TAGS: [ResolvedFieldTag; 1] = [ResolvedFieldTag::__metrique_private_new(
            TypeId::of::<u8>(),
            FieldTagState::Present,
        )];
        static FIELDS: [FieldDescriptor; 1] = [FieldDescriptor::__metrique_private_new(
            "f",
            &FIELD_TAGS,
            FieldShape::Opaque,
            None,
        )];
        static DESC: EntryDescriptor = EntryDescriptor::__metrique_private_new("T", &FIELDS, None);

        let d = DescriptorRef::from_static(&DESC).with_default_tags(&DEFAULT_TAGS);
        let tags: Vec<_> = d.field_tags(0).collect();
        // Field-level Absent wins, default Present is not added
        assert_eq!(tags.len(), 1);
        assert_eq!(tags[0].state(), FieldTagState::Absent);
    }

    #[test]
    fn field_view_iteration() {
        static TAGS: [ResolvedFieldTag; 0] = [];
        static FIELDS: [FieldDescriptor; 2] = [
            FieldDescriptor::__metrique_private_new("Alpha", &TAGS, FieldShape::Opaque, None),
            FieldDescriptor::__metrique_private_new(
                "Beta",
                &TAGS,
                FieldShape::Opaque,
                Some(Unit::Count),
            ),
        ];
        static DESC: EntryDescriptor = EntryDescriptor::__metrique_private_new("T", &FIELDS, None);

        let d = DescriptorRef::from_static(&DESC);
        let fields: Vec<_> = d.fields().collect();
        assert_eq!(fields.len(), 2);
        assert_eq!(fields[0].name(), "Alpha");
        assert_eq!(fields[1].name(), "Beta");
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
        assert_eq!(HandWritten.descriptors().count(), 0);
    }

    #[test]
    fn boxentry_forwards() {
        use crate::{BoxEntry, Entry, EntryWriter};
        static DESC: EntryDescriptor = EntryDescriptor::__metrique_private_new("X", &[], None);
        struct WithDesc;
        impl Entry for WithDesc {
            fn write<'a>(&'a self, _w: &mut impl EntryWriter<'a>) {}
            fn descriptors(&self) -> impl Iterator<Item = DescriptorRef<'_>> {
                std::iter::once(DescriptorRef::from_static(&DESC))
            }
        }
        let boxed = BoxEntry::new(WithDesc);
        let descs: Vec<_> = boxed.descriptors().collect();
        assert_eq!(descs.len(), 1);
        assert_eq!(descs[0].name(), "X");
    }
}
