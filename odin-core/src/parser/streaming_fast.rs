//! Streaming fast-path parser for the common ODIN shape.
//!
//! Walks the source bytes once, building the `OdinDocument` directly without
//! materializing a `Vec<Token>`. Bails (returns `None`) the moment it sees
//! any feature outside its supported subset — the caller then falls back to
//! the regular tokenize-then-parse pipeline.
//!
//! Supported subset (what large.odin and similar scalar-heavy docs use):
//! - `{$}` metadata header
//! - `{section}`, `{section.sub}`, `{.relative}` headers (no brackets, no `:`)
//! - `; line comments` and blank lines
//! - `path = value` assignments where `path` is dotted identifiers with
//!   optional `[N]` indices, and `value` is one of:
//!   string (no escapes), `##integer`, `#number`, `#$currency[:CODE]`,
//!   `#%percent`, `?true`/`?false`, `true`/`false`, `~`, `@reference`,
//!   `YYYY-MM-DD`, `YYYY-MM-DDT…` timestamps, `T…` times, `P…` durations.
//!
//! Bails on: modifiers (`!`/`*`/`-`), directives (`:foo`), escape sequences,
//! multi-line strings, verbs (`%`), binary (`^`), `@import`/`@schema`/`@if`,
//! tabular (`[] :`), `$table.` headers, document separators (`---`),
//! conditionals.

use rustc_hash::FxHashMap;

use crate::types::document::OdinDocument;
use crate::types::errors::{ParseError, ParseErrorCode};
use crate::types::ordered_map::OrderedMap;
use crate::types::options::ParseOptions;
use crate::types::values::{OdinValue, OdinValues};

const MAX_ARRAY_INDEX: i64 = 1_000_000;

pub(super) fn try_parse_fast(source: &str, options: &ParseOptions) -> Option<Result<OdinDocument, ParseError>> {
    if source.len() > options.max_size {
        return Some(Err(ParseError::new(ParseErrorCode::MaximumDocumentSizeExceeded, 1, 1)));
    }
    if source.len() > u32::MAX as usize {
        return Some(Err(ParseError::new(ParseErrorCode::MaximumDocumentSizeExceeded, 1, 1)));
    }
    // Upfront scan for bail markers — much cheaper than partially parsing
    // and then bailing mid-stream (which throws away the partial work and
    // forces the full parser to re-parse from byte 0).
    if !is_fast_path_eligible(source.as_bytes()) {
        return None;
    }
    let p = FastParser::new(source, options);
    p.run()
}

/// Quick byte-pattern scan for features that the fast path doesn't handle.
/// Conservative: any false positive just falls through to the regular parser.
fn is_fast_path_eligible(bytes: &[u8]) -> bool {
    if memchr::memmem::find(bytes, b"[]").is_some() { return false; }
    if memchr::memmem::find(bytes, b"$table").is_some() { return false; }
    if memchr::memmem::find(bytes, b"\n---").is_some() { return false; }
    if memchr::memmem::find(bytes, b"@import").is_some() { return false; }
    if memchr::memmem::find(bytes, b"@schema").is_some() { return false; }
    if memchr::memmem::find(bytes, b"@if ").is_some() { return false; }
    if memchr::memchr(b'\\', bytes).is_some() { return false; }
    if memchr::memchr(b'^', bytes).is_some() { return false; }
    true
}

struct FastParser<'a> {
    source: &'a str,
    bytes: &'a [u8],
    pos: usize,
    line: u32,
    line_start: usize,
    options: &'a ParseOptions,
    metadata: OrderedMap<String, OdinValue>,
    assignments: OrderedMap<String, OdinValue>,
    array_indices: FxHashMap<String, usize>,
    current_header: Option<String>,
    previous_header: Option<String>,
    in_metadata: bool,
    path_buf: String,
    norm_buf: String,
}

