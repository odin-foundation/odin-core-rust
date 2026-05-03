//! Token types for the ODIN tokenizer.
//!
//! Tokens carry only byte offsets into the source string — the value text
//! is recovered on demand via [`Token::value`]. This keeps `Token` to ~20
//! bytes (vs ~48 bytes when it stored `Cow<'a, str>`) so the per-document
//! token vector fits in L1 cache for typical workloads.

/// Types of tokens produced by the tokenizer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenType {
    Path,
    Equals,
    /// A quoted string with no escape sequences. `start..end` covers the
    /// content between the quotes.
    QuotedString,
    /// A quoted string containing escape sequences. `start..end` covers
    /// the raw content between the quotes; consumers must unescape.
    QuotedStringEscaped,
    BareWord,
    NumberPrefix,
    IntegerPrefix,
    CurrencyPrefix,
    PercentPrefix,
    BooleanPrefix,
    Null,
    /// `@path` reference. `start..end` covers the raw path text after `@`;
    /// consumers must normalize leading zeros in array indices.
    ReferencePrefix,
    BinaryPrefix,
    VerbPrefix,
    /// A section header. `start..end` covers the content between `{}`.
    Header,
    /// A line comment. `start..end` covers the full text starting at `;`.
    Comment,
    Directive,
    Import,
    Schema,
    Newline,
    Eof,
    NumericLiteral,
    BooleanLiteral,
    DateLiteral,
    TimestampLiteral,
    TimeLiteral,
    DurationLiteral,
    Modifier,
    Pipe,
    DocumentSeparator,
    Conditional,
    Comma,
}

/// A single token. The `value` text is recovered as `&source[start..end]`.
/// `start..end` is the *logical value range* — quotes, braces, and the
/// leading `@` of references are NOT included.
#[derive(Debug, Clone, Copy)]
pub struct Token {
    pub start: u32,
    pub end: u32,
    pub line: u32,
    pub column: u32,
    pub token_type: TokenType,
}

impl Token {
    #[inline]
    pub fn new(
        token_type: TokenType,
        start: usize,
        end: usize,
        line: usize,
        column: usize,
    ) -> Self {
        Self {
            token_type,
            start: start as u32,
            end: end as u32,
            line: line as u32,
            column: column as u32,
        }
    }

    /// The token's text content as a slice of the source string.
    #[inline]
    pub fn value<'s>(&self, source: &'s str) -> &'s str {
        &source[self.start as usize..self.end as usize]
    }
}
