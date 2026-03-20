use rstest::rstest;
use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;

/// Validate that markdown files don't contain link patterns that break in certain
/// rendering contexts (GitHub, crates.io, rustdoc).
#[rstest]
fn test_no_broken_link_patterns(
    #[files("../README.md")]
    #[files("../docs/*.md")]
    #[files("../metrique*/**/README.md")]
    #[files("README.md")]
    #[files("docs/*.md")]
    md_path: PathBuf,
) {
    let content = fs::read_to_string(&md_path)
        .unwrap_or_else(|e| panic!("Failed to read {}: {}", md_path.display(), e));
    let display = md_path.display();

    let mut errors: Vec<String> = Vec::new();

    let non_code_lines = strip_code_blocks(&content);

    // Check for Rust-style paths (foo::bar) outside code blocks and inline
    // code spans. These are rustdoc intra-doc links that only resolve in .rs
    // files, not on GitHub or crates.io.
    for (line_num, line) in &non_code_lines {
        let stripped = strip_inline_code(line);
        if stripped.contains("::") {
            errors.push(format!(
                "  line {line_num}: contains `::` path (use docs.rs URL instead)"
            ));
        }
    }

    // Check for hardcoded version paths like /0.1.22/ in docs.rs URLs.
    // These should use /latest/ instead.
    for (line_num, line) in &non_code_lines {
        if let Some(pos) = line.find("docs.rs/") {
            let after = &line[pos..];
            // Look for /0.DIGITS.DIGITS/ pattern
            if regex_lite::Regex::new(r"/\d+\.\d+\.\d+/")
                .unwrap()
                .is_match(after)
            {
                errors.push(format!(
                    "  line {line_num}: hardcoded version in docs.rs URL (use /latest/ instead)"
                ));
            }
        }
    }

    // Check for URL typos (htps://, htpp://, etc.)
    for (line_num, line) in &non_code_lines {
        for typo in ["htps://", "htpp://", "htp://"] {
            if line.contains(typo) {
                errors.push(format!("  line {line_num}: URL typo `{typo}`"));
            }
        }
    }

    // Check for inline links [text](url). All links should use reference-style
    // definitions for readability and easier auditing.
    let inline_link_re = regex_lite::Regex::new(r"\[[^\]]+\]\(https?://[^)]+\)").unwrap();
    for (line_num, line) in &non_code_lines {
        if let Some(m) = inline_link_re.find(line) {
            errors.push(format!(
                "  line {line_num}: inline link `{}` (use a reference link definition instead)",
                m.as_str()
            ));
        }
    }

    // Check for undefined reference links.
    // Collect all link definitions: [label]: URL
    let definitions: HashSet<String> = non_code_lines
        .iter()
        .filter_map(|(_, line)| {
            let trimmed = line.trim();
            if trimmed.starts_with('[') {
                // Match [label]: URL pattern
                if let Some(bracket_end) = trimmed.find("]: ") {
                    let label = &trimmed[1..bracket_end];
                    return Some(label.to_lowercase());
                }
            }
            None
        })
        .collect();

    // Collect all reference link usages: [text] or [text][label]
    // Skip lines that are definitions themselves
    let link_ref_re = regex_lite::Regex::new(r"\[([^\]\[]+)\](?:\[([^\]]*)\])?").unwrap();
    let inline_code_re = regex_lite::Regex::new(r"`[^`]+`").unwrap();
    for (line_num, line) in &non_code_lines {
        let trimmed = line.trim();
        // Skip definition lines
        if trimmed.starts_with('[') && trimmed.contains("]: ") {
            continue;
        }
        // Find byte ranges of inline code spans so we can skip brackets inside them
        let code_ranges: Vec<(usize, usize)> = inline_code_re
            .find_iter(line)
            .map(|m| (m.start(), m.end()))
            .collect();

        for cap in link_ref_re.captures_iter(line) {
            let m = cap.get(0).unwrap();
            let bracket_pos = m.start();

            // Skip if the opening [ falls inside an inline code span
            // (e.g., `#[metrics]`), but allow [`Foo`] where the backticks
            // are inside the brackets
            if code_ranges
                .iter()
                .any(|&(start, end)| start < bracket_pos && bracket_pos < end)
            {
                continue;
            }

            // Skip if preceded by ! (image link) or [ (nested badge link)
            if bracket_pos > 0 {
                let prev = line.as_bytes()[bracket_pos - 1];
                if prev == b'!' || prev == b'[' {
                    continue;
                }
            }
            // Skip if this is an inline link [text](url)
            let end = m.end();
            if line.len() > end && line.as_bytes()[end] == b'(' {
                continue;
            }

            // Determine the label used for lookup
            let label = if let Some(explicit) = cap.get(2) {
                let s = explicit.as_str();
                if s.is_empty() {
                    // [text][] form: label is the text
                    cap.get(1).unwrap().as_str()
                } else {
                    s
                }
            } else {
                // [text] form: label is the text
                cap.get(1).unwrap().as_str()
            };

            // Skip things that are clearly not reference links:
            // - footnotes like [^1]
            // - checkbox items like [x] or [ ]
            if label.starts_with('^') || label == "x" || label == " " || label == "X" {
                continue;
            }

            // Skip labels that look like they're part of markdown structure
            // (e.g., header anchors) rather than reference links
            if label.starts_with('#') {
                continue;
            }

            if !definitions.contains(&label.to_lowercase()) {
                errors.push(format!(
                    "  line {line_num}: reference `[{label}]` has no link definition"
                ));
            }
        }
    }

    assert!(
        errors.is_empty(),
        "Doc link issues in {display}:\n{}",
        errors.join("\n")
    );
}

/// Strip fenced code blocks from content, returning (line_number, line) pairs
/// for lines that are outside code blocks.
fn strip_code_blocks(content: &str) -> Vec<(usize, String)> {
    let mut result = Vec::new();
    let mut in_code_block = false;

    for (i, line) in content.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            in_code_block = !in_code_block;
            continue;
        }
        if !in_code_block {
            result.push((i + 1, line.to_string()));
        }
    }
    result
}

/// Remove inline code spans (backtick-delimited) from a line, replacing them
/// with empty strings so we don't match patterns inside code.
fn strip_inline_code(line: &str) -> String {
    let re = regex_lite::Regex::new(r"`[^`]+`").unwrap();
    re.replace_all(line, "").to_string()
}
