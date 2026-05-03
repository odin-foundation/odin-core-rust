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

use crate::types::document::{OdinComment, OdinDocument, OdinImport, OdinSchema};
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
    let p = FastParser::new(source, options);
    p.run()
}

pub(super) fn parse_documents(source: &str, options: &ParseOptions) -> Result<Vec<OdinDocument>, ParseError> {
    if source.len() > options.max_size {
        return Err(ParseError::new(ParseErrorCode::MaximumDocumentSizeExceeded, 1, 1));
    }
    if source.len() > u32::MAX as usize {
        return Err(ParseError::new(ParseErrorCode::MaximumDocumentSizeExceeded, 1, 1));
    }
    let p = FastParser::new(source, options);
    p.run_documents()
}

fn empty_doc() -> OdinDocument {
    OdinDocument {
        metadata: OrderedMap::new(),
        assignments: OrderedMap::new(),
        modifiers: OrderedMap::new(),
        imports: Vec::new(),
        schemas: Vec::new(),
        conditionals: Vec::new(),
        comments: Vec::new(),
    }
}

/// Generic fallback when an internal Bail reaches the top-level run loop.
/// Most bail conditions should produce a more specific error closer to the
/// failure point — this is the catch-all for anything that hasn't been
/// converted yet.
fn bail_to_err(line: usize, col: usize) -> ParseError {
    ParseError::with_message(
        ParseErrorCode::UnexpectedCharacter,
        line, col,
        "Unexpected input",
    )
}

