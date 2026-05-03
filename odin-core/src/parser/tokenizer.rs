//! ODIN Tokenizer — single-pass character scanner.
//!
//! Converts ODIN source text into a stream of tokens.
//! Optimized for zero-allocation: most token values are borrowed slices
//! of the source text. Only tokens requiring processing (e.g., strings
//! with escape sequences) allocate.

use crate::types::errors::{ParseError, ParseErrorCode};
use crate::types::options::ParseOptions;
use super::tokens::{Token, TokenType};

/// Tokenizer state.
struct Tokenizer<'a> {
    source: &'a str,
    bytes: &'a [u8],
    pos: usize,
    line: usize,
    /// Byte offset where the current line starts. Column is derived as
    /// `pos - line_start + 1`, so we don't update a counter per byte.
    line_start: usize,
}

impl<'a> Tokenizer<'a> {
    fn new(source: &'a str) -> Self {
        // Strip UTF-8 BOM if present
        let source = source.strip_prefix('\u{FEFF}').unwrap_or(source);
        Self {
            source,
            bytes: source.as_bytes(),
            pos: 0,
            line: 1,
            line_start: 0,
        }
    }

    #[inline]
    fn column(&self) -> usize {
        self.pos - self.line_start + 1
    }

    #[inline]
    fn is_at_end(&self) -> bool {
        self.pos >= self.bytes.len()
    }

    #[inline]
    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    #[inline]
    fn current_byte(&self) -> u8 {
        self.bytes[self.pos]
    }

    #[inline]
    fn peek_at(&self, offset: usize) -> Option<u8> {
        self.bytes.get(self.pos + offset).copied()
    }

    #[inline]
    fn advance(&mut self) -> u8 {
        let ch = self.bytes[self.pos];
        self.pos += 1;
        if ch == b'\n' {
            self.line += 1;
            self.line_start = self.pos;
        }
        ch
    }

    /// Advance past a multi-byte UTF-8 character, returning the char.
    fn advance_utf8_char(&mut self) -> Option<char> {
        let start = self.pos;
        if start >= self.bytes.len() {
            return None;
        }
        let first = self.bytes[start];
        let char_len = utf8_char_len(first);
        if start + char_len > self.bytes.len() {
            self.advance();
            return None;
        }
        let s = &self.source[start..start + char_len];
        let ch = s.chars().next();
        for _ in 0..char_len {
            self.advance();
        }
        ch
    }

    #[inline]
    fn skip_whitespace(&mut self) {
        // Bypass advance()'s `\n` check — we already know the byte is space/tab.
        while let Some(b' ' | b'\t') = self.peek() {
            self.pos += 1;
        }
    }

    /// Construct a token whose logical value is `&source[value_start..value_end]`.
    #[inline]
    fn make(&self, token_type: TokenType, value_start: usize, value_end: usize, line: usize, col: usize) -> Token {
        Token::new(token_type, value_start, value_end, line, col)
    }

    fn scan_comment(&mut self) -> Token {
        let start = self.pos;
        let start_line = self.line;
        let start_col = self.column();
        // Comments contain no newlines by definition; SIMD-search for the
        // line terminator (or run to EOF).
        self.pos = match memchr::memchr(b'\n', &self.bytes[self.pos..]) {
            Some(off) => self.pos + off,
            None => self.bytes.len(),
        };
        self.make(TokenType::Comment, start, self.pos, start_line, start_col)
    }

    fn scan_quoted_string(&mut self) -> Result<Token, ParseError> {
        let start = self.pos;
        let start_line = self.line;
        let start_col = self.column();
        self.advance(); // skip opening `"`
        let content_start = self.pos;

        // SIMD-accelerated search for the first interesting byte.
        // UTF-8 safety: continuation bytes (0x80-0xBF) never equal ASCII bytes,
        // so scanning for `"`, `\`, `\n` byte-wise is safe even with multi-byte chars.
        let (peek_pos, needs_processing) = match memchr::memchr3(b'"', b'\\', b'\n', &self.bytes[content_start..]) {
            Some(off) => {
                let p = content_start + off;
                (p, self.bytes[p] != b'"')
            }
            None => (self.bytes.len(), true), // unterminated
        };

        if !needs_processing {
            // Fast path: no escapes, value is the bytes between the quotes.
            let content_end = peek_pos;
            self.pos = content_end;
            self.advance(); // skip closing `"`
            return Ok(self.make(TokenType::QuotedString, content_start, content_end, start_line, start_col));
        }

        // Slow path: escapes / newlines / unterminated. Walk to the end without
        // unescaping; emit QuotedStringEscaped with the raw content range and
        // let parse_value handle the unescape on demand.
        let _ = start;
        while !self.is_at_end() {
            let ch = self.current_byte();
            if ch == b'"' {
                let content_end = self.pos;
                self.advance(); // skip closing `"`
                return Ok(self.make(TokenType::QuotedStringEscaped, content_start, content_end, start_line, start_col));
            }
            if ch == b'\\' {
                self.advance();
                if self.is_at_end() {
                    return Err(ParseError::new(ParseErrorCode::UnterminatedString, start_line, start_col));
                }
                // Validate the escape character so the tokenizer rejects bad
                // sequences as early as today; the actual translation happens
                // in `unescape_quoted` during value construction.
                let esc = self.advance();
                match esc {
                    b'n' | b'r' | b't' | b'\\' | b'"' | b'/' | b'0' => {}
                    b'u' => {
                        // consume 4 hex digits (validation only)
                        let _ = self.scan_unicode_escape(4, start_line, start_col)?;
                        // optional surrogate continuation: \uXXXX (low surrogate)
                        if self.peek() == Some(b'\\') && self.peek_at(1) == Some(b'u') {
                            self.advance();
                            self.advance();
                            let _ = self.scan_unicode_escape(4, start_line, start_col)?;
                        }
                    }
                    b'U' => {
                        let _ = self.scan_unicode_escape(8, start_line, start_col)?;
                    }
                    _ => {
                        return Err(ParseError::with_message(
                            ParseErrorCode::InvalidEscapeSequence,
                            self.line, self.column(),
                            &format!("unknown escape: \\{}", esc as char),
                        ));
                    }
                }
            } else if ch == b'\n' {
                return Err(ParseError::new(ParseErrorCode::UnterminatedString, start_line, start_col));
            } else if ch >= 0x80 {
                let _ = self.advance_utf8_char();
            } else {
                self.advance();
            }
        }

        Err(ParseError::new(ParseErrorCode::UnterminatedString, start_line, start_col))
    }