impl<'a> FastParser<'a> {
    fn new(source: &'a str, options: &'a ParseOptions) -> Self {
        let est_paths = source.len() / 28 + 16;
        Self {
            source,
            bytes: source.as_bytes(),
            pos: 0,
            line: 1,
            line_start: 0,
            options,
            metadata: OrderedMap::with_capacity(est_paths.min(32)),
            assignments: OrderedMap::with_capacity(est_paths),
            array_indices: FxHashMap::default(),
            current_header: None,
            previous_header: None,
            in_metadata: false,
            path_buf: String::with_capacity(64),
            norm_buf: String::with_capacity(64),
        }
    }

    #[inline]
    fn col(&self) -> u32 {
        (self.pos - self.line_start) as u32 + 1
    }

    fn run(mut self) -> Option<Result<OdinDocument, ParseError>> {
        loop {
            if !self.skip_blanks_and_comments() { break; }
            let b = self.bytes[self.pos];
            match b {
                b'{' => match self.parse_header() {
                    FastResult::Ok => {}
                    FastResult::Bail => return None,
                    FastResult::Err(e) => return Some(Err(e)),
                },
                b'-' if self.peek_eq(b"---") => return None,
                b'@' => return None,
                _ => match self.parse_assignment() {
                    FastResult::Ok => {}
                    FastResult::Bail => return None,
                    FastResult::Err(e) => return Some(Err(e)),
                },
            }
        }
        Some(Ok(OdinDocument {
            metadata: self.metadata,
            assignments: self.assignments,
            modifiers: OrderedMap::new(),
            imports: Vec::new(),
            schemas: Vec::new(),
            conditionals: Vec::new(),
            comments: Vec::new(),
        }))
    }

    /// Advance through whitespace, blank lines, and `;` line comments.
    /// Two consecutive newlines inside the root `{$}` metadata section exit
    /// metadata mode (matches parser_impl semantics). Returns `false` on EOF.
    fn skip_blanks_and_comments(&mut self) -> bool {
        let mut newlines_in_a_row: u32 = 0;
        while self.pos < self.bytes.len() {
            match self.bytes[self.pos] {
                b' ' | b'\t' | b'\r' => { self.pos += 1; }
                b'\n' => {
                    self.pos += 1;
                    self.line += 1;
                    self.line_start = self.pos;
                    newlines_in_a_row += 1;
                    if newlines_in_a_row >= 2 && self.in_metadata && self.current_header.is_none() {
                        self.in_metadata = false;
                    }
                }
                b';' => {
                    self.pos = match memchr::memchr(b'\n', &self.bytes[self.pos..]) {
                        Some(off) => self.pos + off,
                        None => self.bytes.len(),
                    };
                }
                _ => return true,
            }
        }
        false
    }

    #[inline]
    fn peek_eq(&self, needle: &[u8]) -> bool {
        self.bytes[self.pos..].starts_with(needle)
    }

    fn parse_header(&mut self) -> FastResult {
        let start_line = self.line;
        let start_col = self.col();
        debug_assert_eq!(self.bytes[self.pos], b'{');
        self.pos += 1;
        let content_start = self.pos;

        // Find `}` on the same line. Bail on multi-line headers (rare).
        let close = match memchr::memchr2(b'}', b'\n', &self.bytes[self.pos..]) {
            Some(off) => off,
            None => return FastResult::Bail,
        };
        let close_pos = self.pos + close;
        if self.bytes[close_pos] != b'}' { return FastResult::Bail; }
        let header = &self.source[content_start..close_pos];

        // Bail on tabular / table-definition headers — too complex for fast path.
        if header.starts_with("$table.") || header.starts_with("$.table.") {
            return FastResult::Bail;
        }
        if header.contains("[]") || header.contains(" : ") {
            return FastResult::Bail;
        }
        if header.contains('[') || header.contains(']') {
            return FastResult::Bail;
        }

        self.pos = close_pos + 1;

        if header == "$" {
            self.in_metadata = true;
            self.current_header = None;
        } else if let Some(rest) = header.strip_prefix('$') {
            // Named metadata `{$const}`, `{$accumulator}`, ...
            self.in_metadata = true;
            self.current_header = Some(rest.to_string());
        } else if header.starts_with('@') {
            // `{@TypeRef}` — bail; handled by full parser.
            return FastResult::Bail;
        } else if let Some(rel) = header.strip_prefix('.') {
            // `{.relative}` — resolve against previous absolute header.
            self.in_metadata = false;
            let resolved = match (&self.previous_header, rel.is_empty()) {
                (Some(base), false) => format!("{base}.{rel}"),
                (Some(base), true) => base.clone(),
                (None, false) => rel.to_string(),
                (None, true) => String::new(),
            };
            self.current_header = Some(resolved);
        } else if header.is_empty() {
            // `{}` resets section.
            self.in_metadata = false;
            self.current_header = None;
            self.previous_header = None;
        } else {
            self.in_metadata = false;
            self.current_header = Some(header.to_string());
            self.previous_header = Some(header.to_string());
        }

        // Trailing whitespace/newline after `}`.
        self.skip_to_line_end(start_line, start_col)
    }

