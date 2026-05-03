//! Stringify implementation — `OdinDocument` to ODIN text.

use std::fmt::Write;

use crate::types::document::OdinDocument;
use crate::types::options::StringifyOptions;
use crate::types::values::{OdinValue, OdinModifiers, OdinDirective, OdinArrayItem};

/// Serialize an `OdinDocument` to ODIN text.
pub fn stringify(doc: &OdinDocument, options: Option<&StringifyOptions>) -> String {
    let opts = options.cloned().unwrap_or_default();
    let estimated = doc.assignments.len() * 32 + doc.metadata.len() * 32 + 64;
    let mut output = String::with_capacity(estimated);

    // Metadata section
    if opts.include_metadata && !doc.metadata.is_empty() {
        if opts.canonical {
            // Canonical mode: metadata is stored in assignments as $.key already
            // Don't emit {$} section — assignments will be emitted in sorted order below
        } else {
            output.push_str("{$}\n");
            for (key, value) in &doc.metadata {
                output.push_str(key);
                output.push_str(" = ");
                write_value(&mut output, value, false);
                output.push('\n');
            }
            output.push('\n');
        }
    }

    let canonical_meta = opts.canonical && !doc.metadata.is_empty();

    if !opts.sort_keys && !canonical_meta {
        // Stream directly — no Vec, no sort.
        let mut current_section: Option<&str> = None;
        for (path, value) in &doc.assignments {
            write_entry(&mut output, path, value, &mut current_section, opts.canonical);
        }
        return output;
    }

    let mut meta_keys: Vec<(String, &OdinValue)> = Vec::new();
    if canonical_meta {
        for (key, value) in &doc.metadata {
            meta_keys.push((format!("$.{key}"), value));
        }
    }
    let mut entries: Vec<(&String, &OdinValue)> = doc.assignments.iter().collect();
    for (key, value) in &meta_keys {
        entries.push((key, value));
    }
    if opts.sort_keys {
        if opts.canonical {
            entries.sort_by_cached_key(|(path, _)| canonical_sort_key(path));
        } else {
            entries.sort_by(|(a, _), (b, _)| a.cmp(b));
        }
    }

    let mut current_section: Option<&str> = None;
    for (path, value) in &entries {
        write_entry(&mut output, path, value, &mut current_section, opts.canonical);
    }

    output
}

fn write_entry<'a>(
    output: &mut String,
    path: &'a str,
    value: &OdinValue,
    current_section: &mut Option<&'a str>,
    canonical: bool,
) {
    let (section, field) = split_path(path);
    if section != *current_section {
        if let Some(sec) = section {
            if !output.is_empty() && !output.ends_with('\n') {
                output.push('\n');
            }
            output.push('{');
            output.push_str(sec);
            output.push_str("}\n");
        }
        *current_section = section;
    }
    output.push_str(field);
    output.push_str(" = ");
    if let Some(mods) = value.modifiers() {
        write_modifiers(output, mods);
    }
    write_value(output, value, canonical);
    output.push('\n');
}

/// Split a path into (section, field) parts — zero allocations.
/// e.g., "Policy.number" -> (Some("Policy"), "number")
/// e.g., "name" -> (None, "name")
fn split_path<'a>(path: &'a str) -> (Option<&'a str>, &'a str) {
    if let Some(dot_pos) = path.find('.') {
        let section = &path[..dot_pos];
        let field = &path[dot_pos + 1..];
        if section.as_bytes().first().is_some_and(|&b| b.is_ascii_uppercase()) {
            (Some(section), field)
        } else {
            (None, path)
        }
    } else {
        (None, path)
    }
}

fn write_modifiers(output: &mut String, mods: &OdinModifiers) {
    // Canonical order: ! (required), * (confidential), - (deprecated)
    if mods.required {
        output.push('!');
    }
    if mods.confidential {
        output.push('*');
    }
    if mods.deprecated {
        output.push('-');
    }
}

fn write_value(output: &mut String, value: &OdinValue, canonical: bool) {
    match value {
        OdinValue::Null { .. } => output.push('~'),
        OdinValue::Boolean { value, .. } => {
            output.push_str(if *value { "true" } else { "false" });
        }
        OdinValue::String { value, .. } => {
            output.push('"');
            write_escaped_string(output, value);
            output.push('"');
        }
        OdinValue::Integer { value, raw, .. } => {
            output.push_str("##");
            if let Some(r) = raw {
                output.push_str(r);
            } else {
                output.push_str(&value.to_string());
            }
        }
        OdinValue::Number { value, raw, decimal_places, .. } => {
            output.push('#');
            if canonical {
                // Canonical: strip trailing zeros, don't use raw
                write_canonical_number(output, *value);
            } else if let Some(r) = raw {
                output.push_str(r);
            } else if let Some(dp) = decimal_places {
                let _ = write!(output, "{value:.prec$}", prec = *dp as usize);
            } else {
                output.push_str(&value.to_string());
            }
        }
        OdinValue::Currency { value, raw, decimal_places, currency_code, .. } => {
            output.push_str("#$");
            if canonical {
                // Canonical: always at least 2 decimal places, don't use raw
                let dp = (*decimal_places as usize).max(2);
                let _ = write!(output, "{value:.prec$}", prec = dp);
                if let Some(code) = currency_code {
                    output.push(':');
                    output.push_str(&code.to_uppercase());
                }
            } else if let Some(r) = raw {
                // Raw already contains numeric part and currency code if present,
                // but we need to uppercase the currency code in canonical form
                if let Some(colon_pos) = r.find(':') {
                    output.push_str(&r[..colon_pos]);
                    output.push(':');
                    output.push_str(&r[colon_pos + 1..].to_uppercase());
                } else {
                    output.push_str(r);
                    if let Some(code) = currency_code {
                        output.push(':');
                        output.push_str(code);
                    }
                }
            } else {
                let _ = write!(output, "{value:.prec$}", prec = *decimal_places as usize);
                if let Some(code) = currency_code {
                    output.push(':');
                    output.push_str(code);
                }
            }
        }
        OdinValue::Percent { value, raw, .. } => {
            output.push_str("#%");
            if let Some(r) = raw {
                output.push_str(r);
            } else {
                output.push_str(&value.to_string());
            }
        }
        OdinValue::Date { raw, .. } | OdinValue::Timestamp { raw, .. } => output.push_str(raw),
        OdinValue::Time { value, .. } | OdinValue::Duration { value, .. } => output.push_str(value),
        OdinValue::Reference { path, .. } => {
            output.push('@');
            output.push_str(path);
        }
        OdinValue::Binary { data, algorithm, .. } => {
            output.push('^');
            if let Some(alg) = algorithm {
                output.push_str(alg);
                output.push(':');
            }
            base64_encode_into(data, output);
        }
        OdinValue::Verb { verb, is_custom, args, .. } => {
            output.push('%');
            if *is_custom {
                output.push('&');
            }
            output.push_str(verb);
            for arg in args {
                output.push(' ');
                write_value(output, arg, canonical);
            }
        }
        OdinValue::Array { items, .. } => {
            // Arrays are typically represented via indexed paths in ODIN
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    output.push_str(", ");
                }
                match item {
                    OdinArrayItem::Value(v) => write_value(output, v, canonical),
                    OdinArrayItem::Record(_) => output.push_str("{...}"),
                }
            }
        }
        OdinValue::Object { value, .. } => {
            output.push_str("{...}");
            let _ = value;
        }
    }

    // Write directives
    for directive in value.directives() {
        write_directive(output, directive);
    }
}

