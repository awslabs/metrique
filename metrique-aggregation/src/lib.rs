#![deny(missing_docs)]
#![doc = include_str!("../README.md")]
#![cfg_attr(docsrs, feature(doc_cfg))]

//! Histogram implementations for aggregating metrique metrics.

pub mod aggregate;
pub mod counter;
pub mod histogram;
pub mod sink;
