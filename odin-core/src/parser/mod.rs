//! ODIN text parser.
//!
//! Converts ODIN text into an `OdinDocument`.
//!
//! The parser operates in two phases:
//! 1. **Tokenization** — single-pass character scanner produces token stream
//! 2. **Parsing** — token stream is consumed to build the document
//!
//! # Example
//!
//! ```rust
//! use odin_core::parser;
//!
//! let doc = parser::parse("name = \"Alice\"\nage = ##30", None).unwrap();
//! assert_eq!(doc.get_string("name"), Some("Alice"));
//! ```

mod tokenizer;
mod tokens;
mod parser_impl;
mod parse_values;
pub mod streaming;

#[cfg(test)]
mod tests;

pub use crate::types::options::ParseOptions;
pub use tokens::{Token, TokenType};
// Token<'a> uses Cow<'a, str> — most values are zero-copy borrows from source text.

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
    let tokens = tokenizer::tokenize(input, opts)?;
    parser_impl::parse_tokens(&tokens, input, opts)
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
    let tokens = tokenizer::tokenize(input, opts)?;
    parser_impl::parse_tokens_multi(&tokens, input, opts)
}
