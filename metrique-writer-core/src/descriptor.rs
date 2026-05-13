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

// ─── Internal storage types (pub for macro, sinks use DescriptorRef) ────────

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

// ─── Public API ─────────────────────────────────────────────────────────────

/// A descriptor segment describing a contiguous group of fields in an entry's
/// write output. Provides resolved field names, tags, shapes, and units.
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
///         let tags = field.tags();             // resolved tags
///         let shape = field.shape();
///     }
/// }
/// ```
#[derive(Clone, Debug)]
pub struct DescriptorRef<'a> {
    descriptor: &'a EntryDescriptor,
    id: DescriptorId,
    prefixes: SmallVec<[&'static str; 1]>,
    default_tag_layers: SmallVec<[&'static [ResolvedFieldTag]; 1]>,
}

impl<'a> DescriptorRef<'a> {
    /// Create a `DescriptorRef` from a `&'static EntryDescriptor`.
    #[doc(hidden)]
    pub fn from_static(descriptor: &'static EntryDescriptor) -> DescriptorRef<'static> {
        let id = DescriptorId::compute(descriptor, &[], &[]);
        DescriptorRef {
            descriptor,
            id,
            prefixes: SmallVec::new(),
            default_tag_layers: SmallVec::new(),
        }
    }

    /// Add a prefix to be prepended to all field names in this segment.
    /// Multiple calls stack (outermost prefix first).
    #[doc(hidden)]
    pub fn with_prefix(mut self, prefix: &'static str) -> Self {
        self.prefixes.push(prefix);
        self.id = DescriptorId::compute(self.descriptor, &self.prefixes, &self.default_tag_layers);
        self
    }

    /// Add a layer of default tags. Field-level tags win; earlier layers win over later ones.
    /// Multiple calls stack (innermost layer first).
    #[doc(hidden)]
    pub fn with_default_tags(mut self, tags: &'static [ResolvedFieldTag]) -> Self {
        self.default_tag_layers.push(tags);
        self.id = DescriptorId::compute(self.descriptor, &self.prefixes, &self.default_tag_layers);
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
    /// Resolved tags for this field.
    pub fn tags(&self) -> impl Iterator<Item = &'a ResolvedFieldTag> {
        // Field-level tags (baked in static) win. Default tag layers fill in for
        // tag ids not already present, with earlier layers taking priority.
        let field_tags = self.desc.descriptor.fields[self.idx].tags;
        let layers = &self.desc.default_tag_layers;
        let mut seen_ids: SmallVec<[TypeId; 4]> = field_tags.iter().map(|t| t.tag_id).collect();
        let mut all_tags: SmallVec<[&'a ResolvedFieldTag; 4]> = field_tags.iter().collect();
        for layer in layers.iter() {
            for tag in layer.iter() {
                if !seen_ids.contains(&tag.tag_id) {
                    seen_ids.push(tag.tag_id);
                    all_tags.push(tag);
                }
            }
        }
        all_tags.into_iter()
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

/// Opaque identifier for a descriptor (including modifiers).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DescriptorId(u64);

impl DescriptorId {
    // TODO: consider using fxhash instead to be a bit more collision resistant
    fn compute(
        descriptor: &EntryDescriptor,
        prefixes: &[&'static str],
        tag_layers: &[&'static [ResolvedFieldTag]],
    ) -> Self {
        let mut id = descriptor as *const EntryDescriptor as u64;
        for p in prefixes {
            id = id.wrapping_mul(31).wrapping_add(p.as_ptr() as u64);
        }
        for layer in tag_layers {
            id = id.wrapping_mul(31).wrapping_add(layer.as_ptr() as u64);
        }
        DescriptorId(id)
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
#[derive(Debug)]
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
        assert_eq!(d.fields().next().unwrap().base_name(), "MyField");
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
        let fields: Vec<_> = d.fields().collect();
        let parts: Vec<&str> = fields[0].name_parts().collect();
        assert_eq!(parts, vec!["Api", "Latency"]);
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
        assert_eq!(d.fields().next().unwrap().tags().count(), 0);

        // With defaults: one tag
        let d = DescriptorRef::from_static(&DESC).with_default_tags(&DEFAULT_TAGS);
        assert_eq!(d.fields().next().unwrap().tags().count(), 1);
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
        let tags: Vec<_> = d.fields().next().unwrap().tags().collect();
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
