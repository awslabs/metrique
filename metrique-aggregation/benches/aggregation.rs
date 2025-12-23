use divan::{black_box, Bencher};
use metrique_aggregation::histogram::{Histogram, LinearAggregationStrategy, SortAndMerge};
use metrique_core::CloseValue;
use std::sync::{Arc, Mutex};

fn main() {
    divan::main();
}

#[divan::bench(consts = [1, 2, 4, 8])]
fn linear_aggregation_add<const THREADS: usize>(bencher: Bencher) {
    bencher
        .with_inputs(|| {
            Arc::new(Mutex::new(Histogram::<f64, _>::new(
                LinearAggregationStrategy::new(10.0, 100),
            )))
        })
        .bench_values(|histogram| {
            std::thread::scope(|s| {
                for _ in 0..THREADS {
                    let hist = histogram.clone();
                    s.spawn(move || {
                        for i in 0..1000 {
                            hist.lock().unwrap().add_value(black_box(i as f64));
                        }
                    });
                }
            });
        });
}

#[divan::bench(consts = [1, 2, 4, 8])]
fn sort_and_merge_add<const THREADS: usize>(bencher: Bencher) {
    bencher
        .with_inputs(|| {
            Arc::new(Mutex::new(Histogram::<f64, _>::new(
                SortAndMerge::<128>::new(),
            )))
        })
        .bench_values(|histogram| {
            std::thread::scope(|s| {
                for _ in 0..THREADS {
                    let hist = histogram.clone();
                    s.spawn(move || {
                        for i in 0..1000 {
                            hist.lock().unwrap().add_value(black_box(i as f64));
                        }
                    });
                }
            });
        });
}

#[divan::bench]
fn linear_aggregation_drain(bencher: Bencher) {
    bencher
        .with_inputs(|| {
            let mut hist = Histogram::<f64, _>::new(LinearAggregationStrategy::new(10.0, 100));
            for i in 0..1000 {
                hist.add_value(i as f64);
            }
            hist
        })
        .bench_values(|histogram| black_box(histogram.close()));
}

#[divan::bench]
fn sort_and_merge_drain(bencher: Bencher) {
    bencher
        .with_inputs(|| {
            let mut hist = Histogram::<f64, _>::new(SortAndMerge::<128>::new());
            for i in 0..1000 {
                hist.add_value(i as f64);
            }
            hist
        })
        .bench_values(|histogram| black_box(histogram.close()));
}
