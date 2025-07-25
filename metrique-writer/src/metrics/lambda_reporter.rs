// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! This module contains a metric reporter that allows easily emitting metrics from a Lambda.
//!
//! It is designed for short-lived Lambda handlers that want to flush metrics once, at the end of
//! an invocation, rather than on a background loop.
//!
//! It provides a default [`install_reporter()`] method that flushes to stdout,
//! or a [`install_reporter_to_writer()`] method that allows customizing the i/o destination.
//!
//! You are responsible for calling [`flush_metrics()`] or [`flush_metrics_sync()`] at the end
//! of your Lambda invocation handler, or else no metrics will be emitted.

use std::io;
use std::io::stdout;
use std::sync::OnceLock;

use crate::FormatExt;
use crate::sink::BackgroundQueue;
use crate::sink::BackgroundQueueJoinHandle;
use metrique_writer_core::format::Format;
use metrique_writer_core::{EntryIoStream, EntrySink, IoStreamError};

use super::MetricAccumulatorEntry;
use super::MetricRecorder;

struct LambdaMetricReporter {
    reporter: BackgroundQueue<MetricAccumulatorEntry>,
    #[allow(unused)]
    join_handle: BackgroundQueueJoinHandle,
    recorder: MetricRecorder,
}

impl LambdaMetricReporter {
    /// Creates a new MetricReporter.
    fn new(sink: impl EntryIoStream + Send + 'static) -> (Self, MetricRecorder) {
        let recorder = MetricRecorder::new();
        let recorder_ = recorder.clone();
        let (reporter, join_handle) = BackgroundQueue::new(sink);
        (
            Self {
                reporter,
                join_handle,
                recorder,
            },
            recorder_,
        )
    }

    pub(crate) async fn report(&self) {
        let entry = self.recorder.readout();
        self.reporter.append(entry);
        self.reporter.flush_async().await;
    }
}

static METRIC_REPORTER: OnceLock<LambdaMetricReporter> = OnceLock::new();

struct BufferingStdoutWriter<W: io::Write, F: Fn() -> W> {
    /// Buf is `None` when there is an error writing to the inner writer
    buf: Option<Vec<u8>>,
    f: F,
}

impl<W: io::Write, F: Fn() -> W> BufferingStdoutWriter<W, F> {
    pub fn new(f: F) -> Self {
        Self {
            buf: Some(vec![]),
            f,
        }
    }
}

impl<W: io::Write, F: Fn() -> W> io::Write for BufferingStdoutWriter<W, F> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match &mut self.buf {
            Some(buf_) => buf_.extend(buf),
            // turn temporary errors into permanent errors
            None => {
                return Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "write after error",
                ));
            }
        };
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        // stdout.write_all guarantees that the buffer will be written in full to avoid stripping.
        match &mut self.buf {
            Some(buf) => {
                let mut writer = (self.f)();
                match writer.write_all(buf) {
                    Ok(()) => {
                        buf.clear();
                        match writer.flush() {
                            Ok(()) => Ok(()),
                            Err(e) => {
                                self.buf.take();
                                Err(e)
                            }
                        }
                    }
                    Err(e) => {
                        self.buf.take();
                        Err(e)
                    }
                }
            }
            // turn temporary errors into permanent errors
            None => Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "write after error",
            )),
        }
    }
}

/// Installs a reporter that outputs to a specific writer.
///
/// For example:
/// ```
/// # use metrique_writer_format_emf::Emf;
/// # use metrique_writer::{IoStreamError, metrics::lambda_reporter};
/// # use metrique_writer_core::test_stream::TestSink;
/// #
/// # let sink = TestSink::default();
/// # let sink_ = sink.clone();
/// # let writer = move || sink_.clone();
/// lambda_reporter::install_reporter_to_writer(Emf::all_validations("MyNS".to_string(), vec![vec![]]), writer);
/// metrics::counter!("my_counter", "request_kind" => "foo").increment(2);
/// lambda_reporter::flush_metrics_sync();
/// assert!(sink.dump().contains("my_counter"));
///
/// # Ok::<_, IoStreamError>(())
/// ```
pub fn install_reporter_to_writer<
    F: Format + Send + 'static,
    W: Fn() -> O + Send + 'static,
    O: io::Write + 'static,
>(
    f: F,
    w: W,
) {
    METRIC_REPORTER.get_or_init(|| {
        let writer = BufferingStdoutWriter::new(w);
        let (reporter, recorder) = LambdaMetricReporter::new(f.output_to(writer));
        metrics::set_global_recorder(recorder).expect("failed to set global recorder");
        reporter
    });
}

