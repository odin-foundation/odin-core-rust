//! Streaming parser for ODIN documents.
//!
//! Processes ODIN text incrementally via chunks, emitting events through
//! a handler callback interface. Designed for memory-efficient parsing of
//! large documents.
//!
//! # Example
//!
//! ```rust
//! use odin_core::parser::streaming::{StreamingParser, ParseHandler};
//! use odin_core::OdinValue;
//!
//! struct MyHandler;
//! impl ParseHandler for MyHandler {
//!     fn on_assignment(&mut self, path: &str, value: OdinValue) {
//!         println!("{} = {:?}", path, value);
//!     }
//! }
//!
//! let mut parser = StreamingParser::new(MyHandler);
//! parser.process_chunk(b"{$}\nodin = \"1.0.0\"\n");
//! parser.process_chunk(b"{person}\nname = \"Alice\"\n");
//! parser.finish();
//! ```

use crate::types::values::{OdinValue, OdinValues, OdinModifiers};

// Security limits
const MAX_TOTAL_BYTES: usize = 100 * 1024 * 1024; // 100 MB
const BUFFER_COMPACT_THRESHOLD: usize = 8 * 1024; // 8 KB

/// Handler trait for streaming parser events.
pub trait ParseHandler {
    /// Called when a new document starts (first non-empty content or after `---`).
    fn on_document_start(&mut self) {}

    /// Called when a header is encountered (e.g., `{section}`).
    fn on_header(&mut self, _path: &str) {}

    /// Called for each assignment (e.g., `key = value`).
    fn on_assignment(&mut self, _path: &str, _value: OdinValue) {}

    /// Called when a document ends (at `---` separator or end of input).
    fn on_document_end(&mut self) {}

    /// Called on parse error.
    fn on_error(&mut self, _error: StreamingParseError) {}
}

/// Error type for streaming parser.
#[derive(Debug, Clone)]
pub struct StreamingParseError {
    /// Error message.
    pub message: String,
    /// Line number where the error occurred.
    pub line: usize,
}

impl std::fmt::Display for StreamingParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Line {}: {}", self.line, self.message)
    }
}

impl std::error::Error for StreamingParseError {}

/// Event-driven streaming parser for ODIN documents.
///
/// Processes UTF-8 byte chunks incrementally and emits events via the handler.
pub struct StreamingParser<H: ParseHandler> {
    handler: H,
    buffer: String,
    buffer_offset: usize,
    pending_bytes: Vec<u8>,
    total_bytes: usize,
    current_header: String,
    document_started: bool,
    has_error: bool,
    line_number: usize,
}

impl<H: ParseHandler> StreamingParser<H> {
    /// Create a new streaming parser with the given handler.
    pub fn new(handler: H) -> Self {
        Self {
            handler,
            buffer: String::new(),
            buffer_offset: 0,
            pending_bytes: Vec::new(),
            total_bytes: 0,
            current_header: String::new(),
            document_started: false,
            has_error: false,
            line_number: 1,
        }
    }

    /// Process a chunk of UTF-8 bytes.
    pub fn process_chunk(&mut self, chunk: &[u8]) {
        if self.has_error {
            return;
        }

        self.total_bytes += chunk.len();
        if self.total_bytes > MAX_TOTAL_BYTES {
            self.emit_error("Input exceeds maximum size limit (100MB)");
            return;
        }

        // Handle incomplete UTF-8 sequences from previous chunk
        let bytes_to_decode = if self.pending_bytes.is_empty() {
            chunk.to_vec()
        } else {
            let mut combined = std::mem::take(&mut self.pending_bytes);
            combined.extend_from_slice(chunk);
            combined
        };

        // Check for incomplete UTF-8 at end
        let (complete, remainder) = split_utf8_boundary(&bytes_to_decode);

        if !remainder.is_empty() {
            self.pending_bytes = remainder;
        }

        if let Ok(text) = std::str::from_utf8(&complete) {
            self.buffer.push_str(text);
        } else {
            self.emit_error("Invalid UTF-8 in input");
            return;
        }

        self.process_lines(false);
    }

