// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Case-conversion primitives shared by the metrique proc-macro crates.
//!
//! The `#[metrics]` and `derive(Entry)` macros parse their own attribute syntax
//! and map the result onto [`CaseStyle`], then call [`CaseStyle::apply`] (or
//! [`CaseStyle::apply_prefix`]) to perform the conversion.
//!
//! Names that already contain acronyms or digits can convert in surprising ways.
//! The exact output for those inputs is not considered part of the API contract:
//! it is chosen to match what metrique has historically emitted, and may change
//! in a future opt-in migration.

// How a name is split into words differs between `delimited` and `camel_like`,
// which is why some inputs convert oddly. Both agree on the easy cases:
//
//  * Non-alphanumeric characters (`_`, `-`, spaces, punctuation) always separate
//    words and are dropped.
//  * A numeric character always starts a new word, and does not itself end that
//    word, so `status_5xx` becomes `Status5Xx` rather than `Status5xx`.
//  * A lowercase-to-uppercase transition starts a new word.
//
// They disagree on runs of uppercase. `delimited` breaks before the last letter
// of a run (`FooBARBaz` -> `foo_bar_baz`), whereas `camel_like` keeps a run
// verbatim once it has seen a lowercase letter (`FooBARBaz` stays `FooBARBaz`,
// but `XMLHttpRequest` becomes `XmlhttpRequest`). See the tests at the bottom of
// this file for worked examples of each rule.

/// A case-conversion style applied to metric field and variant names.
///
/// This is the union of the styles accepted by the `#[metrics]` and
/// `derive(Entry)` macros. Each macro maps its own parsed attribute onto this
/// enum, so not every variant is reachable from every macro.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum CaseStyle {
    /// Keep the original name unchanged.
    Preserve,
    /// `lowercase`
    Lower,
    /// `UPPERCASE`
    Upper,
    /// `PascalCase`
    Pascal,
    /// `camelCase`
    Camel,
    /// `snake_case`
    Snake,
    /// `SCREAMING_SNAKE_CASE`
    ScreamingSnake,
    /// `kebab-case`
    Kebab,
    /// `SCREAMING-KEBAB-CASE`
    ScreamingKebab,
}

impl CaseStyle {
    /// Convert `name` to this case style.
    pub fn apply(self, name: &str) -> String {
        match self {
            CaseStyle::Preserve => name.to_string(),
            CaseStyle::Lower => name.to_ascii_lowercase(),
            CaseStyle::Upper => name.to_ascii_uppercase(),
            CaseStyle::Pascal => camel_like(name, Case::Upper),
            CaseStyle::Camel => camel_like(name, Case::Lower),
            CaseStyle::Snake => delimited(name, '_', Case::Lower),
            CaseStyle::ScreamingSnake => delimited(name, '_', Case::Upper),
            CaseStyle::Kebab => delimited(name, '-', Case::Lower),
            CaseStyle::ScreamingKebab => delimited(name, '-', Case::Lower).to_ascii_uppercase(),
        }
    }

    /// Convert `name` to this case style for use as a prefix, ensuring the
    /// result ends with the style's delimiter so it can be concatenated with a
    /// following name.
    ///
    /// Styles without a delimiter (`PascalCase`, `camelCase`, `lowercase`,
    /// `UPPERCASE`) and `Preserve` are returned unchanged from [`apply`].
    ///
    /// [`apply`]: CaseStyle::apply
    pub fn apply_prefix(self, name: &str) -> String {
        match self {
            CaseStyle::Snake | CaseStyle::ScreamingSnake => {
                let mut res = self.apply(name);
                if !res.ends_with('_') {
                    res.push('_');
                }
                res
            }
            CaseStyle::Kebab | CaseStyle::ScreamingKebab => {
                let mut res = self.apply(name);
                if !res.ends_with('-') {
                    res.push('-');
                }
                res
            }
            CaseStyle::Preserve
            | CaseStyle::Lower
            | CaseStyle::Upper
            | CaseStyle::Pascal
            | CaseStyle::Camel => self.apply(name),
        }
    }
}

/// Which case a converted character is folded to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Case {
    Lower,
    Upper,
}

/// Characters that separate words and are dropped from the output.
fn is_separator(c: char) -> bool {
    !c.is_alphanumeric()
}

fn push_cased(out: &mut String, c: char, case: Case) {
    match case {
        Case::Lower => out.extend(c.to_lowercase()),
        Case::Upper => out.extend(c.to_uppercase()),
    }
}

