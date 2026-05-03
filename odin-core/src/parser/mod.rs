//! ODIN text parser.
//!
//! Converts ODIN text into an `OdinDocument` via a single-pass byte walker
//! (`parser`). Handles scalars, sections (including indexed
//! `{records[N]}` and `{@TypeRef}`), tabular shapes (record-style,
//! primitive arrays, relative dotted sub-blocks, lookup tables via
//! `{$table.NAME[...]}`), modifiers, trailing directives, verbs, binary,
//! document separators, `@import` / `@schema` / `@if` directives, and
//! (when enabled) comment preservation.
//!
//! For chunk-based incremental parsing, see [`streaming`].
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
mod parser;
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
    parser::parse(source, opts)
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
    parser::parse_documents(source, opts)
}
