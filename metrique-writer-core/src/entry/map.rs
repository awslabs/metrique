// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::collections::{BTreeMap, HashMap};

use crate::{Entry, EntryWriter, Value};

// Map Entry impls intentionally do not provide descriptors (Unavailable).
// They emit a dynamic number of fields with runtime-determined keys,
// which cannot be described statically.

impl<K: AsRef<str>, V: Value, S> Entry for HashMap<K, V, S> {
    fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
        for (k, v) in self {
            writer.value(k.as_ref(), v);
        }
    }
}

impl<K: AsRef<str>, V: Value> Entry for BTreeMap<K, V> {
    fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
        for (k, v) in self {
            writer.value(k.as_ref(), v);
        }
    }
}

// impl for slices of (key, value) pairs (e.g. an associative array)

impl<K: AsRef<str>, V: Value> Entry for [(K, V)] {
    fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
        for (k, v) in self {
            writer.value(k.as_ref(), v);
        }
    }
}