/// Convert to a delimiter-joined style (`snake_case`, `kebab-case`, and their
/// screaming variants).
// Word breaks are found by looking at each character's neighbors: a character
// that is not lowercase begins a new word when either neighbor is lowercase.
// That neighbor test is what keeps interior uppercase runs together, so
// `HTTPSConnection` splits as `https_connection` rather than `h_t_t_p_s_...`.
fn delimited(name: &str, delim: char, case: Case) -> String {
    // Trimming both ends leaves every separator between two words, so a
    // delimiter is always wanted once the following word arrives, and
    // `out.is_empty()` identifies exactly the first character.
    let name = name.trim_matches(is_separator);
    let mut out = String::with_capacity(name.len());
    // A run of separators contributes a single delimiter, emitted only when the
    // next word turns up.
    let mut pending_delim = false;
    let mut prev = None;
    let mut chars = name.chars().peekable();
    while let Some(c) = chars.next() {
        if is_separator(c) {
            pending_delim = true;
        } else {
            let neighbor_is_lowercase = prev.is_some_and(char::is_lowercase)
                || chars.peek().is_some_and(|next| next.is_lowercase());
            let starts_word = !out.is_empty() && !c.is_lowercase() && neighbor_is_lowercase;
            if pending_delim || starts_word {
                out.push(delim);
            }
            pending_delim = false;
            push_cased(&mut out, c, case);
        }
        prev = Some(c);
    }
    out
}

/// Convert to a delimiter-free style (`PascalCase`, `camelCase`).
// Unlike `delimited`, this only looks backwards. `last_lowered` holds the most
// recent character that was folded to lowercase, and a word break is detected
// when that character was *already* lowercase and the current one is uppercase.
// Characters capitalized as a word start deliberately do not update it, so once
// a lowercase letter has been seen every following uppercase letter starts a
// word and is preserved as-is. That is why `FooBARBaz` is left alone, while the
// leading uppercase run of `XMLHttpRequest` - never preceded by a lowercase
// letter - collapses to `Xmlhttp`.
fn camel_like(name: &str, first_word: Case) -> String {
    // Trimming both ends leaves every separator after the first character, and
    // every character that is kept pushes at least one character, so an empty
    // `out` means we are still on the first word.
    let name = name.trim_matches(is_separator);
    let mut out = String::with_capacity(name.len());
    // Pascal capitalizes immediately; camel lets the first character fall
    // through so an already-lowercase leading run is preserved verbatim.
    let mut new_word = first_word == Case::Upper;
    let mut last_lowered = ' ';
    for c in name.chars() {
        if is_separator(c) {
            new_word = true;
        } else if c.is_numeric() {
            // Digits pass through and leave the next character to start a word.
            out.push(c);
            new_word = true;
        } else if new_word || (last_lowered.is_lowercase() && c.is_uppercase()) {
            // Only the first word answers to `first_word`; later words always
            // start capitalized.
            let case = if out.is_empty() {
                first_word
            } else {
                Case::Upper
            };
            push_cased(&mut out, c, case);
            new_word = false;
        } else {
            last_lowered = c;
            push_cased(&mut out, c, Case::Lower);
        }
    }
    out
}

/// Returns the first character in `name` that cannot be inflected (anything
/// other than an alphanumeric, `_`, or `-`), if any.
pub fn name_contains_uninflectables(name: &str) -> Option<char> {
    name.chars()
        .find(|&c| !c.is_alphanumeric() && c != '_' && c != '-')
}

/// Returns `true` if `name` ends with a `_` or `-` delimiter.
pub fn name_ends_with_delimiter(name: &str) -> bool {
    let last = name.chars().last();
    last == Some('_') || last == Some('-')
}

// `.` is currently used in production, make it a warning instead of an error
/// Returns `true` if `name` contains a `.`.
pub fn name_contains_dot(name: &str) -> bool {
    name.contains('.')
}

#[cfg(test)]
mod tests {
    use super::CaseStyle::*;
    use super::*;

    #[track_caller]
    fn check(name: &str, expected: &[(CaseStyle, &str)]) {
        for &(style, want) in expected {
            assert_eq!(style.apply(name), want, "{style:?} of {name:?}");
        }
    }

    #[test]
    fn simple_words() {
        for input in [
            "fooBar", "FooBar", "foo-bar", "foo_bar", "FOO_BAR", "Foo bar", "Foo Bar", "Foo-Bar",
        ] {
            check(
                input,
                &[
                    (Camel, "fooBar"),
                    (Pascal, "FooBar"),
                    (Snake, "foo_bar"),
                    (ScreamingSnake, "FOO_BAR"),
                    (Kebab, "foo-bar"),
                    (ScreamingKebab, "FOO-BAR"),
                ],
            );
        }
    }

    #[test]
    fn preserve_lower_upper() {
        check(
            "fooBar",
            &[(Preserve, "fooBar"), (Lower, "foobar"), (Upper, "FOOBAR")],
        );
    }

    #[test]
    fn separators_are_dropped_and_collapsed() {
        check(
            "  __foo--_bar__  ",
            &[
                (Camel, "fooBar"),
                (Pascal, "FooBar"),
                (Snake, "foo_bar"),
                (Kebab, "foo-bar"),
            ],
        );
        // Trailing separators must not leave a dangling delimiter.
        check("foo_", &[(Snake, "foo"), (Kebab, "foo"), (Pascal, "Foo")]);
        check(
            "Random text with *(bad) chars",
            &[
                (Snake, "random_text_with_bad_chars"),
                (Pascal, "RandomTextWithBadChars"),
                (Camel, "randomTextWithBadChars"),
            ],
        );
    }