    fn skip_to_line_end(&mut self, _line: u32, _col: u32) -> FastResult {
        while self.pos < self.bytes.len() {
            match self.bytes[self.pos] {
                b' ' | b'\t' | b'\r' => self.pos += 1,
                b';' => {
                    self.pos = match memchr::memchr(b'\n', &self.bytes[self.pos..]) {
                        Some(off) => self.pos + off,
                        None => self.bytes.len(),
                    };
                }
                b'\n' => return FastResult::Ok,
                _ => return FastResult::Bail,
            }
        }
        FastResult::Ok
    }

    fn parse_assignment(&mut self) -> FastResult {
        let path_line = self.line as usize;
        let path_col = self.col() as usize;

        // Read the path identifier. Allowed bytes mirror the tokenizer's
        // identifier scan — alphanumeric, `_`, `.`, `[`, `]`, leading `$`.
        let path_start = self.pos;
        let mut saw_bracket = false;
        let first = self.bytes[self.pos];
        if !is_path_start(first) {
            return FastResult::Bail;
        }
        while self.pos < self.bytes.len() {
            let b = self.bytes[self.pos];
            if is_path_byte(b) {
                if b == b'[' { saw_bracket = true; }
                self.pos += 1;
            } else {
                break;
            }
        }
        let path_end = self.pos;
        let raw_path = &self.source[path_start..path_end];

        // Build full_path into the reusable buffer.
        self.path_buf.clear();
        if let Some(ref h) = self.current_header {
            self.path_buf.push_str(h);
            self.path_buf.push('.');
        }
        self.path_buf.push_str(raw_path);

        // Normalize leading-zero indices `[007]` → `[7]` only when needed.
        if saw_bracket && needs_index_norm(self.path_buf.as_bytes()) {
            self.norm_buf.clear();
            normalize_indices(&self.path_buf, &mut self.norm_buf);
            std::mem::swap(&mut self.path_buf, &mut self.norm_buf);
        }

        // Whitespace then `=`.
        while self.pos < self.bytes.len() && (self.bytes[self.pos] == b' ' || self.bytes[self.pos] == b'\t') {
            self.pos += 1;
        }
        if self.pos >= self.bytes.len() || self.bytes[self.pos] != b'=' {
            return FastResult::Err(ParseError::with_message(
                ParseErrorCode::UnexpectedCharacter,
                path_line, path_col,
                &format!("Expected '=' after '{}'", self.path_buf),
            ));
        }
        self.pos += 1;
        while self.pos < self.bytes.len() && (self.bytes[self.pos] == b' ' || self.bytes[self.pos] == b'\t') {
            self.pos += 1;
        }

        // Validate path (depth + bracket ranges + first-bracket capture).
        match self.validate_path(path_line, path_col) {
            FastResult::Ok => {}
            other => return other,
        }

        // Parse value. Empty value = empty string.
        let value = if self.pos >= self.bytes.len()
            || self.bytes[self.pos] == b'\n'
            || self.bytes[self.pos] == b'\r'
            || self.bytes[self.pos] == b';'
        {
            OdinValues::string("")
        } else {
            match self.parse_value(path_line, path_col) {
                FastValue::Ok(v) => v,
                FastValue::Bail => return FastResult::Bail,
                FastValue::Err(e) => return FastResult::Err(e),
            }
        };

        // Trailing whitespace, comment, or newline. Anything else bails
        // (could be a directive, modifier mid-line, etc.).
        while self.pos < self.bytes.len() {
            match self.bytes[self.pos] {
                b' ' | b'\t' | b'\r' => self.pos += 1,
                b';' => {
                    self.pos = match memchr::memchr(b'\n', &self.bytes[self.pos..]) {
                        Some(off) => self.pos + off,
                        None => self.bytes.len(),
                    };
                }
                b'\n' => break,
                _ => return FastResult::Bail,
            }
        }
        // Leave any trailing `\n` for `skip_blanks_and_comments` to count —
        // that's where the blank-line-exits-metadata logic lives.

        // Insert with entry-API dup detection. Replicates parser_impl semantics
        // (bare-key when path starts with `$.` outside metadata mode). Use
        // clone() rather than mem::take so path_buf retains its capacity
        // across assignments — taking the String would force reallocation
        // on the next push_str.
        let (key, target_is_metadata) = if self.in_metadata {
            (self.path_buf.clone(), true)
        } else if self.path_buf.starts_with("$.") {
            (self.path_buf[2..].to_string(), true)
        } else {
            (self.path_buf.clone(), false)
        };
        let target = if target_is_metadata { &mut self.metadata } else { &mut self.assignments };
        if self.options.allow_duplicates {
            target.insert(key, value);
        } else {
            match target.entry(key) {
                indexmap::map::Entry::Occupied(o) => {
                    return FastResult::Err(ParseError::with_message(
                        ParseErrorCode::DuplicatePathAssignment,
                        path_line, path_col,
                        o.key(),
                    ));
                }
                indexmap::map::Entry::Vacant(e) => { e.insert(value); }
            }
        }
        FastResult::Ok
    }

