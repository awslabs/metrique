#![deny(missing_docs)]
#![doc = include_str!("../README.md")]
#![cfg_attr(docsrs, feature(doc_cfg))]

//! Histogram implementations for aggregating metrique metrics.

pub mod counter;
pub mod histogram;
pub mod keyed_sink;
pub mod sink;
pub mod traits;

#[doc(hidden)]
pub mod __macro_plumbing {
    pub use crate::sink::MergeOnDropExt;
    pub use crate::traits::{AggregateEntry, AggregateEntryRef, AggregateValue};
}

pub use metrique_macro::aggregate;
