//! Ordered whitespace normalization for `charset_v1`.

/// Apply the exact RFC whitespace sub-order.
#[must_use]
pub fn normalize_whitespace(input: &str) -> String {
    let line_normalized = normalize_line_endings(input);
    let tabs_normalized = tabs_to_spaces(&line_normalized);
    let trailing_trimmed = trim_trailing_ascii_spaces(&tabs_normalized);
    collapse_internal_ascii_spaces(&trailing_trimmed)
}

/// CRLF/CR to LF.
#[must_use]
pub fn normalize_line_endings(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\r' {
            if chars.peek() == Some(&'\n') {
                chars.next();
            }
            out.push('\n');
        } else {
            out.push(ch);
        }
    }
    out
}

/// Tab to ASCII space.
#[must_use]
pub fn tabs_to_spaces(input: &str) -> String {
    input.replace('\t', " ")
}

/// Trim trailing ASCII spaces before LF and at end of example.
#[must_use]
pub fn trim_trailing_ascii_spaces(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for segment in input.split_inclusive('\n') {
        if let Some(line) = segment.strip_suffix('\n') {
            out.push_str(line.trim_end_matches(' '));
            out.push('\n');
        } else {
            out.push_str(segment.trim_end_matches(' '));
        }
    }
    out
}

/// Collapse runs of two or more ASCII spaces to one while preserving LF.
#[must_use]
pub fn collapse_internal_ascii_spaces(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut previous_was_space = false;
    for ch in input.chars() {
        if ch == ' ' {
            if !previous_was_space {
                out.push(ch);
            }
            previous_was_space = true;
        } else {
            out.push(ch);
            previous_was_space = false;
        }
    }
    out
}
