# amzn-timesource

A flexible time source abstraction for Rust applications that allows for easy testing and mocking of time-dependent code.

This was originally written to support `Metrique`, but can be used for any application that wants to use deterministic time in tests.

## Features

- Zero-cost abstraction when not compiled with the `custom-timesource` enabled
- Built in support for `tokio`'s time [`pause`](https://docs.rs/tokio/latest/tokio/time/fn.pause.html) with `tokio` feature
- Provide a time source manually or via a thread-local
- Compatible with `std::time::Instant` and `std::time::SystemTime`

## Usage

Add the crate to your `Cargo.toml`:

```toml
[dependencies]
metrique-timesource = "0.1.0"

# If you plan on testing:
[dev-dependencies]
metrique-timesource = { version = "0.1.0", features = ["custom-timesource", "tokio"] }
tokio = { version = "1", features = ["test-util", "full"] }
```

### In production code

In production code, you can choose from two patterns to access a time source. Generally, loading a timesource automatically is the recommended pattern, but some teams prefer the benefits of threading it explicitly (or, e.g. are testing across many threads that makes the mocks difficult to use).

1. Manually thread the `TimeSource` where needed.

```rust
use metrique_timesource::TimeSource;

struct MyThingThatUsesTime {
    time_source: TimeSource
}

impl MyThingThatUsesTime {
    pub fn new(time_source: TimeSource) -> Self {
        Self {
            time_source
        }
    }
}
```

2. Use `get_time_source`/`time_source` to source it automatically:

```rust
use metrique_timesource::{TimeSource, time_source};

struct MyThingThatUsesTime {
    time_source: TimeSource
}

impl MyThingThatUsesTime {
    pub fn new() -> Self {
        Self {
            time_source: time_source()
        }
    }
}
```

`get_time_source` allows passing an `Option<TimeSource>` that is used as the highest priority option. This is equivalent to `maybe_ts.unwrap_or_else(||time_source())`.

```rust
use metrique_timesource::{TimeSource, get_time_source};

struct Thing { timesource: TimeSource }

struct Builder {
    timesource: Option<TimeSource>,
}

impl Builder {
    pub fn build(self) -> Thing {
        let timesource = get_time_source(self.timesource);
        Thing { timesource }
    }
}
```

In both cases, you can then use the timesource to source SystemTimes and Instants that can be externally controlled. Even in the time_source case, when the custom-timesource feature is not enabled, the function is inlined to return a zero-sized-type.

### Working with TimeSource

TimeSource returns wrapped versions of Instant and SystemTime. This allows elapsed to function as expected, even when a time source is overridden.

They provide many of the same methods as the std variants. If you need a method that is not available, you can use `.as_std()`.

### In tests
If you use `time_source`, you can override the timesource for the current thread with `set_time_source` which returns a guard:

```rust
use metrique_timesource::{TimeSource, time_source, set_time_source, Instant};
use std::time::{Duration, UNIX_EPOCH};

struct MyThingThatUsesTime {
    create_time: Instant
}

impl MyThingThatUsesTime {
    pub fn new() -> Self {
        Self {
            create_time: time_source().instant()
        }
    }
}

# async fn test() {
// Note: when using the tokio time source, you can't use multiple threads—tokio::time::pause only works
// on the current-thread runtime. See https://docs.rs/tokio/latest/tokio/time/fn.pause.html
tokio::time::pause();
let _guard = set_time_source(TimeSource::tokio(UNIX_EPOCH));
let my_thing = MyThingThatUsesTime::new();
assert_eq!(my_thing.create_time.elapsed(), Duration::from_secs(0));
tokio::time::advance(Duration::from_secs(5)).await;
assert_eq!(my_thing.create_time.elapsed(), Duration::from_secs(5));
# }

# tokio::runtime::Builder::new_current_thread().build().unwrap().block_on(test())
```

`with_time_source` is also provided which allows running a given closure with a `time_source` installed.

```rust
use metrique_timesource::{TimeSource, fakes::StaticTimeSource, time_source, with_time_source};
use std::time::UNIX_EPOCH;

let ts = StaticTimeSource::at_time(UNIX_EPOCH);
let custom = TimeSource::custom(ts);

// Run code with the custom time source
with_time_source(custom, || {
    // Code here will use the custom time source
    let now = time_source().system_time();
    assert_eq!(now, UNIX_EPOCH);
});
```

## Writing your own mock time
2 mock time sources are provided:
1. `fake::StaticTimeSource` which always returns the same time and instant
2. `TokioTime` which uses `tokio::time::Instant::now`

It is also possible to write your own by implementing the `Time` trait. See the `fakes` module for an example.
