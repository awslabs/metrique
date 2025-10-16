use metrique::unit_of_work::metrics;
use metrique_writer::{
    MetricFlags,
    value::{FlagConstructor, ForceFlag, MetricOptions},
};

#[derive(Debug)]
struct CounterFlag;
impl FlagConstructor for CounterFlag {
    fn construct() -> metrique_writer::MetricFlags<'static> {
        MetricFlags::upcast(&CounterFlag)
    }
}

impl MetricOptions for CounterFlag {}

type Counter<T> = ForceFlag<T, CounterFlag>;

#[metrics]
struct WorkerMetrics {
    poll_count: Counter<usize>,
}

fn main() {}
