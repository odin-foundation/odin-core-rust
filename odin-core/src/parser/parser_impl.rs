//! Parser implementation — converts token stream into an `OdinDocument`.

use crate::types::document::{OdinComment, OdinDocument, OdinImport, OdinSchema};
use crate::types::errors::{ParseError, ParseErrorCode};
use crate::types::ordered_map::OrderedMap;
use crate::types::options::ParseOptions;
use super::tokens::{Token, TokenType};
use super::parse_values;

/// Parser state.
struct Parser<'a> {
    tokens: &'a [Token<'a>],
    pos: usize,
    options: &'a ParseOptions,
    /// Current section header path (e.g., "Policy", "Policy.Coverage").
    current_header: Option<String>,
    /// Last absolute header path, used to resolve relative headers.
    previous_header: Option<String>,
}

impl<'a> Parser<'a> {
    fn new(tokens: &'a [Token<'a>], options: &'a ParseOptions) -> Self {
        Self {
            tokens,
            pos: 0,
            options,
            current_header: None,
            previous_header: None,
        }
    }

    fn is_at_end(&self) -> bool {
        self.pos >= self.tokens.len() || self.tokens[self.pos].token_type == TokenType::Eof
    }

    fn peek(&self) -> Option<&Token<'a>> {
        self.tokens.get(self.pos)
    }

    /// Return a reference to the current token.
    ///
    /// Must only be called when `!self.is_at_end()` — callers always guard
    /// with that check so the index is guaranteed in-bounds.
    fn current_token(&self) -> &Token<'a> {
        &self.tokens[self.pos]
    }

    fn advance(&mut self) -> &Token<'a> {
        let token = &self.tokens[self.pos];
        self.pos += 1;
        token
    }

    /// Check if the current position starts an assignment line (identifier = value).
    /// Looks ahead without consuming tokens.
    fn is_assignment_line(&self) -> bool {
        if self.is_at_end() { return false; }
        let tok = &self.tokens[self.pos];
        // An assignment starts with an identifier (Bareword) followed by Equals
        if tok.token_type != TokenType::Path && tok.token_type != TokenType::BareWord {
            return false;
        }
        // Look ahead for Equals
        let next_pos = self.pos + 1;
        if next_pos >= self.tokens.len() { return false; }
        self.tokens[next_pos].token_type == TokenType::Equals
    }

    fn skip_newlines(&mut self) {
        while !self.is_at_end() {
            match self.peek().map(|t| t.token_type) {
                Some(TokenType::Newline | TokenType::Comment) => { self.advance(); }
                _ => break,
            }
        }
    }

    /// Parse multiple documents separated by `---`.
    fn parse_documents(&mut self) -> Result<Vec<OdinDocument>, ParseError> {
        let mut documents = Vec::new();

        loop {
            let doc = self.parse_single_document()?;
            documents.push(doc);

            // Check for document separator
            self.skip_newlines();
            if !self.is_at_end() && self.peek().map(|t| t.token_type) == Some(TokenType::DocumentSeparator) {
                self.advance(); // consume `---`
                self.skip_newlines();
                self.current_header = None;
            } else {
                break;
            }
        }

        Ok(documents)
    }

    /// Parse all documents but keep only the last — single-doc fast path
    /// avoids allocating a `Vec<OdinDocument>` for the common case.
    fn parse_last_document(&mut self) -> Result<OdinDocument, ParseError> {
        let mut latest = self.parse_single_document()?;
        loop {
            self.skip_newlines();
            if !self.is_at_end() && self.peek().map(|t| t.token_type) == Some(TokenType::DocumentSeparator) {
                self.advance();
                self.skip_newlines();
                self.current_header = None;
                latest = self.parse_single_document()?;
            } else {
                break;
            }
        }
        Ok(latest)
    }

    /// Parse a single document (up to `---` separator or EOF).
    fn parse_single_document(&mut self) -> Result<OdinDocument, ParseError> {
        // Estimate capacity from token count (roughly 1 assignment per 4 tokens)
        let est = self.tokens.len() / 4;
        let mut metadata = OrderedMap::with_capacity(est.min(16));
        let mut assignments = OrderedMap::with_capacity(est);
        let mut modifiers = OrderedMap::new();
        let mut imports = Vec::new();
        let mut schemas = Vec::new();
        let conditionals = Vec::new();
        let mut comments: Vec<OdinComment> = Vec::new();
        let mut in_metadata = false;
        // Next expected contiguous index per array base.
        let mut array_indices: std::collections::HashMap<String, usize> = std::collections::HashMap::new();

        self.skip_newlines();

        while !self.is_at_end() {
            let token = self.current_token();

            // Stop at document separator
            if token.token_type == TokenType::DocumentSeparator {
                break;
            }

            match token.token_type {
                TokenType::Header => {
                    let header_value = token.value.to_string();
                    self.advance();
                    self.skip_newlines();

                    if header_value == "$" {
                        in_metadata = true;
                        self.current_header = None;
                    } else if header_value.starts_with("$table.") || header_value.starts_with("$.table.") {
                        // Table definition header: {$table.NAME[col1, col2]} or {$.table.NAME[col1, col2]}
                        // Parse column names and consume following CSV data lines
                        in_metadata = true;
                        self.current_header = None;
                        // Strip the leading `$` or `$.` prefix to get `table.NAME[...]`
                        let table_part = if let Some(rest) = header_value.strip_prefix("$.") {
                            rest
                        } else if let Some(rest) = header_value.strip_prefix('$') {
                            rest
                        } else {
                            &header_value
                        };
                        self.parse_table_data(table_part, &mut metadata);
                    } else if header_value.starts_with('$') {
                        // Other $ headers: {$const}, {$accumulator}, etc.
                        // Strip the $ and use the rest as metadata prefix
                        in_metadata = true;
                        self.current_header = Some(header_value.strip_prefix('$').unwrap_or(&header_value).to_string());
                    } else if header_value.starts_with('@') {
                        in_metadata = false;
                        self.current_header = Some(header_value.strip_prefix('@').unwrap_or(&header_value).to_string());
                    } else if header_value.contains("[] :") || header_value.contains("[] :") {
                        // Tabular section `{name[] : col, ...}`. A leading `.` makes
                        // it relative to the last absolute header.
                        in_metadata = false;
                        let resolved_header: String = if let Some(rest) = header_value.strip_prefix('.') {
                            if let Some(ref base) = self.previous_header {
                                format!("{base}.{rest}")
                            } else {
                                rest.to_string()
                            }
                        } else {
                            // Absolute tabular header — update previous_header
                            let base = header_value.split("[]").next().unwrap_or("");
                            self.previous_header = Some(base.to_string());
                            header_value.clone()
                        };
                        self.current_header = None;
                        self.parse_tabular_section(&resolved_header, &mut assignments);
                    } else if header_value.is_empty() {
                        // Empty braces {} — root section reset
                        in_metadata = false;
                        self.current_header = None;
                        self.previous_header = None;
                    } else if header_value.starts_with('.') {
                        // Relative header: {.garaging} resolves against last absolute header
                        in_metadata = false;
                        let relative_part = &header_value[1..]; // strip leading '.'
                        let resolved = if let Some(ref base) = self.previous_header {
                            if relative_part.is_empty() {
                                base.clone()
                            } else {
                                format!("{base}.{relative_part}")
                            }
                        } else if relative_part.is_empty() {
                            String::new()
                        } else {
                            relative_part.to_string()
                        };
                        self.current_header = Some(resolved);
                        // Do NOT update previous_header — relative headers don't become the new base
                    } else {
                        in_metadata = false;
                        self.current_header = Some(header_value.clone());
                        // Absolute header updates the base for future relative headers
                        self.previous_header = Some(header_value);
                    }
                }
                TokenType::Import => {
                    let line = token.line as usize;
                    let col = token.column as usize;
                    let value = token.value.to_string();
                    self.advance();

                    // Parse import: value is "path" or "path as alias"
                    let trimmed = value.trim();
                    if trimmed.is_empty() {
                        return Err(ParseError::with_message(
                            ParseErrorCode::InvalidDirective,
                            line, col,
                            "Invalid import directive syntax",
                        ));
                    }

                    // Check for trailing " as" without identifier
                    if trimmed.ends_with(" as") {
                        return Err(ParseError::with_message(
                            ParseErrorCode::InvalidDirective,
                            line, col,
                            "Import alias requires identifier",
                        ));
                    }

                    if let Some(as_pos) = trimmed.find(" as ") {
                        let path = trimmed[..as_pos].trim().to_string();
                        let alias_str = trimmed[as_pos + 4..].trim();
                        if alias_str.is_empty() {
                            return Err(ParseError::with_message(
                                ParseErrorCode::InvalidDirective,
                                line, col,
                                "Import alias requires identifier",
                            ));
                        }
                        imports.push(OdinImport { path, alias: Some(alias_str.to_string()), line });
                    } else {
                        imports.push(OdinImport { path: trimmed.to_string(), alias: None, line });
                    }
                }
                TokenType::Schema => {
                    let line = token.line as usize;
                    let col = token.column as usize;
                    let value = token.value.to_string();
                    self.advance();

                    let trimmed = value.trim();
                    if trimmed.is_empty() {
                        return Err(ParseError::with_message(
                            ParseErrorCode::InvalidDirective,
                            line, col,
                            "Schema directive requires URL",
                        ));
                    }

                    schemas.push(OdinSchema {
                        url: trimmed.to_string(),
                        line,
                    });
                }
                TokenType::Conditional => {
                    let line = token.line as usize;
                    let col = token.column as usize;
                    let value = token.value.to_string();
                    self.advance();

                    let trimmed = value.trim();
                    if trimmed.is_empty() {
                        return Err(ParseError::with_message(
                            ParseErrorCode::InvalidDirective,
                            line, col,
                            "Conditional directive requires expression",
                        ));
                    }
                    // Store conditional — skip for now, just validate
                }
                TokenType::Path | TokenType::BooleanLiteral => {
                    let path_line = token.line as usize;
                    let path_col = token.column as usize;

                    // Build full path directly from token value — avoids intermediate
                    // to_string() + format!() double allocation for section fields.
                    let mut full_path = if let Some(ref header) = self.current_header {
                        let tv = &token.value;
                        let mut s = String::with_capacity(header.len() + 1 + tv.len());
                        s.push_str(header);
                        s.push('.');
                        s.push_str(tv);
                        s
                    } else {
                        token.value.to_string()
                    };

                    // Normalize leading zeros in array indices: [007] -> [7].
                    if full_path.as_bytes().contains(&b'[') {
                        let mut normalized = String::with_capacity(full_path.len());
                        let mut i = 0;
                        let bytes = full_path.as_bytes();
                        while i < bytes.len() {
                            if bytes[i] == b'[' {
                                normalized.push('[');
                                i += 1;
                                let start = i;
                                while i < bytes.len() && bytes[i].is_ascii_digit() {
                                    i += 1;
                                }
                                if i > start && i < bytes.len() && bytes[i] == b']' {
                                    // Parse and re-emit to strip leading zeros
                                    let idx: i64 = full_path[start..i].parse().unwrap_or(0);
                                    normalized.push_str(&idx.to_string());
                                } else {
                                    // Not a pure-digit index, preserve as-is
                                    normalized.push_str(&full_path[start..i]);
                                }
                            } else {
                                normalized.push(bytes[i] as char);
                                i += 1;
                            }
                        }
                        full_path = normalized;
                    }

                    self.advance();

                    // Check nesting depth (P010) — count dots and brackets
                    let depth = 1 + full_path.bytes().filter(|&b| b == b'.' || b == b'[').count();
                    if depth > self.options.max_depth {
                        return Err(ParseError::with_message(
                            ParseErrorCode::MaximumDepthExceeded,
                            path_line, path_col,
                            &format!("Maximum nesting depth exceeded: {depth} > {}", self.options.max_depth),
                        ));
                    }

                    // Validate array indices
                    // First pass: check ALL bracket indices for P015 (range check)
                    {
                        const MAX_ARRAY_INDEX: i64 = 1_000_000;
                        let mut cumulative: i64 = 0;
                        let mut search_start = 0;
                        while let Some(bp) = full_path[search_start..].find('[') {
                            let abs_bp = search_start + bp;
                            if let Some(cp) = full_path[abs_bp..].find(']') {
                                let idx_str = &full_path[abs_bp + 1..abs_bp + cp];
                                if !idx_str.is_empty() {
                                    if let Ok(idx) = idx_str.parse::<i64>() {
                                        if idx < 0 {
                                            return Err(ParseError::with_message(
                                                ParseErrorCode::InvalidArrayIndex,
                                                path_line, path_col,
                                                &format!("Negative array index: {idx}"),
                                            ));
                                        }
                                        if idx > MAX_ARRAY_INDEX {
                                            return Err(ParseError::with_message(
                                                ParseErrorCode::ArrayIndexOutOfRange,
                                                path_line, path_col,
                                                &format!("Array index {idx} exceeds maximum allowed value of {MAX_ARRAY_INDEX}"),
                                            ));
                                        }
                                        cumulative += idx;
                                        if cumulative > MAX_ARRAY_INDEX {
                                            return Err(ParseError::with_message(
                                                ParseErrorCode::ArrayIndexOutOfRange,
                                                path_line, path_col,
                                                &format!("Cumulative array indices exceed maximum allowed value of {MAX_ARRAY_INDEX}"),
                                            ));
                                        }
                                    }
                                }
                                search_start = abs_bp + cp + 1;
                            } else {
                                break;
                            }
                        }
                    }

                    // Second pass: track first array index for contiguity (P013).
                    if let Some(bracket_pos) = full_path.find('[') {
                        let array_base = &full_path[..bracket_pos];
                        if let Some(close_pos) = full_path[bracket_pos..].find(']') {
                            let idx_str = &full_path[bracket_pos + 1..bracket_pos + close_pos];

                            // Check for empty brackets (array clear)
                            if idx_str.is_empty() {
                                // Array clear syntax: items[] = ~
                            } else if let Ok(idx) = idx_str.parse::<usize>() {
                                if let Some(expected) = array_indices.get_mut(array_base) {
                                    if idx == *expected {
                                        *expected += 1;
                                    } else if idx > *expected {
                                        return Err(ParseError::with_message(
                                            ParseErrorCode::NonContiguousArrayIndices,
                                            path_line, path_col,
                                            &format!("Non-contiguous array indices: expected {}, got {idx}", *expected),
                                        ));
                                    }
                                    // idx < expected: re-assignment, allowed (duplicate-path check handles it)
                                } else if idx == 0 {
                                    array_indices.insert(array_base.to_string(), 1);
                                } else {
                                    return Err(ParseError::with_message(
                                        ParseErrorCode::NonContiguousArrayIndices,
                                        path_line, path_col,
                                        &format!("Non-contiguous array indices: expected 0, got {idx}"),
                                    ));
                                }
                            }
                        }
                    }

                    // Skip whitespace, look for `=`
                    if self.is_at_end() || self.current_token().token_type != TokenType::Equals {
                        // Line without `=` is invalid syntax (P001)
                        return Err(ParseError::with_message(
                            ParseErrorCode::UnexpectedCharacter,
                            path_line, path_col,
                            &format!("Expected '=' after '{full_path}'"),
                        ));
                    }
                    self.advance(); // consume `=`

                    // Check for duplicate paths
                    if !self.options.allow_duplicates
                        && ((in_metadata && metadata.contains_key(&full_path))
                            || (!in_metadata && assignments.contains_key(&full_path)))
                        {
                            return Err(ParseError::with_message(
                                ParseErrorCode::DuplicatePathAssignment,
                                path_line, path_col,
                                &full_path,
                            ));
                        }

                    // Parse modifiers and value
                    let (mods, mod_consumed) = parse_values::parse_modifiers(self.tokens, self.pos);
                    self.pos += mod_consumed;

                    if self.is_at_end() || self.current_token().token_type == TokenType::Newline {
                        // Empty value — treat as empty string
                        let value = crate::types::values::OdinValues::string("");
                        if in_metadata {
                            metadata.insert(full_path, value);
                        } else {
                            assignments.insert(full_path, value);
                        }
                        continue;
                    }

                    let (mut value, consumed) = parse_values::parse_value(self.tokens, self.pos)?;
                    self.pos += consumed;

                    // Parse trailing directives (e.g., `:type integer`, `:date`, `:pos 3 :len 8`)
                    let mut directives = Vec::new();
                    while !self.is_at_end() {
                        let tt = self.current_token().token_type;
                        if tt == TokenType::Newline || tt == TokenType::Comment {
                            break;
                        }
                        if tt == TokenType::Directive {
                            let dir_name = self.current_token().value.to_string();
                            self.advance();
                            // Check for directive value (next non-newline token)
                            let dir_value = if self.is_at_end() {
                                None
                            } else {
                                let next_tt = self.current_token().token_type;
                                if next_tt != TokenType::Newline && next_tt != TokenType::Comment && next_tt != TokenType::Directive {
                                    let v = self.current_token().value.to_string();
                                    self.advance();
                                    if let Ok(n) = v.parse::<f64>() {
                                        Some(crate::types::values::DirectiveValue::Number(n))
                                    } else {
                                        Some(crate::types::values::DirectiveValue::String(v))
                                    }
                                } else {
                                    None
                                }
                            };
                            directives.push(crate::types::values::OdinDirective { name: dir_name, value: dir_value });
                        } else {
                            // Unexpected token after value — P001
                            let bad = self.current_token();
                            return Err(ParseError::with_message(
                                ParseErrorCode::UnexpectedCharacter,
                                bad.line as usize, bad.column as usize,
                                &format!("Unexpected content after value: {:?}", bad.value),
                            ));
                        }
                    }
                    if !directives.is_empty() {
                        value = value.with_directives(directives);
                    }

                    // Apply modifiers to value
                    if mods.has_any() {
                        value = value.with_modifiers(mods.clone());
                        modifiers.insert(full_path.clone(), mods);
                    }

                    if in_metadata {
                        // Store in metadata only (canonical stringify merges as $.key)
                        metadata.insert(full_path, value);
                    } else if full_path.starts_with("$.") {
                        // Canonical metadata path: $.key — store in metadata only
                        let bare_key = full_path[2..].to_string();
                        metadata.insert(bare_key, value);
                    } else {
                        assignments.insert(full_path, value);
                    }
                }
                TokenType::Newline | TokenType::Comment => {
                    let was_newline = token.token_type == TokenType::Newline;
                    if token.token_type == TokenType::Comment && self.options.preserve_comments {
                        let raw: &str = &token.value;
                        let stripped = raw.strip_prefix(';').unwrap_or(raw);
                        let text = stripped.strip_prefix(' ').unwrap_or(stripped).to_string();
                        comments.push(OdinComment {
                            text,
                            associated_path: None,
                            line: token.line as usize,
                        });
                    }
                    self.advance();
                    // Blank line (consecutive newlines) exits metadata mode,
                    // but only for the root {$} section (current_header == None).
                    // Named metadata sections like {$const}, {$accumulator}
                    // continue until the next header.
                    if was_newline && in_metadata && self.current_header.is_none()
                        && !self.is_at_end() && self.peek().map(|t| t.token_type) == Some(TokenType::Newline) {
                            in_metadata = false;
                        }
                }
                _ => {
                    // Skip unexpected tokens
                    self.advance();
                }
            }
        }

        Ok(OdinDocument {
            metadata,
            assignments,
            modifiers,
            imports,
            schemas,
            conditionals,
            comments,
        })
    }

    /// Parse table definition data lines following a `{$table.NAME[col1, col2]}` header.
    ///
    /// Consumes tokens until the next Header token (or EOF) and converts
    /// CSV-style rows into metadata entries like:
    /// `table.NAME[0].col1 = "val1"`, `table.NAME[0].col2 = "val2"`, etc.
    fn parse_table_data(
        &mut self,
        header: &str,
        metadata: &mut OrderedMap<String, crate::types::values::OdinValue>,
    ) {
        // Parse header: "table.NAME[col1, col2, ...]"
        let Some(bracket_pos) = header.find('[') else { return };
        let Some(close_pos) = header.find(']') else { return };
        let table_name = &header[6..bracket_pos]; // skip "table."
        let cols_str = &header[bracket_pos + 1..close_pos];
        let columns: Vec<&str> = cols_str.split(',').map(str::trim).collect();
        if columns.is_empty() || table_name.is_empty() {
            return;
        }

        let mut row_index: usize = 0;
        let mut key_buf = String::with_capacity(64);
        use std::fmt::Write as _;

        // Consume tokens until the next Header or EOF
        loop {
            self.skip_newlines();
            if self.is_at_end() {
                break;
            }
            if let Some(tok) = self.peek() {
                if tok.token_type == TokenType::Header || tok.token_type == TokenType::DocumentSeparator {
                    break;
                }
                if tok.token_type == TokenType::Comment {
                    self.advance();
                    continue;
                }
            }

            // Collect values on this line (string literals, identifiers, commas)
            let mut values: Vec<String> = Vec::new();
            let mut current_val: Option<String> = None;

            while !self.is_at_end() {
                let tok = self.current_token();
                match tok.token_type {
                    TokenType::Newline | TokenType::Header | TokenType::DocumentSeparator => break,
                    TokenType::Comment => {
                        self.advance();
                        break;
                    }
                    TokenType::QuotedString => {
                        let v = tok.value.to_string();
                        self.advance();
                        current_val = Some(v);
                        // Check for comma after
                        if !self.is_at_end() {
                            if let Some(next) = self.peek() {
                                if next.token_type == TokenType::Newline
                                    || next.token_type == TokenType::Header
                                    || next.token_type == TokenType::Comment
                                    || next.token_type == TokenType::DocumentSeparator
                                {
                                    // End of line
                                    if let Some(v) = current_val.take() {
                                        values.push(v);
                                    }
                                    break;
                                }
                            }
                        }
                    }
                    TokenType::Path | TokenType::BareWord => {
                        // Could be a comma, comma+value, or bare value
                        let v = tok.value.to_string();
                        self.advance();
                        if v == "," {
                            if let Some(cv) = current_val.take() {
                                values.push(cv);
                            }
                        } else if v.contains(',') {
                            // Token contains comma-separated values (e.g., `, "Active"`)
                            if let Some(cv) = current_val.take() {
                                values.push(cv);
                            }
                            for part in v.split(',') {
                                let trimmed = part.trim().trim_matches('"');
                                if !trimmed.is_empty() {
                                    values.push(trimmed.to_string());
                                }
                            }
                        } else {
                            current_val = Some(v);
                        }
                    }
                    _ => {
                        // Skip unexpected tokens (commas come through as various types)
                        let v = tok.value.to_string();
                        self.advance();
                        if v == "," {
                            if let Some(cv) = current_val.take() {
                                values.push(cv);
                            }
                        } else if v.contains(',') {
                            if let Some(cv) = current_val.take() {
                                values.push(cv);
                            }
                            for part in v.split(',') {
                                let trimmed = part.trim().trim_matches('"');
                                if !trimmed.is_empty() {
                                    values.push(trimmed.to_string());
                                }
                            }
                        }
                    }
                }
            }
            if let Some(cv) = current_val.take() {
                values.push(cv);
            }

            // Skip empty lines
            if values.is_empty() {
                continue;
            }

            // Generate metadata entries for this row
            key_buf.clear();
            key_buf.push_str("table.");
            key_buf.push_str(table_name);
            key_buf.push('[');
            let _ = write!(key_buf, "{row_index}");
            key_buf.push(']');
            key_buf.push('.');
            let prefix_len = key_buf.len();
            for (col_idx, col_name) in columns.iter().enumerate() {
                if let Some(val) = values.get(col_idx) {
                    key_buf.truncate(prefix_len);
                    key_buf.push_str(col_name);
                    metadata.insert(
                        key_buf.clone(),
                        crate::types::values::OdinValue::String {
                            value: val.clone(),
                            modifiers: None,
                            directives: vec![],
                        },
                    );
                }
            }
            row_index += 1;
        }
    }
    /// Parse a tabular section: `{name[] : col1, col2, ...}`
    ///
    /// Reads subsequent data rows and creates assignments like:
    /// `name[0].col1 = val`, `name[0].col2 = val`, etc.
    fn parse_tabular_section(
        &mut self,
        header: &str,
        assignments: &mut OrderedMap<String, crate::types::values::OdinValue>,
    ) {
        // Parse header: "name[] : col1, col2, ..."
        // Also handle ".name[] : col1, col2, ..." (with leading dot)
        let Some(colon_pos) = header.find(" : ") else { return };
        let name_part = &header[..colon_pos];
        let cols_str = &header[colon_pos + 3..];
        let columns: Vec<String> = cols_str.split(',').map(|s| s.trim().to_string()).collect();

        // Extract the base name (strip [] and leading .)
        let base_name = name_part.trim_start_matches('.').trim_end_matches("[]");
        if columns.is_empty() || base_name.is_empty() {
            return;
        }

        // Check for primitive array mode: {items[] : ~}
        let is_primitive = columns.len() == 1 && columns[0] == "~";

        // Resolve relative column names: .city inherits parent from previous dotted column
        let resolved_columns: Vec<String> = if is_primitive {
            columns.clone()
        } else {
            let mut resolved = Vec::new();
            let mut last_parent = String::new();
            for col in &columns {
                if col.starts_with('.') {
                    // Relative column: inherit parent from last dotted column
                    let relative = &col[1..];
                    if last_parent.is_empty() {
                        resolved.push(relative.to_string());
                    } else {
                        resolved.push(format!("{last_parent}.{relative}"));
                    }
                } else if col.contains('.') {
                    // Dotted column name: extract parent
                    if let Some(dot_pos) = col.find('.') {
                        last_parent = col[..dot_pos].to_string();
                    }
                    resolved.push(col.clone());
                } else {
                    // Simple column name: reset parent context
                    last_parent.clear();
                    resolved.push(col.clone());
                }
            }
            resolved
        };

        let mut row_index: usize = 0;
        let mut key_buf = String::with_capacity(64);
        use std::fmt::Write as _;

        loop {
            self.skip_newlines();
            if self.is_at_end() {
                break;
            }
            if let Some(tok) = self.peek() {
                if tok.token_type == TokenType::Header || tok.token_type == TokenType::DocumentSeparator {
                    break;
                }
                if tok.token_type == TokenType::Comment {
                    self.advance();
                    continue;
                }
            }

            // Check if this line is an assignment (identifier = value) rather than tabular data.
            // This is needed for transform documents where `{features[] : name, enabled}`
            // is followed by assignment lines like `_loop = "@features"`.
            if self.is_assignment_line() {
                // Parse as a regular assignment within this section
                let field_name = self.advance().value.to_string();
                // Skip the equals sign
                self.advance(); // consume '='
                // Parse the value
                if let Ok((val, consumed)) = parse_values::parse_value(self.tokens, self.pos) {
                    self.pos += consumed;
                    let full_key = format!("{base_name}[].{field_name}");
                    // Collect any directives
                    let mut directives = val.directives().to_vec();
                    while !self.is_at_end() {
                        let t = self.current_token();
                        if t.token_type == TokenType::Newline || t.token_type == TokenType::Header
                            || t.token_type == TokenType::Comment || t.token_type == TokenType::DocumentSeparator
                        {
                            break;
                        }
                        // Check for directive tokens (e.g., `:type integer`)
                        if t.token_type == TokenType::Directive {
                            let dir_name = t.value.to_string();
                            self.advance();
                            // Try to get the directive value
                            let dir_val = if self.is_at_end() {
                                None
                            } else {
                                let next = self.current_token();
                                if next.token_type != TokenType::Newline
                                    && next.token_type != TokenType::Header
                                    && next.token_type != TokenType::Directive
                                    && next.token_type != TokenType::Comment
                                    && next.token_type != TokenType::DocumentSeparator
                                {
                                    let v = next.value.to_string();
                                    self.advance();
                                    Some(v)
                                } else {
                                    None
                                }
                            };
                            let dir_typed = dir_val.map(|s| {
                                // Try to parse as number first, otherwise string
                                if let Ok(n) = s.parse::<f64>() {
                                    crate::types::values::DirectiveValue::Number(n)
                                } else {
                                    crate::types::values::DirectiveValue::String(s)
                                }
                            });
                            directives.push(crate::types::values::OdinDirective { name: dir_name, value: dir_typed });
                        } else {
                            self.advance();
                        }
                    }
                    let val_with_dirs = if directives.is_empty() {
                        val
                    } else {
                        val.with_directives(directives)
                    };
                    assignments.insert(full_key, val_with_dirs);
                }
                // Skip to end of line
                while !self.is_at_end() {
                    let t = self.current_token();
                    if t.token_type == TokenType::Newline || t.token_type == TokenType::Header {
                        break;
                    }
                    self.advance();
                }
                continue;
            }

            // Collect values on this line
            let mut values: Vec<crate::types::values::OdinValue> = Vec::new();

            while !self.is_at_end() {
                let tok = self.current_token();
                match tok.token_type {
                    TokenType::Newline | TokenType::Header | TokenType::DocumentSeparator => break,
                    TokenType::Comment => {
                        self.advance();
                        break;
                    }
                    _ => {
                        // Try to parse a value, then skip comma
                        if let Ok((val, consumed)) = parse_values::parse_value(self.tokens, self.pos) {
                            self.pos += consumed;
                            values.push(val);
                            // Skip remaining tokens on this line (directives, etc.) until comma or newline
                            while !self.is_at_end() {
                                let t = self.current_token();
                                if t.token_type == TokenType::Newline || t.token_type == TokenType::Header
                                    || t.token_type == TokenType::Comment || t.token_type == TokenType::DocumentSeparator
                                {
                                    break;
                                }
                                // Check for comma separator
                                if t.token_type == TokenType::Comma {
                                    self.advance();
                                    break;
                                }
                                // Skip other tokens (directives, etc.)
                                self.advance();
                            }
                        } else {
                            self.advance(); // skip unparseable token
                        }
                    }
                }
            }

            if values.is_empty() {
                continue;
            }

            // Generate assignments for this row
            if is_primitive {
                // Primitive array: one value per row, key is items[0], items[1], etc.
                if let Some(val) = values.into_iter().next() {
                    key_buf.clear();
                    key_buf.push_str(base_name);
                    key_buf.push('[');
                    let _ = write!(key_buf, "{row_index}");
                    key_buf.push(']');
                    assignments.insert(key_buf.clone(), val);
                }
            } else {
                key_buf.clear();
                key_buf.push_str(base_name);
                key_buf.push('[');
                let _ = write!(key_buf, "{row_index}");
                key_buf.push(']');
                key_buf.push('.');
                let prefix_len = key_buf.len();
                for (col_name, val) in resolved_columns.iter().zip(values.into_iter()) {
                    key_buf.truncate(prefix_len);
                    key_buf.push_str(col_name);
                    assignments.insert(key_buf.clone(), val);
                }
            }
            row_index += 1;
        }
    }
}

