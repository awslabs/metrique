//! Fuzz target for the pure JSON formatter.
//!
//! Invariants tested:
//! - Successful formatting always produces exactly one valid, newline-terminated JSON object.
//! - Formatter state reuse across entries does not corrupt output.
//! - Both regular and sampled paths are exercised.

#![no_main]

mod fuzz_entry;

use arbitrary::Unstructured;
use libfuzzer_sys::fuzz_target;

use metrique_writer_core::format::Format;
use metrique_writer_core::sample::SampledFormat;
use metrique_writer_format_json::Json;

use fuzz_entry::{FuzzEntry, arbitrary_sample_rate};

/// Assert that output is exactly one newline-terminated JSON object.
///
/// The JSON formatter documents single-line output: one JSON object followed by `\n`.
fn assert_valid_json_line(output: &[u8], context: &str) {
    assert!(
        output.ends_with(b"\n"),
        "JSON output must end with newline ({context}): {:?}",
        String::from_utf8_lossy(output),
    );

    // Strip the trailing newline; the remainder must contain no newlines.
    let body = &output[..output.len() - 1];
    assert!(
        !body.contains(&b'\n'),
        "JSON output must be a single line ({context}): {:?}",
        String::from_utf8_lossy(output),
    );

    let parsed = serde_json::from_slice::<serde_json::Value>(body).unwrap_or_else(|_| {
        panic!(
            "JSON formatter produced invalid JSON ({context}): {}",
            String::from_utf8_lossy(output),
        )
    });
    assert!(
        parsed.is_object(),
        "JSON formatter produced non-object JSON ({context}): {}",
        String::from_utf8_lossy(output),
    );
}

fuzz_target!(|data: &[u8]| {
    let mut u = Unstructured::new(data);
    let Ok((entry_a, entry_b)) =
        u.arbitrary::<(FuzzEntry, FuzzEntry)>()
    else {
        return;
    };

    // Regular (non-sampled) path
    let mut format = Json::new();
    let mut output = Vec::new();

    // Format the entry, we don't care if it returns a validation error,
    // but it must never panic.
    let result = format.format(&entry_a, &mut output);

    if let Ok(()) = result {
        assert_valid_json_line(&output, "first call");
    }

    // Format a different entry through the same formatter to test state reuse.
    output.clear();
    let result = format.format(&entry_b, &mut output);
    if let Ok(()) = result {
        assert_valid_json_line(&output, "state reuse call");
    }

    // Sampled path
    let Ok(rate_a) = arbitrary_sample_rate(&mut u) else {
        return;
    };
    let Ok(rate_b) = arbitrary_sample_rate(&mut u) else {
        return;
    };

    let mut sampled = Json::new().with_sampling();
    output.clear();
    let result = sampled.format_with_sample_rate(&entry_a, &mut output, rate_a);
    if let Ok(()) = result {
        assert_valid_json_line(&output, "sampled first call");
    }

    output.clear();
    let result = sampled.format_with_sample_rate(&entry_b, &mut output, rate_b);
    if let Ok(()) = result {
        assert_valid_json_line(&output, "sampled state reuse call");
    }
});
