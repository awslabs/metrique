// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use metrique_writer::format::Format;
use metrique_writer::sample::{FixedFractionSample, SampledFormatExt as _};
use metrique_writer::{Entry, EntryIoStream, EntryIoStreamExt, FormatExt, IoStreamError};
use metrique_writer_format_emf::{Emf, SampledEmf};
use std::io;
use std::sync::Arc;
use std::sync::atomic::{self, AtomicBool};
use std::time::SystemTime;

// duplicate the doctest since coverage doesn't seem to handle doctests
#[test]
fn fixed_fraction_sample_test() {
    #[derive(Entry)]
    #[entry(rename_all = "PascalCase")]
    struct MyMetrics {
        #[entry(timestamp)]
        start: SystemTime,
        operation: String,
    }

    #[derive(Entry)]
    #[entry(rename_all = "PascalCase")]
    struct Globals {
        az: String,
    }

    struct MyFormatter {
        inner: FixedFractionSample<SampledEmf>,
        bypass_sampling: Arc<AtomicBool>,
    }

    impl Format for MyFormatter {
        fn format(
            &mut self,
            entry: &impl Entry,
            output: &mut impl io::Write,
        ) -> Result<(), IoStreamError> {
            if self.bypass_sampling.load(atomic::Ordering::Relaxed) {
                self.inner.format_mut().format(entry, output)
            } else {
                self.inner.format(entry, output)
            }
        }
    }

    let bypass_sampling = Arc::new(AtomicBool::new(false));
    let format = MyFormatter {
        // pick a very low fraction to see that this works
        inner: Emf::all_validations("MyApp".into(), vec![vec![]])
            .with_sampling()
            .sample_by_fixed_fraction(1.5e-38),
        bypass_sampling: bypass_sampling.clone(),
    };

    let globals = Globals {
        az: "us-east-1a".into(),
    };

    let mut output = vec![];
    let mut stream = format.output_to(&mut output).merge_globals(globals);

    // this is sampled with a probability and potentially dropped
    stream
        .next(&MyMetrics {
            start: SystemTime::UNIX_EPOCH, // use SystemTime::now() in the real world
            operation: "WillBePotentiallyDropped".to_string(),
        })
        .unwrap();

    // this bypasses sampling
    bypass_sampling.store(true, atomic::Ordering::Relaxed);
    stream
        .next(&MyMetrics {
            start: SystemTime::UNIX_EPOCH, // use SystemTime::now() in the real world
            operation: "WillRemain".to_string(),
        })
        .unwrap();

    let output = std::str::from_utf8(&output).unwrap();
    // Since the probability is 1e-100, we know WillBePotentiallyDropped will be dropped
    assert!(!output.contains("WillBePotentiallyDropped"));
    assert!(output.contains("WillRemain"));
}