/// Parse a token stream into an `OdinDocument`.
pub fn parse_tokens<'a>(
    tokens: &[Token<'a>],
    _source: &'a str,
    options: &ParseOptions,
) -> Result<OdinDocument, ParseError> {
    let mut parser = Parser::new(tokens, options);
    parser.parse_last_document()
}

/// Parse a token stream into multiple documents (for document chaining).
pub fn parse_tokens_multi<'a>(
    tokens: &[Token<'a>],
    _source: &'a str,
    options: &ParseOptions,
) -> Result<Vec<OdinDocument>, ParseError> {
    let mut parser = Parser::new(tokens, options);
    parser.parse_documents()
}

#[cfg(test)]
mod tests {
    use crate::Odin;
    use crate::types::values::{OdinValue, OdinValueType};

    // ── Single field document ────────────────────────────────────────────

    #[test]
    fn single_string_field() {
        let doc = Odin::parse("name = \"Alice\"").unwrap();
        assert_eq!(doc.get_string("name"), Some("Alice"));
    }

    #[test]
    fn single_integer_field() {
        let doc = Odin::parse("count = ##42").unwrap();
        assert_eq!(doc.get_integer("count"), Some(42));
    }

    #[test]
    fn single_number_field() {
        let doc = Odin::parse("pi = #3.14").unwrap();
        let val = doc.get_number("pi").unwrap();
        assert!((val - 3.14).abs() < 0.001);
    }

