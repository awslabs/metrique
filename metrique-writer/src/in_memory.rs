// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! An in-memory, drainable [`MakeWriter`] destination for *pull-based* metric
//! delivery.
//!
//! [`FormatExt::output_to_makewriter`](crate::format::FormatExt::output_to_makewriter)
//! lets a [`Format`](crate::format::Format) push formatted records (EMF, JSON,
//! ...) to any [`MakeWriter`] destination. The destinations metrique points at
//! are all *push* targets: a rotating file (`tracing-appender`), stdout, a TCP
//! socket. On a host that cannot push — a diskless or network-isolated process
//! whose only egress is a fetcher that pulls on demand — none of those fit.
//!
//! [`InMemoryMakeWriter`] is such a destination. It accumulates the formatted
//! bytes in memory and lets a fetcher [`drain`](InMemoryMakeWriter::drain) them
//! whenever it likes, bridging metrique's push-only writer pipeline to a
//! pull-based transport while reusing the full formatting stack (`Format` +
//! [`BackgroundQueue`](crate::sink::BackgroundQueue) or
//! [`FlushImmediately`](crate::sink::FlushImmediately)) unchanged.
//!
//! # Record integrity
//!
//! [`output_to_makewriter`](crate::format::FormatExt::output_to_makewriter)
//! calls [`MakeWriter::make_writer`] exactly once per entry and formats the
//! whole entry into the returned writer (a format may issue *several*
//! [`io::Write::write`] calls for one entry). [`InMemoryMakeWriter`] holds the
//! buffer lock for that writer's entire lifetime, so every write for an entry
//! happens under a single lock hold released only at the entry boundary. A
//! concurrent [`drain`](InMemoryMakeWriter::drain) therefore can only run
//! *between* entries and always observes whole records — no partial records, so
//! no reassembly is needed by the fetcher.
//!
//! # Unboundedness
//!
//! The buffer is **unbounded**: nothing is evicted until it is drained. A
//! fetcher that stops draining will let it grow until memory is exhausted. If
//! you need a ceiling, drain on a timer or from a bounded loop rather than
//! relying on the buffer to shed load.
//!
//! # Example
//!
//! ```
//! # use metrique_writer::{
//! #     Entry, EntryIoStream, format::FormatExt as _, in_memory::InMemoryMakeWriter,
//! # };
//! # use metrique_writer_format_emf::Emf;
//! # use std::time::SystemTime;
//! # #[derive(Entry)]
//! # #[entry(rename_all = "PascalCase")]
//! # struct RequestMetrics {
//! #     #[entry(timestamp)]
//! #     timestamp: SystemTime,
//! #     number_of_ducks: u64,
//! # }
//! let buffer = InMemoryMakeWriter::new();
//!
//! // Wire the buffer in wherever a `MakeWriter` destination is expected. In a
//! // real service this stream would be driven by a `BackgroundQueue`.
//! let mut stream = Emf::all_validations("MyApp".into(), vec![vec![]])
//!     .output_to_makewriter(buffer.clone());
//!
//! stream
//!     .next(&RequestMetrics {
//!         timestamp: SystemTime::now(),
//!         number_of_ducks: 5,
//!     })
//!     .unwrap();
//!
//! // The fetcher pulls the formatted EMF records on demand.
//! let records = buffer.drain();
//! assert!(!records.is_empty());
//! assert!(buffer.is_empty());
//! ```

use std::{
    io::{self, Write},
    sync::{Arc, Mutex, MutexGuard},
};

use tracing_subscriber::fmt::MakeWriter;

/// An unbounded, in-memory [`MakeWriter`] that buffers formatted metric records
/// (newline-delimited) for a fetcher to [`drain`](Self::drain) on demand.
///
/// This is the pull-based counterpart to the push destinations normally handed
/// to
/// [`FormatExt::output_to_makewriter`](crate::format::FormatExt::output_to_makewriter);
/// see the [module documentation](self) for the delivery model, the
/// record-integrity guarantee, and the unboundedness caveat.
///
/// Cloning is cheap and yields another handle to the *same* underlying buffer
/// (an `Arc<Mutex<Vec<u8>>>`), so the writer side and the fetcher side can each
/// hold a clone.
#[derive(Clone, Default, Debug)]
pub struct InMemoryMakeWriter {
    inner: Arc<Mutex<Vec<u8>>>,
}

impl InMemoryMakeWriter {
    /// Create a new, empty [`InMemoryMakeWriter`].
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a new [`InMemoryMakeWriter`] with room for at least `capacity`
    /// bytes before reallocating.
    ///
    /// This is purely a performance hint; the buffer resizes as needed and is
    /// limited only by available memory.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            inner: Arc::new(Mutex::new(Vec::with_capacity(capacity))),
        }
    }

    /// Remove and return all buffered bytes (newline-delimited records), leaving
    /// the buffer empty.
    ///
    /// Because every entry is written under a single lock hold (see the [module
    /// documentation](self)), the returned bytes always end on a complete,
    /// newline-terminated record.
    pub fn drain(&self) -> Vec<u8> {
        let mut buffer = self.inner.lock().unwrap();
        std::mem::take(&mut *buffer)
    }

    /// The number of bytes currently buffered across all pending records.
    pub fn len(&self) -> usize {
        self.inner.lock().unwrap().len()
    }

    /// Whether no bytes are currently buffered.
    pub fn is_empty(&self) -> bool {
        self.inner.lock().unwrap().is_empty()
    }
}