fn write_directive(output: &mut String, directive: &OdinDirective) {
    output.push_str(" :");
    output.push_str(&directive.name);
    if let Some(ref val) = directive.value {
        output.push(' ');
        match val {
            crate::types::values::DirectiveValue::String(s) => output.push_str(s),
            crate::types::values::DirectiveValue::Number(n) => output.push_str(&n.to_string()),
        }
    }
}

fn write_escaped_string(output: &mut String, s: &str) {
    // Fast path: if no special chars, push the whole slice at once
    let bytes = s.as_bytes();
    if !bytes.iter().any(|&b| b == b'"' || b == b'\\' || b < 0x20) {
        output.push_str(s);
        return;
    }
    // Slow path: scan for runs of safe chars and copy them in bulk
    let mut last = 0;
    for (i, ch) in s.char_indices() {
        let escape = match ch {
            '"' => Some("\\\""),
            '\\' => Some("\\\\"),
            '\n' => Some("\\n"),
            '\r' => Some("\\r"),
            '\t' => Some("\\t"),
            c if c.is_control() => {
                output.push_str(&s[last..i]);
                let _ = write!(output, "\\u{:04x}", c as u32);
                last = i + ch.len_utf8();
                continue;
            }
            _ => None,
        };
        if let Some(esc) = escape {
            output.push_str(&s[last..i]);
            output.push_str(esc);
            last = i + ch.len_utf8();
        }
    }
    output.push_str(&s[last..]);
}

/// Simple base64 encoder. The `_into` variant writes directly into a caller's
/// buffer to avoid an intermediate `String` allocation in the hot path.
#[cfg(test)]
fn base64_encode(data: &[u8]) -> String {
    let mut s = String::with_capacity(data.len().div_ceil(3) * 4);
    base64_encode_into(data, &mut s);
    s
}

fn base64_encode_into(data: &[u8], output: &mut String) {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    output.reserve(data.len().div_ceil(3) * 4);
    let mut i = 0;

    while i + 2 < data.len() {
        let n = (u32::from(data[i]) << 16) | (u32::from(data[i + 1]) << 8) | u32::from(data[i + 2]);
        output.push(ALPHABET[(n >> 18) as usize & 0x3F] as char);
        output.push(ALPHABET[(n >> 12) as usize & 0x3F] as char);
        output.push(ALPHABET[(n >> 6) as usize & 0x3F] as char);
        output.push(ALPHABET[n as usize & 0x3F] as char);
        i += 3;
    }

    match data.len() - i {
        1 => {
            let n = u32::from(data[i]) << 16;
            output.push(ALPHABET[(n >> 18) as usize & 0x3F] as char);
            output.push(ALPHABET[(n >> 12) as usize & 0x3F] as char);
            output.push('=');
            output.push('=');
        }
        2 => {
            let n = (u32::from(data[i]) << 16) | (u32::from(data[i + 1]) << 8);
            output.push(ALPHABET[(n >> 18) as usize & 0x3F] as char);
            output.push(ALPHABET[(n >> 12) as usize & 0x3F] as char);
            output.push(ALPHABET[(n >> 6) as usize & 0x3F] as char);
            output.push('=');
        }
        _ => {}
    }
}

/// Write a number in canonical form directly to the output buffer — zero intermediate allocations.
fn write_canonical_number(output: &mut String, value: f64) {
    if value.fract() == 0.0 {
        let _ = write!(output, "{value}");
    } else {
        // Write with high precision, then strip trailing zeros in-place
        let start = output.len();
        let _ = write!(output, "{value:.15}");
        let trimmed_len = output[start..].trim_end_matches('0').trim_end_matches('.').len();
        output.truncate(start + trimmed_len);
    }
}

/// Sort key for a path; numeric for `[N]` segments, lexicographic otherwise.
fn canonical_sort_key(path: &str) -> Vec<SegmentKey<'_>> {
    PathSegmentIter::new(path)
        .map(|seg| {
            let numeric = if seg.starts_with('[') && seg.ends_with(']') {
                seg[1..seg.len() - 1].parse::<usize>().ok()
            } else {
                None
            };
            SegmentKey { text: seg, numeric }
        })
        .collect()
}

#[derive(Eq, PartialEq)]
struct SegmentKey<'a> {
    text: &'a str,
    numeric: Option<usize>,
}

impl<'a> Ord for SegmentKey<'a> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        match (self.numeric, other.numeric) {
            (Some(a), Some(b)) => a.cmp(&b),
            _ => self.text.cmp(other.text),
        }
    }
}

