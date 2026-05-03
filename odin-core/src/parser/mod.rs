//! ODIN text parser.
//!
//! Converts ODIN text into an `OdinDocument`.
//!
//! The primary path is `streaming_fast` — a single-pass byte walker that
//! never materializes a token stream. It handles scalars, sections,
//! tabular shapes (including primitive arrays and relative dotted
//! sub-blocks), modifiers, trailing directives, verbs, binary,
//! document separators, `@import` / `@schema` / `@if` directives,
//! and (when enabled) comment preservation.
//!
//! When the streaming parser sees a feature it doesn't implement
//! (`{$table.NAME[...]}` headers, multi-line headers, type-ref
//! header shapes), it bails and the tokenize+parse fallback in
//! `tokenizer` + `parser_impl` takes over.
//!
//! # Example
//!
//! ```rust
//! use odin_core::parser;
//!
//! let doc = parser::parse("name = \"Alice\"\nage = ##30", None).unwrap();
//! assert_eq!(doc.get_string("name"), Some("Alice"));
//! ```

mod parse_values;
mod streaming_fast;
pub mod streaming;

#[cfg(test)]
mod tests;

pub use crate::types::options::ParseOptions;

use crate::types::document::OdinDocument;
use crate::types::errors::ParseError;

/// Parse ODIN text into a document.
///
/// For document chains (separated by `---`), returns the last document.
///
/// # Errors
///
/// Returns `ParseError` if the input is not valid ODIN text.
pub fn parse(input: &str, options: Option<&ParseOptions>) -> Result<OdinDocument, ParseError> {
    let default_opts;
    let opts = match options {
        Some(o) => o,
        None => { default_opts = ParseOptions::default(); &default_opts }
    };
    let source = input.strip_prefix('\u{FEFF}').unwrap_or(input);
    streaming_fast::try_parse_fast(source, opts)
        .expect("streaming parser always returns a result")
}

/// Parse ODIN text into a chain of documents.
///
/// Returns all documents separated by `---`.
///
/// # Errors
///
/// Returns `ParseError` if the input is not valid ODIN text.
pub fn parse_documents(input: &str, options: Option<&ParseOptions>) -> Result<Vec<OdinDocument>, ParseError> {
    let default_opts;
    let opts = match options {
        Some(o) => o,
        None => { default_opts = ParseOptions::default(); &default_opts }
    };
    let source = input.strip_prefix('\u{FEFF}').unwrap_or(input);
    streaming_fast::parse_documents(source, opts)
}