    fn validate_path(&mut self, line: usize, col: usize) -> FastResult {
        let bytes = self.path_buf.as_bytes();
        let mut depth: usize = 1;
        if memchr::memchr(b'[', bytes).is_none() {
            depth += memchr::memchr_iter(b'.', bytes).count();
            if depth > self.options.max_depth {
                return FastResult::Err(ParseError::with_message(
                    ParseErrorCode::MaximumDepthExceeded,
                    line, col,
                    &format!("Maximum nesting depth exceeded: {depth} > {}", self.options.max_depth),
                ));
            }
            return FastResult::Ok;
        }
        let mut cumulative: i64 = 0;
        let mut first_bracket: Option<(usize, usize, usize)> = None;
        let mut i = 0;
        while i < bytes.len() {
            let b = bytes[i];
            if b == b'.' { depth += 1; i += 1; }
            else if b == b'[' {
                depth += 1;
                let bracket_start = i;
                let idx_start = i + 1;
                let close = bytes[idx_start..].iter().position(|&c| c == b']');
                match close {
                    None => break,
                    Some(off) => {
                        let idx_end = idx_start + off;
                        let idx_slice = &self.path_buf[idx_start..idx_end];
                        // Negative indices like `[-1]` — error early to match
                        // the regular parser's contract.
                        if idx_slice.starts_with('-') {
                            return FastResult::Err(ParseError::with_message(
                                ParseErrorCode::InvalidArrayIndex,
                                line, col,
                                &format!("Negative array index: {idx_slice}"),
                            ));
                        }
                        if !idx_slice.is_empty() && idx_slice.bytes().all(|c| c.is_ascii_digit()) {
                            match idx_slice.parse::<i64>() {
                                Ok(idx) => {
                                    if idx > MAX_ARRAY_INDEX {
                                        return FastResult::Err(ParseError::with_message(
                                            ParseErrorCode::ArrayIndexOutOfRange,
                                            line, col,
                                            &format!("Array index {idx} exceeds maximum allowed value of {MAX_ARRAY_INDEX}"),
                                        ));
                                    }
                                    cumulative += idx;
                                    if cumulative > MAX_ARRAY_INDEX {
                                        return FastResult::Err(ParseError::with_message(
                                            ParseErrorCode::ArrayIndexOutOfRange,
                                            line, col,
                                            &format!("Cumulative array indices exceed maximum allowed value of {MAX_ARRAY_INDEX}"),
                                        ));
                                    }
                                }
                                Err(_) => {
                                    return FastResult::Err(ParseError::with_message(
                                        ParseErrorCode::ArrayIndexOutOfRange,
                                        line, col,
                                        &format!("Array index {idx_slice} exceeds maximum allowed value of {MAX_ARRAY_INDEX}"),
                                    ));
                                }
                            }
                        }
                        if first_bracket.is_none() {
                            first_bracket = Some((bracket_start, idx_start, idx_end));
                        }
                        i = idx_end + 1;
                    }
                }
            } else { i += 1; }
        }
        if depth > self.options.max_depth {
            return FastResult::Err(ParseError::with_message(
                ParseErrorCode::MaximumDepthExceeded,
                line, col,
                &format!("Maximum nesting depth exceeded: {depth} > {}", self.options.max_depth),
            ));
        }
        if let Some((base_end, idx_start, idx_end)) = first_bracket {
            let idx_slice = &self.path_buf[idx_start..idx_end];
            if !idx_slice.is_empty() {
                if let Ok(idx) = idx_slice.parse::<usize>() {
                    let array_base = &self.path_buf[..base_end];
                    if let Some(expected) = self.array_indices.get_mut(array_base) {
                        if idx == *expected { *expected += 1; }
                        else if idx > *expected {
                            return FastResult::Err(ParseError::with_message(
                                ParseErrorCode::NonContiguousArrayIndices,
                                line, col,
                                &format!("Non-contiguous array indices: expected {}, got {idx}", *expected),
                            ));
                        }
                    } else if idx == 0 {
                        self.array_indices.insert(array_base.to_string(), 1);
                    } else {
                        return FastResult::Err(ParseError::with_message(
                            ParseErrorCode::NonContiguousArrayIndices,
                            line, col,
                            &format!("Non-contiguous array indices: expected 0, got {idx}"),
                        ));
                    }
                }
            }
        }
        FastResult::Ok
    }

