// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! `EntryIoStream` implementation that pushes observations into an OTel meter
//! provider. Use [`OtelSink::stream`] to obtain one when composing with
//! `tee` / `merge_globals` / `merge_global_dimensions` / sampling wrappers.
//!
//! [`OtelSink::stream`]: crate::OtelSink::stream

use std::{io, sync::Arc};

use metrique_writer_core::{Entry, stream::EntryIoStream};

use crate::{metrics::InstrumentCache, translator::append_with_pool};

/// An [`EntryIoStream`] that records observations against an
/// [`SdkMeterProvider`]. Obtain one via [`OtelSink::stream`].
///
/// `Clone` is shallow (Arc bump). All clones share the same instrument cache
/// and meter provider — that's the right behavior for composition: every
/// branch records into the same OTel pipeline.
///
/// `flush` is a no-op: observations are pushed during [`Self::next`], and the
/// OTel `PeriodicReader` is what eventually exports them. Use
/// [`OtelSink::flush`] / [`OtelSink::flush_async`] to trigger an export
/// explicitly.
///
/// [`OtelSink::stream`]: crate::OtelSink::stream
/// [`OtelSink::flush`]: crate::OtelSink::flush
/// [`OtelSink::flush_async`]: crate::OtelSink::flush_async
/// [`SdkMeterProvider`]: opentelemetry_sdk::metrics::SdkMeterProvider
#[derive(Clone)]
pub struct OtelStream {
    inner: Arc<OtelStreamInner>,
}

pub(crate) struct OtelStreamInner {
    pub(crate) instruments: InstrumentCache,
    pub(crate) scope: &'static str,
}

impl OtelStream {
    pub(crate) fn from_inner(inner: Arc<OtelStreamInner>) -> Self {
        Self { inner }
    }
}

impl EntryIoStream for OtelStream {
    fn next(
        &mut self,
        entry: &impl Entry,
    ) -> Result<(), metrique_writer_core::stream::IoStreamError> {
        append_with_pool(&self.inner.instruments, self.inner.scope, entry);
        Ok(())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
