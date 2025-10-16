// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! This example demonstrates using `flatten` and `prefix` together to create
//! prefixed metric fields from nested structs.

use metrique::unit_of_work::metrics;
use std::time::{Duration, SystemTime};

#[metrics(subfield)]
struct GenericOperationMetric {
    serialization_time: Duration,
    plugin_time: Duration,
}

#[metrics]
struct CountDucksOperation {
    #[metrics(timestamp)]
    timestamp: SystemTime,

    #[metrics(flatten, prefix = "count_ducks_")]
    core: GenericOperationMetric,

    number_of_ducks: usize,
}

#[metrics]
struct JugglePineapplesOperation {
    #[metrics(timestamp)]
    timestamp: SystemTime,

    #[metrics(flatten, prefix = "juggle_pineapples_")]
    core: GenericOperationMetric,

    pineapples_dropped: usize,
    juggling_difficulty: String,
}

fn main() {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_count_ducks_with_prefix() {
        let test_util::TestEntrySink { inspector, sink } = test_util::test_entry_sink();

        let duck_metrics = CountDucksOperation {
            timestamp: SystemTime::now(),
            core: GenericOperationMetric {
                serialization_time: Duration::from_millis(50),
                plugin_time: Duration::from_millis(120),
            },
            number_of_ducks: 42,
        }
        .append_on_drop(sink);

        drop(duck_metrics);

        let entries = inspector.entries();
        assert_eq!(entries.len(), 1);

        let entry = &entries[0];
        assert_eq!(entry.metrics["count_ducks_serialization_time"].as_u64(), 50);
        assert_eq!(entry.metrics["count_ducks_plugin_time"].as_u64(), 120);
        assert_eq!(entry.metrics["number_of_ducks"].as_u64(), 42);
    }

    #[test]
    fn test_juggle_pineapples_with_prefix() {
        let test_util::TestEntrySink { inspector, sink } = test_util::test_entry_sink();

        let pineapple_metrics = JugglePineapplesOperation {
            timestamp: SystemTime::now(),
            core: GenericOperationMetric {
                serialization_time: Duration::from_millis(75),
                plugin_time: Duration::from_millis(200),
            },
            pineapples_dropped: 3,
            juggling_difficulty: "Expert".to_string(),
        }
        .append_on_drop(sink);

        drop(pineapple_metrics);

        let entries = inspector.entries();
        assert_eq!(entries.len(), 1);

        let entry = &entries[0];
        assert_eq!(
            entry.metrics["juggle_pineapples_serialization_time"].as_u64(),
            75
        );
        assert_eq!(entry.metrics["juggle_pineapples_plugin_time"].as_u64(), 200);
        assert_eq!(entry.metrics["pineapples_dropped"].as_u64(), 3);
        assert_eq!(entry.values["juggling_difficulty"], "Expert");
    }
}