    fn scan_unicode_escape(&mut self, digits: usize, start_line: usize, start_col: usize) -> Result<char, ParseError> {
        let hex_start = self.pos;
        for _ in 0..digits {
            if self.is_at_end() {
                return Err(ParseError::with_message(
                    ParseErrorCode::InvalidEscapeSequence,
                    start_line, start_col,
                    "incomplete unicode escape",
                ));
            }
            self.advance();
        }
        let hex = &self.source[hex_start..self.pos];
        let code = u32::from_str_radix(hex, 16).map_err(|_| {
            ParseError::with_message(
                ParseErrorCode::InvalidEscapeSequence,
                start_line, start_col,
                &format!("invalid hex in unicode escape: \\u{hex}"),
            )
        })?;
        char::from_u32(code).ok_or_else(|| {
            ParseError::with_message(
                ParseErrorCode::InvalidEscapeSequence,
                start_line, start_col,
                &format!("invalid unicode code point: U+{code:04X}"),
            )
        })
    }

    fn scan_header(&mut self) -> Result<Token, ParseError> {
        let start = self.pos;
        let start_line = self.line;
        let start_col = self.column();
        self.advance(); // skip `{`
        let content_start = self.pos;

        while !self.is_at_end() {
            let ch = self.current_byte();
            if ch == b'}' {
                let value = &self.source[content_start..self.pos];
                self.advance(); // skip `}`

                // Validate bracket usage in headers
                if let Some(bracket_start) = value.find('[') {
                    if !value.starts_with("$table") {
                        let bracket_end = value.find(']');
                        match bracket_end {
                            None => {
                                return Err(ParseError::with_message(
                                    ParseErrorCode::InvalidArrayIndex,
                                    start_line, start_col,
                                    value,
                                ));
                            }
                            Some(end) => {
                                let bracket_content = &value[bracket_start + 1..end];
                                let valid = bracket_content.is_empty()
                                    || bracket_content.chars().all(|c| c.is_ascii_digit())
                                    || bracket_content.chars().all(|c| c.is_alphanumeric() || c == '_' || c == ',' || c == ' ');
                                if !valid {
                                    return Err(ParseError::with_message(
                                        ParseErrorCode::InvalidArrayIndex,
                                        start_line, start_col,
                                        value,
                                    ));
                                }
                            }
                        }
                    }
                }
                let content_end = content_start + value.len();
                let _ = start;
                return Ok(self.make(TokenType::Header, content_start, content_end, start_line, start_col));
            }
            if ch == b'\n' {
                return Err(ParseError::new(ParseErrorCode::InvalidHeaderSyntax, start_line, start_col));
            }
            self.advance();
        }

        Err(ParseError::new(ParseErrorCode::InvalidHeaderSyntax, start_line, start_col))
    }

    fn scan_identifier(&mut self) -> Result<Token, ParseError> {
        let start = self.pos;
        let start_line = self.line;
        let start_col = self.column();

        let first = self.current_byte();

        // Check for time literal: T + digit
        if first == b'T' && self.peek_at(1).is_some_and(|c| c.is_ascii_digit()) {
            return Ok(self.scan_time());
        }

        // Check for duration literal: P + (digit|T)
        if first == b'P' && self.peek_at(1).is_some_and(|c| c.is_ascii_digit() || c == b'T') {
            return Ok(self.scan_duration());
        }

        let mut in_bracket = false;
        while !self.is_at_end() {
            match self.peek() {
                Some(b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_' | b'.' | b'$') => {
                    self.advance();
                }
                Some(b'[') => {
                    in_bracket = true;
                    self.advance();
                }
                Some(b']') => {
                    in_bracket = false;
                    self.advance();
                }
                Some(b'-') if in_bracket => {
                    self.advance();
                }
                _ => break,
            }
        }

        let value = &self.source[start..self.pos];

        // Check for negative array indices → P003
        if value.contains("[-") {
            return Err(ParseError::with_message(
                ParseErrorCode::InvalidArrayIndex,
                start_line, start_col,
                &format!("Negative array index in path: {value}"),
            ));
        }

        match value {
            "true" | "false" => Ok(self.make(TokenType::BooleanLiteral, start, self.pos, start_line, start_col)),
            _ => Ok(self.make(TokenType::Path, start, self.pos, start_line, start_col)),
        }
    }

    fn scan_bare_value(&mut self) -> Token {
        let start = self.pos;
        let start_line = self.line;
        let start_col = self.column();

        while !self.is_at_end() {
            match self.peek() {
                Some(b'\n' | b'\r' | b';') => break,
                Some(b' ' | b'\t') => {
                    // skip_whitespace only consumes ' ' and '\t' (no newlines),
                    // so line/line_start are invariant — only pos needs restore.
                    let saved_pos = self.pos;
                    self.skip_whitespace();
                    if self.is_at_end() || matches!(self.peek(), Some(b'\n' | b'\r' | b';')) {
                        break;
                    }
                    self.pos = saved_pos;
                    self.advance();
                }
                _ => { self.advance(); }
            }
        }

        let raw = &self.source[start..self.pos];
        let trimmed_end = start + raw.trim_end().len();
        self.make(TokenType::BareWord, start, trimmed_end, start_line, start_col)
    }

    /// Advance past a numeric value (digits, decimal point, scientific notation).
    /// Does NOT create a token — caller uses the range to borrow from source.
    #[inline]
    fn scan_number_inline(&mut self) {
        if self.peek() == Some(b'-') {
            self.advance();
        }
        while !self.is_at_end() {
            match self.peek() {
                Some(b'0'..=b'9' | b'.' | b'e' | b'E' | b'+' | b'-') => { self.advance(); }
                _ => break,
            }
        }
    }

    /// Scan a standalone number token (e.g., bare `42` or `3.14` in value position).
    fn scan_number(&mut self) -> Token {
        let start = self.pos;
        let start_line = self.line;
        let start_col = self.column();
        self.scan_number_inline();
        self.make(TokenType::NumericLiteral, start, self.pos, start_line, start_col)
    }

    /// Scan a date or timestamp value starting with YYYY-...
    fn scan_date_or_timestamp(&mut self) -> Token {
        let start = self.pos;
        let start_line = self.line;
        let start_col = self.column();

        while !self.is_at_end() {
            match self.peek() {
                Some(b'\n' | b'\r' | b' ' | b'\t' | b';') => break,
                _ => { self.advance(); }
            }
        }

        let value = &self.source[start..self.pos];

        if value.contains('T') {
            self.make(TokenType::TimestampLiteral, start, self.pos, start_line, start_col)
        } else {
            self.make(TokenType::DateLiteral, start, self.pos, start_line, start_col)
        }
    }

