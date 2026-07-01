#![deny(missing_docs)]
#![doc = include_str!("../README.md")]
#![cfg_attr(docsrs, feature(doc_cfg))]

//! Histogram implementations for aggregating metrique metrics.

pub mod aggregator;
pub mod dimension_probe;
pub mod histogram;
pub mod sink;
pub mod traits;
pub mod value;

#[doc(hidden)]
pub mod __macro_plumbing {
    pub use crate::dimension_probe::extract_dimensions;
    pub use crate::traits::{AggregateStrategy, AggregateValue, Key, Merge, MergeRef};
    pub use crate::value::{CopyWrapper, NoKey};
    pub use metrique_writer_core::config::EntryDimensions;
    pub use smallvec::SmallVec;
}

pub use metrique_macro::aggregate;
