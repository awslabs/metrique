## Metrique

A set of crates for collecting and exporting metrics, especially unit-of-work metrics.

This currently supports exporting metrics in [Amazon EMF] format to CloudWatch.
More formats might be supported in future versions.


## Getting Started

Most applications and libraries will use [`metrique`](metrique) directly and configure a writer with [`metrique-writer`](metrique-writer). See the [examples](metrique/examples) for several examples of different common patterns.

Applications will define a metrics struct that they annotate with `#[metrics]`:
```rust
use metrique::unit_of_work::metrics;

#[metrics]
struct RequestMetrics {
    operation: &'static str,
    #[metrics(timestamp)]
    timestamp: Timestamp,
    number_of_ducks: usize,
    #[metrics(unit = Millisecond)]
    operation_time: Timer,
}
```

On its own, this is just a normal struct, there is no magic. To use it as a metric, you can call `.append_on_drop`:
```rust
impl RequestMetrics {
    // It is generally a good practice to expose a single initializer that sets up
    // append on drop.
    fn init(operation: &'static str) -> RequestMetricsGuard {
        RequestMetrics {
            timestamp: Timestamp::now(),
            operation,
            number_of_ducks: 0,
            operation_time: Timer::start_now(),
        }.append_on_drop(ServiceMetrics::sink())
    }
}
```

The `guard` object can still be mutated via `DerefMut` impl:
```rust
async fn count_ducks() {
    let mut metrics = RequestMetrics::init("CountDucks");
    metrics.number_of_ducks = 5;
    // metrics flushes as scope drops
    // timer records the total time until scope exits
}
```

But when it drops, it will be appended to the queue to be formatted and flushed. 

To control how it is written, when you start your application, you must configure a queue:
```rust
global_entry_sink! { ServiceMetrics }

fn initialize_metrics(service_log_dir: PathBuf) -> AttachHandle {
    ServiceMetrics::attach_to_stream(
        Emf::builder("Ns".to_string(), vec![vec![]])
            .build()
            .output_to_makewriter(RollingFileAppender::new(
                Rotation::MINUTELY,
                &service_log_dir,
                "service_log.log",
            )),
    )
}
```

> See [`metrique-writer`](metrique-writer) for more information about queues and destinations.

You can either attach it to a global destination or thread the queue to the location you construct your metrics object directly. Currently, only formatters for [Amazon EMF] are provided, but more may be added in the future.

## Security

See [CONTRIBUTING](CONTRIBUTING.md#security-issue-notifications) for more information.

[Amazon EMF]: https://docs.aws.amazon.com/AmazonCloudWatch/latest/monitoring/CloudWatch_Embedded_Metric_Format_Specification.html


## License

This project is licensed under the Apache-2.0 License.
