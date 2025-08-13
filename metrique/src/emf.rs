// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

#![doc = include_str!("../docs/emf.md")]

use metrique_writer_core::Entry;

pub use metrique_writer_core::config::EntryDimensions;

#[cfg(feature = "emf")]
pub use metrique_writer_format_emf::{
    AllowSplitEntries, Emf, EmfBuilder, HighStorageResolution, HighStorageResolutionCtor,
    MetricDefinition, MetricDirective, NoMetric, NoMetricCtor, SampledEmf, StorageResolution,
};

/// Add EMF Entry-specific dimensions
///
/// Generally, you will not use this directly. Instead, use the `#[metrics(emf::dimension_sets)]` attribute. See the
/// [module documentation](crate::emf) for more information.
pub struct SetEntryDimensions {
    /// The dimensions to add to the EMF entry.
    pub dimensions: EntryDimensions,
}

impl Entry for SetEntryDimensions {
    fn write<'a>(&'a self, writer: &mut impl metrique_writer_core::EntryWriter<'a>) {
        writer.config(&self.dimensions);
    }
}

pub use metrique_writer_core::config::EntryDimensions as __EntryDimensions;

/// Internal macro used by the `#[metrics]` macro to construct the `SetEntryDimsions`
#[macro_export]
#[doc(hidden)]
macro_rules! __plumbing_entry_dimensions {
     (dims: [$([$($inner_dims:expr),*]),*]) => {
         $crate::emf::SetEntryDimensions {
             dimensions: $crate::emf::__EntryDimensions::new(::std::borrow::Cow::Borrowed(&[
                 $(
                     ::std::borrow::Cow::Borrowed(&[
                         $(::std::borrow::Cow::Borrowed($inner_dims)),*
                     ])
                 ),*
             ]))
         }
     };
 }

#[cfg(test)]
mod test {
    use std::time::{SystemTime, UNIX_EPOCH};

    use metrique::writer::{Entry, format::Format};

    use crate::emf::SetEntryDimensions;

    #[test]
    fn add_entry_level_nested_dimensions() {
        #[derive(Entry)]
        struct MetricEntry {
            #[entry(flatten)]
            conf: SetEntryDimensions,
            field1: &'static str,
            field2: &'static str,
            field3: &'static str,
            field4: &'static str,
            #[entry(timestamp)]
            ts: SystemTime,
        }
        let dims = __plumbing_entry_dimensions!(
            dims: [["field1", "field2"], ["field3"]]
        );
        let entry = MetricEntry {
            conf: dims,
            field1: "a",
            field2: "2",
            field3: "3",
            field4: "4",
            ts: UNIX_EPOCH,
        };

        let mut emf_writer = metrique::emf::Emf::all_validations(
            "test".to_string(),
            vec![vec!["field4".to_string()]],
        );
        let mut output: Vec<u8> = vec![];
        emf_writer.format(&entry, &mut output).unwrap();
        let output = String::from_utf8(output).expect("invalid UTF-8");
        let expected = "{\"_aws\":{\"CloudWatchMetrics\":[{\"Namespace\":\"test\",\"Dimensions\":[[\"field4\",\"field1\",\"field2\"],[\"field4\",\"field3\"]],\"Metrics\":[]}],\"Timestamp\":0},\"field1\":\"a\",\"field2\":\"2\",\"field3\":\"3\",\"field4\":\"4\"}\n";
        assert_eq!(output, expected);
    }
}
