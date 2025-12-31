#![deny(missing_docs)]
#![doc = include_str!("../README.md")]
#![cfg_attr(docsrs, feature(doc_cfg))]

//! Histogram implementations for aggregating metrique metrics.

pub mod counter;
pub mod histogram;
pub mod keyed_sink;
pub mod sink;
pub mod traits;

pub use metrique_macro::aggregate;