    fn scan_time(&mut self) -> Token {
        let start = self.pos;
        let start_line = self.line;
        let start_col = self.column();

        while !self.is_at_end() {
            match self.peek() {
                Some(b'0'..=b'9' | b'T' | b':' | b'.') => { self.advance(); }
                _ => break,
            }
        }

        self.make(TokenType::TimeLiteral, start, self.pos, start_line, start_col)
    }

    fn scan_duration(&mut self) -> Token {
        let start = self.pos;
        let start_line = self.line;
        let start_col = self.column();

        while !self.is_at_end() {
            match self.peek() {
                Some(b'P' | b'T' | b'Y' | b'M' | b'W' | b'D' | b'H' | b'S' | b'0'..=b'9' | b'.') => {
                    self.advance();
                }
                _ => break,
            }
        }

        self.make(TokenType::DurationLiteral, start, self.pos, start_line, start_col)
    }

    fn scan_directive(&mut self) -> Token {
        let start = self.pos;
        let start_line = self.line;
        let start_col = self.column();
        self.advance(); // skip `:`

        let name_start = self.pos;
        while !self.is_at_end() {
            match self.peek() {
                Some(b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_') => {
                    self.advance();
                }
                _ => break,
            }
        }

        self.make(TokenType::Directive, name_start, self.pos, start_line, start_col)
    }