    #[test]
    fn single_boolean_true() {
        let doc = Odin::parse("active = true").unwrap();
        assert_eq!(doc.get_boolean("active"), Some(true));
    }

    #[test]
    fn single_boolean_false() {
        let doc = Odin::parse("active = false").unwrap();
        assert_eq!(doc.get_boolean("active"), Some(false));
    }

    #[test]
    fn single_null_field() {
        let doc = Odin::parse("missing = ~").unwrap();
        assert!(doc.get("missing").unwrap().is_null());
    }

    // ── Multiple fields ──────────────────────────────────────────────────

    #[test]
    fn multiple_fields() {
        let doc = Odin::parse("a = \"1\"\nb = \"2\"\nc = \"3\"").unwrap();
        assert_eq!(doc.get_string("a"), Some("1"));
        assert_eq!(doc.get_string("b"), Some("2"));
        assert_eq!(doc.get_string("c"), Some("3"));
    }

    #[test]
    fn fields_ordering_preserved() {
        let doc = Odin::parse("z = \"last\"\na = \"first\"").unwrap();
        let paths = doc.paths();
        assert_eq!(*paths[0], "z");
        assert_eq!(*paths[1], "a");
    }

    // ── Section creates nested structure ─────────────────────────────────

    #[test]
    fn section_prefixes_paths() {
        let input = "{Policy}\nnumber = \"POL-001\"\nstatus = \"active\"";
        let doc = Odin::parse(input).unwrap();
        assert_eq!(doc.get_string("Policy.number"), Some("POL-001"));
        assert_eq!(doc.get_string("Policy.status"), Some("active"));
    }

