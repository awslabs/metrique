use divan::{black_box, Bencher};
use metrique_aggregation::histogram::{Histogram, LinearAggregationStrategy, SortAndMerge};
use std::sync::{Arc, Mutex};

fn main() {
    divan::main();
}

#[divan::bench(consts = [1, 2, 4, 8])]
fn linear_aggregation<const THREADS: usize>(bencher: Bencher) {
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
fn sort_and_merge<const THREADS: usize>(bencher: Bencher) {
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