    /// Finish parsing — flush any remaining content.
    pub fn finish(mut self) -> H {
        if !self.pending_bytes.is_empty() {
            // Try to decode remaining bytes
            if let Ok(text) = std::str::from_utf8(&self.pending_bytes) {
                self.buffer.push_str(text);
            }
            self.pending_bytes.clear();
        }

        self.process_lines(true);

        if self.document_started {
            self.handler.on_document_end();
        }

        self.handler
    }

    /// Get a reference to the handler.
    pub fn handler(&self) -> &H {
        &self.handler
    }

    /// Get a mutable reference to the handler.
    pub fn handler_mut(&mut self) -> &mut H {
        &mut self.handler
    }

    fn process_lines(&mut self, is_final: bool) {
        loop {
            if self.has_error {
                break;
            }

            let remaining = &self.buffer[self.buffer_offset..];
            if let Some(newline_pos) = remaining.find('\n') {
                let line_end = self.buffer_offset + newline_pos;
                let mut line = self.buffer[self.buffer_offset..line_end].to_string();

                // Strip \r for \r\n
                if line.ends_with('\r') {
                    line.pop();
                }

                self.process_line(&line);
                self.line_number += 1;
                self.buffer_offset = line_end + 1;
            } else if is_final && self.buffer_offset < self.buffer.len() {
                // Process remaining content without trailing newline
                let line = self.buffer[self.buffer_offset..].to_string();
                let line = line.trim_end_matches('\r').to_string();
                if !line.is_empty() {
                    self.process_line(&line);
                }
                self.buffer_offset = self.buffer.len();
                break;
            } else {
                break;
            }
        }

        // Compact buffer when offset exceeds threshold
        if self.buffer_offset > BUFFER_COMPACT_THRESHOLD {
            self.buffer = self.buffer[self.buffer_offset..].to_string();
            self.buffer_offset = 0;
        }
    }

    fn process_line(&mut self, line: &str) {
        let trimmed = line.trim();

        // Skip empty lines and comments
        if trimmed.is_empty() || trimmed.starts_with(';') {
            return;
        }

        // Document separator
        if trimmed == "---" {
            if self.document_started {
                self.handler.on_document_end();
            }
            self.document_started = false;
            self.current_header.clear();
            return;
        }

        // Auto-start document
        if !self.document_started {
            self.document_started = true;
            self.handler.on_document_start();
        }

        // Header line: {path}
        if trimmed.starts_with('{') && trimmed.ends_with('}') {
            self.process_header(trimmed);
            return;
        }

        // Assignment line: key = value
        self.process_assignment(trimmed);
    }

    fn process_header(&mut self, line: &str) {
        let path = &line[1..line.len() - 1].trim();

        // Map meta headers
        let header = if *path == "$" {
            "$".to_string()
        } else if let Some(rest) = path.strip_prefix("$") {
            format!("$.{}", rest.trim_start_matches('.'))
        } else {
            (*path).to_string()
        };

        self.current_header.clone_from(&header);
        self.handler.on_header(&header);
    }

    fn process_assignment(&mut self, line: &str) {
        // Find '=' separator
        // Not an assignment — could be a bare line
        let Some(eq_pos) = find_assignment_eq(line) else { return };

        let key = line[..eq_pos].trim();
        let value_text = line[eq_pos + 1..].trim();

        // Strip inline comments
        let value_text = strip_inline_comment(value_text);

        // Build full path
        let full_path = if self.current_header.is_empty() {
            key.to_string()
        } else {
            format!("{}.{}", self.current_header, key)
        };

        // Parse value
        match parse_streaming_value(value_text) {
            Ok(value) => {
                self.handler.on_assignment(&full_path, value);
            }
            Err(msg) => {
                self.emit_error(&format!("Failed to parse value for '{key}': {msg}"));
            }
        }
    }

    fn emit_error(&mut self, message: &str) {
        self.has_error = true;
        self.handler.on_error(StreamingParseError {
            message: message.to_string(),
            line: self.line_number,
        });
    }
}