    /// Scan an @ directive or reference.
    fn scan_at(&mut self) -> Result<Token, ParseError> {
        let start = self.pos;
        let start_line = self.line;
        let start_col = self.column();
        self.advance(); // skip `@`

        // Read the keyword/path after @
        let word_start = self.pos;
        while !self.is_at_end() && matches!(self.peek(), Some(b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_' | b'.' | b'[' | b']' | b'$' | b':' | b'-' | b'@')) {
            self.advance();
        }
        let word = &self.source[word_start..self.pos];

        match word {
            "import" => {
                self.skip_whitespace();
                let rest_start = self.pos;
                while !self.is_at_end() && !matches!(self.peek(), Some(b'\n' | b'\r')) {
                    self.advance();
                }
                let rest = self.source[rest_start..self.pos].trim_end();
                let rest = if let Some(idx) = find_comment_start(rest) {
                    rest[..idx].trim_end()
                } else {
                    rest
                };
                let rest_end = rest_start + rest.len();
                let _ = start;
                Ok(self.make(TokenType::Import, rest_start, rest_end, start_line, start_col))
            }
            "schema" => {
                self.skip_whitespace();
                let rest_start = self.pos;
                while !self.is_at_end() && !matches!(self.peek(), Some(b'\n' | b'\r')) {
                    self.advance();
                }
                let rest = self.source[rest_start..self.pos].trim_end();
                let rest = if let Some(idx) = find_comment_start(rest) {
                    rest[..idx].trim_end()
                } else {
                    rest
                };
                let rest_end = rest_start + rest.len();
                Ok(self.make(TokenType::Schema, rest_start, rest_end, start_line, start_col))
            }
            "if" => {
                self.skip_whitespace();
                let rest_start = self.pos;
                while !self.is_at_end() && !matches!(self.peek(), Some(b'\n' | b'\r')) {
                    self.advance();
                }
                let rest = self.source[rest_start..self.pos].trim_end();
                let rest = if let Some(idx) = find_comment_start(rest) {
                    rest[..idx].trim_end()
                } else {
                    rest
                };
                let rest_end = rest_start + rest.len();
                Ok(self.make(TokenType::Conditional, rest_start, rest_end, start_line, start_col))
            }
            "" => {
                if start_col == 1 {
                    return Err(ParseError::with_message(
                        ParseErrorCode::UnexpectedCharacter,
                        start_line, start_col,
                        "Unexpected character: @",
                    ));
                }
                Ok(self.make(TokenType::ReferencePrefix, word_start, word_start, start_line, start_col))
            }
            _ => {
                if start_col == 1 && !word.contains('[') && !word.contains('.') {
                    return Err(ParseError::with_message(
                        ParseErrorCode::UnexpectedCharacter,
                        start_line, start_col,
                        &format!("Invalid directive: @{word}"),
                    ));
                }
                // Emit the raw text; consumers (parse_value) normalize
                // leading-zero array indices on demand.
                let _ = word;
                Ok(self.make(TokenType::ReferencePrefix, word_start, self.pos, start_line, start_col))
            }
        }
    }

    /// Scan an extension path: &com.acme.field
    fn scan_extension_path(&mut self) -> Token {
        let start = self.pos;
        let start_line = self.line;
        let start_col = self.column();
        self.advance(); // skip `&`

        while !self.is_at_end() && matches!(self.peek(), Some(b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_' | b'.')) {
            self.advance();
        }
        // source[start..self.pos] includes the `&` prefix
        self.make(TokenType::Path, start, self.pos, start_line, start_col)
    }

    fn next_token(&mut self) -> Result<Option<Token>, ParseError> {
        self.skip_whitespace();

        if self.is_at_end() {
            return Ok(None);
        }

        let ch = self.current_byte();
        let start_line = self.line;
        let start_col = self.column();
        let start_pos = self.pos;

        match ch {
            b'\n' => {
                self.advance();
                Ok(Some(Token::new(TokenType::Newline, start_pos, self.pos, start_line, start_col)))
            }
            b'\r' => {
                self.advance();
                if self.peek() == Some(b'\n') {
                    self.advance();
                }
                Ok(Some(Token::new(TokenType::Newline, start_pos, self.pos, start_line, start_col)))
            }
            b';' => Ok(Some(self.scan_comment())),
            b'{' => Ok(Some(self.scan_header()?)),
            b'=' => {
                self.advance();
                Ok(Some(Token::new(TokenType::Equals, start_pos, self.pos, start_line, start_col)))
            }
            b'"' => Ok(Some(self.scan_quoted_string()?)),
            b'~' => {
                self.advance();
                Ok(Some(Token::new(TokenType::Null, start_pos, self.pos, start_line, start_col)))
            }
            b'@' => Ok(Some(self.scan_at()?)),
            b'^' => {
                self.advance();
                let val_start = self.pos;
                while !self.is_at_end() && !matches!(self.peek(), Some(b'\n' | b'\r' | b' ' | b'\t' | b';')) {
                    self.advance();
                }
                Ok(Some(self.make(TokenType::BinaryPrefix, val_start, self.pos, start_line, start_col)))
            }
            b'#' => {
                self.advance();
                match self.peek() {
                    Some(b'#') => {
                        // Integer prefix ##
                        self.advance();
                        let val_start = self.pos;
                        self.scan_number_inline();
                        Ok(Some(self.make(TokenType::IntegerPrefix, val_start, self.pos, start_line, start_col)))
                    }
                    Some(b'$') => {
                        // Currency prefix #$
                        self.advance();
                        let val_start = self.pos;
                        self.scan_number_inline();
                        // Check for currency code after colon
                        if self.peek() == Some(b':') {
                            self.advance();
                            while !self.is_at_end() && matches!(self.peek(), Some(b'A'..=b'Z' | b'a'..=b'z')) {
                                self.advance();
                            }
                        }
                        Ok(Some(self.make(TokenType::CurrencyPrefix, val_start, self.pos, start_line, start_col)))
                    }
                    Some(b'%') => {
                        // Percent prefix #%
                        self.advance();
                        let val_start = self.pos;
                        self.scan_number_inline();
                        Ok(Some(self.make(TokenType::PercentPrefix, val_start, self.pos, start_line, start_col)))
                    }
                    Some(b'0'..=b'9' | b'-' | b'.') => {
                        // Number prefix #
                        let val_start = self.pos;
                        self.scan_number_inline();
                        Ok(Some(self.make(TokenType::NumberPrefix, val_start, self.pos, start_line, start_col)))
                    }
                    _ => {
                        Err(ParseError::with_message(
                            ParseErrorCode::InvalidTypePrefix,
                            start_line, start_col,
                            "expected number after '#'",
                        ))
                    }
                }
            }
            b'?' => {
                self.advance();
                Ok(Some(Token::new(TokenType::BooleanPrefix, start_pos, self.pos, start_line, start_col)))
            }
            b'%' => {
                self.advance();
                let name_start = self.pos;
                if self.peek() == Some(b'&') {
                    self.advance();
                }
                while !self.is_at_end() && matches!(self.peek(), Some(b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_' | b'.')) {
                    self.advance();
                }
                Ok(Some(self.make(TokenType::VerbPrefix, name_start, self.pos, start_line, start_col)))
            }
            b'!' => {
                self.advance();
                Ok(Some(Token::new(TokenType::Modifier, start_pos, self.pos, start_line, start_col)))
            }
            b'*' => {
                self.advance();
                Ok(Some(Token::new(TokenType::Modifier, start_pos, self.pos, start_line, start_col)))
            }
            b'-' => {
                // Check for document separator `---`
                if self.peek_at(1) == Some(b'-') && self.peek_at(2) == Some(b'-') {
                    self.advance();
                    self.advance();
                    self.advance();
                    return Ok(Some(Token::new(TokenType::DocumentSeparator, start_pos, self.pos, start_line, start_col)));
                }
                // Check if this is a deprecated modifier before a date: -YYYY-MM-DD
                if self.peek_at(1).is_some_and(|c| c.is_ascii_digit())
                    && self.peek_at(2).is_some_and(|c| c.is_ascii_digit())
                    && self.peek_at(3).is_some_and(|c| c.is_ascii_digit())
                    && self.peek_at(4).is_some_and(|c| c.is_ascii_digit())
                    && self.peek_at(5) == Some(b'-')
                {
                    self.advance();
                    return Ok(Some(Token::new(TokenType::Modifier, start_pos, self.pos, start_line, start_col)));
                }
                // Check if this is a negative number (followed by digit)
                if self.peek_at(1).is_some_and(|c| c.is_ascii_digit()) {
                    return Ok(Some(self.scan_bare_value()));
                }
                // Otherwise it's a deprecated modifier
                self.advance();
                Ok(Some(Token::new(TokenType::Modifier, start_pos, self.pos, start_line, start_col)))
            }
            b',' => {
                self.advance();
                Ok(Some(Token::new(TokenType::Comma, start_pos, self.pos, start_line, start_col)))
            }
            b':' => {
                Ok(Some(self.scan_directive()))
            }
            b'|' => {
                self.advance();
                Ok(Some(Token::new(TokenType::Pipe, start_pos, self.pos, start_line, start_col)))
            }
            b'0'..=b'9' => {
                if self.looks_like_date() {
                    Ok(Some(self.scan_date_or_timestamp()))
                } else {
                    Ok(Some(self.scan_number()))
                }
            }
            b'&' => {
                Ok(Some(self.scan_extension_path()))
            }
            _ if is_identifier_start(ch) => {
                Ok(Some(self.scan_identifier()?))
            }
            _ => {
                Ok(Some(self.scan_bare_value()))
            }
        }
    }

    /// Check if the current position looks like a date (YYYY-MM-DD).
    fn looks_like_date(&self) -> bool {
        if self.pos + 10 > self.bytes.len() {
            return false;
        }
        for i in 0..4 {
            if !self.bytes[self.pos + i].is_ascii_digit() {
                return false;
            }
        }
        self.bytes[self.pos + 4] == b'-'
            && self.bytes[self.pos + 5].is_ascii_digit()
            && self.bytes[self.pos + 6].is_ascii_digit()
            && self.bytes[self.pos + 7] == b'-'
            && self.bytes[self.pos + 8].is_ascii_digit()
            && self.bytes[self.pos + 9].is_ascii_digit()
    }
}

#[inline]
fn is_identifier_start(ch: u8) -> bool {
    ch.is_ascii_alphabetic() || ch == b'_' || ch == b'$'
}

#[inline]
fn utf8_char_len(first: u8) -> usize {
    if first < 0x80 { 1 }
    else if first < 0xE0 { 2 }
    else if first < 0xF0 { 3 }
    else { 4 }
}

/// Find the start of a comment (`;`) in a string, respecting quotes.
fn find_comment_start(s: &str) -> Option<usize> {
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

/// Tokenize ODIN source text into a vector of tokens.
pub fn tokenize<'a>(source: &'a str, options: &ParseOptions) -> Result<Vec<Token>, ParseError> {
    if source.len() > options.max_size {
        return Err(ParseError::new(ParseErrorCode::MaximumDocumentSizeExceeded, 1, 1));
    }
    // Token positions are u32; reject sources that would overflow.
    if source.len() > u32::MAX as usize {
        return Err(ParseError::new(ParseErrorCode::MaximumDocumentSizeExceeded, 1, 1));
    }

    let mut tokenizer = Tokenizer::new(source);
    // Scalar-heavy ODIN docs run ~7-8 bytes per token; previous /12 estimate
    // forced a realloc partway through tokenization for typical inputs.
    let estimated_size = source.len() / 8 + 16;
    let mut tokens = Vec::with_capacity(estimated_size);

    while !tokenizer.is_at_end() {
        if let Some(token) = tokenizer.next_token()? {
            tokens.push(token);
        }
    }

    tokens.push(Token::new(
        TokenType::Eof,
        tokenizer.pos,
        tokenizer.pos,
        tokenizer.line,
        tokenizer.column(),
    ));

    Ok(tokens)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::options::ParseOptions;

    fn opts() -> ParseOptions {
        ParseOptions::default()
    }

    /// Test-only token wrapper: eagerly resolves `value` (and unescapes
    /// `QuotedStringEscaped` to `QuotedString`) so existing tests keep their
    /// `tokens[i].value` field-access pattern working.
    pub struct TestToken {
        pub token_type: TokenType,
        pub value: String,
        pub start: u32,
        pub end: u32,
        pub line: u32,
        pub column: u32,
    }

    /// Helper: tokenize and return value-resolved tokens for assertions.
    fn tok(input: &str) -> Vec<TestToken> {
        tokenize(input, &opts()).unwrap()
            .into_iter()
            .map(|t| {
                let raw = t.value(input);
                let (token_type, value) = match t.token_type {
                    TokenType::QuotedStringEscaped => {
                        (TokenType::QuotedString, super::super::parse_values::unescape_string(raw))
                    }
                    other => (other, raw.to_string()),
                };
                TestToken {
                    token_type,
                    value,
                    start: t.start,
                    end: t.end,
                    line: t.line,
                    column: t.column,
                }
            })
            .collect()
    }

    fn types(input: &str) -> Vec<TokenType> {
        tok(input).iter().map(|t| t.token_type).collect()
    }

    fn values(input: &str) -> Vec<String> {
        tok(input).iter().map(|t| t.value.clone()).collect()
    }

    // ── Empty / minimal input ─────────────────────────────────────────────

    #[test]
    fn empty_input_produces_eof() {
        let tokens = tok("");
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0].token_type, TokenType::Eof);
    }

    #[test]
    fn whitespace_only_produces_eof() {
        let tokens = tok("   \t  ");
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0].token_type, TokenType::Eof);
    }

    #[test]
    fn newline_only() {
        let t = types("\n");
        assert_eq!(t, vec![TokenType::Newline, TokenType::Eof]);
    }

    #[test]
    fn multiple_empty_lines() {
        let t = types("\n\n\n");
        assert_eq!(t, vec![TokenType::Newline, TokenType::Newline, TokenType::Newline, TokenType::Eof]);
    }

    // ── Key-value pair ────────────────────────────────────────────────────

    #[test]
    fn simple_key_value() {
        let t = types("name = \"Alice\"");
        assert_eq!(t, vec![TokenType::Path, TokenType::Equals, TokenType::QuotedString, TokenType::Eof]);
    }

    #[test]
    fn key_value_values() {
        let v = values("name = \"Alice\"");
        assert_eq!(v[0], "name");
        assert_eq!(v[1], "=");
        assert_eq!(v[2], "Alice");
    }

    #[test]
    fn multiple_key_values() {
        let input = "name = \"Alice\"\nage = ##30";
        let t = types(input);
        assert_eq!(t, vec![
            TokenType::Path, TokenType::Equals, TokenType::QuotedString, TokenType::Newline,
            TokenType::Path, TokenType::Equals, TokenType::IntegerPrefix, TokenType::Eof,
        ]);
    }

    // ── Quoted strings ────────────────────────────────────────────────────

    #[test]
    fn simple_quoted_string() {
        let tokens = tok("\"hello world\"");
        assert_eq!(tokens[0].token_type, TokenType::QuotedString);
        assert_eq!(tokens[0].value, "hello world");
    }

    #[test]
    fn empty_quoted_string() {
        let tokens = tok("\"\"");
        assert_eq!(tokens[0].token_type, TokenType::QuotedString);
        assert_eq!(tokens[0].value, "");
    }

    #[test]
    fn string_with_newline_escape() {
        let tokens = tok("\"line1\\nline2\"");
        assert_eq!(tokens[0].value, "line1\nline2");
    }

    #[test]
    fn string_with_tab_escape() {
        let tokens = tok("\"col1\\tcol2\"");
        assert_eq!(tokens[0].value, "col1\tcol2");
    }

    #[test]
    fn string_with_backslash_escape() {
        let tokens = tok("\"path\\\\file\"");
        assert_eq!(tokens[0].value, "path\\file");
    }

    #[test]
    fn string_with_quote_escape() {
        let tokens = tok("\"say \\\"hi\\\"\"");
        assert_eq!(tokens[0].value, "say \"hi\"");
    }

    #[test]
    fn string_with_slash_escape() {
        let tokens = tok("\"a\\/b\"");
        assert_eq!(tokens[0].value, "a/b");
    }

    #[test]
    fn string_with_null_escape() {
        let tokens = tok("\"a\\0b\"");
        assert_eq!(tokens[0].value, "a\0b");
    }

    #[test]
    fn string_with_unicode_escape() {
        let tokens = tok("\"\\u0041\"");
        assert_eq!(tokens[0].value, "A");
    }

    #[test]
    fn string_with_unicode_escape_8_digit() {
        let tokens = tok("\"\\U00000041\"");
        assert_eq!(tokens[0].value, "A");
    }

    #[test]
    fn string_with_emoji_via_big_u() {
        let tokens = tok("\"\\U0001F600\"");
        assert_eq!(tokens[0].value, "\u{1F600}");
    }

    #[test]
    fn string_with_return_escape() {
        let tokens = tok("\"a\\rb\"");
        assert_eq!(tokens[0].value, "a\rb");
    }

    #[test]
    fn string_no_escapes_preserves_content() {
        let tokens = tok("\"no escapes here\"");
        assert_eq!(tokens[0].value, "no escapes here");
    }

    #[test]
    fn unterminated_string_error() {
        let result = tokenize("\"no closing quote", &opts());
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.error_code, ParseErrorCode::UnterminatedString);
    }

    #[test]
    fn string_with_literal_newline_error() {
        let result = tokenize("\"line1\nline2\"", &opts());
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().error_code, ParseErrorCode::UnterminatedString);
    }

    #[test]
    fn invalid_escape_sequence_error() {
        let result = tokenize("\"bad \\q escape\"", &opts());
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().error_code, ParseErrorCode::InvalidEscapeSequence);
    }

    // ── Number prefix # ──────────────────────────────────────────────────

    #[test]
    fn number_prefix_integer_value() {
        let tokens = tok("#42");
        assert_eq!(tokens[0].token_type, TokenType::NumberPrefix);
        assert_eq!(tokens[0].value, "42");
    }

    #[test]
    fn number_prefix_decimal() {
        let tokens = tok("#3.14");
        assert_eq!(tokens[0].token_type, TokenType::NumberPrefix);
        assert_eq!(tokens[0].value, "3.14");
    }

    #[test]
    fn number_prefix_negative() {
        let tokens = tok("#-5.5");
        assert_eq!(tokens[0].token_type, TokenType::NumberPrefix);
        assert_eq!(tokens[0].value, "-5.5");
    }

    #[test]
    fn number_prefix_scientific() {
        let tokens = tok("#1.5e10");
        assert_eq!(tokens[0].token_type, TokenType::NumberPrefix);
        assert_eq!(tokens[0].value, "1.5e10");
    }

    #[test]
    fn number_prefix_zero() {
        let tokens = tok("#0");
        assert_eq!(tokens[0].token_type, TokenType::NumberPrefix);
        assert_eq!(tokens[0].value, "0");
    }

    #[test]
    fn bare_hash_error() {
        let result = tokenize("#abc", &opts());
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().error_code, ParseErrorCode::InvalidTypePrefix);
    }

    // ── Integer prefix ## ────────────────────────────────────────────────

    #[test]
    fn integer_prefix_positive() {
        let tokens = tok("##42");
        assert_eq!(tokens[0].token_type, TokenType::IntegerPrefix);
        assert_eq!(tokens[0].value, "42");
    }

    #[test]
    fn integer_prefix_negative() {
        let tokens = tok("##-7");
        assert_eq!(tokens[0].token_type, TokenType::IntegerPrefix);
        assert_eq!(tokens[0].value, "-7");
    }

    #[test]
    fn integer_prefix_zero() {
        let tokens = tok("##0");
        assert_eq!(tokens[0].token_type, TokenType::IntegerPrefix);
        assert_eq!(tokens[0].value, "0");
    }

    // ── Currency prefix #$ ───────────────────────────────────────────────

    #[test]
    fn currency_prefix_basic() {
        let tokens = tok("#$99.99");
        assert_eq!(tokens[0].token_type, TokenType::CurrencyPrefix);
        assert_eq!(tokens[0].value, "99.99");
    }

    #[test]
    fn currency_prefix_with_code() {
        let tokens = tok("#$100.00:USD");
        assert_eq!(tokens[0].token_type, TokenType::CurrencyPrefix);
        assert_eq!(tokens[0].value, "100.00:USD");
    }

    #[test]
    fn currency_prefix_negative() {
        let tokens = tok("#$-50.00");
        assert_eq!(tokens[0].token_type, TokenType::CurrencyPrefix);
        assert_eq!(tokens[0].value, "-50.00");
    }

    // ── Percent prefix #% ────────────────────────────────────────────────

    #[test]
    fn percent_prefix_basic() {
        let tokens = tok("#%85.5");
        assert_eq!(tokens[0].token_type, TokenType::PercentPrefix);
        assert_eq!(tokens[0].value, "85.5");
    }

    #[test]
    fn percent_prefix_integer() {
        let tokens = tok("#%100");
        assert_eq!(tokens[0].token_type, TokenType::PercentPrefix);
        assert_eq!(tokens[0].value, "100");
    }

    // ── Boolean ──────────────────────────────────────────────────────────

    #[test]
    fn boolean_true_literal() {
        let tokens = tok("true");
        assert_eq!(tokens[0].token_type, TokenType::BooleanLiteral);
        assert_eq!(tokens[0].value, "true");
    }

    #[test]
    fn boolean_false_literal() {
        let tokens = tok("false");
        assert_eq!(tokens[0].token_type, TokenType::BooleanLiteral);
        assert_eq!(tokens[0].value, "false");
    }

    #[test]
    fn boolean_prefix_question_mark() {
        let tokens = tok("?");
        assert_eq!(tokens[0].token_type, TokenType::BooleanPrefix);
        assert_eq!(tokens[0].value, "?");
    }

    // ── Null ─────────────────────────────────────────────────────────────

    #[test]
    fn null_tilde() {
        let tokens = tok("~");
        assert_eq!(tokens[0].token_type, TokenType::Null);
        assert_eq!(tokens[0].value, "~");
    }

    // ── Reference @ ──────────────────────────────────────────────────────

    #[test]
    fn reference_simple_path() {
        let tokens = tok("x = @somePath");
        let ref_tok = &tokens[2];
        assert_eq!(ref_tok.token_type, TokenType::ReferencePrefix);
        assert_eq!(ref_tok.value, "somePath");
    }

    #[test]
    fn reference_dotted_path() {
        let tokens = tok("x = @parent.child");
        let ref_tok = &tokens[2];
        assert_eq!(ref_tok.token_type, TokenType::ReferencePrefix);
        assert_eq!(ref_tok.value, "parent.child");
    }

    #[test]
    fn bare_at_sign() {
        let tokens = tok("x = @");
        let ref_tok = &tokens[2];
        assert_eq!(ref_tok.token_type, TokenType::ReferencePrefix);
        assert_eq!(ref_tok.value, "");
    }

    // ── Binary ^ ─────────────────────────────────────────────────────────

    #[test]
    fn binary_prefix_base64() {
        let tokens = tok("^SGVsbG8=");
        assert_eq!(tokens[0].token_type, TokenType::BinaryPrefix);
        assert_eq!(tokens[0].value, "SGVsbG8=");
    }

    #[test]
    fn binary_prefix_empty() {
        let tokens = tok("^ ");
        assert_eq!(tokens[0].token_type, TokenType::BinaryPrefix);
        assert_eq!(tokens[0].value, "");
    }

    // ── Section headers ──────────────────────────────────────────────────

    #[test]
    fn section_header_simple() {
        let tokens = tok("{Policy}");
        assert_eq!(tokens[0].token_type, TokenType::Header);
        assert_eq!(tokens[0].value, "Policy");
    }

    #[test]
    fn section_header_nested_dot() {
        let tokens = tok("{Parent.Child.Grandchild}");
        assert_eq!(tokens[0].token_type, TokenType::Header);
        assert_eq!(tokens[0].value, "Parent.Child.Grandchild");
    }

    #[test]
    fn section_header_deeply_nested() {
        let tokens = tok("{A.B.C.D.E}");
        assert_eq!(tokens[0].token_type, TokenType::Header);
        assert_eq!(tokens[0].value, "A.B.C.D.E");
    }

    #[test]
    fn metadata_section_header() {
        let tokens = tok("{$}");
        assert_eq!(tokens[0].token_type, TokenType::Header);
        assert_eq!(tokens[0].value, "$");
    }

    #[test]
    fn section_header_unclosed_error() {
        let result = tokenize("{NoClose\n", &opts());
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().error_code, ParseErrorCode::InvalidHeaderSyntax);
    }

    #[test]
    fn section_header_unterminated_eof_error() {
        let result = tokenize("{NoClose", &opts());
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().error_code, ParseErrorCode::InvalidHeaderSyntax);
    }

    #[test]
    fn section_header_with_array_push() {
        let tokens = tok("{employees[]}");
        assert_eq!(tokens[0].token_type, TokenType::Header);
        assert_eq!(tokens[0].value, "employees[]");
    }

    #[test]
    fn section_header_with_index() {
        let tokens = tok("{items[0]}");
        assert_eq!(tokens[0].token_type, TokenType::Header);
        assert_eq!(tokens[0].value, "items[0]");
    }

    // ── Array notation ───────────────────────────────────────────────────

    #[test]
    fn array_index_path() {
        let tokens = tok("items[0]");
        assert_eq!(tokens[0].token_type, TokenType::Path);
        assert_eq!(tokens[0].value, "items[0]");
    }

    #[test]
    fn array_nested_path() {
        let tokens = tok("data[2].name");
        assert_eq!(tokens[0].token_type, TokenType::Path);
        assert_eq!(tokens[0].value, "data[2].name");
    }

    #[test]
    fn negative_array_index_error() {
        let result = tokenize("items[-1] = \"x\"", &opts());
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().error_code, ParseErrorCode::InvalidArrayIndex);
    }

    // ── Comments ─────────────────────────────────────────────────────────

    #[test]
    fn comment_standalone() {
        let tokens = tok("; this is a comment");
        assert_eq!(tokens[0].token_type, TokenType::Comment);
        assert_eq!(tokens[0].value, "; this is a comment");
    }

    #[test]
    fn comment_after_value() {
        let tokens = tok("name = \"Alice\" ; inline comment");
        let types: Vec<_> = tokens.iter().map(|t| t.token_type).collect();
        assert!(types.contains(&TokenType::Comment));
    }

    #[test]
    fn comment_empty() {
        let tokens = tok(";");
        assert_eq!(tokens[0].token_type, TokenType::Comment);
        assert_eq!(tokens[0].value, ";");
    }

    // ── Document separator ───────────────────────────────────────────────

    #[test]
    fn document_separator() {
        let tokens = tok("---");
        assert_eq!(tokens[0].token_type, TokenType::DocumentSeparator);
        assert_eq!(tokens[0].value, "---");
    }

    #[test]
    fn document_separator_between_docs() {
        let input = "a = \"1\"\n---\nb = \"2\"";
        let t = types(input);
        assert!(t.contains(&TokenType::DocumentSeparator));
    }

    // ── Modifier prefixes ────────────────────────────────────────────────

    #[test]
    fn modifier_required() {
        let tokens = tok("!");
        assert_eq!(tokens[0].token_type, TokenType::Modifier);
        assert_eq!(tokens[0].value, "!");
    }

    #[test]
    fn modifier_confidential() {
        let tokens = tok("*");
        assert_eq!(tokens[0].token_type, TokenType::Modifier);
        assert_eq!(tokens[0].value, "*");
    }

    #[test]
    fn modifier_deprecated() {
        let tokens = tok("x = -\"old\"");
        let mod_tok = tokens.iter().find(|t| t.token_type == TokenType::Modifier).unwrap();
        assert_eq!(mod_tok.value, "-");
    }

    #[test]
    fn modifier_combined_required_confidential() {
        let input = "x = !*\"secret\"";
        let t = types(input);
        assert_eq!(t[2], TokenType::Modifier);
        assert_eq!(t[3], TokenType::Modifier);
    }

    #[test]
    fn all_three_modifiers() {
        let input = "x = !-*\"legacy_secret\"";
        let mods: Vec<_> = tok(input).iter()
            .filter(|t| t.token_type == TokenType::Modifier)
            .map(|t| t.value.to_string())
            .collect();
        assert_eq!(mods, vec!["!", "-", "*"]);
    }

    // ── Date/time/duration ───────────────────────────────────────────────

    #[test]
    fn date_literal() {
        let tokens = tok("2024-06-15");
        assert_eq!(tokens[0].token_type, TokenType::DateLiteral);
        assert_eq!(tokens[0].value, "2024-06-15");
    }

    #[test]
    fn timestamp_literal() {
        let tokens = tok("2024-06-15T14:30:00Z");
        assert_eq!(tokens[0].token_type, TokenType::TimestampLiteral);
        assert_eq!(tokens[0].value, "2024-06-15T14:30:00Z");
    }

    #[test]
    fn time_literal() {
        let tokens = tok("T10:30:00");
        assert_eq!(tokens[0].token_type, TokenType::TimeLiteral);
        assert_eq!(tokens[0].value, "T10:30:00");
    }

    #[test]
    fn time_literal_with_fraction() {
        let tokens = tok("T10:30:00.500");
        assert_eq!(tokens[0].token_type, TokenType::TimeLiteral);
        assert_eq!(tokens[0].value, "T10:30:00.500");
    }

    #[test]
    fn duration_basic() {
        let tokens = tok("P1Y6M");
        assert_eq!(tokens[0].token_type, TokenType::DurationLiteral);
        assert_eq!(tokens[0].value, "P1Y6M");
    }

    #[test]
    fn duration_with_time() {
        let tokens = tok("P1DT12H");
        assert_eq!(tokens[0].token_type, TokenType::DurationLiteral);
        assert_eq!(tokens[0].value, "P1DT12H");
    }

    #[test]
    fn duration_week() {
        let tokens = tok("P2W");
        assert_eq!(tokens[0].token_type, TokenType::DurationLiteral);
        assert_eq!(tokens[0].value, "P2W");
    }

    // ── Directive ────────────────────────────────────────────────────────

    #[test]
    fn directive_token() {
        let tokens = tok(":pos");
        assert_eq!(tokens[0].token_type, TokenType::Directive);
        assert_eq!(tokens[0].value, "pos");
    }

    #[test]
    fn directive_with_value() {
        let input = ":len 8";
        let tokens = tok(input);
        assert_eq!(tokens[0].token_type, TokenType::Directive);
        assert_eq!(tokens[0].value, "len");
    }

    // ── Import and Schema ────────────────────────────────────────────────

    #[test]
    fn import_directive() {
        let tokens = tok("@import ./types.odin");
        assert_eq!(tokens[0].token_type, TokenType::Import);
        assert_eq!(tokens[0].value, "./types.odin");
    }

    #[test]
    fn import_with_alias() {
        let tokens = tok("@import ./base.odin as base");
        assert_eq!(tokens[0].token_type, TokenType::Import);
        assert_eq!(tokens[0].value, "./base.odin as base");
    }

    #[test]
    fn schema_directive() {
        let tokens = tok("@schema ./policy.schema.odin");
        assert_eq!(tokens[0].token_type, TokenType::Schema);
        assert_eq!(tokens[0].value, "./policy.schema.odin");
    }

    // ── Conditional ──────────────────────────────────────────────────────

    #[test]
    fn conditional_directive() {
        let tokens = tok("@if state == \"CA\"");
        assert_eq!(tokens[0].token_type, TokenType::Conditional);
    }

    // ── Verb prefix ──────────────────────────────────────────────────────

    #[test]
    fn verb_prefix() {
        let tokens = tok("%map");
        assert_eq!(tokens[0].token_type, TokenType::VerbPrefix);
        assert_eq!(tokens[0].value, "map");
    }

    #[test]
    fn verb_prefix_custom() {
        let tokens = tok("%&customVerb");
        assert_eq!(tokens[0].token_type, TokenType::VerbPrefix);
        assert_eq!(tokens[0].value, "&customVerb");
    }

    // ── Pipe and Comma ───────────────────────────────────────────────────

    #[test]
    fn pipe_token() {
        let tokens = tok("|");
        assert_eq!(tokens[0].token_type, TokenType::Pipe);
    }

    #[test]
    fn comma_token() {
        let tokens = tok(",");
        assert_eq!(tokens[0].token_type, TokenType::Comma);
    }

    // ── Equals ───────────────────────────────────────────────────────────

    #[test]
    fn equals_token() {
        let tokens = tok("=");
        assert_eq!(tokens[0].token_type, TokenType::Equals);
        assert_eq!(tokens[0].value, "=");
    }

    // ── Whitespace handling ──────────────────────────────────────────────

    #[test]
    fn tabs_are_whitespace() {
        let tokens = tok("name\t=\t\"val\"");
        let t: Vec<_> = tokens.iter().map(|t| t.token_type).collect();
        assert_eq!(t, vec![TokenType::Path, TokenType::Equals, TokenType::QuotedString, TokenType::Eof]);
    }

    #[test]
    fn crlf_newlines() {
        let tokens = tok("a = \"1\"\r\nb = \"2\"");
        let newlines: Vec<_> = tokens.iter().filter(|t| t.token_type == TokenType::Newline).collect();
        assert_eq!(newlines.len(), 1);
    }

    // ── Identifier / path parsing ────────────────────────────────────────

    #[test]
    fn identifier_with_underscores() {
        let tokens = tok("my_field");
        assert_eq!(tokens[0].token_type, TokenType::Path);
        assert_eq!(tokens[0].value, "my_field");
    }

    #[test]
    fn dotted_path_identifier() {
        let tokens = tok("policy.number");
        assert_eq!(tokens[0].token_type, TokenType::Path);
        assert_eq!(tokens[0].value, "policy.number");
    }

    #[test]
    fn deeply_nested_dotted_path() {
        let tokens = tok("a.b.c.d.e.f");
        assert_eq!(tokens[0].token_type, TokenType::Path);
        assert_eq!(tokens[0].value, "a.b.c.d.e.f");
    }

    // ── Numeric literal in value position ────────────────────────────────

    #[test]
    fn numeric_literal_bare() {
        let tokens = tok("42");
        assert_eq!(tokens[0].token_type, TokenType::NumericLiteral);
        assert_eq!(tokens[0].value, "42");
    }

    #[test]
    fn numeric_literal_decimal_bare() {
        let tokens = tok("3.14");
        assert_eq!(tokens[0].token_type, TokenType::NumericLiteral);
        assert_eq!(tokens[0].value, "3.14");
    }

    // ── Line number tracking ─────────────────────────────────────────────

    #[test]
    fn line_numbers_correct() {
        let input = "a = \"1\"\nb = \"2\"\nc = \"3\"";
        let tokens = tok(input);
        let paths: Vec<_> = tokens.iter().filter(|t| t.token_type == TokenType::Path).collect();
        assert_eq!(paths[0].line, 1);
        assert_eq!(paths[1].line, 2);
        assert_eq!(paths[2].line, 3);
    }

    #[test]
    fn column_numbers_correct() {
        let tokens = tok("name = \"Alice\"");
        assert_eq!(tokens[0].column, 1); // name
        assert_eq!(tokens[1].column, 6); // =
        assert_eq!(tokens[2].column, 8); // "Alice"
    }

    // ── Max size limit ───────────────────────────────────────────────────

    #[test]
    fn max_size_exceeded_error() {
        let mut opts = ParseOptions::default();
        opts.max_size = 10;
        let result = tokenize("this is a very long input that exceeds limit", &opts);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().error_code, ParseErrorCode::MaximumDocumentSizeExceeded);
    }

    // ── Multiple sections ────────────────────────────────────────────────

    #[test]
    fn multiple_sections_tokenized() {
        let input = "{A}\nfoo = \"1\"\n{B}\nbar = \"2\"";
        let headers: Vec<_> = tok(input).iter()
            .filter(|t| t.token_type == TokenType::Header)
            .map(|t| t.value.to_string())
            .collect();
        assert_eq!(headers, vec!["A", "B"]);
    }

    // ── @ directive errors ───────────────────────────────────────────────

    #[test]
    fn invalid_at_directive_at_col1() {
        let result = tokenize("@badDirective", &opts());
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().error_code, ParseErrorCode::UnexpectedCharacter);
    }

    // ── Bare value scanning ──────────────────────────────────────────────

    #[test]
    fn bare_value_negative_number() {
        let tokens = tok("x = -42");
        let bare = tokens.iter().find(|t| t.token_type == TokenType::BareWord).unwrap();
        assert_eq!(bare.value, "-42");
    }

    // ── UTF-8 in strings ─────────────────────────────────────────────────

    #[test]
    fn utf8_string_content() {
        let tokens = tok("\"cafe\u{0301}\"");
        assert_eq!(tokens[0].token_type, TokenType::QuotedString);
        assert!(tokens[0].value.contains("caf"));
    }

    #[test]
    fn utf8_emoji_string() {
        let tokens = tok("\"\u{1F600}\"");
        assert_eq!(tokens[0].token_type, TokenType::QuotedString);
    }

    // ── find_comment_start helper ────────────────────────────────────────

    #[test]
    fn find_comment_start_simple() {
        assert_eq!(find_comment_start("hello ; world"), Some(6));
    }

    #[test]
    fn find_comment_start_in_quotes() {
        assert_eq!(find_comment_start("\"no ; comment\""), None);
    }

    #[test]
    fn find_comment_start_after_quotes() {
        assert_eq!(find_comment_start("\"quoted\" ; real"), Some(9));
    }

    #[test]
    fn find_comment_start_no_comment() {
        assert_eq!(find_comment_start("no comment here"), None);
    }
}
