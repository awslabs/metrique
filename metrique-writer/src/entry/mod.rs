// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Contains various utilities for [Entry](crate::Entry)

mod dimensions;
mod map;
mod quantized;
pub use dimensions::WithGlobalDimensions;
pub use map::EnumMapEntry;
pub use quantized::{QuantizationPolicy, QuantizedEntry};