/// Find the position of the assignment `=` separator, skipping `=` inside quoted strings.
fn find_assignment_eq(line: &str) -> Option<usize> {
    let mut in_quote = false;
    let mut escape = false;

    for (i, ch) in line.char_indices() {
        if escape {
            escape = false;
            continue;
        }
        match ch {
            '\\' if in_quote => escape = true,
            '"' => in_quote = !in_quote,
            '=' if !in_quote => return Some(i),
            _ => {}
        }
    }
    None
}

/// Strip inline comment (semicolon outside quotes).
fn strip_inline_comment(text: &str) -> &str {
    let mut in_quote = false;
    let mut escape = false;

    for (i, ch) in text.char_indices() {
        if escape {
            escape = false;
            continue;
        }
        match ch {
            '\\' if in_quote => escape = true,
            '"' => in_quote = !in_quote,
            ';' if !in_quote => return text[..i].trim_end(),
            _ => {}
        }
    }
    text
}

/// Parse a value from raw text (streaming context — no tokenizer).
fn parse_streaming_value(text: &str) -> Result<OdinValue, String> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(OdinValues::string(""));
    }

    // Parse modifiers prefix: !, *, -
    let (modifiers, rest) = parse_modifiers(text);
    let mut value = parse_value_core(rest)?;

    // Apply modifiers
    if modifiers.required || modifiers.confidential || modifiers.deprecated {
        match &mut value {
            OdinValue::String { modifiers: m, .. }
            | OdinValue::Integer { modifiers: m, .. }
            | OdinValue::Number { modifiers: m, .. }
            | OdinValue::Currency { modifiers: m, .. }
            | OdinValue::Percent { modifiers: m, .. }
            | OdinValue::Boolean { modifiers: m, .. }
            | OdinValue::Null { modifiers: m, .. }
            | OdinValue::Date { modifiers: m, .. }
            | OdinValue::Timestamp { modifiers: m, .. }
            | OdinValue::Time { modifiers: m, .. }
            | OdinValue::Duration { modifiers: m, .. }
            | OdinValue::Reference { modifiers: m, .. }
            | OdinValue::Binary { modifiers: m, .. } => {
                *m = Some(OdinModifiers {
                    required: modifiers.required,
                    confidential: modifiers.confidential,
                    deprecated: modifiers.deprecated,
                    ..OdinModifiers::default()
                });
            }
            _ => {}
        }
    }

    Ok(value)
}

fn parse_modifiers(text: &str) -> (OdinModifiers, &str) {
    let mut mods = OdinModifiers::default();
    let mut i = 0;
    let bytes = text.as_bytes();

    while i < bytes.len() {
        match bytes[i] {
            b'!' => { mods.required = true; i += 1; }
            b'*' => { mods.confidential = true; i += 1; }
            b'-' if i + 1 < bytes.len() && (bytes[i + 1] == b'!' || bytes[i + 1] == b'*' || bytes[i + 1] == b'"' || bytes[i + 1] == b'-') => {
                mods.deprecated = true;
                i += 1;
            }
            _ => break,
        }
    }

    (mods, &text[i..])
}

