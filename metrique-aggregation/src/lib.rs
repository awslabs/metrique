#![deny(missing_docs)]
#![doc = include_str!("../README.md")]
#![cfg_attr(docsrs, feature(doc_cfg))]

//! Aggregation support for metrique metrics.

pub mod aggregate;
pub mod counter;
pub mod histogram;

pub use counter::Counter;
