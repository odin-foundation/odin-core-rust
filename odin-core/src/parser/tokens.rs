//! Token types for the ODIN tokenizer.

use std::borrow::Cow;

/// Types of tokens produced by the tokenizer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenType {
    /// A path segment (e.g., `name`, `policy.number`, `items[0]`).
    Path,
    /// The `=` assignment operator.
    Equals,
    /// A quoted string value (e.g., `"hello"`).
    QuotedString,
    /// A bare word value (unquoted string).
    BareWord,
    /// A number prefix `#`.
    NumberPrefix,
    /// An integer prefix `##`.
    IntegerPrefix,
    /// A currency prefix `#$`.
    CurrencyPrefix,
    /// A percent prefix `#%`.
    PercentPrefix,
    /// A boolean prefix `?` (optional).
    BooleanPrefix,
    /// A null value `~`.
    Null,
    /// A reference prefix `@`.
    ReferencePrefix,
    /// A binary prefix `^`.
    BinaryPrefix,
    /// A verb prefix `%`.
    VerbPrefix,
    /// A section header (e.g., `{Policy}`, `{$}`).
    Header,
    /// A comment (`;` to end of line).
    Comment,
    /// A directive (e.g., `:pos`, `:len`, `:format`).
    Directive,
    /// An `@import` directive.
    Import,
    /// An `@schema` directive.
    Schema,
    /// A newline.
    Newline,
    /// End of file.
    Eof,
    /// A numeric literal (the digits following a prefix).
    NumericLiteral,
    /// A boolean literal (`true` or `false`).
    BooleanLiteral,
    /// A date literal (e.g., `2024-06-15`).
    DateLiteral,
    /// A timestamp literal (e.g., `2024-06-15T14:30:00Z`).
    TimestampLiteral,
    /// A time literal (e.g., `T14:30:00`).
    TimeLiteral,
    /// A duration literal (e.g., `P1Y6M`).
    DurationLiteral,
    /// A modifier prefix (`!`, `*`, `-`).
    Modifier,
    /// Tabular column separator `|`.
    Pipe,
    /// Document separator `---`.
    DocumentSeparator,
    /// An `@if` conditional directive.
    Conditional,
    /// Comma separator `,`.
    Comma,
}

/// A token produced by the tokenizer.
///
/// Uses `Cow<'a, str>` for the value field to avoid heap allocations.
/// Most token values are borrowed slices of the source text; only tokens
/// that require processing (e.g., strings with escape sequences) allocate.
#[derive(Debug, Clone)]
pub struct Token<'a> {
    /// The token type.
    pub token_type: TokenType,
    /// Byte offset in the source text where the token starts.
    pub start: usize,
    /// Byte offset in the source text where the token ends (exclusive).
    pub end: usize,
    /// Line number (1-based).
    pub line: usize,
    /// Column number (1-based).
    pub column: usize,
    /// The token's text content — borrowed from source when possible.
    pub value: Cow<'a, str>,
}

impl<'a> Token<'a> {
    /// Create a new token with a borrowed value (zero allocation).
    #[inline]
    pub fn borrowed(
        token_type: TokenType,
        start: usize,
        end: usize,
        line: usize,
        column: usize,
        value: &'a str,
    ) -> Self {
        Self {
            token_type,
            start,
            end,
            line,
            column,
            value: Cow::Borrowed(value),
        }
    }

    /// Create a new token with an owned value (allocates).
    #[inline]
    pub fn owned(
        token_type: TokenType,
        start: usize,
        end: usize,
        line: usize,
        column: usize,
        value: String,
    ) -> Self {
        Self {
            token_type,
            start,
            end,
            line,
            column,
            value: Cow::Owned(value),
        }
    }
}