fn parse_value_core(text: &str) -> Result<OdinValue, String> {
    let text = text.trim();

    if text.is_empty() {
        return Ok(OdinValues::string(""));
    }

    // Null
    if text == "~" {
        return Ok(OdinValues::null());
    }

    // Boolean (with or without prefix)
    if text == "true" || text == "?true" {
        return Ok(OdinValues::boolean(true));
    }
    if text == "false" || text == "?false" {
        return Ok(OdinValues::boolean(false));
    }

    // Reference: @path
    if let Some(ref_path) = text.strip_prefix('@') {
        return Ok(OdinValues::reference(ref_path));
    }

    // Binary: ^base64 or ^algo:hex
    if let Some(data_text) = text.strip_prefix('^') {
        if let Some(colon_pos) = data_text.find(':') {
            let algo = &data_text[..colon_pos];
            let hex = &data_text[colon_pos + 1..];
            let bytes = hex_decode(hex).map_err(|e| format!("Invalid hex: {e}"))?;
            return Ok(OdinValue::Binary {
                data: bytes,
                algorithm: Some(algo.to_string()),
                modifiers: None,
                directives: Vec::new(),
            });
        }
        let bytes = crate::utils::base64::decode(data_text)
            .map_err(|e| format!("Invalid base64: {e}"))?;
        return Ok(OdinValue::Binary {
            data: bytes,
            algorithm: None,
            modifiers: None,
            directives: Vec::new(),
        });
    }

    // Currency: #$value[:code]
    if let Some(raw) = text.strip_prefix("#$") {
        let (numeric_part, code) = if let Some(colon) = raw.rfind(':') {
            let potential_code = &raw[colon + 1..];
            if potential_code.len() == 3 && potential_code.chars().all(|c| c.is_ascii_uppercase()) {
                (&raw[..colon], Some(potential_code.to_string()))
            } else {
                (raw, None)
            }
        } else {
            (raw, None)
        };
        let value = numeric_part.parse::<f64>().unwrap_or(0.0);
        let dp = numeric_part.find('.').map_or(2, |p| (numeric_part.len() - p - 1) as u8);
        return Ok(OdinValue::Currency {
            value,
            decimal_places: dp,
            currency_code: code,
            raw: Some(raw.to_string()),
            modifiers: None,
            directives: Vec::new(),
        });
    }

    // Percent: #%value
    if let Some(raw_str) = text.strip_prefix("#%") {
        let value = raw_str.parse::<f64>().unwrap_or(0.0);
        return Ok(OdinValue::Percent {
            value,
            raw: Some(raw_str.to_string()),
            modifiers: None,
            directives: Vec::new(),
        });
    }

    // Integer: ##value
    if let Some(int_str) = text.strip_prefix("##") {
        let n = int_str.parse::<i64>()
            .map_err(|_| format!("Invalid integer: {int_str}"))?;
        return Ok(OdinValues::integer(n));
    }

    // Number: #value
    if let Some(raw_str) = text.strip_prefix('#') {
        let value = raw_str.parse::<f64>().unwrap_or(0.0);
        let dp = raw_str.find('.').map(|p| (raw_str.len() - p - 1) as u8);
        return Ok(OdinValue::Number {
            value,
            decimal_places: dp,
            raw: Some(raw_str.to_string()),
            modifiers: None,
            directives: Vec::new(),
        });
    }

    // Quoted string
    if text.starts_with('"') && text.ends_with('"') && text.len() >= 2 {
        let inner = &text[1..text.len() - 1];
        let unescaped = unescape_string(inner);
        return Ok(OdinValues::string(&unescaped));
    }

    // ISO patterns
    // Timestamp: YYYY-MM-DDTHH:MM:SS...
    if text.len() >= 19 && text.as_bytes().get(10) == Some(&b'T') && is_date_prefix(text) {
        return Ok(OdinValue::Timestamp {
            epoch_ms: 0, // Streaming parser doesn't compute epoch
            raw: text.to_string(),
            modifiers: None,
            directives: Vec::new(),
        });
    }

    // Date: YYYY-MM-DD
    if text.len() == 10 && is_date_prefix(text) {
        let year = text[0..4].parse::<i32>().unwrap_or(0);
        let month = text[5..7].parse::<u8>().unwrap_or(0);
        let day = text[8..10].parse::<u8>().unwrap_or(0);
        return Ok(OdinValue::Date {
            year,
            month,
            day,
            raw: text.to_string(),
            modifiers: None,
            directives: Vec::new(),
        });
    }

    // Time: T14:30:00 or THH:MM
    if text.starts_with('T') && text.len() >= 5 && text.as_bytes()[3] == b':' {
        return Ok(OdinValue::Time {
            value: text.to_string(),
            modifiers: None,
            directives: Vec::new(),
        });
    }

    // Duration: P...
    if text.starts_with('P') && text.len() >= 2 {
        return Ok(OdinValue::Duration {
            value: text.to_string(),
            modifiers: None,
            directives: Vec::new(),
        });
    }

    // Unquoted string — treat as string
    Ok(OdinValues::string(text))
}

fn is_date_prefix(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() >= 10
        && b[0..4].iter().all(u8::is_ascii_digit)
        && b[4] == b'-'
        && b[5..7].iter().all(u8::is_ascii_digit)
        && b[7] == b'-'
        && b[8..10].iter().all(u8::is_ascii_digit)
}