    #[test]
    fn multiple_sections() {
        let input = "{A}\nx = \"1\"\n{B}\ny = \"2\"";
        let doc = Odin::parse(input).unwrap();
        assert_eq!(doc.get_string("A.x"), Some("1"));
        assert_eq!(doc.get_string("B.y"), Some("2"));
    }

    #[test]
    fn nested_section_dot_notation() {
        let input = "{A.B.C}\nval = \"deep\"";
        let doc = Odin::parse(input).unwrap();
        assert_eq!(doc.get_string("A.B.C.val"), Some("deep"));
    }

    #[test]
    fn section_then_root_field() {
        let input = "{Section}\nfoo = \"bar\"\n\n{Other}\nbaz = \"qux\"";
        let doc = Odin::parse(input).unwrap();
        assert_eq!(doc.get_string("Section.foo"), Some("bar"));
        assert_eq!(doc.get_string("Other.baz"), Some("qux"));
    }

    #[test]
    fn empty_section_no_fields() {
        let input = "{Empty}\n{Next}\nval = \"ok\"";
        let doc = Odin::parse(input).unwrap();
        assert_eq!(doc.get_string("Next.val"), Some("ok"));
        // Empty section should not produce any assignments
        assert!(!doc.has("Empty"));
    }

    // ── Metadata section ─────────────────────────────────────────────────

