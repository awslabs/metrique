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
//! [`io::Write::write`] calls for one entry). That formatting happens into a
//! private scratch buffer with **no lock held**; only the finished record is
//! appended to the shared buffer, under a brief lock, when the writer is
//! dropped at the entry boundary. A concurrent
//! [`drain`](InMemoryMakeWriter::drain) therefore either sees a record fully
//! committed or not at all — never a partial one, so no reassembly is needed by
//! the fetcher. Keeping the lock out of the formatting path also avoids
//! serializing concurrent writers and means a panic mid-format can neither
//! poison the shared lock nor leave a half-written record behind.
//!
//! # Capacity and overflow
//!
//! The buffer has a configurable **maximum capacity** in bytes (default: 1 MiB).
//! When a newly completed record would cause the buffer to exceed this limit,
//! the oldest records are dropped (from the front) until there is room. This
//! **drop-oldest** policy matches
//! [`BackgroundQueue`](crate::sink::BackgroundQueue)'s ring-buffer semantics:
//! recent metrics are more valuable for describing the current state of a
//! system, so shedding the oldest data is preferred over shedding the newest.
//!
//! If you need an unbounded buffer, pass [`usize::MAX`] as the capacity.
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
//! // A second drain returns nothing — the buffer was emptied.
//! assert!(buffer.drain().is_empty());
//! ```

use std::{
    io::{self, Write},
    sync::{Arc, Mutex},
};

use tracing_subscriber::fmt::MakeWriter;

/// Default maximum buffer capacity: 1 MiB.
const DEFAULT_MAX_CAPACITY: usize = 1024 * 1024;

/// Shared state behind an [`InMemoryMakeWriter`].
#[derive(Debug)]
struct Inner {
    /// Accumulated newline-delimited records.
    buf: Vec<u8>,
    /// Maximum byte capacity before oldest records are evicted.
    max_capacity: usize,
    /// Number of records dropped due to overflow since the last drain.
    records_dropped: u64,
}

/// A bounded, in-memory [`MakeWriter`] that buffers formatted metric records
/// (newline-delimited) for a fetcher to [`drain`](Self::drain) on demand.
///
/// When the buffer exceeds its configured capacity after a record is written,
/// the **oldest** records are evicted until the buffer fits within the limit.
/// This drop-oldest policy mirrors
/// [`BackgroundQueue`](crate::sink::BackgroundQueue)'s behaviour: recent
/// metrics better describe the current system state.
///
/// This is the pull-based counterpart to the push destinations normally handed
/// to
/// [`FormatExt::output_to_makewriter`](crate::format::FormatExt::output_to_makewriter);
/// see the [module documentation](self) for the delivery model and the
/// record-integrity guarantee.
///
/// Cloning is cheap and yields another handle to the *same* underlying buffer
/// (an `Arc<Mutex<…>>`), so the writer side and the fetcher side can each hold
/// a clone.
#[derive(Clone, Debug)]
pub struct InMemoryMakeWriter {
    inner: Arc<Mutex<Inner>>,
}

impl Default for InMemoryMakeWriter {
    fn default() -> Self {
        Self::with_max_capacity(DEFAULT_MAX_CAPACITY)
    }
}

impl InMemoryMakeWriter {
    /// Create a new [`InMemoryMakeWriter`] with the default maximum capacity
    /// (1 MiB).
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a new [`InMemoryMakeWriter`] that will evict oldest records once
    /// the buffered bytes exceed `max_capacity`.
    ///
    /// Pass [`usize::MAX`] to get effectively unbounded behaviour.
    ///
    /// # Panics
    ///
    /// Panics if `max_capacity` is zero.
    pub fn with_max_capacity(max_capacity: usize) -> Self {
        assert!(max_capacity > 0, "max_capacity must be greater than zero");
        Self {
            inner: Arc::new(Mutex::new(Inner {
                buf: Vec::new(),
                max_capacity,
                records_dropped: 0,
            })),
        }
    }

    /// Remove and return all buffered bytes (newline-delimited records), leaving
    /// the buffer empty.
    ///
    /// Because every entry is written under a single lock hold (see the [module
    /// documentation](self)), the returned bytes always end on a complete,
    /// newline-terminated record.
    pub fn drain(&self) -> Vec<u8> {
        let mut inner = self.inner.lock().unwrap();
        inner.records_dropped = 0;
        std::mem::take(&mut inner.buf)
    }

    /// The number of records that have been dropped (evicted) due to overflow
    /// since the last [`drain`](Self::drain).
    pub fn records_dropped(&self) -> u64 {
        self.inner.lock().unwrap().records_dropped
    }
}

