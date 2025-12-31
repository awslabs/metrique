use assert2::check;
use metrique::timers::Timer;
use metrique::unit::{Byte, Microsecond, Millisecond};
use metrique::unit_of_work::metrics;
use metrique_aggregation::counter::Counter;
use metrique_aggregation::histogram::{Histogram, SortAndMerge};
use metrique_aggregation::sink::{AggregateSink, MergeOnDropExt, MutexAggregator};
use metrique_aggregation::traits;
use metrique_aggregation::traits::Aggregate;
use metrique_writer::test_util::test_metric;
use metrique_writer::unit::{NegativeScale, PositiveScale};
use metrique_writer::{Observation, Unit};
use std::time::Duration;

#[aggregate(entry)]
#[metrics]
struct ApiCallWithTimer2 {
    #[aggregate(strategy = Histogram<Duration, SortAndMerge>)]
    // doesn't work yet: `unit = ...`
    #[metrics(name = "latency_2", unit = Microsecond)]
    latency: Timer,
}

#[derive(Default)]
struct AggregatedApiCallWithTimer {
    latency: <Histogram<Duration, SortAndMerge> as metrique_aggregation::traits::AggregateValue<
        <Timer as metrique_core::CloseValue>::Closed,
    >>::Aggregated,
}
#[doc(hidden)]
pub struct AggregatedApiCallWithTimerEntry {
    #[deprecated(
        note = "these fields will become private in a future release. To introspect an entry, use `metrique::writer::test_util::test_entry`"
    )]
    #[doc(hidden)]
    latency: <<<Histogram<Duration, SortAndMerge> as metrique_aggregation::traits::AggregateValue<
        <Timer as metrique_core::CloseValue>::Closed,
    >>::Aggregated as metrique::CloseValue>::Closed as ::metrique::unit::AttachUnit>::Output<
        Microsecond,
    >,
}
const _: () = {
    #[expect(deprecated)]
    impl<NS: ::metrique::NameStyle> ::metrique::InflectableEntry<NS>
        for AggregatedApiCallWithTimerEntry
    {
        fn write<'a>(&'a self, writer: &mut impl ::metrique::writer::EntryWriter<'a>) {
            ::metrique::writer::EntryWriter::value(
                writer,
                {
                    #[allow(non_camel_case_types)]
                    struct latencyPreserve;

                    impl ::metrique::concat::ConstStr for latencyPreserve {
                        const VAL: &'static str = "latency_2";
                    }
                    #[allow(non_camel_case_types)]
                    struct latencyKebab;

                    impl ::metrique::concat::ConstStr for latencyKebab {
                        const VAL: &'static str = "latency_2";
                    }
                    #[allow(non_camel_case_types)]
                    struct latencyPascal;

                    impl ::metrique::concat::ConstStr for latencyPascal {
                        const VAL: &'static str = "latency_2";
                    }
                    #[allow(non_camel_case_types)]
                    struct latencySnake;

                    impl ::metrique::concat::ConstStr for latencySnake {
                        const VAL: &'static str = "latency_2";
                    }
                    ::metrique::concat::const_str_value::<
                        <NS as ::metrique::NameStyle>::Inflect<
                            latencyPreserve,
                            latencyPascal,
                            latencySnake,
                            latencyKebab,
                        >,
                    >()
                },
                &self.latency,
            );
        }
        fn sample_group(
            &self,
        ) -> impl ::std::iter::Iterator<
            Item = (
                ::std::borrow::Cow<'static, str>,
                ::std::borrow::Cow<'static, str>,
            ),
        > {
            ::std::iter::empty()
        }
    }
};
impl metrique::CloseValue for AggregatedApiCallWithTimer {
    type Closed = AggregatedApiCallWithTimerEntry;
    fn close(self) -> Self::Closed {
        #[allow(deprecated)]
        AggregatedApiCallWithTimerEntry {
            latency: metrique::CloseValue::close(self.latency).into(),
        }
    }
}
#[doc = concat!("Metrics guard returned from [`","AggregatedApiCallWithTimer","::append_on_drop`], closes the entry and appends the metrics to a sink when dropped.")]
type AggregatedApiCallWithTimerGuard<Q = ::metrique::DefaultSink> =
    ::metrique::AppendAndCloseOnDrop<AggregatedApiCallWithTimer, Q>;
#[doc = concat!("Metrics handle returned from [`","AggregatedApiCallWithTimerGuard","::handle`], similar to an `Arc<","AggregatedApiCallWithTimerGuard",">`.")]
type AggregatedApiCallWithTimerHandle<Q = ::metrique::DefaultSink> =
    ::metrique::AppendAndCloseOnDropHandle<AggregatedApiCallWithTimer, Q>;