/// Installs a reporter that outputs to stdout using `f` as a formatter.
/// `f` should normally be an [EMF] formatter - this will work natively with Lambda to output
/// your metrics to CloudWatch.
///
/// For this to work, your Lambda's role needs to have permissions to write CloudWatch Logs.
/// The `AWSLambdaBasicExecutionRole` IAM permission policy will do that, but it comes
/// with the right permissions to write to *all* CloudWatch Logs streams in your account, which might
/// be more powerful than you intended. If you want to add permissions yourself, see
/// <https://docs.aws.amazon.com/lambda/latest/operatorguide/access-logs.html>
///
/// Due to the way [EMF] works, you don't need to give anyone write permissions to
/// CloudWatch *Metrics*. Any CloudWatch Logs emitter can use EMF to emit arbitrary
/// CloudWatch Metrics in the same account.
///
/// [EMF]: https://docs.aws.amazon.com/AmazonCloudWatch/latest/monitoring/CloudWatch_Embedded_Metric_Format_Specification.html
///
/// for example:
///
/// ```
/// # use metrique_writer_format_emf::Emf;
/// # use metrique_writer::{IoStreamError, metrics::lambda_reporter};
///
/// // on Lambda initialization
/// lambda_reporter::install_reporter(Emf::all_validations("MyNS".to_string(), vec![vec![]]));
/// // during runtime
/// metrics::counter!("my_counter").increment(2);
///
/// // This will create a `my_counter` in namespace `MyNS` in your CloudWatch Metrics,
/// // no extra setup needed.
/// # futures::executor::block_on(async {
/// lambda_reporter::flush_metrics().await;
/// # });
///
/// # Ok::<_, IoStreamError>(())
/// ```
pub fn install_reporter<F: Format + Send + 'static>(f: F) {
    install_reporter_to_writer(f, stdout)
}

/// Synchronously flush the metrics in the current reporter. This function blocks
/// until the metrics are flushed so it is undesirable to use it in an async context.
///
/// You are responsible for calling [`flush_metrics()`] or [`flush_metrics_sync()`] at the end
/// of your Lambda invocation handler, or else no metrics will be emitted.
pub fn flush_metrics_sync() -> Result<(), IoStreamError> {
    futures::executor::block_on(flush_metrics())
}

/// Asynchronously flush the metrics in the current reporter
/// You are responsible for calling [`flush_metrics()`] or [`flush_metrics_sync()`] at the end
/// of your Lambda invocation handler, or else no metrics will be emitted.
pub async fn flush_metrics() -> Result<(), IoStreamError> {
    if let Some(metrics) = METRIC_REPORTER.get() {
        metrics.report().await;
        Ok(())
    } else {
        Err(IoStreamError::Io(io::Error::other(
            "flushing metrics that are not initialized",
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::BufferingStdoutWriter;
    use metrique_writer_core::test_stream::TestSink;
    use std::cell::Cell;
    use std::io::ErrorKind;
    use std::io::Write;

    fn check_buffering_stdout_writer(err_on_flush: bool) {
        let sink = TestSink::default();
        struct ErrWrite {
            err_on_flush: bool,
        }
        impl Write for ErrWrite {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                if self.err_on_flush {
                    Ok(buf.len())
                } else {
                    Err(std::io::Error::new(ErrorKind::Other, ""))
                }
            }

            fn flush(&mut self) -> std::io::Result<()> {
                Err(std::io::Error::new(ErrorKind::Other, ""))
            }
        }
        let is_error = Cell::new(false);
        let sink_fn = Box::new(|| {
            if is_error.get() {
                Box::new(ErrWrite { err_on_flush }) as Box<dyn Write>
            } else {
                Box::new(sink.clone()) as Box<dyn Write>
            }
        }) as Box<dyn Fn() -> Box<dyn Write>>;
        let mut writer = BufferingStdoutWriter::new(sink_fn);
        writer.write(b"1").unwrap();
        is_error.set(true);
        writer.flush().unwrap_err();
        is_error.set(false);
        // check writer in an error state
        writer.write(b"3").unwrap_err();
        writer.flush().unwrap_err();
        assert!(sink.dump().is_empty());
    }

    #[test]
    fn test_buffering_stdout_writer() {
        check_buffering_stdout_writer(false);
    }

    #[test]
    fn test_buffering_stdout_writer_flush() {
        check_buffering_stdout_writer(true);
    }

    #[test]
    fn test_buffering_stdout_writer_ok() {
        let sink = TestSink::default();
        let mut writer: BufferingStdoutWriter<_, _> = BufferingStdoutWriter::new(|| sink.clone());
        writer.write(b"1").unwrap();
        writer.write(b"2").unwrap();
        assert!(sink.dump().is_empty());
        writer.flush().unwrap();
        assert_eq!(sink.dump(), "12");
        writer.write(b"3").unwrap();
        writer.write(b"4").unwrap();
        assert_eq!(sink.dump(), "12");
        writer.flush().unwrap();
        assert_eq!(sink.dump(), "1234");
    }
}