impl<'a> MakeWriter<'a> for InMemoryMakeWriter {
    type Writer = InMemoryWriter<'a>;

    /// Acquire the buffer lock and hold it for the returned writer's lifetime,
    /// making all of one entry's writes atomic with respect to
    /// [`drain`](InMemoryMakeWriter::drain).
    fn make_writer(&'a self) -> Self::Writer {
        InMemoryWriter {
            guard: self.inner.lock().unwrap(),
        }
    }
}

/// The [`Write`] handle handed to a [`Format`](crate::format::Format) for a
/// single entry by [`InMemoryMakeWriter`].
///
/// It holds the buffer lock for its whole lifetime, so an entry's (possibly
/// multiple) writes cannot interleave with a
/// [`drain`](InMemoryMakeWriter::drain).
#[derive(Debug)]
pub struct InMemoryWriter<'a> {
    guard: MutexGuard<'a, Vec<u8>>,
}

impl Write for InMemoryWriter<'_> {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        self.guard.extend_from_slice(data);
        Ok(data.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::time::SystemTime;

    use rstest::rstest;

    use super::*;
    use crate::{Entry, EntryIoStream, EntryWriter, format::FormatExt as _};

    /// Write/accumulate/drain behavior across a range of write groupings.
    ///
    /// `groups` is one `make_writer()` per inner `Vec` (an entry), with each
    /// `&str` a separate `write` call within that writer — covering the empty
    /// buffer, a single record, a format's multiple-writes-per-entry, and
    /// sequential entries.
    #[rstest]
    #[case(vec![], "")]
    #[case(vec![vec!["record\n"]], "record\n")]
    #[case(vec![vec![r#"{"Value":7}"#, "\n"]], "{\"Value\":7}\n")]
    #[case(vec![vec!["a\n"], vec!["b\n"]], "a\nb\n")]
    fn accumulates_then_drains(#[case] groups: Vec<Vec<&str>>, #[case] expected: &str) {
        let buffer = InMemoryMakeWriter::new();
        assert!(buffer.is_empty());

        for entry in &groups {
            let mut writer = buffer.make_writer();
            for chunk in entry {
                assert_eq!(writer.write(chunk.as_bytes()).unwrap(), chunk.len());
            }
        }

        assert_eq!(buffer.len(), expected.len());
        assert_eq!(buffer.is_empty(), expected.is_empty());
        assert_eq!(buffer.drain(), expected.as_bytes().to_vec());

        // Drain always empties the buffer.
        assert!(buffer.is_empty());
        assert_eq!(buffer.len(), 0);
    }

    #[test]
    fn writer_holds_the_lock_for_its_lifetime() {
        // The property that makes an entry's writes atomic: while a writer is
        // alive the buffer lock is held, so a concurrent drain (which takes the
        // same lock) cannot observe a partial entry.
        let buffer = InMemoryMakeWriter::new();
        let writer = buffer.make_writer();
        assert!(
            buffer.inner.try_lock().is_err(),
            "writer must hold the lock while alive"
        );
        drop(writer);
        assert!(
            buffer.inner.try_lock().is_ok(),
            "lock must be released once the writer is dropped"
        );
    }

    #[test]
    fn clones_share_one_underlying_buffer() {
        let buffer = InMemoryMakeWriter::new();
        let clone = buffer.clone();

        buffer.make_writer().write_all(b"shared\n").unwrap();
        // The clone observes the write and drains it.
        assert_eq!(clone.drain(), b"shared\n".to_vec());
        assert!(buffer.is_empty());
    }

    struct Ping {
        timestamp: SystemTime,
        value: u64,
    }

    impl Entry for Ping {
        fn write<'a>(&'a self, writer: &mut impl EntryWriter<'a>) {
            writer.timestamp(self.timestamp);
            writer.value("Value", &self.value);
        }
    }

    #[test]
    fn buffers_one_emf_record_end_to_end() {
        let buffer = InMemoryMakeWriter::new();
        let mut stream = Emf::all_validations("CloudHSM/Test".into(), vec![vec![]])
            .output_to_makewriter(buffer.clone());

        stream
            .next(&Ping {
                timestamp: SystemTime::now(),
                value: 7,
            })
            .unwrap();

        let bytes = buffer.drain();
        let line = bytes
            .strip_suffix(b"\n")
            .expect("EMF record should be newline-terminated");
        assert!(
            !line.contains(&b'\n'),
            "expected exactly one EMF record, got: {}",
            String::from_utf8_lossy(&bytes)
        );

        let emf: serde_json::Value = serde_json::from_slice(line).unwrap();
        assert_eq!(emf["Value"], serde_json::Value::from(7));
        assert_eq!(
            emf["_aws"]["CloudWatchMetrics"][0]["Namespace"],
            "CloudHSM/Test"
        );

        // Buffer is reusable after a drain.
        assert!(buffer.is_empty());
    }

    use metrique_writer_format_emf::Emf;
}