impl AggregatedApiCallWithTimer {
    #[doc = "Creates an AppendAndCloseOnDrop that will be automatically appended to `sink` on drop."]
    fn append_on_drop<
        Q: ::metrique::writer::EntrySink<::metrique::RootEntry<AggregatedApiCallWithTimerEntry>>
            + Send
            + Sync
            + 'static,
    >(
        self,
        sink: Q,
    ) -> AggregatedApiCallWithTimerGuard<Q> {
        ::metrique::append_and_close(self, sink)
    }
}
impl metrique_aggregation::sink::MergeOnDropExt for ApiCallWithTimer {}

impl metrique_aggregation::traits::AggregateEntry for ApiCallWithTimer {
    type Source = <Self as metrique_core::CloseValue>::Closed;
    type Aggregated = AggregatedApiCallWithTimer;
    type Key = ();
    fn merge_entry(accum: &mut Self::Aggregated, entry: Self::Source) {
        #[allow(deprecated)]
        <Histogram<Duration, SortAndMerge> as metrique_aggregation::traits::AggregateValue<
            <Timer as metrique_core::CloseValue>::Closed,
        >>::add_value(&mut accum.latency, *entry.latency);
    }
    fn new_aggregated(key: &Self::Key) -> Self::Aggregated {
        Self::Aggregated::default()
    }
    fn key(source: &Self::Source) -> Self::Key {
        #[allow(deprecated)]
        ()
    }
}
struct ApiCallWithTimer {
    latency: Timer,
}
#[doc(hidden)]
pub struct ApiCallWithTimerEntry {
    #[deprecated(
        note = "these fields will become private in a future release. To introspect an entry, use `metrique::writer::test_util::test_entry`"
    )]
    #[doc(hidden)]
    latency: <<Timer as metrique::CloseValue>::Closed as ::metrique::unit::AttachUnit>::Output<
        Microsecond,
    >,
}
const _: () = {
    #[expect(deprecated)]
    impl<NS: ::metrique::NameStyle> ::metrique::InflectableEntry<NS> for ApiCallWithTimerEntry {
        fn write<'a>(&'a self, writer: &mut impl ::metrique::writer::EntryWriter<'a>) {
            ::metrique::writer::EntryWriter::value(
                writer,
                {
                    #[allow(non_camel_case_types)]
                    struct latencyPreserve;

                    impl ::metrique::concat::ConstStr for latencyPreserve {
                        const VAL: &'static str = "latency_2";
                    }
                    #[allow(non_camel_case_types)]
                    struct latencyKebab;

                    impl ::metrique::concat::ConstStr for latencyKebab {
                        const VAL: &'static str = "latency_2";
                    }
                    #[allow(non_camel_case_types)]
                    struct latencyPascal;

                    impl ::metrique::concat::ConstStr for latencyPascal {
                        const VAL: &'static str = "latency_2";
                    }
                    #[allow(non_camel_case_types)]
                    struct latencySnake;

                    impl ::metrique::concat::ConstStr for latencySnake {
                        const VAL: &'static str = "latency_2";
                    }
                    ::metrique::concat::const_str_value::<
                        <NS as ::metrique::NameStyle>::Inflect<
                            latencyPreserve,
                            latencyPascal,
                            latencySnake,
                            latencyKebab,
                        >,
                    >()
                },
                &self.latency,
            );
        }
        fn sample_group(
            &self,
        ) -> impl ::std::iter::Iterator<
            Item = (
                ::std::borrow::Cow<'static, str>,
                ::std::borrow::Cow<'static, str>,
            ),
        > {
            ::std::iter::empty()
        }
    }
};
impl metrique::CloseValue for ApiCallWithTimer {
    type Closed = ApiCallWithTimerEntry;
    fn close(self) -> Self::Closed {
        #[allow(deprecated)]
        ApiCallWithTimerEntry {
            latency: metrique::CloseValue::close(self.latency).into(),
        }
    }
}
#[doc = concat!("Metrics guard returned from [`","ApiCallWithTimer","::append_on_drop`], closes the entry and appends the metrics to a sink when dropped.")]
type ApiCallWithTimerGuard<Q = ::metrique::DefaultSink> =
    ::metrique::AppendAndCloseOnDrop<ApiCallWithTimer, Q>;
#[doc = concat!("Metrics handle returned from [`","ApiCallWithTimerGuard","::handle`], similar to an `Arc<","ApiCallWithTimerGuard",">`.")]
type ApiCallWithTimerHandle<Q = ::metrique::DefaultSink> =
    ::metrique::AppendAndCloseOnDropHandle<ApiCallWithTimer, Q>;
impl ApiCallWithTimer {
    #[doc = "Creates an AppendAndCloseOnDrop that will be automatically appended to `sink` on drop."]
    fn append_on_drop<
        Q: ::metrique::writer::EntrySink<::metrique::RootEntry<ApiCallWithTimerEntry>>
            + Send
            + Sync
            + 'static,
    >(
        self,
        sink: Q,
    ) -> ApiCallWithTimerGuard<Q> {
        ::metrique::append_and_close(self, sink)
    }
}
