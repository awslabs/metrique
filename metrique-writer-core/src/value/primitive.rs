// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::time::Duration;

use super::{MetricValue, Observation, Value, ValueWriter};
use crate::{
    Unit,
    unit::{self, NegativeScale::Milli},
};

use crate::value::MetricFlags;

fn duration_as_millis_with_nano_precision(duration: Duration) -> f64 {
    //         milli
    // unit  * ----- = milli
    //         unit
    duration.as_secs_f64() * (Milli.reduction_factor() as f64)
}

impl Value for str {
    #[inline]
    fn write(&self, writer: impl ValueWriter) {
        writer.string(self)
    }
}

impl Value for String {
    #[inline]
    fn write(&self, writer: impl ValueWriter) {
        writer.string(self)
    }
}

macro_rules! counter {
    ($t:ty) => {
        impl Value for $t {
            #[inline]
            fn write(&self, writer: impl ValueWriter) {
                writer.metric(
                    [Observation::Unsigned((*self).into())],
                    Unit::None,
                    [],
                    MetricFlags::empty(),
                )
            }
        }

        impl MetricValue for $t {
            type Unit = unit::None;
        }
    };
}

counter!(u64);
counter!(u32);
counter!(u16);
counter!(u8);
counter!(bool);

impl Value for usize {
    #[inline]
    fn write(&self, writer: impl ValueWriter) {
        writer.metric(
            [Observation::Unsigned(*self as u64)],
            Unit::None,
            [],
            MetricFlags::empty(),
        )
    }
}

impl MetricValue for usize {
    type Unit = unit::None;
}

macro_rules! float {
    ($t:ty) => {
        impl Value for $t {
            #[inline]
            fn write(&self, writer: impl ValueWriter) {
                writer.metric(
                    [Observation::Floating((*self).into())],
                    Unit::None,
                    [],
                    MetricFlags::empty(),
                )
            }
        }

        impl MetricValue for $t {
            type Unit = unit::None;
        }
    };
}

float!(f32);
float!(f64);

impl Value for Duration {
    #[inline]
    fn write(&self, writer: impl ValueWriter) {
        writer.metric(
            [Observation::Floating(
                duration_as_millis_with_nano_precision(*self),
            )],
            Unit::Second(Milli),
            [],
            MetricFlags::empty(),
        )
    }
}

/// Time based metrics must be of type `Duration` to have units, or else they will be unitless.
///
/// #[derive(Entry)]
/// #[entry(rename_all = "PascalCase")]
/// pub struct Metrics {
///     latency: Duration, // Time based metric with units
///     gpa: f64, // Unitless metric
/// }
impl MetricValue for Duration {
    type Unit = unit::Millisecond;
}
