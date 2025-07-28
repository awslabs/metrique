// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

pub(crate) trait JsonString {
    fn json_string(&mut self, value: &str) -> &mut Self;
}

impl JsonString for String {
    fn json_string(&mut self, value: &str) -> &mut Self {
        unsafe {
            // XX: find crate that doesn't require the alloc/copy or pull out format_escaped_str_contents()
            // Safety: `as_mut_vec` is safe as long as only UTF-8 is written to the `Vec`, and JSON is
            // always valid UTF-8.
            serde_json::to_writer(self.as_mut_vec(), value).ok();
        }
        self
    }
}

#[test]
fn test_json_string() {
    let mut s = "\"x\",".to_string();
    s.json_string("\u{3b1}\"\u{00}\u{0e}");
    assert_eq!(s, "\"x\",\"\u{3b1}\\\"\\u0000\\u000e\"");
}