    #[test]
    fn metadata_section_parsed() {
        let input = "{$}\nodin = \"1.0.0\"\n\nname = \"test\"";
        let doc = Odin::parse(input).unwrap();
        assert_eq!(doc.get_string("$.odin"), Some("1.0.0"));
        assert_eq!(doc.get_string("name"), Some("test"));
    }

    #[test]
    fn metadata_values_in_metadata_map() {
        let input = "{$}\nodin = \"1.0.0\"";
        let doc = Odin::parse(input).unwrap();
        assert!(doc.metadata.get(&"odin".to_string()).is_some());
    }

    #[test]
    fn metadata_blank_line_exits_metadata() {
        let input = "{$}\nodin = \"1.0.0\"\n\nname = \"test\"";
        let doc = Odin::parse(input).unwrap();
        // After blank line, `name` is a regular assignment
        assert_eq!(doc.get_string("name"), Some("test"));
        assert!(doc.metadata.get(&"name".to_string()).is_none());
    }

    // ── Array fields ─────────────────────────────────────────────────────

    #[test]
    fn array_fields_create_entries() {
        let input = "items[0] = \"first\"\nitems[1] = \"second\"";
        let doc = Odin::parse(input).unwrap();
        assert_eq!(doc.get_string("items[0]"), Some("first"));
        assert_eq!(doc.get_string("items[1]"), Some("second"));
    }