fn find_comment_start_quote_aware(s: &str) -> Option<usize> {
    let mut in_quotes = false;
    for (i, ch) in s.char_indices() {
        match ch {
            '"' => in_quotes = !in_quotes,
            ';' if !in_quotes => return Some(i),
            _ => {}
        }
    }
    None
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
    imports: Vec<OdinImport>,
    schemas: Vec<OdinSchema>,
    comments: Vec<OdinComment>,
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
    is_primitive: bool,
    /// When true, rows are written to `metadata` with key `table.NAME[i].col`
    /// (lookup-table form `{$table.NAME[col1, col2]}`). When false, rows are
    /// written to `assignments` with key `name[i].col` (record-shaped tabular).
    is_metadata_table: bool,
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
            imports: Vec::new(),
            schemas: Vec::new(),
            comments: Vec::new(),
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

    fn run(self) -> Option<Result<OdinDocument, ParseError>> {
        match self.run_loop(false) {
            Ok(mut docs) => Some(Ok(docs.pop().unwrap_or_else(empty_doc))),
            Err(e) => Some(Err(e)),
        }
    }

    fn run_documents(self) -> Result<Vec<OdinDocument>, ParseError> {
        let docs = self.run_loop(true)?;
        Ok(if docs.is_empty() { vec![empty_doc()] } else { docs })
    }

    fn run_loop(mut self, keep_all: bool) -> Result<Vec<OdinDocument>, ParseError> {
        let mut docs: Vec<OdinDocument> = Vec::new();
        loop {
            if !self.skip_blanks_and_comments() { break; }
            let b = self.bytes[self.pos];
            let line = self.line as usize;
            let col = self.col() as usize;
            // A new header (or EOF) closes any active tabular section.
            if self.tabular.is_some() && b == b'{' {
                self.tabular = None;
            }
            if self.tabular.is_some() {
                match self.parse_tabular_row() {
                    FastResult::Ok => continue,
                    FastResult::Bail => return Err(bail_to_err(line, col)),
                    FastResult::Err(e) => return Err(e),
                }
            }
            match b {
                b'{' => match self.parse_header() {
                    FastResult::Ok => {}
                    FastResult::Bail => return Err(bail_to_err(line, col)),
                    FastResult::Err(e) => return Err(e),
                },
                b'-' if self.peek_eq(b"---") => {
                    self.pos += 3;
                    while self.pos < self.bytes.len() && self.bytes[self.pos] != b'\n' {
                        self.pos += 1;
                    }
                    if keep_all {
                        docs.push(self.snapshot_doc());
                    }
                    self.reset_doc_state();
                    continue;
                }
                b'@' => match self.parse_top_directive() {
                    FastResult::Ok => {}
                    FastResult::Bail => return Err(bail_to_err(line, col)),
                    FastResult::Err(e) => return Err(e),
                },
                _ => match self.parse_assignment() {
                    FastResult::Ok => {}
                    FastResult::Bail => return Err(bail_to_err(line, col)),
                    FastResult::Err(e) => return Err(e),
                },
            }
        }
        docs.push(self.into_doc());
        Ok(docs)
    }

    fn snapshot_doc(&mut self) -> OdinDocument {
        OdinDocument {
            metadata: std::mem::take(&mut self.metadata),
            assignments: std::mem::take(&mut self.assignments),
            modifiers: std::mem::take(&mut self.modifiers_map),
            imports: std::mem::take(&mut self.imports),
            schemas: std::mem::take(&mut self.schemas),
            conditionals: Vec::new(),
            comments: std::mem::take(&mut self.comments),
        }
    }

    fn into_doc(self) -> OdinDocument {
        OdinDocument {
            metadata: self.metadata,
            assignments: self.assignments,
            modifiers: self.modifiers_map,
            imports: self.imports,
            schemas: self.schemas,
            conditionals: Vec::new(),
            comments: self.comments,
        }
    }

    fn reset_doc_state(&mut self) {
        self.metadata.clear();
        self.assignments.clear();
        self.modifiers_map.clear();
        self.array_indices.clear();
        self.imports.clear();
        self.schemas.clear();
        self.comments.clear();
        self.current_header = None;
        self.previous_header = None;
        self.in_metadata = false;
        self.tabular = None;
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
                b';' => self.skip_comment(),
                _ => return true,
            }
        }
        false
    }

    #[inline]
    fn peek_eq(&self, needle: &[u8]) -> bool {
        self.bytes[self.pos..].starts_with(needle)
    }

    /// Skip past a `;` line comment. When `preserve_comments` is enabled,
    /// capture it. Position must be on `;`; advances to (but not past) `\n`.
    fn skip_comment(&mut self) {
        debug_assert_eq!(self.bytes[self.pos], b';');
        let end = match memchr::memchr(b'\n', &self.bytes[self.pos..]) {
            Some(off) => self.pos + off,
            None => self.bytes.len(),
        };
        if self.options.preserve_comments {
            let raw = &self.source[self.pos..end];
            let stripped = raw.strip_prefix(';').unwrap_or(raw);
            let text = stripped.strip_prefix(' ').unwrap_or(stripped).to_string();
            self.comments.push(OdinComment {
                text,
                associated_path: None,
                line: self.line as usize,
            });
        }
        self.pos = end;
    }

    fn parse_header(&mut self) -> FastResult {
        let start_line = self.line;
        let start_col = self.col();
        debug_assert_eq!(self.bytes[self.pos], b'{');
        self.pos += 1;
        let content_start = self.pos;

        // Headers must close on the same line.
        let close = match memchr::memchr2(b'}', b'\n', &self.bytes[self.pos..]) {
            Some(off) => off,
            None => {
                return FastResult::Err(ParseError::with_message(
                    ParseErrorCode::InvalidHeaderSyntax,
                    start_line as usize, start_col as usize,
                    "Unclosed section header",
                ));
            }
        };
        let close_pos = self.pos + close;
        if self.bytes[close_pos] != b'}' {
            return FastResult::Err(ParseError::with_message(
                ParseErrorCode::InvalidHeaderSyntax,
                start_line as usize, start_col as usize,
                "Unclosed section header",
            ));
        }
        let header = &self.source[content_start..close_pos];

        // Tabular header `{name[] : col1, col2, ...}` (absolute or relative).
        if let Some(colon_pos) = header.find(" : ") {
            let name_part = &header[..colon_pos];
            let cols_str = &header[colon_pos + 3..];
            // Must end with `[]` after the name.
            let Some(name) = name_part.strip_suffix("[]") else { return FastResult::Bail; };
            if name.is_empty() { return FastResult::Bail; }

            // Resolve relative base `.name` against previous absolute header.
            let resolved_base = if let Some(rest) = name.strip_prefix('.') {
                if rest.is_empty() { return FastResult::Bail; }
                match &self.previous_header {
                    Some(base) => format!("{base}.{rest}"),
                    None => rest.to_string(),
                }
            } else {
                name.to_string()
            };

            // Primitive array mode: `{name[] : ~}` — single sentinel column `~`.
            let is_primitive = cols_str.trim() == "~";
            let columns = if is_primitive {
                Vec::new()
            } else {
                // Columns may include `name[N]` (uniform sub-array) and
                // `.name` (relative — inherits parent from prior dotted col).
                let mut cols = Vec::with_capacity(8);
                let mut last_parent = String::new();
                for raw in cols_str.split(',') {
                    let trimmed = raw.trim();
                    if trimmed.is_empty() {
                        return FastResult::Err(ParseError::with_message(
                            ParseErrorCode::InvalidHeaderSyntax,
                            start_line as usize, start_col as usize,
                            "Empty column name in tabular header",
                        ));
                    }
                    if let Some(rest) = trimmed.strip_prefix('.') {
                        if last_parent.is_empty() {
                            cols.push(rest.to_string());
                        } else {
                            cols.push(format!("{last_parent}.{rest}"));
                        }
                    } else {
                        if let Some(dot) = trimmed.find('.') {
                            last_parent = trimmed[..dot].to_string();
                        } else {
                            last_parent.clear();
                        }
                        cols.push(trimmed.to_string());
                    }
                }
                if cols.is_empty() {
                    return FastResult::Err(ParseError::with_message(
                        ParseErrorCode::InvalidHeaderSyntax,
                        start_line as usize, start_col as usize,
                        "Tabular header must have at least one column",
                    ));
                }
                cols
            };

            self.pos = close_pos + 1;
            // For relative bases, don't overwrite previous_header — sub-blocks
            // like `{.tags[] : ~}` should scope to their parent record.
            if !name.starts_with('.') {
                self.previous_header = Some(resolved_base.clone());
            }
            self.current_header = None;
            self.in_metadata = false;
            self.tabular = Some(TabularContext {
                base_name: resolved_base,
                columns,
                row_index: 0,
                key_buf: String::with_capacity(64),
                is_primitive,
                is_metadata_table: false,
            });
            return self.skip_to_line_end(start_line, start_col);
        }

        // Lookup-table header `{$table.NAME[col1, col2, ...]}` (also `$.table.NAME`).
        // Body is CSV-shaped; each cell is a fully typed ODIN value (Number,
        // Integer, etc.) — required for %lookup to match typed result columns.
        if header.starts_with("$table.") || header.starts_with("$.table.") {
            let table_part = header
                .strip_prefix("$.")
                .or_else(|| header.strip_prefix('$'))
                .unwrap_or(header);
            let Some(rest) = table_part.strip_prefix("table.") else { return FastResult::Bail; };
            let Some(bracket_pos) = rest.find('[') else { return FastResult::Bail; };
            let Some(close_idx) = rest.find(']') else { return FastResult::Bail; };
            if close_idx <= bracket_pos { return FastResult::Bail; }
            let table_name = &rest[..bracket_pos];
            if table_name.is_empty() { return FastResult::Bail; }
            let cols_str = &rest[bracket_pos + 1..close_idx];
            let mut columns = Vec::with_capacity(8);
            for raw in cols_str.split(',') {
                let trimmed = raw.trim();
                if trimmed.is_empty() { return FastResult::Bail; }
                columns.push(trimmed.to_string());
            }
            if columns.is_empty() { return FastResult::Bail; }

            self.pos = close_pos + 1;
            self.in_metadata = true;
            self.current_header = None;
            self.tabular = Some(TabularContext {
                base_name: format!("table.{table_name}"),
                columns,
                row_index: 0,
                key_buf: String::with_capacity(64),
                is_primitive: false,
                is_metadata_table: true,
            });
            return self.skip_to_line_end(start_line, start_col);
        }
        // Brackets must balance and indices must be parseable. `{records[0]}`
        // is fine; `{invalid[}` is not.
        let mut bracket_depth: i32 = 0;
        let mut in_index = false;
        let mut idx_start = 0usize;
        for (i, b) in header.as_bytes().iter().enumerate() {
            match b {
                b'[' => {
                    if in_index { return FastResult::Bail; }
                    bracket_depth += 1;
                    in_index = true;
                    idx_start = i + 1;
                }
                b']' => {
                    if !in_index { return FastResult::Bail; }
                    bracket_depth -= 1;
                    if bracket_depth < 0 { return FastResult::Bail; }
                    let idx = &header[idx_start..i];
                    if !idx.is_empty() && idx.parse::<i64>().is_err() {
                        return FastResult::Err(ParseError::with_message(
                            ParseErrorCode::InvalidArrayIndex,
                            start_line as usize, start_col as usize,
                            &format!("Invalid array index: {idx}"),
                        ));
                    }
                    in_index = false;
                }
                _ => {}
            }
        }
        if bracket_depth != 0 {
            return FastResult::Err(ParseError::with_message(
                ParseErrorCode::InvalidArrayIndex,
                start_line as usize, start_col as usize,
                "Invalid array index",
            ));
        }

        self.pos = close_pos + 1;

        if header == "$" {
            self.in_metadata = true;
            self.current_header = None;
        } else if let Some(rest) = header.strip_prefix('$') {
            // Named metadata `{$const}`, `{$accumulator}`, ...
            self.in_metadata = true;
            self.current_header = Some(rest.to_string());
        } else if let Some(rest) = header.strip_prefix('@') {
            // `{@TypeRef}` — type-reference section. Body fields land at
            // `TypeRef.field`, matching parser_impl semantics.
            self.in_metadata = false;
            self.current_header = Some(rest.to_string());
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
                b';' => self.skip_comment(),
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
                b';' => self.skip_comment(),
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

    /// Parse a top-level `@import` / `@schema` / `@if` directive. Anything
    /// else after `@` (e.g. `@TypeRef` in unexpected position) bails.
    fn parse_top_directive(&mut self) -> FastResult {
        let line = self.line as usize;
        let col = self.col() as usize;
        debug_assert_eq!(self.bytes[self.pos], b'@');
        self.pos += 1;
        let kw_start = self.pos;
        while self.pos < self.bytes.len() {
            let b = self.bytes[self.pos];
            if matches!(b, b'a'..=b'z' | b'A'..=b'Z') {
                self.pos += 1;
            } else { break; }
        }
        let kw_len = self.pos - kw_start;
        if kw_len == 0 { return FastResult::Bail; }
        let kw = &self.source[kw_start..self.pos];
        let kw_owned = kw.to_string();

        // Whitespace then rest of line.
        while self.pos < self.bytes.len() && (self.bytes[self.pos] == b' ' || self.bytes[self.pos] == b'\t') {
            self.pos += 1;
        }
        let rest_start = self.pos;
        while self.pos < self.bytes.len() && !matches!(self.bytes[self.pos], b'\n' | b'\r') {
            self.pos += 1;
        }
        let raw_rest = self.source[rest_start..self.pos].trim();
        let rest = match find_comment_start_quote_aware(raw_rest) {
            Some(idx) => raw_rest[..idx].trim(),
            None => raw_rest,
        };

        match kw_owned.as_str() {
            "import" => {
                if rest.is_empty() {
                    return FastResult::Err(ParseError::with_message(
                        ParseErrorCode::InvalidDirective, line, col,
                        "Invalid import directive syntax",
                    ));
                }
                if rest.ends_with(" as") {
                    return FastResult::Err(ParseError::with_message(
                        ParseErrorCode::InvalidDirective, line, col,
                        "Import alias requires identifier",
                    ));
                }
                if let Some(as_pos) = rest.find(" as ") {
                    let path = rest[..as_pos].trim().to_string();
                    let alias = rest[as_pos + 4..].trim();
                    if alias.is_empty() {
                        return FastResult::Err(ParseError::with_message(
                            ParseErrorCode::InvalidDirective, line, col,
                            "Import alias requires identifier",
                        ));
                    }
                    self.imports.push(OdinImport { path, alias: Some(alias.to_string()), line });
                } else {
                    self.imports.push(OdinImport { path: rest.to_string(), alias: None, line });
                }
            }
            "schema" => {
                if rest.is_empty() {
                    return FastResult::Err(ParseError::with_message(
                        ParseErrorCode::InvalidDirective, line, col,
                        "Schema directive requires URL",
                    ));
                }
                self.schemas.push(OdinSchema { url: rest.to_string(), line });
            }
            "if" => {
                if rest.is_empty() {
                    return FastResult::Err(ParseError::with_message(
                        ParseErrorCode::InvalidDirective, line, col,
                        "Conditional directive requires expression",
                    ));
                }
                // Validate-only: parser_impl also drops the parsed expression.
            }
            _ => return FastResult::Bail,
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

    /// Handle `field = value` lines mixed into a tabular section. Stores at
    /// `{base_name}[].{field}` and consumes the rest of the line (directives
    /// included). Position must be just past the `=`.
    fn parse_tabular_assignment(
        &mut self,
        ctx: &TabularContext,
        field: &str,
        line: usize,
        col: usize,
    ) -> FastResult {
        while self.pos < self.bytes.len() && (self.bytes[self.pos] == b' ' || self.bytes[self.pos] == b'\t') {
            self.pos += 1;
        }
        let value = match self.parse_value(line, col) {
            FastValue::Ok(v) => v,
            FastValue::Bail => return FastResult::Err(bail_to_err(line, col)),
            FastValue::Err(e) => return FastResult::Err(e),
        };
        // Skip any trailing directives or whitespace until end of line.
        while self.pos < self.bytes.len() {
            match self.bytes[self.pos] {
                b'\n' => break,
                b';' => { self.skip_comment(); break; }
                _ => self.pos += 1,
            }
        }
        let key = format!("{}[].{}", ctx.base_name, field);
        if ctx.is_metadata_table {
            self.metadata.insert(key, value);
        } else {
            self.assignments.insert(key, value);
        }
        FastResult::Ok
    }

    fn parse_tabular_row_inner(&mut self, ctx: &mut TabularContext) -> FastResult {
        let line = self.line as usize;
        let col = self.col() as usize;

        // Detect assignment-style lines mixed into tabular data, e.g.
        // `_loop = "@features"` inside `{features[] : name, enabled}`. These
        // store at `{base_name}[].{field}` and don't advance the row index.
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
                        let field_start = self.pos + id_start;
                        let field_end = self.pos + p;
                        let field = self.source[field_start..field_end].to_string();
                        self.pos += eq_off + 1;
                        return self.parse_tabular_assignment(ctx, &field, line, col);
                    }
                }
            }
        }

        // Build prefix `base[row]` (primitive) or `base[row].` (record).
        ctx.key_buf.clear();
        ctx.key_buf.push_str(&ctx.base_name);
        ctx.key_buf.push('[');
        use std::fmt::Write as _;
        let _ = write!(ctx.key_buf, "{}", ctx.row_index);
        ctx.key_buf.push(']');
        if !ctx.is_primitive {
            ctx.key_buf.push('.');
        }
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
                // Empty cell: `,,` or `, ,` — skip this column without writing
                // an assignment. Matches parser_impl semantics for absent cells.
                b',' if !ctx.is_primitive => {
                    self.pos += 1;
                    col_idx += 1;
                    had_value = true;
                    continue;
                }
                _ => {}
            }

            let value = match self.parse_value(line, col) {
                FastValue::Ok(v) => v,
                FastValue::Bail => return FastResult::Bail,
                FastValue::Err(e) => return FastResult::Err(e),
            };
            had_value = true;

            if ctx.is_primitive {
                if col_idx == 0 {
                    self.assignments.insert(ctx.key_buf.clone(), value);
                }
            } else if col_idx < ctx.columns.len() {
                ctx.key_buf.truncate(prefix_len);
                ctx.key_buf.push_str(&ctx.columns[col_idx]);
                if ctx.is_metadata_table {
                    self.metadata.insert(ctx.key_buf.clone(), value);
                } else {
                    self.assignments.insert(ctx.key_buf.clone(), value);
                }
            }
            col_idx += 1;

            // Optional whitespace, then `,` or end of row
            while self.pos < self.bytes.len() && (self.bytes[self.pos] == b' ' || self.bytes[self.pos] == b'\t') {
                self.pos += 1;
            }
            if self.pos >= self.bytes.len() { break; }
            match self.bytes[self.pos] {
                b',' => {
                    if ctx.is_primitive { return FastResult::Bail; }
                    self.pos += 1;
                }
                b'\n' | b'\r' | b';' => break,
                _ => return FastResult::Bail,
            }
        }

        // Trailing comment / newline.
        while self.pos < self.bytes.len() {
            match self.bytes[self.pos] {
                b' ' | b'\t' | b'\r' => self.pos += 1,
                b';' => self.skip_comment(),
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
            b'%' => self.parse_verb(line, col),
            b'^' => self.parse_binary_value(line, col),
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
            // Letters or other identifier bytes here mean a bare unquoted
            // string was supplied as a value (`name = John`). ODIN requires
            // strings to be quoted.
            b'a'..=b'z' | b'A'..=b'Z' | b'_' => FastValue::Err(ParseError::with_message(
                ParseErrorCode::BareStringNotAllowed,
                line, col,
                "Strings must be quoted",
            )),
            _ => FastValue::Err(ParseError::with_message(
                ParseErrorCode::UnexpectedCharacter,
                line, col,
                &format!("Unexpected character '{}'", b as char),
            )),
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

    fn parse_verb(&mut self, _line: usize, _col: usize) -> FastValue {
        debug_assert_eq!(self.bytes[self.pos], b'%');
        let start = self.pos;
        self.pos += 1; // skip `%`
        let is_custom = self.bytes.get(self.pos).copied() == Some(b'&');
        // Verb expression runs to newline or `;` comment terminator.
        let raw_end = match memchr::memchr2(b'\n', b';', &self.bytes[self.pos..]) {
            Some(off) => self.pos + off,
            None => self.bytes.len(),
        };
        // Trim trailing whitespace.
        let mut trimmed = raw_end;
        while trimmed > self.pos && matches!(self.bytes[trimmed - 1], b' ' | b'\t' | b'\r') {
            trimmed -= 1;
        }
        // Build the canonical raw expression: collapse runs of internal
        // whitespace to a single space (matches parser_impl's token-by-token
        // reconstruction so verb-string equality holds).
        let slice = &self.source[start..trimmed];
        let mut raw_expr = String::with_capacity(slice.len());
        let mut last_was_space = false;
        for ch in slice.chars() {
            let is_space = ch == ' ' || ch == '\t';
            if is_space {
                if !last_was_space { raw_expr.push(' '); }
            } else {
                raw_expr.push(ch);
            }
            last_was_space = is_space;
        }
        self.pos = trimmed;
        FastValue::Ok(OdinValue::Verb {
            verb: raw_expr,
            is_custom,
            args: vec![],
            modifiers: None,
            directives: vec![],
        })
    }

    fn parse_binary_value(&mut self, line: usize, col: usize) -> FastValue {
        debug_assert_eq!(self.bytes[self.pos], b'^');
        self.pos += 1; // skip `^`
        let val_start = self.pos;
        while self.pos < self.bytes.len() {
            match self.bytes[self.pos] {
                b'\n' | b'\r' | b' ' | b'\t' | b';' | b',' => break,
                _ => self.pos += 1,
            }
        }
        let raw = &self.source[val_start..self.pos];
        match super::parse_values::parse_binary(raw, line, col) {
            Ok(v) => FastValue::Ok(v),
            Err(e) => FastValue::Err(e),
        }
    }

    fn parse_duration_or_bail(&mut self) -> FastValue {
        // ISO-8601 durations start with `P` followed by a digit or `T`.
        // Anything else (`P` alone, `Pfoo`) is malformed.
        let line = self.line as usize;
        let col = self.col() as usize;
        if self.bytes.get(self.pos + 1).copied().is_none_or(|b| !(b.is_ascii_digit() || b == b'T')) {
            return FastValue::Err(ParseError::with_message(
                ParseErrorCode::BareStringNotAllowed,
                line, col,
                "Invalid duration format",
            ));
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
    matches!(b, b'a'..=b'z' | b'A'..=b'Z' | b'_' | b'$' | b'&')
}

#[inline]
fn is_path_byte(b: u8) -> bool {
    matches!(b,
        b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_' | b'.' | b'[' | b']' | b'$' | b'-' | b'&')
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