impl<'a> PartialOrd for SegmentKey<'a> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// Zero-allocation path segment iterator.
/// "items[2].name" yields "items", "[2]", "name" as &str slices.
struct PathSegmentIter<'a> {
    path: &'a str,
    pos: usize,
}

impl<'a> PathSegmentIter<'a> {
    fn new(path: &'a str) -> Self {
        Self { path, pos: 0 }
    }
}

impl<'a> Iterator for PathSegmentIter<'a> {
    type Item = &'a str;

    fn next(&mut self) -> Option<&'a str> {
        let bytes = self.path.as_bytes();
        if self.pos >= bytes.len() {
            return None;
        }
        // Skip leading dot
        if bytes[self.pos] == b'.' {
            self.pos += 1;
            if self.pos >= bytes.len() {
                return None;
            }
        }
        let start = self.pos;
        if bytes[start] == b'[' {
            // Array index segment: consume until ']'
            while self.pos < bytes.len() && bytes[self.pos] != b']' {
                self.pos += 1;
            }
            if self.pos < bytes.len() {
                self.pos += 1; // consume ']'
            }
            Some(&self.path[start..self.pos])
        } else {
            // Regular segment: consume until '.' or '['
            while self.pos < bytes.len() && bytes[self.pos] != b'.' && bytes[self.pos] != b'[' {
                self.pos += 1;
            }
            Some(&self.path[start..self.pos])
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::document::OdinDocument;
    use crate::types::options::StringifyOptions;
    use crate::types::values::{OdinValues, OdinModifiers, OdinDirective, DirectiveValue, OdinArrayItem};
    use crate::OdinDocumentBuilder;

    // ── Helper ──────────────────────────────────────────────────────────────

    fn default_opts() -> StringifyOptions {
        StringifyOptions::default()
    }

    fn opts_with_sort() -> StringifyOptions {
        StringifyOptions { sort_keys: true, ..Default::default() }
    }

    fn opts_no_metadata() -> StringifyOptions {
        StringifyOptions { include_metadata: false, ..Default::default() }
    }

    // ── Empty / minimal documents ───────────────────────────────────────────

    #[test]
    fn empty_document_produces_empty_string() {
        let doc = OdinDocument::empty();
        let out = stringify(&doc, None);
        assert_eq!(out, "");
    }

    #[test]
    fn document_with_only_metadata() {
        let doc = OdinDocumentBuilder::new()
            .metadata("odin", OdinValues::string("1.0.0"))
            .build()
            .unwrap();
        let out = stringify(&doc, Some(&default_opts()));
        assert!(out.contains("{$}"));
        assert!(out.contains("odin = \"1.0.0\""));
    }

    #[test]
    fn metadata_excluded_when_option_false() {
        let doc = OdinDocumentBuilder::new()
            .metadata("odin", OdinValues::string("1.0.0"))
            .set("x", OdinValues::integer(1))
            .build()
            .unwrap();
        let out = stringify(&doc, Some(&opts_no_metadata()));
        assert!(!out.contains("{$}"));
        assert!(out.contains("x = ##1"));
    }

    // ── String values ───────────────────────────────────────────────────────

    #[test]
    fn simple_string() {
        let doc = OdinDocumentBuilder::new()
            .set("name", OdinValues::string("Alice"))
            .build()
            .unwrap();
        let out = stringify(&doc, None);
        assert!(out.contains("name = \"Alice\""));
    }

    #[test]
    fn string_with_newline_escape() {
        let doc = OdinDocumentBuilder::new()
            .set("msg", OdinValues::string("line1\nline2"))
            .build()
            .unwrap();
        let out = stringify(&doc, None);
        assert!(out.contains(r#""line1\nline2""#));
    }

    #[test]
    fn string_with_tab_escape() {
        let doc = OdinDocumentBuilder::new()
            .set("msg", OdinValues::string("col1\tcol2"))
            .build()
            .unwrap();
        let out = stringify(&doc, None);
        assert!(out.contains(r#""col1\tcol2""#));
    }

    #[test]
    fn string_with_backslash_escape() {
        let doc = OdinDocumentBuilder::new()
            .set("path", OdinValues::string("C:\\Users\\test"))
            .build()
            .unwrap();
        let out = stringify(&doc, None);
        assert!(out.contains(r#""C:\\Users\\test""#));
    }

    #[test]
    fn string_with_quote_escape() {
        let doc = OdinDocumentBuilder::new()
            .set("quote", OdinValues::string("She said \"hello\""))
            .build()
            .unwrap();
        let out = stringify(&doc, None);
        assert!(out.contains(r#""She said \"hello\"""#));
    }

    #[test]
    fn string_with_carriage_return() {
        let doc = OdinDocumentBuilder::new()
            .set("cr", OdinValues::string("a\rb"))
            .build()
            .unwrap();
        let out = stringify(&doc, None);
        assert!(out.contains(r#""a\rb""#));
    }

    #[test]
    fn string_with_all_escapes_combined() {
        let doc = OdinDocumentBuilder::new()
            .set("mixed", OdinValues::string("a\nb\\c\"d\te"))
            .build()
            .unwrap();
        let out = stringify(&doc, None);
        assert!(out.contains(r#""a\nb\\c\"d\te""#));
    }

    #[test]
    fn empty_string_value() {
        let doc = OdinDocumentBuilder::new()
            .set("empty", OdinValues::string(""))
            .build()
            .unwrap();
        let out = stringify(&doc, None);
        assert!(out.contains("empty = \"\""));
    }

    #[test]
    fn string_with_unicode() {
        let doc = OdinDocumentBuilder::new()
            .set("emoji", OdinValues::string("Hello World"))
            .build()
            .unwrap();
        let out = stringify(&doc, None);
        assert!(out.contains("\"Hello World\""));
    }

    #[test]
    fn string_with_control_char_uses_unicode_escape() {
        let doc = OdinDocumentBuilder::new()
            .set("ctrl", OdinValues::string("a\x01b"))
            .build()
            .unwrap();
        let out = stringify(&doc, None);
        assert!(out.contains(r"\u0001"));
    }

    // ── Integer values ──────────────────────────────────────────────────────

    #[test]
    fn positive_integer() {
        let doc = OdinDocumentBuilder::new()
            .set("age", OdinValues::integer(30))
            .build()
            .unwrap();
        let out = stringify(&doc, None);
        assert!(out.contains("age = ##30"));
    }

    #[test]
    fn negative_integer() {
        let doc = OdinDocumentBuilder::new()
            .set("temp", OdinValues::integer(-10))
            .build()
            .unwrap();
        let out = stringify(&doc, None);
        assert!(out.contains("temp = ##-10"));
    }

    #[test]
    fn zero_integer() {
        let doc = OdinDocumentBuilder::new()
            .set("zero", OdinValues::integer(0))
            .build()
            .unwrap();
        let out = stringify(&doc, None);
        assert!(out.contains("zero = ##0"));
    }

    #[test]
    fn large_integer() {
        let doc = OdinDocumentBuilder::new()
            .set("big", OdinValues::integer(9_999_999_999))
            .build()
            .unwrap();
        let out = stringify(&doc, None);
        assert!(out.contains("big = ##9999999999"));
    }

    #[test]
    fn integer_with_raw_string() {
        let doc = OdinDocumentBuilder::new()
            .set("raw", OdinValues::integer_from_str("12345"))
            .build()
            .unwrap();
        let out = stringify(&doc, None);
        assert!(out.contains("raw = ##12345"));
    }

    // ── Number values ───────────────────────────────────────────────────────

    #[test]
    fn simple_number() {
        let doc = OdinDocumentBuilder::new()
            .set("pi", OdinValues::number(3.14))
            .build()
            .unwrap();
        let out = stringify(&doc, None);
        assert!(out.contains("pi = #3.14"));
    }

    #[test]
    fn number_with_decimal_places() {
        let doc = OdinDocumentBuilder::new()
            .set("val", OdinValues::number_with_places(1.5, 3))
            .build()
            .unwrap();
        let out = stringify(&doc, None);
        assert!(out.contains("val = #1.500"));
    }

    #[test]
    fn negative_number() {
        let doc = OdinDocumentBuilder::new()
            .set("neg", OdinValues::number(-42.5))
            .build()
            .unwrap();
        let out = stringify(&doc, None);
        assert!(out.contains("neg = #-42.5"));
    }

    // ── Currency values ─────────────────────────────────────────────────────

    #[test]
    fn simple_currency() {
        let doc = OdinDocumentBuilder::new()
            .set("price", OdinValues::currency(99.99, 2))
            .build()
            .unwrap();
        let out = stringify(&doc, None);
        assert!(out.contains("price = #$99.99"));
    }

    #[test]
    fn currency_with_code() {
        let doc = OdinDocumentBuilder::new()
            .set("total", OdinValues::currency_with_code(1234.56, 2, "USD"))
            .build()
            .unwrap();
        let out = stringify(&doc, None);
        assert!(out.contains("total = #$1234.56:USD"));
    }

    #[test]
    fn currency_zero() {
        let doc = OdinDocumentBuilder::new()
            .set("free", OdinValues::currency(0.0, 2))
            .build()
            .unwrap();
        let out = stringify(&doc, None);
        assert!(out.contains("free = #$0.00"));
    }

    // ── Percent values ──────────────────────────────────────────────────────

    #[test]
    fn percent_value() {
        let doc = OdinDocumentBuilder::new()
            .set("rate", OdinValues::percent(0.15))
            .build()
            .unwrap();
        let out = stringify(&doc, None);
        assert!(out.contains("rate = #%0.15"));
    }

    // ── Boolean values ──────────────────────────────────────────────────────

    #[test]
    fn boolean_true() {
        let doc = OdinDocumentBuilder::new()
            .set("active", OdinValues::boolean(true))
            .build()
            .unwrap();
        let out = stringify(&doc, None);
        assert!(out.contains("active = true"));
    }

    #[test]
    fn boolean_false() {
        let doc = OdinDocumentBuilder::new()
            .set("deleted", OdinValues::boolean(false))
            .build()
            .unwrap();
        let out = stringify(&doc, None);
        assert!(out.contains("deleted = false"));
    }

    // ── Null value ──────────────────────────────────────────────────────────

    #[test]
    fn null_value() {
        let doc = OdinDocumentBuilder::new()
            .set("missing", OdinValues::null())
            .build()
            .unwrap();
        let out = stringify(&doc, None);
        assert!(out.contains("missing = ~"));
    }

    // ── Date values ─────────────────────────────────────────────────────────

    #[test]
    fn date_value() {
        let doc = OdinDocumentBuilder::new()
            .set("dob", OdinValues::date(2024, 6, 15))
            .build()
            .unwrap();
        let out = stringify(&doc, None);
        assert!(out.contains("dob = 2024-06-15"));
    }

    #[test]
    fn date_from_string() {
        let doc = OdinDocumentBuilder::new()
            .set("start", OdinValues::date_from_str("2023-01-01").unwrap())
            .build()
            .unwrap();
        let out = stringify(&doc, None);
        assert!(out.contains("start = 2023-01-01"));
    }

    // ── Timestamp values ────────────────────────────────────────────────────

    #[test]
    fn timestamp_value() {
        let doc = OdinDocumentBuilder::new()
            .set("ts", OdinValues::timestamp(0, "2024-06-15T14:30:00Z"))
            .build()
            .unwrap();
        let out = stringify(&doc, None);
        assert!(out.contains("ts = 2024-06-15T14:30:00Z"));
    }

    // ── Time values ─────────────────────────────────────────────────────────

    #[test]
    fn time_value() {
        let doc = OdinDocumentBuilder::new()
            .set("meeting", OdinValues::time("T10:30:00"))
            .build()
            .unwrap();
        let out = stringify(&doc, None);
        assert!(out.contains("meeting = T10:30:00"));
    }

    // ── Duration values ─────────────────────────────────────────────────────

    #[test]
    fn duration_value() {
        let doc = OdinDocumentBuilder::new()
            .set("term", OdinValues::duration("P1Y6M"))
            .build()
            .unwrap();
        let out = stringify(&doc, None);
        assert!(out.contains("term = P1Y6M"));
    }

    #[test]
    fn duration_time_only() {
        let doc = OdinDocumentBuilder::new()
            .set("timeout", OdinValues::duration("PT30M"))
            .build()
            .unwrap();
        let out = stringify(&doc, None);
        assert!(out.contains("timeout = PT30M"));
    }

    // ── Binary values ───────────────────────────────────────────────────────

    #[test]
    fn binary_value() {
        let doc = OdinDocumentBuilder::new()
            .set("data", OdinValues::binary(b"Hello".to_vec()))
            .build()
            .unwrap();
        let out = stringify(&doc, None);
        assert!(out.contains("data = ^SGVsbG8="));
    }

    #[test]
    fn binary_empty() {
        let doc = OdinDocumentBuilder::new()
            .set("empty", OdinValues::binary(vec![]))
            .build()
            .unwrap();
        let out = stringify(&doc, None);
        assert!(out.contains("empty = ^"));
    }

    #[test]
    fn binary_with_algorithm() {
        let doc = OdinDocumentBuilder::new()
            .set("hash", OdinValues::binary_with_algorithm(vec![0xAB, 0xCD], "sha256"))
            .build()
            .unwrap();
        let out = stringify(&doc, None);
        assert!(out.contains("hash = ^sha256:"));
    }

    // ── Reference values ────────────────────────────────────────────────────

    #[test]
    fn reference_value() {
        let doc = OdinDocumentBuilder::new()
            .set("ref", OdinValues::reference("policy.id"))
            .build()
            .unwrap();
        let out = stringify(&doc, None);
        assert!(out.contains("ref = @policy.id"));
    }

    #[test]
    fn reference_relative() {
        let doc = OdinDocumentBuilder::new()
            .set("self_ref", OdinValues::reference(".current"))
            .build()
            .unwrap();
        let out = stringify(&doc, None);
        assert!(out.contains("self_ref = @.current"));
    }

    // ── Sections ────────────────────────────────────────────────────────────

    #[test]
    fn section_header_generated() {
        let doc = OdinDocumentBuilder::new()
            .set("Policy.number", OdinValues::string("POL-001"))
            .build()
            .unwrap();
        let out = stringify(&doc, None);
        assert!(out.contains("{Policy}"));
        assert!(out.contains("number = \"POL-001\""));
    }

    #[test]
    fn multiple_sections() {
        let doc = OdinDocumentBuilder::new()
            .set("Policy.number", OdinValues::string("POL-001"))
            .set("Agent.name", OdinValues::string("Smith"))
            .build()
            .unwrap();
        let out = stringify(&doc, None);
        assert!(out.contains("{Policy}"));
        assert!(out.contains("{Agent}"));
    }

    #[test]
    fn lowercase_prefix_not_treated_as_section() {
        let doc = OdinDocumentBuilder::new()
            .set("policy.number", OdinValues::string("POL-001"))
            .build()
            .unwrap();
        let out = stringify(&doc, None);
        // lowercase prefix should NOT generate a section header
        assert!(!out.contains("{policy}"));
        assert!(out.contains("policy.number = \"POL-001\""));
    }

    // ── Modifiers ───────────────────────────────────────────────────────────

    #[test]
    fn required_modifier() {
        let mods = OdinModifiers { required: true, ..Default::default() };
        let doc = OdinDocumentBuilder::new()
            .set("name", OdinValues::string("Alice").with_modifiers(mods))
            .build()
            .unwrap();
        let out = stringify(&doc, None);
        assert!(out.contains("name = !\"Alice\""));
    }

    #[test]
    fn confidential_modifier() {
        let mods = OdinModifiers { confidential: true, ..Default::default() };
        let doc = OdinDocumentBuilder::new()
            .set("ssn", OdinValues::string("123-45-6789").with_modifiers(mods))
            .build()
            .unwrap();
        let out = stringify(&doc, None);
        assert!(out.contains("ssn = *\"123-45-6789\""));
    }

    #[test]
    fn deprecated_modifier() {
        let mods = OdinModifiers { deprecated: true, ..Default::default() };
        let doc = OdinDocumentBuilder::new()
            .set("old", OdinValues::string("legacy").with_modifiers(mods))
            .build()
            .unwrap();
        let out = stringify(&doc, None);
        assert!(out.contains("old = -\"legacy\""));
    }

    #[test]
    fn all_modifiers_combined() {
        let mods = OdinModifiers {
            required: true,
            confidential: true,
            deprecated: true,
            attr: false,
        };
        let doc = OdinDocumentBuilder::new()
            .set("field", OdinValues::string("value").with_modifiers(mods))
            .build()
            .unwrap();
        let out = stringify(&doc, None);
        // Canonical order: ! * -
        assert!(out.contains("field = !*-\"value\""));
    }

    #[test]
    fn required_and_confidential_modifiers() {
        let mods = OdinModifiers {
            required: true,
            confidential: true,
            deprecated: false,
            attr: false,
        };
        let doc = OdinDocumentBuilder::new()
            .set("secret", OdinValues::string("xxx").with_modifiers(mods))
            .build()
            .unwrap();
        let out = stringify(&doc, None);
        assert!(out.contains("secret = !*\"xxx\""));
    }

    #[test]
    fn modifier_on_integer() {
        let mods = OdinModifiers { required: true, ..Default::default() };
        let doc = OdinDocumentBuilder::new()
            .set("id", OdinValues::integer(42).with_modifiers(mods))
            .build()
            .unwrap();
        let out = stringify(&doc, None);
        assert!(out.contains("id = !##42"));
    }

    #[test]
    fn modifier_on_boolean() {
        let mods = OdinModifiers { required: true, ..Default::default() };
        let doc = OdinDocumentBuilder::new()
            .set("active", OdinValues::boolean(true).with_modifiers(mods))
            .build()
            .unwrap();
        let out = stringify(&doc, None);
        assert!(out.contains("active = !true"));
    }

    #[test]
    fn modifier_on_null() {
        let mods = OdinModifiers { deprecated: true, ..Default::default() };
        let doc = OdinDocumentBuilder::new()
            .set("gone", OdinValues::null().with_modifiers(mods))
            .build()
            .unwrap();
        let out = stringify(&doc, None);
        assert!(out.contains("gone = -~"));
    }

    // ── Directives ──────────────────────────────────────────────────────────

    #[test]
    fn directive_without_value() {
        let val = OdinValues::string("test").with_directives(vec![
            OdinDirective { name: "trim".to_string(), value: None },
        ]);
        let doc = OdinDocumentBuilder::new()
            .set("field", val)
            .build()
            .unwrap();
        let out = stringify(&doc, None);
        assert!(out.contains("\"test\" :trim"));
    }

    #[test]
    fn directive_with_string_value() {
        let val = OdinValues::string("test").with_directives(vec![
            OdinDirective {
                name: "format".to_string(),
                value: Some(DirectiveValue::String("ssn".to_string())),
            },
        ]);
        let doc = OdinDocumentBuilder::new()
            .set("field", val)
            .build()
            .unwrap();
        let out = stringify(&doc, None);
        assert!(out.contains("\"test\" :format ssn"));
    }

    #[test]
    fn directive_with_numeric_value() {
        let val = OdinValues::reference("_line").with_directives(vec![
            OdinDirective {
                name: "pos".to_string(),
                value: Some(DirectiveValue::Number(3.0)),
            },
            OdinDirective {
                name: "len".to_string(),
                value: Some(DirectiveValue::Number(8.0)),
            },
        ]);
        let doc = OdinDocumentBuilder::new()
            .set("field", val)
            .build()
            .unwrap();
        let out = stringify(&doc, None);
        assert!(out.contains("@_line :pos 3 :len 8"));
    }

    // ── Options: sort_keys ──────────────────────────────────────────────────

    #[test]
    fn sort_keys_alphabetical() {
        let doc = OdinDocumentBuilder::new()
            .set("z_last", OdinValues::integer(3))
            .set("a_first", OdinValues::integer(1))
            .set("m_mid", OdinValues::integer(2))
            .build()
            .unwrap();
        let out = stringify(&doc, Some(&opts_with_sort()));
        let a_pos = out.find("a_first").unwrap();
        let m_pos = out.find("m_mid").unwrap();
        let z_pos = out.find("z_last").unwrap();
        assert!(a_pos < m_pos);
        assert!(m_pos < z_pos);
    }

    #[test]
    fn unsorted_preserves_insertion_order() {
        let doc = OdinDocumentBuilder::new()
            .set("z_last", OdinValues::integer(3))
            .set("a_first", OdinValues::integer(1))
            .build()
            .unwrap();
        let out = stringify(&doc, None);
        let z_pos = out.find("z_last").unwrap();
        let a_pos = out.find("a_first").unwrap();
        assert!(z_pos < a_pos);
    }

    // ── Metadata section ────────────────────────────────────────────────────

    #[test]
    fn metadata_with_multiple_keys() {
        let doc = OdinDocumentBuilder::new()
            .metadata("odin", OdinValues::string("1.0.0"))
            .metadata("transform", OdinValues::string("1.0.0"))
            .set("name", OdinValues::string("test"))
            .build()
            .unwrap();
        let out = stringify(&doc, Some(&default_opts()));
        assert!(out.starts_with("{$}\n"));
        assert!(out.contains("odin = \"1.0.0\""));
        assert!(out.contains("transform = \"1.0.0\""));
    }

    // ── Arrays ──────────────────────────────────────────────────────────────

    #[test]
    fn array_of_values() {
        let arr = OdinValues::array(vec![
            OdinArrayItem::Value(OdinValues::integer(1)),
            OdinArrayItem::Value(OdinValues::integer(2)),
            OdinArrayItem::Value(OdinValues::integer(3)),
        ]);
        let doc = OdinDocumentBuilder::new()
            .set("nums", arr)
            .build()
            .unwrap();
        let out = stringify(&doc, None);
        assert!(out.contains("nums = ##1, ##2, ##3"));
    }

    #[test]
    fn array_single_item() {
        let arr = OdinValues::array(vec![
            OdinArrayItem::Value(OdinValues::string("only")),
        ]);
        let doc = OdinDocumentBuilder::new()
            .set("items", arr)
            .build()
            .unwrap();
        let out = stringify(&doc, None);
        assert!(out.contains("items = \"only\""));
    }

    // ── Verb expressions ────────────────────────────────────────────────────

    #[test]
    fn verb_expression() {
        let val = OdinValues::verb("upper", vec![OdinValues::reference("name")]);
        let doc = OdinDocumentBuilder::new()
            .set("Name", val)
            .build()
            .unwrap();
        let out = stringify(&doc, None);
        assert!(out.contains("%upper @name"));
    }

    #[test]
    fn custom_verb_expression() {
        let val = OdinValues::custom_verb("myNs.doIt", vec![]);
        let doc = OdinDocumentBuilder::new()
            .set("result", val)
            .build()
            .unwrap();
        let out = stringify(&doc, None);
        assert!(out.contains("%&myNs.doIt"));
    }

    // ── Large document ──────────────────────────────────────────────────────

    #[test]
    fn large_document_many_fields() {
        let mut builder = OdinDocumentBuilder::new();
        for i in 0..100 {
            builder = builder.set(&format!("field_{i}"), OdinValues::integer(i));
        }
        let doc = builder.build().unwrap();
        let out = stringify(&doc, None);
        assert!(out.contains("field_0 = ##0"));
        assert!(out.contains("field_99 = ##99"));
        // Should have 100 lines of assignments
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), 100);
    }

    // ── Mixed type document ─────────────────────────────────────────────────

    #[test]
    fn mixed_types_in_document() {
        let doc = OdinDocumentBuilder::new()
            .set("name", OdinValues::string("Test"))
            .set("count", OdinValues::integer(42))
            .set("rate", OdinValues::number(3.14))
            .set("active", OdinValues::boolean(true))
            .set("notes", OdinValues::null())
            .build()
            .unwrap();
        let out = stringify(&doc, None);
        assert!(out.contains("name = \"Test\""));
        assert!(out.contains("count = ##42"));
        assert!(out.contains("rate = #3.14"));
        assert!(out.contains("active = true"));
        assert!(out.contains("notes = ~"));
    }

    // ── split_path unit tests ───────────────────────────────────────────────

    #[test]
    fn split_path_no_dot() {
        let (section, field) = split_path("name");
        assert_eq!(section, None);
        assert_eq!(field, "name");
    }

    #[test]
    fn split_path_uppercase_section() {
        let (section, field) = split_path("Policy.number");
        assert_eq!(section, Some("Policy"));
        assert_eq!(field, "number");
    }

    #[test]
    fn split_path_lowercase_not_section() {
        let (section, field) = split_path("policy.number");
        assert_eq!(section, None);
        assert_eq!(field, "policy.number");
    }

    // ── base64 encoding ─────────────────────────────────────────────────────

    #[test]
    fn base64_encode_hello() {
        assert_eq!(base64_encode(b"Hello"), "SGVsbG8=");
    }

    #[test]
    fn base64_encode_empty() {
        assert_eq!(base64_encode(b""), "");
    }

    #[test]
    fn base64_encode_single_byte() {
        assert_eq!(base64_encode(b"A"), "QQ==");
    }

    #[test]
    fn base64_encode_two_bytes() {
        assert_eq!(base64_encode(b"AB"), "QUI=");
    }

    #[test]
    fn base64_encode_three_bytes() {
        assert_eq!(base64_encode(b"ABC"), "QUJD");
    }

    // ── Escaped string helper ───────────────────────────────────────────────

    #[test]
    fn write_escaped_string_plain() {
        let mut out = String::new();
        write_escaped_string(&mut out, "hello");
        assert_eq!(out, "hello");
    }

    #[test]
    fn write_escaped_string_special_chars() {
        let mut out = String::new();
        write_escaped_string(&mut out, "a\"b\\c\nd\te\rf");
        assert_eq!(out, r#"a\"b\\c\nd\te\rf"#);
    }

    // ── Modifier ordering ───────────────────────────────────────────────────

    #[test]
    fn write_modifiers_canonical_order() {
        let mut out = String::new();
        let mods = OdinModifiers {
            required: true,
            confidential: true,
            deprecated: true,
            attr: false,
        };
        write_modifiers(&mut out, &mods);
        assert_eq!(out, "!*-");
    }

    #[test]
    fn write_modifiers_required_only() {
        let mut out = String::new();
        let mods = OdinModifiers { required: true, ..Default::default() };
        write_modifiers(&mut out, &mods);
        assert_eq!(out, "!");
    }

    #[test]
    fn write_modifiers_none_set() {
        let mut out = String::new();
        let mods = OdinModifiers::default();
        write_modifiers(&mut out, &mods);
        assert_eq!(out, "");
    }

    // ── Currency with raw string ────────────────────────────────────────────

    #[test]
    fn currency_raw_with_colon_uppercases_code() {
        let val = OdinValue::Currency {
            value: 100.0,
            decimal_places: 2,
            currency_code: None,
            raw: Some("100.00:usd".to_string()),
            modifiers: None,
            directives: vec![],
        };
        let doc = OdinDocumentBuilder::new()
            .set("amt", val)
            .build()
            .unwrap();
        let out = stringify(&doc, None);
        assert!(out.contains("#$100.00:USD"));
    }

    // ── Sections with multiple fields ───────────────────────────────────────

    #[test]
    fn section_groups_fields() {
        let doc = OdinDocumentBuilder::new()
            .set("Policy.number", OdinValues::string("POL-001"))
            .set("Policy.status", OdinValues::string("active"))
            .set("Policy.premium", OdinValues::currency(1500.0, 2))
            .build()
            .unwrap();
        let out = stringify(&doc, None);
        // Section header should appear once
        let count = out.matches("{Policy}").count();
        assert_eq!(count, 1);
        assert!(out.contains("number = \"POL-001\""));
        assert!(out.contains("status = \"active\""));
        assert!(out.contains("premium = #$1500.00"));
    }

    // ── Roundtrip through stringify ─────────────────────────────────────────

    #[test]
    fn metadata_and_assignments_together() {
        let doc = OdinDocumentBuilder::new()
            .metadata("odin", OdinValues::string("1.0.0"))
            .set("name", OdinValues::string("roundtrip"))
            .set("count", OdinValues::integer(7))
            .build()
            .unwrap();
        let out = stringify(&doc, Some(&default_opts()));
        assert!(out.starts_with("{$}\n"));
        assert!(out.contains("odin = \"1.0.0\""));
        assert!(out.contains("name = \"roundtrip\""));
        assert!(out.contains("count = ##7"));
    }

    // ── Number with raw ─────────────────────────────────────────────────────

    #[test]
    fn number_with_raw_preserves_raw() {
        let val = OdinValue::Number {
            value: 3.14159265358979,
            decimal_places: None,
            raw: Some("3.14159265358979".to_string()),
            modifiers: None,
            directives: vec![],
        };
        let doc = OdinDocumentBuilder::new()
            .set("pi", val)
            .build()
            .unwrap();
        let out = stringify(&doc, None);
        assert!(out.contains("#3.14159265358979"));
    }

    // ── Percent with raw ────────────────────────────────────────────────────

    #[test]
    fn percent_with_raw_preserves_raw() {
        let val = OdinValue::Percent {
            value: 0.15,
            raw: Some("0.15".to_string()),
            modifiers: None,
            directives: vec![],
        };
        let doc = OdinDocumentBuilder::new()
            .set("rate", val)
            .build()
            .unwrap();
        let out = stringify(&doc, None);
        assert!(out.contains("#%0.15"));
    }

    // ── Tabular ragged sub-arrays (regression guard) ────────────────────────
    // The flat stringify path here cannot pad ragged sub-arrays — it has no
    // tabular emission. These tests guard against that changing.

    #[test]
    fn ragged_string_subarrays_do_not_emit_padded_tabular_header() {
        let doc = OdinDocumentBuilder::new()
            .set("records[0].name", OdinValues::string("Alice"))
            .set("records[0].tags[0]", OdinValues::string("red"))
            .set("records[0].tags[1]", OdinValues::string("green"))
            .set("records[0].tags[2]", OdinValues::string("blue"))
            .set("records[1].name", OdinValues::string("Bob"))
            .set("records[1].tags[0]", OdinValues::string("yellow"))
            .build()
            .unwrap();
        let out = stringify(&doc, None);
        // Must NOT contain a tabular column header listing the indexed
        // tag positions for the records[] array.
        assert!(!out.contains("{records[] :"));
        // Each row's tags should be present individually, not padded.
        assert!(out.contains("records[0].tags[2]"));
        assert!(out.contains("records[1].tags[0]"));
        assert!(!out.contains("records[1].tags[1]"));
    }

    #[test]
    fn ragged_numeric_subarrays_do_not_emit_padded_tabular_header() {
        let doc = OdinDocumentBuilder::new()
            .set("points[0].label", OdinValues::string("A"))
            .set("points[0].coords[0]", OdinValues::integer(1))
            .set("points[0].coords[1]", OdinValues::integer(2))
            .set("points[1].label", OdinValues::string("B"))
            .set("points[1].coords[0]", OdinValues::integer(3))
            .set("points[1].coords[1]", OdinValues::integer(4))
            .set("points[1].coords[2]", OdinValues::integer(5))
            .set("points[1].coords[3]", OdinValues::integer(6))
            .build()
            .unwrap();
        let out = stringify(&doc, None);
        assert!(!out.contains("{points[] :"));
        assert!(out.contains("points[1].coords[3]"));
        assert!(!out.contains("points[0].coords[2]"));
    }

    #[test]
    fn ragged_subarrays_round_trip_without_data_loss() {
        let doc = OdinDocumentBuilder::new()
            .set("entries[0].slug", OdinValues::string("a/one"))
            .set("entries[0].title", OdinValues::string("One"))
            .set("entries[0].types[0]", OdinValues::string("alpha"))
            .set("entries[0].types[1]", OdinValues::string("beta"))
            .set("entries[0].fields[0]", OdinValues::string("id"))
            .set("entries[0].fields[1]", OdinValues::string("name"))
            .set("entries[0].fields[2]", OdinValues::string("desc"))
            .set("entries[1].slug", OdinValues::string("b/two"))
            .set("entries[1].title", OdinValues::string("Two"))
            .set("entries[1].types[0]", OdinValues::string("gamma"))
            .set("entries[1].fields[0]", OdinValues::string("id"))
            .build()
            .unwrap();
        let text = stringify(&doc, None);
        let reparsed = crate::parser::parse(&text, None).expect("reparse");
        // Every original assignment must survive the round-trip.
        for (path, _) in &doc.assignments {
            assert!(
                reparsed.assignments.iter().any(|(p, _)| p == path),
                "lost assignment after round-trip: {path}"
            );
        }
        assert_eq!(doc.assignments.len(), reparsed.assignments.len());
    }

    #[test]
    fn dense_scalar_records_serialize_without_loss() {
        // Pure scalar columns, every row populated. The TypeScript SDK
        // emits this as tabular; the Rust SDK emits flat assignments.
        // Either way, every assignment must be present in the output.
        let doc = OdinDocumentBuilder::new()
            .set("rows[0].name", OdinValues::string("Alice"))
            .set("rows[0].age", OdinValues::integer(30))
            .set("rows[1].name", OdinValues::string("Bob"))
            .set("rows[1].age", OdinValues::integer(25))
            .build()
            .unwrap();
        let out = stringify(&doc, None);
        assert!(out.contains("rows[0].name"));
        assert!(out.contains("rows[0].age"));
        assert!(out.contains("rows[1].name"));
        assert!(out.contains("rows[1].age"));
    }

    #[test]
    fn uniform_width_subarrays_serialize_without_loss() {
        let doc = OdinDocumentBuilder::new()
            .set("points[0].label", OdinValues::string("A"))
            .set("points[0].coords[0]", OdinValues::integer(1))
            .set("points[0].coords[1]", OdinValues::integer(2))
            .set("points[1].label", OdinValues::string("B"))
            .set("points[1].coords[0]", OdinValues::integer(3))
            .set("points[1].coords[1]", OdinValues::integer(4))
            .build()
            .unwrap();
        let out = stringify(&doc, None);
        assert!(out.contains("points[0].coords[1]"));
        assert!(out.contains("points[1].coords[1]"));
    }

    #[test]
    fn search_index_fixture_does_not_emit_widest_column_header() {
        // Build a search-index-style fixture: 20 records with very
        // different sub-array widths (1 .. 39 tags). Without the rule,
        // a tabular emitter would pad every row to the widest record's
        // column count. The output must never contain such a header.
        let mut builder = OdinDocumentBuilder::new();
        for r in 0..20 {
            builder = builder
                .set(&format!("entries[{r}].slug"), OdinValues::string(&format!("record/{r}")))
                .set(&format!("entries[{r}].title"), OdinValues::string(&format!("Record {r}")));
            let tag_count = 1 + r * 2;
            for t in 0..tag_count {
                builder = builder.set(
                    &format!("entries[{r}].tags[{t}]"),
                    OdinValues::string(&format!("tag-{r}-{t}")),
                );
            }
        }
        let doc = builder.build().unwrap();
        let out = stringify(&doc, None);

        // Must not contain a tabular column header for entries[].
        assert!(!out.contains("{entries[] :"));
        // Sanity: the widest row is fully present.
        assert!(out.contains("entries[19].tags[38]"));
        // Sanity: the narrowest row only has its single tag.
        assert!(out.contains("entries[0].tags[0]"));
        assert!(!out.contains("entries[0].tags[1]"));
    }
}