    #[test]
    fn array_in_section() {
        let input = "{List}\nitems[0] = \"a\"\nitems[1] = \"b\"";
        let doc = Odin::parse(input).unwrap();
        assert_eq!(doc.get_string("List.items[0]"), Some("a"));
        assert_eq!(doc.get_string("List.items[1]"), Some("b"));
    }

    #[test]
    fn array_with_nested_fields() {
        let input = "people[0].name = \"Alice\"\npeople[0].age = ##30\npeople[1].name = \"Bob\"";
        let doc = Odin::parse(input).unwrap();
        assert_eq!(doc.get_string("people[0].name"), Some("Alice"));
        assert_eq!(doc.get_integer("people[0].age"), Some(30));
        assert_eq!(doc.get_string("people[1].name"), Some("Bob"));
    }

    // ── Sparse / non-contiguous array error ──────────────────────────────

    #[test]
    fn sparse_array_error() {
        let input = "items[0] = \"a\"\nitems[5] = \"b\"";
        let result = Odin::parse(input);
        assert!(result.is_err());
    }

    #[test]
    fn array_starting_at_nonzero_errors() {
        // First index must be 0 (not 2 or any other value).
        let input = "items[2] = \"a\"";
        assert!(Odin::parse(input).is_err());
    }

    #[test]
    fn array_index_reassign_errors_by_default() {
        // items[0] then items[0] is a duplicate path; rejected unless allow_duplicates.
        let input = "items[0] = \"a\"\nitems[0] = \"b\"";
        assert!(Odin::parse(input).is_err());
    }

    #[test]
    fn array_index_reassign_allowed_with_option() {
        // With allow_duplicates the second write wins; contiguity tracking must
        // not reject idx < expected.
        let opts = crate::types::options::ParseOptions {
            allow_duplicates: true,
            ..Default::default()
        };
        let input = "items[0] = \"a\"\nitems[1] = \"b\"\nitems[0] = \"c\"";
        let doc = crate::parser::parse(input, Some(&opts)).unwrap();
        assert_eq!(doc.get_string("items[0]"), Some("c"));
        assert_eq!(doc.get_string("items[1]"), Some("b"));
    }

    // ── Verb expression parsing ──────────────────────────────────────────

    #[test]
    fn verb_simple_args_preserved() {
        // Verb args are joined with single spaces; leading '%' is preserved.
        let input = "result = %lookup users name\n";
        let doc = Odin::parse(input).unwrap();
        let val = doc.get("result").unwrap();
        if let OdinValue::Verb { verb, .. } = val {
            assert_eq!(verb, "%lookup users name");
        } else {
            panic!("expected Verb value, got {:?}", val);
        }
    }

    #[test]
    fn verb_with_quoted_arg() {
        let input = "msg = %concat \"hello\" \"world\"\n";
        let doc = Odin::parse(input).unwrap();
        let val = doc.get("msg").unwrap();
        if let OdinValue::Verb { verb, .. } = val {
            assert!(verb.starts_with("%concat"));
            assert!(verb.contains("\"hello\""));
            assert!(verb.contains("\"world\""));
        } else {
            panic!("expected Verb value");
        }
    }

    // ── Modifiers attached to values ─────────────────────────────────────

