use divan::{black_box, Bencher};
use metrique_aggregation::histogram::{
    AggregationStrategy, Histogram, LinearAggregationStrategy, SortAndMerge,
};
use metrique_core::CloseValue;
use std::sync::{Arc, Mutex};

fn main() {
    divan::main();
}

const THREADS: &[usize] = &[1, 2, 4, 8];
const ITEMS: &[usize] = &[100, 1000, 10000];

#[divan::bench(
    types = [LinearAggregationStrategy, SortAndMerge<128>],
    consts = THREADS,
    args = ITEMS,
)]
fn add<S: AggregationStrategy + Default + Send, const T: usize>(
    bencher: Bencher,
    items: usize,
) {
    bencher
        .with_inputs(|| Arc::new(Mutex::new(Histogram::<f64, S>::new(S::default()))))
        .bench_values(|histogram| {
            std::thread::scope(|s| {
                for _ in 0..T {
                    let hist = histogram.clone();
                    s.spawn(move || {
                        for i in 0..items {
                            hist.lock().unwrap().add_value(black_box(i as f64));
                        }
                    });
                }
            });
        });
}

#[divan::bench(
    types = [LinearAggregationStrategy, SortAndMerge<128>],
    args = ITEMS,
)]
fn drain<S: AggregationStrategy + Default>(bencher: Bencher, items: usize) {
    bencher
        .with_inputs(|| {
            let mut hist = Histogram::<f64, S>::new(S::default());
            for i in 0..items {
                hist.add_value(i as f64);
            }
            hist
        })
        .bench_values(|histogram| black_box(histogram.close()));
}
