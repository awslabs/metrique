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
    // 1–4 entries to format through the same formatter instance.
    let entry_count = match u.arbitrary::<u8>() {
        Ok(n) => (n % 4) as usize + 1,
        Err(_) => return,
    };
    let mut entries = Vec::with_capacity(entry_count);
    for _ in 0..entry_count {
        let Ok(entry) = u.arbitrary::<FuzzEntry>() else {
            return;
        };
        entries.push(entry);
    }

    // Regular (non-sampled) path — format all entries through the same formatter.
    // We don't care if formatting returns a validation error, but it must never panic.
    let mut format = Json::new();
    let mut output = Vec::new();
    for (i, entry) in entries.iter().enumerate() {
        output.clear();
        let result = format.format(entry, &mut output);
        if let Ok(()) = result {
            assert_valid_json_line(&output, &format!("entry {i}"));
        }
    }

    // Sampled path, same entries, fresh formatter.
    let mut sampled = Json::new().with_sampling();
    for (i, entry) in entries.iter().enumerate() {
        let Ok(rate) = arbitrary_sample_rate(&mut u) else {
            return;
        };
        output.clear();
        let result = sampled.format_with_sample_rate(entry, &mut output, rate);
        if let Ok(()) = result {
            assert_valid_json_line(&output, &format!("sampled entry {i}"));
        }
    }
});