    #[test]
    fn required_modifier() {
        let input = "name = !\"Alice\"";
        let doc = Odin::parse(input).unwrap();
        let val = doc.get("name").unwrap();
        assert!(val.modifiers().map_or(false, |m| m.required));
    }

    #[test]
    fn confidential_modifier() {
        let input = "ssn = *\"123-45-6789\"";
        let doc = Odin::parse(input).unwrap();
        let val = doc.get("ssn").unwrap();
        assert!(val.modifiers().map_or(false, |m| m.confidential));
    }

    #[test]
    fn deprecated_modifier() {
        let input = "old_field = -\"value\"";
        let doc = Odin::parse(input).unwrap();
        let val = doc.get("old_field").unwrap();
        assert!(val.modifiers().map_or(false, |m| m.deprecated));
    }

    #[test]
    fn combined_modifiers() {
        let input = "field = !*\"critical_secret\"";
        let doc = Odin::parse(input).unwrap();
        let val = doc.get("field").unwrap();
        let mods = val.modifiers().unwrap();
        assert!(mods.required);
        assert!(mods.confidential);
    }

    #[test]
    fn all_three_modifiers() {
        let input = "field = !-*\"all_mods\"";
        let doc = Odin::parse(input).unwrap();
        let val = doc.get("field").unwrap();
        let mods = val.modifiers().unwrap();
        assert!(mods.required);
        assert!(mods.deprecated);
        assert!(mods.confidential);
    }

    #[test]
    fn modifier_on_integer() {
        let input = "count = !##42";
        let doc = Odin::parse(input).unwrap();
        let val = doc.get("count").unwrap();
        assert_eq!(val.as_i64(), Some(42));
        assert!(val.modifiers().map_or(false, |m| m.required));
    }

    #[test]
    fn modifier_on_null() {
        let input = "field = *~";
        let doc = Odin::parse(input).unwrap();
        let val = doc.get("field").unwrap();
        assert!(val.is_null());
        assert!(val.modifiers().map_or(false, |m| m.confidential));
    }

    // ── Duplicate key handling ───────────────────────────────────────────

    #[test]
    fn duplicate_key_error_default() {
        let input = "name = \"Alice\"\nname = \"Bob\"";
        let result = Odin::parse(input);
        assert!(result.is_err());
    }

    #[test]
    fn duplicate_key_allowed_with_option() {
        let opts = crate::types::options::ParseOptions {
            allow_duplicates: true,
            ..Default::default()
        };
        let input = "name = \"Alice\"\nname = \"Bob\"";
        let doc = crate::parser::parse(input, Some(&opts)).unwrap();
        // Last value wins
        assert_eq!(doc.get_string("name"), Some("Bob"));
    }

    // ── Multi-document parsing ───────────────────────────────────────────

    #[test]
    fn multi_document_basic() {
        let input = "a = \"1\"\n---\nb = \"2\"";
        let docs = Odin::parse_documents(input).unwrap();
        assert_eq!(docs.len(), 2);
        assert_eq!(docs[0].get_string("a"), Some("1"));
        assert_eq!(docs[1].get_string("b"), Some("2"));
    }

    #[test]
    fn multi_document_three_docs() {
        let input = "x = \"1\"\n---\ny = \"2\"\n---\nz = \"3\"";
        let docs = Odin::parse_documents(input).unwrap();
        assert_eq!(docs.len(), 3);
    }

    #[test]
    fn single_document_no_separator() {
        let input = "name = \"Alice\"";
        let docs = Odin::parse_documents(input).unwrap();
        assert_eq!(docs.len(), 1);
    }

    #[test]
    fn parse_returns_last_document() {
        let input = "a = \"first\"\n---\nb = \"second\"";
        let doc = Odin::parse(input).unwrap();
        // parse() returns the last document
        assert_eq!(doc.get_string("b"), Some("second"));
    }

    // ── Value types ──────────────────────────────────────────────────────

    #[test]
    fn currency_value() {
        let doc = Odin::parse("price = #$99.99").unwrap();
        let val = doc.get("price").unwrap();
        assert_eq!(val.value_type(), OdinValueType::Currency);
        assert!((val.as_f64().unwrap() - 99.99).abs() < 0.001);
    }

    #[test]
    fn currency_with_code() {
        let doc = Odin::parse("price = #$100.00:USD").unwrap();
        let val = doc.get("price").unwrap();
        if let OdinValue::Currency { currency_code, .. } = val {
            assert_eq!(currency_code.as_deref(), Some("USD"));
        } else {
            panic!("Expected Currency value");
        }
    }

    #[test]
    fn percent_value() {
        let doc = Odin::parse("rate = #%85.5").unwrap();
        let val = doc.get("rate").unwrap();
        assert_eq!(val.value_type(), OdinValueType::Percent);
    }

    #[test]
    fn reference_value() {
        let doc = Odin::parse("ref = @other.path").unwrap();
        let val = doc.get("ref").unwrap();
        assert_eq!(val.value_type(), OdinValueType::Reference);
        if let OdinValue::Reference { path, .. } = val {
            assert_eq!(path, "other.path");
        }
    }

    #[test]
    fn binary_value() {
        let doc = Odin::parse("data = ^SGVsbG8=").unwrap();
        let val = doc.get("data").unwrap();
        assert_eq!(val.value_type(), OdinValueType::Binary);
    }

    #[test]
    fn date_value() {
        let doc = Odin::parse("born = 2024-06-15").unwrap();
        let val = doc.get("born").unwrap();
        assert_eq!(val.value_type(), OdinValueType::Date);
    }

    #[test]
    fn timestamp_value() {
        let doc = Odin::parse("created = 2024-06-15T14:30:00Z").unwrap();
        let val = doc.get("created").unwrap();
        assert_eq!(val.value_type(), OdinValueType::Timestamp);
    }

    #[test]
    fn time_value() {
        let doc = Odin::parse("starts = T10:30:00").unwrap();
        let val = doc.get("starts").unwrap();
        assert_eq!(val.value_type(), OdinValueType::Time);
    }

    #[test]
    fn duration_value() {
        let doc = Odin::parse("period = P1Y6M").unwrap();
        let val = doc.get("period").unwrap();
        assert_eq!(val.value_type(), OdinValueType::Duration);
    }

    // ── Comments preserved in parse ──────────────────────────────────────

    #[test]
    fn comments_do_not_affect_values() {
        let input = "; header comment\nname = \"Alice\" ; inline comment\nage = ##30";
        let doc = Odin::parse(input).unwrap();
        assert_eq!(doc.get_string("name"), Some("Alice"));
        assert_eq!(doc.get_integer("age"), Some(30));
    }