    fn parse_value(&mut self, line: usize, col: usize) -> FastValue {
        let b = self.bytes[self.pos];
        match b {
            b'"' => self.parse_quoted_string(line, col),
            b'#' => self.parse_hash_value(line, col),
            b'?' => self.parse_boolean_prefix(line, col),
            b'~' => { self.pos += 1; FastValue::Ok(OdinValues::null()) }
            b'@' => self.parse_reference(line, col),
            b't' if self.peek_eq(b"true") && !self.is_value_continuation(4) => {
                self.pos += 4;
                FastValue::Ok(OdinValues::boolean(true))
            }
            b'f' if self.peek_eq(b"false") && !self.is_value_continuation(5) => {
                self.pos += 5;
                FastValue::Ok(OdinValues::boolean(false))
            }
            b'T' => self.parse_time(line, col),
            b'P' => self.parse_duration_or_bail(),
            b'0'..=b'9' => self.parse_date_like(line, col),
            // Modifiers, verbs, binary, directives, etc. → bail to full parser.
            _ => FastValue::Bail,
        }
    }

    fn is_value_continuation(&self, offset: usize) -> bool {
        let p = self.pos + offset;
        p < self.bytes.len() && {
            let b = self.bytes[p];
            !(b == b'\n' || b == b'\r' || b == b' ' || b == b'\t' || b == b';')
        }
    }

