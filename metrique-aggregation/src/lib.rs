#![deny(missing_docs)]
#![doc = include_str!("../README.md")]
#![cfg_attr(docsrs, feature(doc_cfg))]

//! Histogram implementations for aggregating metrique metrics.

pub mod histogram;
pub mod keyed_sink;
pub mod sink;
pub mod split_sink;
pub mod traits;
pub mod value;

#[doc(hidden)]
pub mod __macro_plumbing {
    pub use crate::traits::{AggregateStrategy, AggregateValue, Key, Merge, MergeRef};
    pub use crate::value::NoKey;
}

pub use metrique_macro::aggregate;