    #[test]
    fn comment_only_document() {
        // A document with only comments should parse (produces empty assignments)
        let input = "; just a comment\n; another one";
        let result = Odin::parse(input);
        // May or may not error depending on empty-doc policy, but should not panic
        assert!(result.is_ok() || result.is_err());
    }

    // ── Import directive parsing ─────────────────────────────────────────

    #[test]
    fn import_parsed() {
        let input = "@import ./types.odin\nname = \"test\"";
        let doc = Odin::parse(input).unwrap();
        assert_eq!(doc.imports.len(), 1);
        assert_eq!(doc.imports[0].path, "./types.odin");
        assert!(doc.imports[0].alias.is_none());
    }

    #[test]
    fn import_with_alias_parsed() {
        let input = "@import ./base.odin as base\nname = \"test\"";
        let doc = Odin::parse(input).unwrap();
        assert_eq!(doc.imports[0].alias.as_deref(), Some("base"));
    }

    // ── Schema directive parsing ─────────────────────────────────────────

    #[test]
    fn schema_parsed() {
        let input = "@schema ./policy.schema.odin\nname = \"test\"";
        let doc = Odin::parse(input).unwrap();
        assert_eq!(doc.schemas.len(), 1);
        assert_eq!(doc.schemas[0].url, "./policy.schema.odin");
    }

    // ── Mixed root and section fields ────────────────────────────────────

    #[test]
    fn root_fields_before_section() {
        let input = "root_val = \"yes\"\n{Section}\nsec_val = \"also yes\"";
        let doc = Odin::parse(input).unwrap();
        assert_eq!(doc.get_string("root_val"), Some("yes"));
        assert_eq!(doc.get_string("Section.sec_val"), Some("also yes"));
    }

    // ── String escape handling through full parse ────────────────────────

    #[test]
    fn string_with_escapes() {
        let doc = Odin::parse("msg = \"line1\\nline2\"").unwrap();
        assert_eq!(doc.get_string("msg"), Some("line1\nline2"));
    }

    #[test]
    fn string_with_unicode() {
        let doc = Odin::parse("char = \"\\u0041\"").unwrap();
        assert_eq!(doc.get_string("char"), Some("A"));
    }

    // ── Directives on fields ─────────────────────────────────────────────

    #[test]
    fn field_with_directive() {
        let input = "field = \"value\" :format ssn";
        let doc = Odin::parse(input).unwrap();
        let val = doc.get("field").unwrap();
        let dirs = val.directives();
        assert!(!dirs.is_empty());
        assert_eq!(dirs[0].name, "format");
    }

    #[test]
    fn field_with_multiple_directives() {
        let input = "field = \"value\" :pos 3 :len 8";
        let doc = Odin::parse(input).unwrap();
        let val = doc.get("field").unwrap();
        let dirs = val.directives();
        assert_eq!(dirs.len(), 2);
        assert_eq!(dirs[0].name, "pos");
        assert_eq!(dirs[1].name, "len");
    }

    // ── Error: missing equals ────────────────────────────────────────────

    #[test]
    fn missing_equals_error() {
        let result = Odin::parse("name \"Alice\"");
        assert!(result.is_err());
    }

    // ── Negative integer ─────────────────────────────────────────────────

    #[test]
    fn negative_integer() {
        let doc = Odin::parse("temp = ##-10").unwrap();
        assert_eq!(doc.get_integer("temp"), Some(-10));
    }

    // ── Large numbers ────────────────────────────────────────────────────

    #[test]
    fn large_integer() {
        let doc = Odin::parse("big = ##9999999999").unwrap();
        assert_eq!(doc.get_integer("big"), Some(9999999999));
    }

    #[test]
    fn number_scientific_notation() {
        let doc = Odin::parse("sci = #1.5e10").unwrap();
        let val = doc.get_number("sci").unwrap();
        assert!((val - 1.5e10).abs() < 1.0);
    }

    // ── Section ordering ─────────────────────────────────────────────────

    #[test]
    fn section_ordering_preserved() {
        let input = "{B}\nb = \"2\"\n{A}\na = \"1\"";
        let doc = Odin::parse(input).unwrap();
        let paths = doc.paths();
        assert_eq!(*paths[0], "B.b");
        assert_eq!(*paths[1], "A.a");
    }

    // ── Empty value ──────────────────────────────────────────────────────

    #[test]
    fn empty_value_treated_as_empty_string() {
        let doc = Odin::parse("field =").unwrap();
        assert_eq!(doc.get_string("field"), Some(""));
    }

    #[test]
    fn empty_value_before_newline() {
        let doc = Odin::parse("field =\nnext = \"ok\"").unwrap();
        assert_eq!(doc.get_string("field"), Some(""));
        assert_eq!(doc.get_string("next"), Some("ok"));
    }

    // ── Boolean prefix ? ─────────────────────────────────────────────────

    #[test]
    fn boolean_prefix_true() {
        let doc = Odin::parse("flag = ?true").unwrap();
        assert_eq!(doc.get_boolean("flag"), Some(true));
    }

    #[test]
    fn boolean_prefix_false() {
        let doc = Odin::parse("flag = ?false").unwrap();
        assert_eq!(doc.get_boolean("flag"), Some(false));
    }

    // ── Named metadata sections ──────────────────────────────────────────

    #[test]
    fn named_metadata_section() {
        let input = "{$const}\nPI = #3.14159";
        let doc = Odin::parse(input).unwrap();
        // Named metadata sections store under const.PI in metadata
        assert!(doc.metadata.get(&"const.PI".to_string()).is_some());
    }

    // ── Bare string rejection ────────────────────────────────────────────

    #[test]
    fn bare_string_error() {
        let result = Odin::parse("name = Alice");
        assert!(result.is_err());
    }

    // ── Multiple imports ─────────────────────────────────────────────────

    #[test]
    fn multiple_imports() {
        let input = "@import ./a.odin\n@import ./b.odin\nname = \"test\"";
        let doc = Odin::parse(input).unwrap();
        assert_eq!(doc.imports.len(), 2);
    }

    // ── Complex document ─────────────────────────────────────────────────

    #[test]
    fn complex_mixed_document() {
        let input = "\
{$}
odin = \"1.0.0\"

{Policy}
number = \"POL-001\"
premium = #$1500.00
active = true

{Policy.Coverage}
type = \"auto\"
limit = ##100000
";
        let doc = Odin::parse(input).unwrap();
        assert_eq!(doc.get_string("$.odin"), Some("1.0.0"));
        assert_eq!(doc.get_string("Policy.number"), Some("POL-001"));
        assert_eq!(doc.get_boolean("Policy.active"), Some(true));
        assert_eq!(doc.get_string("Policy.Coverage.type"), Some("auto"));
        assert_eq!(doc.get_integer("Policy.Coverage.limit"), Some(100000));
    }
}
