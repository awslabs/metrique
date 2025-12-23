#![deny(missing_docs)]
#![doc = include_str!("../README.md")]
#![cfg_attr(docsrs, feature(doc_cfg))]

//! Histogram implementations for aggregating metrique metrics.

/// Histogram types and aggregation strategies.
pub mod histogram;
