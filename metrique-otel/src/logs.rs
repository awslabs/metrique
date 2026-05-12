// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::time::SystemTime;

use opentelemetry::logs::{AnyValue, LogRecord, Logger, LoggerProvider};
use opentelemetry_sdk::logs::SdkLoggerProvider;

/// In-memory accumulator for the OTEL [`LogRecord`] we emit per `Entry`.
///
/// String fields on the entry become log attributes; the entry timestamp,
/// if any, becomes the record timestamp. The body is currently left empty
/// because `EntryWriter` does not expose a notion of "entry name" we could
/// use as the log body.
pub(crate) struct LogBuilder {
    // Stored as (String, String) rather than `KeyValue` because log
    // attribute values use `AnyValue`, not the metric-flavored `Value` that
    // `KeyValue` wraps. Keeping them as raw strings until `emit()` avoids
    // converting between the two type hierarchies.
    pub(crate) attributes: Vec<(String, String)>,
    pub(crate) timestamp: Option<SystemTime>,
    pub(crate) body: Option<String>,
}

impl LogBuilder {
    pub(crate) fn new() -> Self {
        Self {
            attributes: Vec::new(),
            timestamp: None,
            body: None,
        }
    }

    pub(crate) fn add_attribute(&mut self, key: &str, value: &str) {
        self.attributes.push((key.to_owned(), value.to_owned()));
    }

    pub(crate) fn set_timestamp(&mut self, ts: SystemTime) {
        self.timestamp = Some(ts);
    }

    /// Emit this log record via the supplied provider, or do nothing if the
    /// accumulator is empty. An entry with only metric fields produces no
    /// attributes and we don't want to spam empty log records.
    pub(crate) fn emit(self, provider: &SdkLoggerProvider) {
        if self.attributes.is_empty() && self.body.is_none() {
            return;
        }
        let logger = provider.logger("metrique-otel");
        let mut record = logger.create_log_record();
        if let Some(ts) = self.timestamp {
            record.set_timestamp(ts);
        }
        for (key, value) in self.attributes {
            record.add_attribute(key, AnyValue::String(value.into()));
        }
        if let Some(body) = self.body {
            record.set_body(AnyValue::String(body.into()));
        }
        logger.emit(record);
    }
}
