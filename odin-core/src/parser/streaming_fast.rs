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
use crate::types::values::{DirectiveValue, OdinDirective, OdinModifiers, OdinValue, OdinValues};

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
    if memchr::memmem::find(bytes, b"$table").is_some() { return false; }
    if memchr::memmem::find(bytes, b"\n---").is_some() { return false; }
    if memchr::memmem::find(bytes, b"@import").is_some() { return false; }
    if memchr::memmem::find(bytes, b"@schema").is_some() { return false; }
    if memchr::memmem::find(bytes, b"@if ").is_some() { return false; }
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
    modifiers_map: OrderedMap<String, OdinModifiers>,
    array_indices: FxHashMap<String, usize>,
    current_header: Option<String>,
    previous_header: Option<String>,
    in_metadata: bool,
    path_buf: String,
    norm_buf: String,
    tabular: Option<TabularContext>,
}

/// Active tabular section state. Set by `parse_header` when it encounters
/// `{name[] : col1, col2, ...}`; cleared when the next `{header}` or EOF
/// is reached.
struct TabularContext {
    base_name: String,
    columns: Vec<String>,
    row_index: usize,
    key_buf: String,
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
            modifiers_map: OrderedMap::new(),
            array_indices: FxHashMap::default(),
            current_header: None,
            previous_header: None,
            in_metadata: false,
            path_buf: String::with_capacity(64),
            norm_buf: String::with_capacity(64),
            tabular: None,
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
            // A new header (or EOF) closes any active tabular section.
            if self.tabular.is_some() && b == b'{' {
                self.tabular = None;
            }
            if self.tabular.is_some() {
                match self.parse_tabular_row() {
                    FastResult::Ok => continue,
                    FastResult::Bail => return None,
                    FastResult::Err(e) => return Some(Err(e)),
                }
            }
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
            modifiers: self.modifiers_map,
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

        // Tabular header `{name[] : col1, col2, ...}` (absolute or relative).
        if let Some(colon_pos) = header.find(" : ") {
            let name_part = &header[..colon_pos];
            let cols_str = &header[colon_pos + 3..];
            // Must end with `[]` after the name.
            let Some(name) = name_part.strip_suffix("[]") else { return FastResult::Bail; };
            if name.is_empty() { return FastResult::Bail; }
            // Bail on relative tabular `.name[]` and primitive arrays `~`.
            if name.starts_with('.') || cols_str.trim() == "~" {
                return FastResult::Bail;
            }
            // Parse columns; bail on relative columns or anything containing `.`/`[`.
            let mut columns = Vec::with_capacity(8);
            for raw in cols_str.split(',') {
                let trimmed = raw.trim();
                if trimmed.is_empty() || trimmed.starts_with('.') {
                    return FastResult::Bail;
                }
                if trimmed.contains('[') || trimmed.contains(']') {
                    return FastResult::Bail;
                }
                columns.push(trimmed.to_string());
            }
            if columns.is_empty() { return FastResult::Bail; }

            self.pos = close_pos + 1;
            self.previous_header = Some(name.to_string());
            self.current_header = None;
            self.in_metadata = false;
            self.tabular = Some(TabularContext {
                base_name: name.to_string(),
                columns,
                row_index: 0,
                key_buf: String::with_capacity(64),
            });
            return self.skip_to_line_end(start_line, start_col);
        }

