//! Example demonstrating manual implementation of the new AggregateStrategy traits.
//! 
//! NOTE: This test is currently disabled because manual implementation of AggregateStrategy
//! for types with #[metrics] requires implementing traits for types from other crates
//! (specifically RootEntry<T>), which violates Rust's orphan rules.
//! 
//! The #[aggregate] macro handles this correctly by generating the necessary impls
//! in the same module as the type definition.
//!
//! For examples of aggregation usage, see aggregation.rs and keyed_sink.rs tests.