    fn parse_quoted_string(&mut self, _line: usize, _col: usize) -> FastValue {
        debug_assert_eq!(self.bytes[self.pos], b'"');
        self.pos += 1;
        let content_start = self.pos;
        match memchr::memchr3(b'"', b'\\', b'\n', &self.bytes[self.pos..]) {
            Some(off) => {
                let p = content_start + off;
                if self.bytes[p] != b'"' {
                    // Escapes or newlines — bail to full parser.
                    return FastValue::Bail;
                }
                let s = &self.source[content_start..p];
                self.pos = p + 1;
                FastValue::Ok(OdinValues::string(s))
            }
            None => FastValue::Bail, // unterminated; let full parser produce the error
        }
    }

    fn parse_hash_value(&mut self, line: usize, col: usize) -> FastValue {
        debug_assert_eq!(self.bytes[self.pos], b'#');
        let next = self.bytes.get(self.pos + 1).copied();
        match next {
            Some(b'#') => {
                self.pos += 2;
                self.parse_typed_numeric(line, col, NumKind::Integer)
            }
            Some(b'$') => {
                self.pos += 2;
                self.parse_typed_numeric(line, col, NumKind::Currency)
            }
            Some(b'%') => {
                self.pos += 2;
                self.parse_typed_numeric(line, col, NumKind::Percent)
            }
            Some(_) => {
                self.pos += 1;
                self.parse_typed_numeric(line, col, NumKind::Number)
            }
            None => FastValue::Err(ParseError::with_message(
                ParseErrorCode::InvalidTypePrefix,
                line, col,
                "empty number after '#'",
            )),
        }
    }

    fn parse_typed_numeric(&mut self, line: usize, col: usize, kind: NumKind) -> FastValue {
        let val_start = self.pos;
        // Match the tokenizer's scan_number_inline: optional leading `-`,
        // then digits / `.` / exponent. Refusing letters here lets
        // `##abc` fall through to parse_integer's empty-value error,
        // matching the regular pipeline.
        if self.bytes.get(self.pos).copied() == Some(b'-') {
            self.pos += 1;
        }
        while self.pos < self.bytes.len() {
            match self.bytes[self.pos] {
                b'0'..=b'9' | b'.' | b'e' | b'E' | b'+' | b'-' => self.pos += 1,
                _ => break,
            }
        }
        // Currency may carry a trailing `:CODE` (e.g. `#$100:USD`).
        if matches!(kind, NumKind::Currency) && self.bytes.get(self.pos).copied() == Some(b':') {
            self.pos += 1;
            while self.pos < self.bytes.len() {
                match self.bytes[self.pos] {
                    b'A'..=b'Z' | b'a'..=b'z' => self.pos += 1,
                    _ => break,
                }
            }
        }
        let raw = &self.source[val_start..self.pos];
        match kind {
            NumKind::Integer => match super::parse_values::parse_integer(raw, line, col) {
                Ok(v) => FastValue::Ok(v),
                Err(e) => FastValue::Err(e),
            },
            NumKind::Number => match super::parse_values::parse_number(raw, line, col) {
                Ok(v) => FastValue::Ok(v),
                Err(e) => FastValue::Err(e),
            },
            NumKind::Currency => match super::parse_values::parse_currency(raw, line, col) {
                Ok(v) => FastValue::Ok(v),
                Err(e) => FastValue::Err(e),
            },
            NumKind::Percent => match super::parse_values::parse_percent(raw, line, col) {
                Ok(v) => FastValue::Ok(v),
                Err(e) => FastValue::Err(e),
            },
        }
    }

    fn parse_boolean_prefix(&mut self, _line: usize, _col: usize) -> FastValue {
        debug_assert_eq!(self.bytes[self.pos], b'?');
        self.pos += 1;
        if self.peek_eq(b"true") && !self.is_value_continuation(4) {
            self.pos += 4;
            FastValue::Ok(OdinValues::boolean(true))
        } else if self.peek_eq(b"false") && !self.is_value_continuation(5) {
            self.pos += 5;
            FastValue::Ok(OdinValues::boolean(false))
        } else {
            FastValue::Ok(OdinValues::boolean(true))
        }
    }