        // Other bracket / colon shapes: bail.
        if header.starts_with("$table.") || header.starts_with("$.table.") {
            return FastResult::Bail;
        }
        if header.contains("[]") {
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

        // Collect modifiers (`!`, `*`, `-`) before the value.
        let mut mods = OdinModifiers::default();
        loop {
            match self.bytes.get(self.pos).copied() {
                Some(b'!') => { mods.required = true; self.pos += 1; }
                Some(b'*') => { mods.confidential = true; self.pos += 1; }
                Some(b'-') => {
                    if self.bytes.get(self.pos + 1).copied() == Some(b'-')
                        && self.bytes.get(self.pos + 2).copied() == Some(b'-')
                    {
                        return FastResult::Bail;
                    }
                    mods.deprecated = true;
                    self.pos += 1;
                }
                _ => break,
            }
        }

        // Parse value. Empty value = empty string (modifiers/directives dropped, matches parser_impl).
        let value = if self.pos >= self.bytes.len()
            || self.bytes[self.pos] == b'\n'
            || self.bytes[self.pos] == b'\r'
            || self.bytes[self.pos] == b';'
        {
            OdinValues::string("")
        } else {
            let mut v = match self.parse_value(path_line, path_col) {
                FastValue::Ok(v) => v,
                FastValue::Bail => return FastResult::Bail,
                FastValue::Err(e) => return FastResult::Err(e),
            };
            if mods.has_any() {
                v = v.with_modifiers(mods.clone());
                self.modifiers_map.insert(self.path_buf.clone(), mods);
            }
            // Trailing directives: `:type integer`, `:format ssn`, ...
            let directives = match self.parse_trailing_directives() {
                Ok(d) => d,
                Err(e) => return FastResult::Err(e),
            };
            if !directives.is_empty() {
                v = v.with_directives(directives);
            }
            v
        };

        // Trailing whitespace, comment, or newline. Anything else bails.
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

    /// Parse a sequence of `:name [value]` trailing directives. Stops at
    /// newline / `;` comment / non-`:` content.
    fn parse_trailing_directives(&mut self) -> Result<Vec<OdinDirective>, ParseError> {
        let mut directives = Vec::new();
        loop {
            while self.pos < self.bytes.len() && (self.bytes[self.pos] == b' ' || self.bytes[self.pos] == b'\t') {
                self.pos += 1;
            }
            if self.pos >= self.bytes.len() { break; }
            if !matches!(self.bytes[self.pos], b':') { break; }
            self.pos += 1;
            let name_start = self.pos;
            while self.pos < self.bytes.len() {
                let b = self.bytes[self.pos];
                if matches!(b, b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_') {
                    self.pos += 1;
                } else { break; }
            }
            if self.pos == name_start {
                return Err(ParseError::with_message(
                    ParseErrorCode::UnexpectedCharacter,
                    self.line as usize, self.col() as usize,
                    "directive name missing",
                ));
            }
            let name = self.source[name_start..self.pos].to_string();
            while self.pos < self.bytes.len() && (self.bytes[self.pos] == b' ' || self.bytes[self.pos] == b'\t') {
                self.pos += 1;
            }
            let dir_value = if self.pos < self.bytes.len() {
                let next = self.bytes[self.pos];
                if matches!(next, b'\n' | b'\r' | b';' | b':') {
                    None
                } else if next == b'"' {
                    // Quoted directive value — parse the same way as a value
                    // string so escapes and quote-stripping are handled.
                    let line = self.line as usize;
                    let col = self.col() as usize;
                    match self.parse_quoted_string(line, col) {
                        FastValue::Ok(OdinValue::String { value, .. }) => {
                            if let Ok(n) = value.parse::<f64>() {
                                Some(DirectiveValue::Number(n))
                            } else {
                                Some(DirectiveValue::String(value))
                            }
                        }
                        FastValue::Err(e) => return Err(e),
                        _ => return Err(ParseError::with_message(
                            ParseErrorCode::UnexpectedCharacter,
                            line, col,
                            "expected quoted directive value",
                        )),
                    }
                } else {
                    let val_start = self.pos;
                    while self.pos < self.bytes.len() {
                        let b = self.bytes[self.pos];
                        if matches!(b, b' ' | b'\t' | b'\n' | b'\r' | b';' | b':') { break; }
                        self.pos += 1;
                    }
                    let v = self.source[val_start..self.pos].to_string();
                    if let Ok(n) = v.parse::<f64>() {
                        Some(DirectiveValue::Number(n))
                    } else {
                        Some(DirectiveValue::String(v))
                    }
                }
            } else {
                None
            };
            directives.push(OdinDirective { name, value: dir_value });
        }
        Ok(directives)
    }

    /// Parse one comma-separated tabular row, generating `name[row].col = val`
    /// assignments. Bails on assignment-style lines (`_loop = "@x"`) or any
    /// non-value content.
    fn parse_tabular_row(&mut self) -> FastResult {
        let mut ctx = self.tabular.take().expect("must be in tabular mode");
        let result = self.parse_tabular_row_inner(&mut ctx);
        if !matches!(result, FastResult::Bail | FastResult::Err(_)) {
            self.tabular = Some(ctx);
        }
        result
    }

    fn parse_tabular_row_inner(&mut self, ctx: &mut TabularContext) -> FastResult {
        let line = self.line as usize;
        let col = self.col() as usize;

        // Bail if this looks like an assignment line (transform docs use this
        // shape for `_loop = "@..."` mixed in with tabular data).
        if let Some(eq_off) = memchr::memchr2(b'=', b'\n', &self.bytes[self.pos..]) {
            if self.bytes[self.pos + eq_off] == b'=' {
                let prefix = &self.bytes[self.pos..self.pos + eq_off];
                let mut p = 0;
                while p < prefix.len() && (prefix[p] == b' ' || prefix[p] == b'\t') { p += 1; }
                let id_start = p;
                while p < prefix.len() && is_path_byte(prefix[p]) { p += 1; }
                if p > id_start {
                    let mut q = p;
                    while q < prefix.len() && (prefix[q] == b' ' || prefix[q] == b'\t') { q += 1; }
                    if q == prefix.len() {
                        // Looks like `<ident>=` — assignment. Bail.
                        return FastResult::Bail;
                    }
                }
            }
        }

        // Build prefix `base[row].`
        ctx.key_buf.clear();
        ctx.key_buf.push_str(&ctx.base_name);
        ctx.key_buf.push('[');
        use std::fmt::Write as _;
        let _ = write!(ctx.key_buf, "{}", ctx.row_index);
        ctx.key_buf.push(']');
        ctx.key_buf.push('.');
        let prefix_len = ctx.key_buf.len();

        let mut col_idx: usize = 0;
        let mut had_value = false;

        loop {
            // Whitespace before value
            while self.pos < self.bytes.len() && (self.bytes[self.pos] == b' ' || self.bytes[self.pos] == b'\t') {
                self.pos += 1;
            }
            if self.pos >= self.bytes.len() { break; }
            match self.bytes[self.pos] {
                b'\n' | b'\r' | b';' => break,
                _ => {}
            }

            let value = match self.parse_value(line, col) {
                FastValue::Ok(v) => v,
                FastValue::Bail => return FastResult::Bail,
                FastValue::Err(e) => return FastResult::Err(e),
            };
            had_value = true;

            if col_idx < ctx.columns.len() {
                ctx.key_buf.truncate(prefix_len);
                ctx.key_buf.push_str(&ctx.columns[col_idx]);
                self.assignments.insert(ctx.key_buf.clone(), value);
            }
            col_idx += 1;

            // Optional whitespace, then `,` or end of row
            while self.pos < self.bytes.len() && (self.bytes[self.pos] == b' ' || self.bytes[self.pos] == b'\t') {
                self.pos += 1;
            }
            if self.pos >= self.bytes.len() { break; }
            match self.bytes[self.pos] {
                b',' => self.pos += 1,
                b'\n' | b'\r' | b';' => break,
                _ => return FastResult::Bail,
            }
        }

        // Trailing comment / newline.
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

        if had_value {
            ctx.row_index += 1;
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
            !(b == b'\n' || b == b'\r' || b == b' ' || b == b'\t' || b == b';' || b == b',')
        }
    }

    fn parse_quoted_string(&mut self, line: usize, col: usize) -> FastValue {
        debug_assert_eq!(self.bytes[self.pos], b'"');
        let start_line = self.line;
        let start_col = self.col();
        self.pos += 1;
        let content_start = self.pos;

        match memchr::memchr3(b'"', b'\\', b'\n', &self.bytes[self.pos..]) {
            Some(off) => {
                let p = content_start + off;
                match self.bytes[p] {
                    b'"' => {
                        // No escapes — slice directly.
                        let s = &self.source[content_start..p];
                        self.pos = p + 1;
                        FastValue::Ok(OdinValues::string(s))
                    }
                    b'\n' => FastValue::Err(ParseError::new(
                        ParseErrorCode::UnterminatedString,
                        start_line as usize, start_col as usize,
                    )),
                    b'\\' => {
                        self.pos = p;
                        self.parse_quoted_string_escaped(content_start, line, col, start_line, start_col)
                    }
                    _ => unreachable!(),
                }
            }
            None => FastValue::Err(ParseError::new(
                ParseErrorCode::UnterminatedString,
                start_line as usize, start_col as usize,
            )),
        }
    }

    /// Slow path: walk to the closing quote validating each escape. Captures
    /// the raw content range and unescapes via parse_values::unescape_string.
    fn parse_quoted_string_escaped(
        &mut self,
        content_start: usize,
        _line: usize,
        _col: usize,
        start_line: u32,
        start_col: u32,
    ) -> FastValue {
        while self.pos < self.bytes.len() {
            let ch = self.bytes[self.pos];
            if ch == b'"' {
                let raw = &self.source[content_start..self.pos];
                self.pos += 1;
                return FastValue::Ok(OdinValues::string(super::parse_values::unescape_string(raw)));
            }
            if ch == b'\n' {
                return FastValue::Err(ParseError::new(
                    ParseErrorCode::UnterminatedString,
                    start_line as usize, start_col as usize,
                ));
            }
            if ch == b'\\' {
                self.pos += 1;
                if self.pos >= self.bytes.len() {
                    return FastValue::Err(ParseError::new(
                        ParseErrorCode::UnterminatedString,
                        start_line as usize, start_col as usize,
                    ));
                }
                let esc = self.bytes[self.pos];
                self.pos += 1;
                match esc {
                    b'n' | b'r' | b't' | b'\\' | b'"' | b'/' | b'0' => {}
                    b'u' => {
                        if let Err(e) = self.consume_unicode_hex(4, start_line, start_col) {
                            return FastValue::Err(e);
                        }
                        // Optional surrogate continuation: \uXXXX (low surrogate)
                        if self.pos + 1 < self.bytes.len()
                            && self.bytes[self.pos] == b'\\'
                            && self.bytes[self.pos + 1] == b'u'
                        {
                            self.pos += 2;
                            if let Err(e) = self.consume_unicode_hex(4, start_line, start_col) {
                                return FastValue::Err(e);
                            }
                        }
                    }
                    b'U' => {
                        if let Err(e) = self.consume_unicode_hex(8, start_line, start_col) {
                            return FastValue::Err(e);
                        }
                    }
                    _ => {
                        return FastValue::Err(ParseError::with_message(
                            ParseErrorCode::InvalidEscapeSequence,
                            self.line as usize, self.col() as usize,
                            &format!("unknown escape: \\{}", esc as char),
                        ));
                    }
                }
            } else {
                self.pos += 1;
            }
        }
        FastValue::Err(ParseError::new(
            ParseErrorCode::UnterminatedString,
            start_line as usize, start_col as usize,
        ))
    }

    fn consume_unicode_hex(&mut self, digits: usize, start_line: u32, start_col: u32) -> Result<(), ParseError> {
        let hex_start = self.pos;
        for _ in 0..digits {
            if self.pos >= self.bytes.len() {
                return Err(ParseError::with_message(
                    ParseErrorCode::InvalidEscapeSequence,
                    start_line as usize, start_col as usize,
                    "incomplete unicode escape",
                ));
            }
            self.pos += 1;
        }
        let hex = &self.source[hex_start..self.pos];
        let code = u32::from_str_radix(hex, 16).map_err(|_| {
            ParseError::with_message(
                ParseErrorCode::InvalidEscapeSequence,
                start_line as usize, start_col as usize,
                &format!("invalid hex in unicode escape: \\u{hex}"),
            )
        })?;
        if char::from_u32(code).is_none() && !(0xD800..=0xDFFF).contains(&code) {
            return Err(ParseError::with_message(
                ParseErrorCode::InvalidEscapeSequence,
                start_line as usize, start_col as usize,
                &format!("invalid unicode code point: U+{code:04X}"),
            ));
        }
        Ok(())
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
                b'\n' | b'\r' | b' ' | b'\t' | b';' | b',' => break,
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
                b'\n' | b'\r' | b' ' | b'\t' | b';' | b',' => break,
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