impl<'a> MakeWriter<'a> for InMemoryMakeWriter {
    type Writer = InMemoryWriter<'a>;

    /// Return a writer that formats into a private scratch buffer. No lock is
    /// held while the entry is being formatted; the finished record is
    /// committed to the shared buffer under a brief lock when the writer is
    /// dropped. See [`InMemoryWriter`].
    fn make_writer(&'a self) -> Self::Writer {
        InMemoryWriter {
            inner: &self.inner,
            scratch: Vec::new(),
        }
    }
}

/// The [`Write`] handle handed to a [`Format`](crate::format::Format) for a
/// single entry by [`InMemoryMakeWriter`].
///
/// The entry is formatted into a private, unlocked `scratch` buffer (a format
/// may issue several [`io::Write::write`] calls). Only when the writer is
/// dropped — i.e. the entry is complete — is the shared buffer locked, the
/// finished record appended, and capacity enforced, all in one brief hold.
///
/// This keeps the shared lock out of the (potentially expensive) formatting
/// path: writers don't serialize on each other while formatting, a concurrent
/// [`drain`](InMemoryMakeWriter::drain) is only ever blocked for the length of
/// an append, and a panic *during* formatting can neither poison the shared
/// lock nor leave a partial record behind (see [`Drop`]).
#[derive(Debug)]
pub struct InMemoryWriter<'a> {
    inner: &'a Mutex<Inner>,
    /// The current entry's bytes, accumulated with no lock held.
    scratch: Vec<u8>,
}