    fn parse_reference(&mut self, _line: usize, _col: usize) -> FastValue {
        debug_assert_eq!(self.bytes[self.pos], b'@');
        self.pos += 1;
        let start = self.pos;
        while self.pos < self.bytes.len() {
            let b = self.bytes[self.pos];
            if matches!(b, b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_' | b'.' | b'[' | b']' | b'$' | b':' | b'-' | b'@') {
                self.pos += 1;
            } else { break; }
        }
        let raw = &self.source[start..self.pos];
        let normalized = if needs_index_norm(raw.as_bytes()) {
            let mut buf = String::with_capacity(raw.len());
            normalize_indices(raw, &mut buf);
            buf
        } else {
            raw.to_string()
        };
        FastValue::Ok(OdinValues::reference(&normalized))
    }

    fn parse_date_like(&mut self, line: usize, col: usize) -> FastValue {
        let start = self.pos;
        while self.pos < self.bytes.len() {
            match self.bytes[self.pos] {
                b'\n' | b'\r' | b' ' | b'\t' | b';' => break,
                _ => self.pos += 1,
            }
        }
        let raw = &self.source[start..self.pos];
        if raw.contains('T') {
            FastValue::Ok(OdinValues::timestamp(0, raw))
        } else {
            match super::parse_values::parse_date_value(raw, line, col) {
                Ok(v) => FastValue::Ok(v),
                Err(e) => FastValue::Err(e),
            }
        }
    }

    fn parse_time(&mut self, _line: usize, _col: usize) -> FastValue {
        let start = self.pos;
        while self.pos < self.bytes.len() {
            match self.bytes[self.pos] {
                b'0'..=b'9' | b'T' | b':' | b'.' => self.pos += 1,
                _ => break,
            }
        }
        let raw = &self.source[start..self.pos];
        FastValue::Ok(OdinValues::time(raw))
    }

    fn parse_duration_or_bail(&mut self) -> FastValue {
        // Only handle simple `P…` durations (`P1Y6M`, `PT4H`, …). Bail if it
        // doesn't look like a duration.
        if self.bytes.get(self.pos + 1).copied().is_none_or(|b| !(b.is_ascii_digit() || b == b'T')) {
            return FastValue::Bail;
        }
        let start = self.pos;
        while self.pos < self.bytes.len() {
            match self.bytes[self.pos] {
                b'\n' | b'\r' | b' ' | b'\t' | b';' => break,
                _ => self.pos += 1,
            }
        }
        FastValue::Ok(OdinValues::duration(&self.source[start..self.pos]))
    }
}

#[derive(Debug)]
enum FastResult { Ok, Bail, Err(ParseError) }

#[derive(Debug)]
enum FastValue { Ok(OdinValue), Bail, Err(ParseError) }

#[derive(Debug, Clone, Copy)]
enum NumKind { Integer, Number, Currency, Percent }

#[inline]
fn is_path_start(b: u8) -> bool {
    matches!(b, b'a'..=b'z' | b'A'..=b'Z' | b'_' | b'$')
}

#[inline]
fn is_path_byte(b: u8) -> bool {
    matches!(b,
        b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_' | b'.' | b'[' | b']' | b'$' | b'-')
}

#[inline]
fn needs_index_norm(bytes: &[u8]) -> bool {
    let mut i = 0;
    while i + 2 < bytes.len() {
        if bytes[i] == b'[' && bytes[i + 1] == b'0' && bytes[i + 2].is_ascii_digit() {
            return true;
        }
        i += 1;
    }
    false
}

fn normalize_indices(path: &str, out: &mut String) {
    let bytes = path.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'[' {
            out.push('[');
            i += 1;
            let start = i;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
            if i > start && i < bytes.len() && bytes[i] == b']' {
                match path[start..i].parse::<i64>() {
                    Ok(idx) => out.push_str(&idx.to_string()),
                    Err(_) => out.push_str(&path[start..i]),
                }
            } else {
                out.push_str(&path[start..i]);
            }
        } else {
            let run_end = bytes[i..].iter().position(|&b| b == b'[').map(|p| i + p).unwrap_or(bytes.len());
            out.push_str(&path[i..run_end]);
            i = run_end;
        }
    }
}
