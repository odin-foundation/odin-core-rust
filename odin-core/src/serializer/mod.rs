//! ODIN serializer — converts `OdinDocument` to ODIN text.

mod stringify_impl;
mod canonicalize_impl;

pub use crate::types::options::StringifyOptions;

use crate::types::document::OdinDocument;

/// Serialize an `OdinDocument` to ODIN text.
pub fn stringify(doc: &OdinDocument, options: Option<&StringifyOptions>) -> String {
    stringify_impl::stringify(doc, options)
}

/// Produce a canonical (deterministic, byte-identical) serialization.
///
/// The canonical form:
/// - Sorts all keys alphabetically
/// - Uses consistent quoting and formatting
/// - Produces identical output for semantically equivalent documents
pub fn canonicalize(doc: &OdinDocument) -> Vec<u8> {
    canonicalize_impl::canonicalize(doc)
}