    #[test]
    fn empty_and_single_character() {
        check("", &[(Snake, ""), (Pascal, ""), (Camel, "")]);
        check("_", &[(Snake, ""), (Pascal, ""), (Camel, "")]);
        check("a", &[(Snake, "a"), (Pascal, "A"), (Camel, "a")]);
        check("A", &[(Snake, "a"), (Pascal, "A"), (Camel, "a")]);
    }

    #[test]
    fn uppercase_runs() {
        // The delimited styles split before the last letter of an uppercase
        // run, because that letter is the start of the following word.
        check(
            "XMLHttpRequest",
            &[
                (Snake, "xml_http_request"),
                (ScreamingSnake, "XML_HTTP_REQUEST"),
                (Kebab, "xml-http-request"),
                // Pascal and Camel instead fold the whole leading run.
                (Pascal, "XmlhttpRequest"),
                (Camel, "xmlhttpRequest"),
            ],
        );
        check(
            "HTTPSConnection",
            &[(Snake, "https_connection"), (Pascal, "Httpsconnection")],
        );
        // An uppercase run after a lowercase letter is left alone by Pascal and
        // Camel.
        check(
            "FooBARBaz",
            &[
                (Snake, "foo_bar_baz"),
                (Pascal, "FooBARBaz"),
                (Camel, "fooBARBaz"),
            ],
        );
        check(
            "myURLPath",
            &[(Snake, "my_url_path"), (Pascal, "MyURLPath")],
        );
        // A run with no following lowercase letter is never split.
        check("ABC", &[(Snake, "abc"), (Pascal, "Abc"), (Camel, "abc")]);
        check("ABCd", &[(Snake, "ab_cd"), (Pascal, "Abcd")]);
        check(
            "aBCD",
            &[(Snake, "a_bcd"), (Pascal, "Abcd"), (Camel, "aBCD")],
        );
        check(
            "A_B_C",
            &[(Snake, "a_b_c"), (Pascal, "ABC"), (Camel, "aBC")],
        );
    }

    #[test]
    fn digits_start_a_word() {
        check(
            "latency_p99",
            &[
                (Snake, "latency_p_99"),
                (ScreamingSnake, "LATENCY_P_99"),
                (Kebab, "latency-p-99"),
                (Pascal, "LatencyP99"),
                (Camel, "latencyP99"),
            ],
        );
        check(
            "s3_requests",
            &[(Snake, "s_3_requests"), (Pascal, "S3Requests")],
        );
        check(
            "utf8_string",
            &[(Snake, "utf_8_string"), (Pascal, "Utf8String")],
        );
        // A digit does not end the word it starts, so Pascal and Camel still
        // capitalize the letter after it.
        check(
            "status_5xx",
            &[(Snake, "status_5xx"), (Pascal, "Status5Xx")],
        );
        check(
            "concurrency_avg_10s",
            &[
                (Snake, "concurrency_avg_1_0s"),
                (Pascal, "ConcurrencyAvg10S"),
            ],
        );
        check(
            "e2e_us",
            &[(Snake, "e_2e_us"), (Pascal, "E2EUs"), (Camel, "e2EUs")],
        );
        // A leading digit consumes the "first word" slot, so camelCase
        // capitalizes what follows.
        check(
            "2foo",
            &[(Snake, "2foo"), (Pascal, "2Foo"), (Camel, "2Foo")],
        );
        // ...but only when a neighbor is lowercase, so `2` here is not a break.
        check("myURL2ABC", &[(Snake, "my_url2abc"), (Pascal, "MyURL2ABC")]);
    }

    #[test]
    fn apply_prefix_appends_delimiter() {
        assert_eq!(Snake.apply_prefix("fooBar"), "foo_bar_");
        assert_eq!(ScreamingSnake.apply_prefix("fooBar"), "FOO_BAR_");
        assert_eq!(Kebab.apply_prefix("fooBar"), "foo-bar-");
        assert_eq!(ScreamingKebab.apply_prefix("fooBar"), "FOO-BAR-");
        // Already-delimited results are not doubled up.
        assert_eq!(Snake.apply_prefix("foo_"), "foo_");
        // Delimiter-free styles are unchanged.
        assert_eq!(Pascal.apply_prefix("fooBar"), "FooBar");
        assert_eq!(Camel.apply_prefix("fooBar"), "fooBar");
        assert_eq!(Preserve.apply_prefix("foo_"), "foo_");
    }

    #[test]
    fn validation_helpers() {
        assert_eq!(name_contains_uninflectables("foo_bar-1"), None);
        assert_eq!(name_contains_uninflectables("foo.bar"), Some('.'));
        assert_eq!(name_contains_uninflectables("foo bar"), Some(' '));
        assert!(name_ends_with_delimiter("foo_"));
        assert!(name_ends_with_delimiter("foo-"));
        assert!(!name_ends_with_delimiter("foo"));
        assert!(!name_ends_with_delimiter(""));
        assert!(name_contains_dot("foo.bar"));
        assert!(!name_contains_dot("foo_bar"));
    }
}