impl Write for InMemoryWriter<'_> {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        self.scratch.extend_from_slice(data);
        Ok(data.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Drop for InMemoryWriter<'_> {
    fn drop(&mut self) {
        // If formatting panicked mid-entry, `scratch` holds a partial,
        // non-newline-terminated record. Dropping it (rather than committing)
        // preserves the whole-records invariant a fetcher relies on.
        if self.scratch.is_empty() || std::thread::panicking() {
            return;
        }

        // Brief critical section: append the finished record, then enforce
        // capacity. This is the only place the shared lock is taken on the
        // write path, and it does no user-supplied work, so it can't poison.
        let mut inner = self.inner.lock().unwrap();
        let record_start = inner.buf.len();
        inner.buf.extend_from_slice(&self.scratch);

        if inner.buf.len() <= inner.max_capacity {
            return;
        }

        // Evict whole records from the front until the buffer fits. We only
        // scan the bytes *before* the record just appended ([record_start..]),
        // so that record is never evicted or split. If it alone exceeds
        // capacity, the loop evicts the entire prefix and we keep it in full —
        // a record is never truncated.
        let target = inner.max_capacity;
        let mut drop_up_to = 0;
        let mut dropped = 0u64;
        for (i, &byte) in inner.buf[..record_start].iter().enumerate() {
            if byte == b'\n' {
                drop_up_to = i + 1;
                dropped += 1;
                if inner.buf.len() - drop_up_to <= target {
                    break;
                }
            }
        }

        if drop_up_to > 0 {
            inner.buf.drain(..drop_up_to);
            inner.records_dropped += dropped;
        }
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
        let buffer = InMemoryMakeWriter::with_max_capacity(usize::MAX);

        for entry in &groups {
            let mut writer = buffer.make_writer();
            for chunk in entry {
                assert_eq!(writer.write(chunk.as_bytes()).unwrap(), chunk.len());
            }
        }

        assert_eq!(buffer.drain(), expected.as_bytes().to_vec());

        // Drain always empties the buffer: a second drain yields nothing.
        assert!(buffer.drain().is_empty());
    }

    #[test]
    fn formatting_does_not_hold_the_shared_lock() {
        // The whole point of the scratch buffer: a writer can be formatting an
        // entry (holding written-but-uncommitted bytes) while a concurrent
        // drain still proceeds, because the shared lock is only taken to commit
        // on drop.
        let buffer = InMemoryMakeWriter::new();
        let mut writer = buffer.make_writer();
        writer.write_all(b"pending\n").unwrap();

        // Lock is free while the entry is still being formatted...
        assert!(
            buffer.inner.try_lock().is_ok(),
            "shared lock must not be held while formatting an entry"
        );
        // ...and the uncommitted record is not visible to a drain yet.
        assert!(
            buffer.drain().is_empty(),
            "record must not be visible before the writer is dropped"
        );

        // Dropping commits the record atomically.
        drop(writer);
        assert_eq!(buffer.drain(), b"pending\n");
    }

    #[test]
    fn partial_record_is_discarded_on_panic() {
        // If a format panics mid-entry, the partial (non-newline-terminated)
        // scratch must be dropped, not committed, so the fetcher never sees a
        // torn record.
        let buffer = InMemoryMakeWriter::new();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut writer = buffer.make_writer();
            writer.write_all(b"partial").unwrap(); // no trailing newline
            panic!("format blew up mid-entry");
        }));
        assert!(result.is_err());

        // Nothing committed, and the lock is not poisoned.
        assert!(buffer.drain().is_empty());
        buffer.make_writer().write_all(b"ok\n").unwrap();
        assert_eq!(buffer.drain(), b"ok\n");
    }

    #[test]
    fn clones_share_one_underlying_buffer() {
        let buffer = InMemoryMakeWriter::new();
        let clone = buffer.clone();

        buffer.make_writer().write_all(b"shared\n").unwrap();
        // The clone observes the write and drains it.
        assert_eq!(clone.drain(), b"shared\n".to_vec());
        // The original handle now sees the emptied buffer too.
        assert!(buffer.drain().is_empty());
    }

    #[test]
    fn evicts_oldest_records_when_over_capacity() {
        // Capacity is 10 bytes. Write 3 records that together exceed it.
        let buffer = InMemoryMakeWriter::with_max_capacity(10);

        // "aaaa\n" = 5 bytes
        buffer.make_writer().write_all(b"aaaa\n").unwrap();
        // "bbbb\n" = 5 bytes, total = 10, still fits
        buffer.make_writer().write_all(b"bbbb\n").unwrap();
        assert_eq!(buffer.records_dropped(), 0);

        // "cccc\n" = 5 bytes, total would be 15 → evict oldest until ≤ 10
        buffer.make_writer().write_all(b"cccc\n").unwrap();
        assert_eq!(buffer.records_dropped(), 1);

        assert_eq!(buffer.drain(), b"bbbb\ncccc\n");
        assert_eq!(buffer.records_dropped(), 0); // reset on drain
    }

    #[test]
    fn evicts_multiple_records_to_fit() {
        // Capacity is 6 bytes; overflow forces *two* evictions in one write.
        let buffer = InMemoryMakeWriter::with_max_capacity(6);

        buffer.make_writer().write_all(b"aa\n").unwrap();
        buffer.make_writer().write_all(b"bb\n").unwrap();

        // Third record is large enough that evicting a single record isn't
        // enough: 6 + 5 = 11 > 6. Evict "aa\n" -> 8 (>6), evict "bb\n" -> 5 (<=6).
        buffer.make_writer().write_all(b"cccc\n").unwrap(); // 5 bytes
        assert_eq!(buffer.records_dropped(), 2);
        assert_eq!(buffer.drain(), b"cccc\n");
    }

    #[test]
    fn single_record_larger_than_capacity_still_kept() {
        let buffer = InMemoryMakeWriter::with_max_capacity(5);

        // Write a record that on its own exceeds capacity.
        buffer.make_writer().write_all(b"huge_record\n").unwrap();
        assert_eq!(buffer.records_dropped(), 0);

        assert_eq!(buffer.drain(), b"huge_record\n");
    }

    #[test]
    fn large_record_evicts_all_prior() {
        let buffer = InMemoryMakeWriter::with_max_capacity(5);

        buffer.make_writer().write_all(b"a\n").unwrap();
        buffer.make_writer().write_all(b"b\n").unwrap();
        // Now write one that alone exceeds capacity:
        buffer.make_writer().write_all(b"large_one\n").unwrap();

        assert_eq!(buffer.records_dropped(), 2);
        assert_eq!(buffer.drain(), b"large_one\n");
    }

    #[test]
    #[should_panic(expected = "max_capacity must be greater than zero")]
    fn zero_capacity_panics() {
        InMemoryMakeWriter::with_max_capacity(0);
    }

    #[test]
    fn unbounded_with_usize_max() {
        let buffer = InMemoryMakeWriter::with_max_capacity(usize::MAX);
        for _ in 0..1000 {
            buffer.make_writer().write_all(b"record\n").unwrap();
        }
        // Nothing is ever evicted: all 1000 records (7 bytes each) are retained.
        assert_eq!(buffer.records_dropped(), 0);
        assert_eq!(buffer.drain().len(), 7000);
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
        let mut stream =
            Emf::all_validations("Test".into(), vec![vec![]]).output_to_makewriter(buffer.clone());

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
        assert_eq!(emf["_aws"]["CloudWatchMetrics"][0]["Namespace"], "Test");

        // Buffer is reusable after a drain: a second drain yields nothing.
        assert!(buffer.drain().is_empty());
    }

    use metrique_writer_format_emf::Emf;
}