fn unescape_string(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            match chars.next() {
                Some('n') => result.push('\n'),
                Some('r') => result.push('\r'),
                Some('t') => result.push('\t'),
                Some('\\') | None => result.push('\\'),
                Some('"') => result.push('"'),
                Some('u') => {
                    let hex: String = chars.by_ref().take(4).collect();
                    if let Ok(code) = u32::from_str_radix(&hex, 16) {
                        if let Some(c) = char::from_u32(code) {
                            result.push(c);
                        }
                    }
                }
                Some(other) => {
                    result.push('\\');
                    result.push(other);
                }
            }
        } else {
            result.push(ch);
        }
    }
    result
}

fn hex_decode(s: &str) -> Result<Vec<u8>, String> {
    if s.len() % 2 != 0 {
        return Err("Odd-length hex string".to_string());
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).map_err(|e| e.to_string()))
        .collect()
}

/// Split a byte slice at the last complete UTF-8 boundary.
/// Returns (`complete_bytes`, `remainder_bytes`).
fn split_utf8_boundary(bytes: &[u8]) -> (Vec<u8>, Vec<u8>) {
    if bytes.is_empty() {
        return (Vec::new(), Vec::new());
    }

    // Find incomplete UTF-8 suffix
    let len = bytes.len();
    let mut check_from = len.saturating_sub(3).max(0);

    // Walk forward from check_from to find start of potential incomplete sequence
    while check_from < len {
        let byte = bytes[check_from];
        if byte < 0x80 {
            // ASCII — complete
            check_from += 1;
            continue;
        }

        // Start of multi-byte sequence
        let expected_len = if byte & 0xE0 == 0xC0 {
            2 // 110xxxxx
        } else if byte & 0xF0 == 0xE0 {
            3 // 1110xxxx
        } else if byte & 0xF8 == 0xF0 {
            4 // 11110xxx
        } else if byte & 0xC0 == 0x80 {
            // Continuation byte — skip
            check_from += 1;
            continue;
        } else {
            // Invalid — treat as complete
            check_from += 1;
            continue;
        };

        let remaining = len - check_from;
        if remaining < expected_len {
            // Incomplete sequence at end
            return (bytes[..check_from].to_vec(), bytes[check_from..].to_vec());
        }

        // Complete sequence, advance past it
        check_from += expected_len;
    }

    (bytes.to_vec(), Vec::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct CollectHandler {
        assignments: Vec<(String, OdinValue)>,
        headers: Vec<String>,
        doc_starts: usize,
        doc_ends: usize,
        errors: Vec<StreamingParseError>,
    }

    impl CollectHandler {
        fn new() -> Self {
            Self {
                assignments: Vec::new(),
                headers: Vec::new(),
                doc_starts: 0,
                doc_ends: 0,
                errors: Vec::new(),
            }
        }
    }

    impl ParseHandler for CollectHandler {
        fn on_document_start(&mut self) { self.doc_starts += 1; }
        fn on_header(&mut self, path: &str) { self.headers.push(path.to_string()); }
        fn on_assignment(&mut self, path: &str, value: OdinValue) {
            self.assignments.push((path.to_string(), value));
        }
        fn on_document_end(&mut self) { self.doc_ends += 1; }
        fn on_error(&mut self, error: StreamingParseError) { self.errors.push(error); }
    }

    #[test]
    fn test_basic_parsing() {
        let mut parser = StreamingParser::new(CollectHandler::new());
        parser.process_chunk(b"name = \"Alice\"\nage = ##30\n");
        let h = parser.finish();
        assert_eq!(h.assignments.len(), 2);
        assert_eq!(h.assignments[0].0, "name");
        assert_eq!(h.assignments[1].0, "age");
        assert_eq!(h.doc_starts, 1);
        assert_eq!(h.doc_ends, 1);
    }

    #[test]
    fn test_headers() {
        let mut parser = StreamingParser::new(CollectHandler::new());
        parser.process_chunk(b"{person}\nname = \"Alice\"\n{address}\ncity = \"NYC\"\n");
        let h = parser.finish();
        assert_eq!(h.headers, vec!["person", "address"]);
        assert_eq!(h.assignments[0].0, "person.name");
        assert_eq!(h.assignments[1].0, "address.city");
    }

    #[test]
    fn test_meta_headers() {
        let mut parser = StreamingParser::new(CollectHandler::new());
        parser.process_chunk(b"{$}\nodin = \"1.0.0\"\n{$target}\nformat = \"json\"\n");
        let h = parser.finish();
        assert_eq!(h.headers, vec!["$", "$.target"]);
        assert_eq!(h.assignments[0].0, "$.odin");
        assert_eq!(h.assignments[1].0, "$.target.format");
    }

    #[test]
    fn test_chunked_input() {
        let mut parser = StreamingParser::new(CollectHandler::new());
        parser.process_chunk(b"na");
        parser.process_chunk(b"me = \"Al");
        parser.process_chunk(b"ice\"\nage = ##30\n");
        let h = parser.finish();
        assert_eq!(h.assignments.len(), 2);
    }

    #[test]
    fn test_all_value_types() {
        let input = b"{types}\n\
            s = \"hello\"\n\
            n = #3.14\n\
            i = ##42\n\
            c = #$99.99\n\
            p = #%0.15\n\
            b = ?true\n\
            null_val = ~\n\
            ref_val = @types.s\n\
            d = 2024-12-15\n\
            ts = 2024-12-15T10:30:00Z\n\
            t = T14:30:00\n\
            dur = P1Y2M3D\n";
        let mut parser = StreamingParser::new(CollectHandler::new());
        parser.process_chunk(input);
        let h = parser.finish();
        assert_eq!(h.assignments.len(), 12);
    }

    #[test]
    fn test_comments_and_empty_lines() {
        let mut parser = StreamingParser::new(CollectHandler::new());
        parser.process_chunk(b"; comment\n\nname = \"Alice\" ; inline comment\n\n");
        let h = parser.finish();
        assert_eq!(h.assignments.len(), 1);
    }

    #[test]
    fn test_multiple_documents() {
        let mut parser = StreamingParser::new(CollectHandler::new());
        parser.process_chunk(b"a = ##1\n---\nb = ##2\n");
        let h = parser.finish();
        assert_eq!(h.doc_starts, 2);
        assert_eq!(h.doc_ends, 2);
        assert_eq!(h.assignments.len(), 2);
    }

    #[test]
    fn test_utf8_boundary() {
        // Split a 3-byte UTF-8 char across chunks
        let full = "name = \"你好\"\n";
        let bytes = full.as_bytes();
        let mut parser = StreamingParser::new(CollectHandler::new());
        // Split in middle of 你 (3 bytes: E4 BD A0)
        parser.process_chunk(&bytes[..9]); // "name = \"" + first byte of 你
        parser.process_chunk(&bytes[9..]);
        let h = parser.finish();
        assert_eq!(h.assignments.len(), 1);
        match &h.assignments[0].1 {
            OdinValue::String { value, .. } => assert_eq!(value, "你好"),
            _ => panic!("Expected string"),
        }
    }

    #[test]
    fn test_modifiers() {
        let mut parser = StreamingParser::new(CollectHandler::new());
        parser.process_chunk(b"req = !\"required\"\nconf = *\"confidential\"\n");
        let h = parser.finish();
        assert_eq!(h.assignments.len(), 2);
        match &h.assignments[0].1 {
            OdinValue::String { modifiers: Some(m), .. } => assert!(m.required),
            _ => panic!("Expected string with modifiers"),
        }
    }

    #[test]
    fn test_currency_with_code() {
        let mut parser = StreamingParser::new(CollectHandler::new());
        parser.process_chunk(b"amount = #$250.00:USD\n");
        let h = parser.finish();
        match &h.assignments[0].1 {
            OdinValue::Currency { raw, currency_code, .. } => {
                assert_eq!(raw.as_deref(), Some("250.00:USD"));
                assert_eq!(currency_code.as_deref(), Some("USD"));
            }
            _ => panic!("Expected currency"),
        }
    }
}
