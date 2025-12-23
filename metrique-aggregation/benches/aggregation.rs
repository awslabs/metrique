use divan::{AllocProfiler, Bencher, black_box};
use metrique_aggregation::histogram::{
    AggregationStrategy, AtomicAggregationStrategy, AtomicHistogram,
    AtomicExponentialAggregationStrategy, Histogram, ExponentialAggregationStrategy, SortAndMerge,
};
use metrique_core::CloseValue;
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use std::sync::{Arc, Mutex};

//#[global_allocator]
//static ALLOC: AllocProfiler = AllocProfiler::system();

fn main() {
    divan::main();
}

const THREADS: &[usize] = &[1, 2, 4, 8];
const ITEMS: &[usize] = &[100, 1000, 10000];

#[divan::bench(
    types = [ExponentialAggregationStrategy, SortAndMerge<128>],
    threads = THREADS,
    args = ITEMS,
)]
fn add<S: AggregationStrategy + Default + Send>(bencher: Bencher, items: usize) {
    bencher
        .counter(items)
        .with_inputs(|| {
            let values: Vec<f64> = {
                let mut rng = ChaCha8Rng::seed_from_u64(0 as u64);
                (0..items).map(|_| rng.random_range(0.0..1000.0)).collect()
            };
            let histogram = Arc::new(Mutex::new(Histogram::<f64, S>::new(S::default())));
            (histogram, values)
        })
        .bench_values(|(histogram, values)| {
            let hist = histogram.clone();
            for &val in &values {
                hist.lock().unwrap().add_value(black_box(val));
            }
        });
}

#[divan::bench(
    types = [ExponentialAggregationStrategy, SortAndMerge<128>],
    args = ITEMS,
)]
fn drain<S: AggregationStrategy + Default>(bencher: Bencher, items: usize) {
    bencher
        .counter(items)
        .with_inputs(|| {
            let mut hist = Histogram::<f64, S>::new(S::default());
            let mut rng = ChaCha8Rng::seed_from_u64(0);
            for _ in 0..items {
                hist.add_value(rng.random_range(0.0..1000.0));
            }
            hist
        })
        .bench_values(|histogram| black_box(histogram.close()));
}

#[divan::bench(
    types = [AtomicExponentialAggregationStrategy],
    threads = THREADS,
    args = ITEMS,
)]
fn add_atomic<S: AtomicAggregationStrategy + Default + Send + Sync>(
    bencher: Bencher,
    items: usize,
) {
    bencher
        .counter(items)
        .with_inputs(|| {
            let values: Vec<f64> = {
                let mut rng = ChaCha8Rng::seed_from_u64(0 as u64);
                (0..items).map(|_| rng.random_range(0.0..1000.0)).collect()
            };
            let histogram = Arc::new(AtomicHistogram::<f64, S>::new(S::default()));
            (histogram, values)
        })
        .bench_values(|(histogram, values)| {
            let hist = histogram.clone();
            for &val in &values {
                hist.add_value(black_box(val));
            }
        });
}

#[divan::bench(
    types = [AtomicExponentialAggregationStrategy],
    args = ITEMS,
)]
fn drain_atomic<S: AtomicAggregationStrategy + Default>(bencher: Bencher, items: usize) {
    bencher
        .counter(items)
        .with_inputs(|| {
            let hist = AtomicHistogram::<f64, S>::new(S::default());
            let mut rng = ChaCha8Rng::seed_from_u64(0);
            for _ in 0..items {
                hist.add_value(rng.random_range(0.0..1000.0));
            }
            hist
        })
        .bench_values(|histogram| black_box(histogram.close()));
}
