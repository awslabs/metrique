// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Entry descriptors: compile-time structural metadata for macro-derived entries.
//!
//! Sinks that call [`Entry::descriptor()`](crate::Entry::descriptor) can introspect
//! the complete set of fields an entry emits, their tags, units, and (in a future
//! release) their closed shapes. Sinks that never call `descriptor()` pay nothing.

use std::any::TypeId;

use crate::Unit;

/// Describes the closed shape of a macro-derived entry: ordered fields, their tags,
/// units, and canonical entry name.
pub struct EntryDescriptor {
    name: &'static str,
    fields: &'static [FieldDescriptor],
    timestamp: Option<TimestampDescriptor>,
}

impl EntryDescriptor {
    /// Canonical name of this entry type (the Rust struct name).
    pub fn name(&self) -> &str {
        self.name
    }

    /// Ordered fields the entry emits via `Entry::write`. Order matches
    /// `Entry::write` callback order. Does not include timestamp or ignored fields.
    pub fn fields(&self) -> &[FieldDescriptor] {
        self.fields
    }

    /// The canonical timestamp field, if the entry has one.
    pub fn timestamp(&self) -> Option<&TimestampDescriptor> {
        self.timestamp.as_ref()
    }

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

/// Describes a single field within an entry's descriptor.
pub struct FieldDescriptor {
    name: &'static str,
    tags: &'static [ResolvedFieldTag],
    shape: FieldShape<'static>,
    unit: Option<Unit>,
}

impl FieldDescriptor {
    /// Field name as it appears in `Entry::write` callbacks (post rename/prefix).
    pub fn name(&self) -> &str {
        self.name
    }

    /// Resolved tags for this field.
    pub fn tags(&self) -> &[ResolvedFieldTag] {
        self.tags
    }

    /// The closed shape of this field.
    pub fn shape(&self) -> FieldShape<'_> {
        self.shape
    }

    /// The unit attached to this field, if any.
    pub fn unit(&self) -> Option<Unit> {
        self.unit
    }

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

/// The closed/emitted shape of a field.
///
/// In the initial release, all fields are emitted as [`FieldShape::Opaque`].
/// Future releases will populate known shapes based on the field's type.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldShape<'a> {
    /// A known scalar shape.
    Known(KnownShape),
    /// An optional wrapper around an inner shape.
    Optional(ShapeRef<'a>),
    /// A dynamic-key map (e.g. `Flex<(String, T)>`).
    Flex {
        /// The key shape (always string-typed currently).
        key: StringShape,
        /// The value shape.
        value: ShapeRef<'a>,
    },
    /// A list/sequence of values.
    List(ShapeRef<'a>),
    /// Shape is not statically known. The field still works through `Entry::write`.
    Opaque,
}

/// Known scalar shapes that a field can emit.
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

/// Opaque handle to a nested [`FieldShape`]. Lifetime-tied to its parent descriptor.
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

/// A resolved field tag: records whether a specific tag type is present or absent
/// for a given field.
pub struct ResolvedFieldTag {
    tag_id: TypeId,
    state: FieldTagState,
}

impl ResolvedFieldTag {
    /// The [`TypeId`] of the tag marker type.
    pub fn tag_id(&self) -> TypeId {
        self.tag_id
    }

    /// Whether this tag is present or explicitly absent for the field.
    pub fn state(&self) -> FieldTagState {
        self.state
    }

    /// Hidden constructor for use by the metrique macro only.
    #[doc(hidden)]
    pub fn __metrique_private_new(tag_id: TypeId, state: FieldTagState) -> Self {
        Self { tag_id, state }
    }
}

/// Whether a field tag is present or explicitly absent.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FieldTagState {
    /// The tag is present on this field.
    Present,
    /// The tag is explicitly absent on this field (via `skip(T)`).
    Absent,
}

/// Opaque handle returned by [`Entry::descriptor()`](crate::Entry::descriptor).
///
/// Carries a stable [`DescriptorId`] for cache keying and a borrow of the
/// underlying [`EntryDescriptor`].
pub struct DescriptorRef<'a> {
    descriptor: &'a EntryDescriptor,
    id: DescriptorId,
}

impl<'a> DescriptorRef<'a> {
    /// Borrow the underlying descriptor.
    pub fn get(&self) -> &EntryDescriptor {
        self.descriptor
    }

    /// A stable identity for this descriptor, suitable for use as a cache key.
    pub fn id(&self) -> DescriptorId {
        self.id
    }

    /// Create a `DescriptorRef` from a `&'static EntryDescriptor`.
    ///
    /// The `DescriptorId` is derived from the pointer address of the static.
    pub fn from_static(descriptor: &'static EntryDescriptor) -> DescriptorRef<'static> {
        let id = DescriptorId(descriptor as *const EntryDescriptor as usize);
        DescriptorRef { descriptor, id }
    }
}

/// Opaque identifier for a descriptor, stable within a single process lifetime.
///
/// Two calls to `descriptor()` on the same entry type return equal ids.
/// Cross-process stability is not guaranteed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DescriptorId(usize);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptor_ref_from_static_has_stable_id() {
        static DESC: EntryDescriptor = EntryDescriptor::__metrique_private_new("Test", &[], None);
        let ref1 = DescriptorRef::from_static(&DESC);
        let ref2 = DescriptorRef::from_static(&DESC);
        assert_eq!(ref1.id(), ref2.id());
        assert_eq!(ref1.get().name(), "Test");
    }

    #[test]
    fn different_descriptors_have_different_ids() {
        static DESC_A: EntryDescriptor = EntryDescriptor::__metrique_private_new("A", &[], None);
        static DESC_B: EntryDescriptor = EntryDescriptor::__metrique_private_new("B", &[], None);
        let ref_a = DescriptorRef::from_static(&DESC_A);
        let ref_b = DescriptorRef::from_static(&DESC_B);
        assert_ne!(ref_a.id(), ref_b.id());
    }

    #[test]
    fn field_descriptor_accessors() {
        static TAGS: [ResolvedFieldTag; 0] = [];
        static FIELD: FieldDescriptor =
            FieldDescriptor::__metrique_private_new("my_field", &TAGS, FieldShape::Opaque, None);
        assert_eq!(FIELD.name(), "my_field");
        assert!(FIELD.tags().is_empty());
        assert_eq!(FIELD.shape(), FieldShape::Opaque);
        assert_eq!(FIELD.unit(), None);
    }

    #[test]
    fn timestamp_descriptor_accessors() {
        let ts = TimestampDescriptor::__metrique_private_new("request_start");
        assert_eq!(ts.name(), "request_start");
    }

    #[test]
    fn field_shape_variants() {
        assert_eq!(FieldShape::Opaque, FieldShape::Opaque);
        assert_eq!(
            FieldShape::Known(KnownShape::U64),
            FieldShape::Known(KnownShape::U64)
        );
        assert_ne!(
            FieldShape::Known(KnownShape::U64),
            FieldShape::Known(KnownShape::String)
        );
    }
}
