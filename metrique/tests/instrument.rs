// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use metrique::{
    instrument::{self, Instrumented},
    unit_of_work::metrics,
};

#[metrics(subfield)]
#[derive(Default)]
pub struct LookupBookMetrics {
    books_considered: usize,
    success: bool,
    error: bool,
}

#[test]
fn instrument_on_error() {
    let (is_odd, metrics) = try_odd(3).into_parts();
    assert_eq!(is_odd, Ok(true));
    assert_eq!(metrics.error, false);
    assert_eq!(metrics.success, true);

    let (is_odd, metrics) = try_odd(4).into_parts();
    assert_eq!(is_odd, Err(4));
    assert_eq!(metrics.error, true);
    assert_eq!(metrics.success, false);
}

fn try_odd(n: usize) -> instrument::Result<bool, usize, LookupBookMetrics> {
    Instrumented::instrument(LookupBookMetrics::default(), |metrics| {
        metrics.books_considered = 10;
        if n % 2 == 0 { Err(n) } else { Ok(true) }
    })
    .on_error(|_res, metrics| metrics.error = true)
    .on_success(|_res, metrics| metrics.success = true)
}

#[tokio::test]
async fn instrument_async() {
    let (res, metrics): (Result<usize, usize>, _) =
        Instrumented::instrument_async(LookupBookMetrics::default(), async |metrics| {
            metrics.books_considered = 10;
            Err(10)
        })
        .await
        .on_error(|_res, metrics| metrics.error = true)
        .into_parts();
    assert_eq!(res, Err(10));
    assert_eq!(metrics.error, true);
}

#[test]
fn instrument_write_to() {
    let mut target: Option<LookupBookMetrics> = None;
    let result = try_odd(4).split_metrics_to(&mut target);
    assert_eq!(target.unwrap().error, true);
    assert_eq!(result, Err(4))
}
