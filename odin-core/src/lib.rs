// Pedantic lint allows — Cargo.toml sets pedantic=warn but individual
// overrides require crate-level attributes to take effect.
#![allow(
    clippy::cast_lossless,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::doc_markdown,
    clippy::float_cmp,
    clippy::implicit_clone,
    clippy::implicit_hasher,
    clippy::items_after_statements,
    clippy::many_single_char_names,
    clippy::match_wildcard_for_single_variants,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::module_name_repetitions,
    clippy::must_use_candidate,
    clippy::needless_pass_by_value,
    clippy::option_if_let_else,
    clippy::ref_option,
    clippy::return_self_not_must_use,
    clippy::similar_names,
    clippy::struct_excessive_bools,
    clippy::too_many_lines,
    clippy::trivially_copy_pass_by_ref,
    clippy::unnecessary_wraps,
    clippy::unnested_or_patterns,
    clippy::unused_self,
    clippy::wildcard_imports,
)]
//! # ODIN Core
//!
//! Reference implementation of the ODIN (Open Data Interchange Notation) format in Rust.
//!
//! ODIN is a data interchange format designed for the AI era, combining token efficiency,
//! nesting capability, type safety, and human readability.
//!
//! ## Quick Start
//!
//! ```rust
//! use odin_core::{Odin, OdinValue, OdinValues};
//!
//! // Parse ODIN text
//! let doc = Odin::parse("name = \"Alice\"\nage = ##30").unwrap();
//!
//! // Access values
//! assert_eq!(doc.get_string("name"), Some("Alice"));
//! assert_eq!(doc.get_integer("age"), Some(30));
//!
//! // Serialize back to ODIN text
//! let text = Odin::stringify(&doc, None);
//! ```
//!
//! ## Modules
//!
//! - [`types`] - Core value types, document, errors, and modifiers
//! - [`parser`] - ODIN text parser (tokenizer + parser)
//! - [`serializer`] - ODIN document serializer (stringify + canonicalize)
//! - [`validator`] - Schema validation
//! - [`transform`] - Transform engine with 266 verbs
//! - [`diff`] - Document diff and patch
//! - `utils` - Internal utilities

pub mod types;
pub mod parser;
pub mod serializer;
pub mod validator;
pub mod transform;
pub mod diff;
pub mod resolver;
pub(crate) mod utils;

#[cfg(test)]
mod integration_tests;
#[cfg(test)]
mod security_tests;

// Re-export primary types at crate root for convenience
pub use types::values::{
    OdinValue, OdinValueType, OdinArrayItem, OdinModifiers, OdinDirective, OdinValues,
};
pub use types::document::{
    OdinDocument, OdinDocumentBuilder, OdinImport, OdinSchema as OdinSchemaRef,
    OdinConditional, OdinHeader, FlattenOptions,
};
pub use types::errors::{
    OdinError, OdinErrorKind, ParseError, PatchError, ValidationError,
    ParseErrorCode, ValidationErrorCode,
};
pub use types::ordered_map::OrderedMap;
pub use types::diff::OdinDiff;
pub use types::transform::{OdinTransform, DynValue, TransformResult};

/// Main entry point for ODIN operations.
///
/// Provides static methods for parsing, serializing, validating,
/// and transforming ODIN documents.
pub struct Odin;

impl Odin {
    /// Parse ODIN text into a document.
    ///
    /// # Errors
    ///
    /// Returns `ParseError` if the input is not valid ODIN.
    pub fn parse(input: &str) -> Result<OdinDocument, ParseError> {
        parser::parse(input, None)
    }

    /// Parse ODIN text with options.
    ///
    /// # Errors
    ///
    /// Returns `ParseError` if the input is not valid ODIN.
    pub fn parse_with_options(
        input: &str,
        options: &parser::ParseOptions,
    ) -> Result<OdinDocument, ParseError> {
        parser::parse(input, Some(options))
    }

    /// Serialize a document to ODIN text.
    pub fn stringify(doc: &OdinDocument, options: Option<&serializer::StringifyOptions>) -> String {
        serializer::stringify(doc, options)
    }

    /// Produce a canonical (deterministic, byte-identical) serialization.
    pub fn canonicalize(doc: &OdinDocument) -> Vec<u8> {
        serializer::canonicalize(doc)
    }

    /// Compute the diff between two documents.
    pub fn diff(a: &OdinDocument, b: &OdinDocument) -> OdinDiff {
        diff::diff(a, b)
    }

    /// Apply a diff to a document, producing a new document.
    ///
    /// # Errors
    ///
    /// Returns `PatchError` if the diff cannot be applied.
    pub fn patch(doc: &OdinDocument, diff: &OdinDiff) -> Result<OdinDocument, PatchError> {
        diff::patch(doc, diff)
    }

    /// Parse ODIN schema text into a schema definition.
    ///
    /// # Errors
    ///
    /// Returns `ParseError` if the input is not valid ODIN schema.
    pub fn parse_schema(input: &str) -> Result<types::schema::OdinSchemaDefinition, ParseError> {
        validator::schema_parser::parse_schema(input)
    }

    /// Validate a document against a schema.
    pub fn validate(
        doc: &OdinDocument,
        schema: &types::schema::OdinSchemaDefinition,
        options: Option<&types::options::ValidateOptions>,
    ) -> types::schema::ValidationResult {
        validator::validate(doc, schema, options)
    }

    /// Parse ODIN transform text into a transform definition.
    ///
    /// This is useful when you want to parse a transform once and execute it
    /// multiple times against different source data.
    ///
    /// # Errors
    ///
    /// Returns `ParseError` if the input is not valid ODIN transform.
    pub fn parse_transform(input: &str) -> Result<types::transform::OdinTransform, ParseError> {
        transform::parse_transform(input)
    }

    /// Parse ODIN text into a chain of documents (separated by `---`).
    ///
    /// # Errors
    ///
    /// Returns `ParseError` if the input is not valid ODIN.
    pub fn parse_documents(input: &str) -> Result<Vec<OdinDocument>, ParseError> {
        parser::parse_documents(input, None)
    }

    /// Create a new document builder.
    pub fn builder() -> OdinDocumentBuilder {
        OdinDocumentBuilder::new()
    }

    /// Create an empty document.
    pub fn empty() -> OdinDocument {
        OdinDocument::empty()
    }

    /// ODIN specification version.
    pub const VERSION: &'static str = "1.0.0";

    /// Build a dot-separated path from segments.
    ///
    /// String segments are joined with `.`, numeric segments become `[N]`.
    ///
    /// ```rust
    /// use odin_core::Odin;
    ///
    /// assert_eq!(Odin::path(&["policy", "vehicles", "[0]", "vin"]), "policy.vehicles[0].vin");
    /// assert_eq!(Odin::path(&["name"]), "name");
    /// assert_eq!(Odin::path(&[]), "");
    /// ```
    pub fn path(segments: &[&str]) -> String {
        if segments.is_empty() {
            return String::new();
        }
        let mut result = String::new();
        for (i, segment) in segments.iter().enumerate() {
            if segment.starts_with('[') {
                // Array index segment like "[0]"
                result.push_str(segment);
            } else if i == 0 {
                result.push_str(segment);
            } else {
                result.push('.');
                result.push_str(segment);
            }
        }
        result
    }

    /// Build a path from mixed string and numeric segments.
    ///
    /// ```rust
    /// use odin_core::Odin;
    ///
    /// assert_eq!(
    ///     Odin::path_with_indices(&[("policy", None), ("vehicles", None), ("", Some(0)), ("vin", None)]),
    ///     "policy.vehicles[0].vin"
    /// );
    /// ```
    pub fn path_with_indices(segments: &[(&str, Option<usize>)]) -> String {
        use std::fmt::Write;
        let mut result = String::new();
        for (i, (name, index)) in segments.iter().enumerate() {
            if let Some(idx) = index {
                let _ = write!(result, "[{idx}]");
            } else if i == 0 {
                result.push_str(name);
            } else {
                result.push('.');
                result.push_str(name);
            }
        }
        result
    }

    /// Convert an `OdinDocument` to JSON string.
    ///
    /// Options:
    /// - `preserve_types`: Include type information (affects numeric precision, no JSON structural change)
    /// - `preserve_modifiers`: Include modifier information (no structural change in JSON output)
    pub fn to_json(doc: &OdinDocument, _preserve_types: bool, _preserve_modifiers: bool) -> String {
        export::odin_doc_to_json(doc)
    }

    /// Convert an `OdinDocument` to XML string.
    ///
    /// Options:
    /// - `preserve_types`: Add `odin:type` attributes to typed elements
    /// - `preserve_modifiers`: Add `odin:required`, `odin:confidential`, `odin:deprecated` attributes
    pub fn to_xml(doc: &OdinDocument, preserve_types: bool, preserve_modifiers: bool) -> String {
        export::odin_doc_to_xml(doc, preserve_types, preserve_modifiers)
    }

    /// Execute a transform specification against source data.
    pub fn transform(
        transform_text: &str,
        source: &types::transform::DynValue,
    ) -> Result<types::transform::TransformResult, ParseError> {
        let t = transform::parse_transform(transform_text)?;
        Ok(transform::execute_transform(&t, source))
    }

    /// Execute a pre-parsed transform against source data.
    ///
    /// Use [`Odin::parse_transform`] to parse the transform once, then call
    /// this method repeatedly with different source data.
    pub fn transform_with(
        transform: &types::transform::OdinTransform,
        source: &types::transform::DynValue,
    ) -> types::transform::TransformResult {
        transform::execute_transform(transform, source)
    }

    /// Execute a transform against an `OdinDocument`.
    ///
    /// Converts the document to a `DynValue` and executes the transform.
    pub fn transform_document(
        transform_text: &str,
        doc: &OdinDocument,
    ) -> Result<types::transform::TransformResult, ParseError> {
        let t = transform::parse_transform(transform_text)?;
        Ok(transform::transform_document(&t, doc))
    }

    /// Execute a pre-parsed transform against an `OdinDocument`.
    pub fn transform_document_with(
        transform: &types::transform::OdinTransform,
        doc: &OdinDocument,
    ) -> types::transform::TransformResult {
        transform::transform_document(transform, doc)
    }

    /// Execute a multi-record transform against raw input records.
    ///
    /// Records are routed to segments based on discriminator values.
    pub fn transform_multi_record(
        transform_text: &str,
        input: &types::transform::MultiRecordInput,
    ) -> Result<types::transform::TransformResult, ParseError> {
        let t = transform::parse_transform(transform_text)?;
        Ok(transform::execute_multi_record(&t, input))
    }

    /// Create a new streaming parser with the given handler.
    ///
    /// The streaming parser processes ODIN text incrementally via byte chunks,
    /// emitting events through the handler. Useful for large documents.
    pub fn streaming_parser<H: parser::streaming::ParseHandler>(handler: H) -> parser::streaming::StreamingParser<H> {
        parser::streaming::StreamingParser::new(handler)
    }

    /// Convert an `OdinDocument` to CSV string.
    ///
    /// Options:
    /// - `array_path`: Path to the array section to export (auto-detects if None)
    /// - `delimiter`: CSV delimiter character (default: ',')
    /// - `header`: Whether to include a header row (default: true)
    pub fn to_csv(doc: &OdinDocument, options: Option<&CsvExportOptions>) -> String {
        export::odin_doc_to_csv(doc, options)
    }

    /// Convert an `OdinDocument` to fixed-width string.
    ///
    /// Each field is placed at a specific position with a specific length.
    pub fn to_fixed_width(doc: &OdinDocument, options: &FixedWidthExportOptions) -> String {
        export::odin_doc_to_fixed_width(doc, options)
    }
}

/// Options for CSV export.
pub struct CsvExportOptions {
    /// Path to the array data to export (e.g., "employees").
    /// If None, scans for the first array in the document.
    pub array_path: Option<String>,
    /// Delimiter character (default: ',').
    pub delimiter: char,
    /// Whether to include a header row (default: true).
    pub header: bool,
}

impl Default for CsvExportOptions {
    fn default() -> Self {
        Self { array_path: None, delimiter: ',', header: true }
    }
}

/// Options for fixed-width export.
pub struct FixedWidthExportOptions {
    /// Total line width.
    pub line_width: usize,
    /// Field specifications.
    pub fields: Vec<FixedWidthField>,
    /// Default pad character (default: ' ').
    pub pad_char: char,
}

/// A field specification for fixed-width export.
pub struct FixedWidthField {
    /// Dot-path to the value (e.g., "policy.number").
    pub path: String,
    /// Starting position (0-based).
    pub pos: usize,
    /// Field length.
    pub len: usize,
    /// Per-field pad character (uses default if None).
    pub pad_char: Option<char>,
    /// Alignment (default: left).
    pub align: FixedWidthAlign,
}

/// Alignment for fixed-width fields.
#[derive(Default, Clone, Copy)]
pub enum FixedWidthAlign {
    /// Left-align the value (pad on right).
    #[default]
    Left,
    /// Right-align the value (pad on left).
    Right,
}

/// A path segment: either a field name or an array index.
///
/// Used with [`Odin::build_path`] to construct dot-separated paths.
#[derive(Debug, Clone)]
pub enum PathSegment<'a> {
    /// A field name segment.
    Field(&'a str),
    /// An array index segment.
    Index(usize),
}

impl Odin {
    /// Build a path from typed segments (matching TS `Odin.path()`).
    ///
    /// ```rust
    /// use odin_core::{Odin, PathSegment};
    ///
    /// let path = Odin::build_path(&[
    ///     PathSegment::Field("policy"),
    ///     PathSegment::Field("vehicles"),
    ///     PathSegment::Index(0),
    ///     PathSegment::Field("vin"),
    /// ]);
    /// assert_eq!(path, "policy.vehicles[0].vin");
    /// ```
    pub fn build_path(segments: &[PathSegment<'_>]) -> String {
        use std::fmt::Write;
        if segments.is_empty() {
            return String::new();
        }
        let mut result = String::new();
        for (i, segment) in segments.iter().enumerate() {
            match segment {
                PathSegment::Index(idx) => {
                    let _ = write!(result, "[{idx}]");
                }
                PathSegment::Field(name) => {
                    if i > 0 {
                        result.push('.');
                    }
                    result.push_str(name);
                }
            }
        }
        result
    }
}

mod export {
    use crate::types::document::OdinDocument;
    use crate::types::values::OdinValue;

    /// Convert `OdinDocument` to JSON string.
    pub fn odin_doc_to_json(doc: &OdinDocument) -> String {
        let json_value = odin_doc_to_serde_value(doc);
        serde_json::to_string_pretty(&json_value).unwrap_or_else(|_| "{}".to_string())
    }

    fn odin_doc_to_serde_value(doc: &OdinDocument) -> serde_json::Value {
        let sections = collect_sections(doc);
        let mut map = serde_json::Map::new();

        for (section_name, fields) in &sections {
            let mut section_map = serde_json::Map::new();
            for (field_name, value) in fields {
                section_map.insert(field_name.clone(), odin_value_to_json(value));
            }
            map.insert(section_name.clone(), serde_json::Value::Object(section_map));
        }

        if !doc.metadata.is_empty() {
            let mut meta_map = serde_json::Map::new();
            // Put "odin" key first
            if let Some(odin_ver) = doc.metadata.get(&"odin".to_string()) {
                meta_map.insert("odin".to_string(), odin_value_to_json(odin_ver));
            }
            for (key, value) in doc.metadata.iter() {
                if key.as_str() != "odin" {
                    meta_map.insert(key.clone(), odin_value_to_json(value));
                }
            }
            map.insert("$".to_string(), serde_json::Value::Object(meta_map));
        }

        serde_json::Value::Object(map)
    }

    fn odin_value_to_json(value: &OdinValue) -> serde_json::Value {
        match value {
            OdinValue::Boolean { value: b, .. } => serde_json::Value::Bool(*b),
            OdinValue::String { value: s, .. } => serde_json::Value::String(s.clone()),
            OdinValue::Integer { value: n, .. } => serde_json::Value::Number(serde_json::Number::from(*n)),
            OdinValue::Number { raw, .. } | OdinValue::Percent { raw, .. } => {
                if let Some(raw_str) = raw {
                    float_str_to_json_value(raw_str)
                } else {
                    serde_json::Value::Number(serde_json::Number::from(0))
                }
            }
            OdinValue::Currency { raw, .. } => {
                if let Some(raw_str) = raw {
                    let numeric_part = raw_str.split(':').next().unwrap_or(raw_str);
                    float_str_to_json_value(numeric_part)
                } else {
                    serde_json::Value::Number(serde_json::Number::from(0))
                }
            }
            OdinValue::Date { raw, .. } | OdinValue::Timestamp { raw, .. } => {
                serde_json::Value::String(raw.clone())
            }
            OdinValue::Time { value: s, .. } | OdinValue::Duration { value: s, .. } => {
                serde_json::Value::String(s.clone())
            }
            OdinValue::Reference { path, .. } => {
                serde_json::Value::String(format!("@{path}"))
            }
            OdinValue::Binary { data, algorithm, .. } => {
                if let Some(algo) = algorithm {
                    let mut hex = String::with_capacity(data.len() * 2);
                    for b in data {
                        use std::fmt::Write;
                        let _ = write!(hex, "{b:02x}");
                    }
                    serde_json::Value::String(format!("^{algo}:{hex}"))
                } else {
                    serde_json::Value::String(format!("^{}", crate::utils::base64::encode(data)))
                }
            }
            _ => serde_json::Value::Null,
        }
    }

    /// Convert a numeric string to a JSON value, rendering whole-number floats as integers.
    fn float_str_to_json_value(s: &str) -> serde_json::Value {
        if let Ok(f) = s.parse::<f64>() {
            // Whole numbers render without decimal point (matching JS behavior)
            if f.fract() == 0.0 && f.is_finite() && f.abs() < 1e21 {
                serde_json::Value::Number(serde_json::Number::from(f as i64))
            } else {
                serde_json::Number::from_f64(f)
                    .map_or(serde_json::Value::Null, serde_json::Value::Number)
            }
        } else {
            serde_json::Value::String(s.to_string())
        }
    }

    fn collect_sections<'a>(doc: &'a OdinDocument) -> Vec<(String, Vec<(String, &'a OdinValue)>)> {
        let mut sections: Vec<(String, Vec<(String, &'a OdinValue)>)> = Vec::new();
        let mut seen: Vec<String> = Vec::new();

        for (path, value) in doc.assignments.iter() {
            if let Some(dot_pos) = path.find('.') {
                let section = &path[..dot_pos];
                if section == "$" {
                    continue;
                }
                let field = &path[dot_pos + 1..];

                if let Some(idx) = seen.iter().position(|s| s == section) {
                    sections[idx].1.push((field.to_string(), value));
                } else {
                    seen.push(section.to_string());
                    sections.push((section.to_string(), vec![(field.to_string(), value)]));
                }
            }
        }
        sections
    }

    /// Convert `OdinDocument` to XML string.
    ///
    /// Type and modifier attributes are always included in XML output since
    /// XML has no native type system — the attributes are the only way to
    /// preserve ODIN type information.
    pub fn odin_doc_to_xml(doc: &OdinDocument, _preserve_types: bool, _preserve_modifiers: bool) -> String {
        // Use hand-written output for precise formatting control matching golden tests.
        // quick-xml::Writer is used for the XML declaration only.
        let mut output = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
        output.push_str("<root xmlns:odin=\"https://odin.foundation/ns\">\n");

        let sections = collect_sections(doc);

        for (section_name, fields) in &sections {
            output.push_str("  <");
            output.push_str(section_name);
            output.push_str(">\n");

            for (field_name, value) in fields {
                if matches!(value, OdinValue::Null { .. }) {
                    continue;
                }

                let full_path = format!("{section_name}.{field_name}");

                output.push_str("    <");
                output.push_str(field_name);

                if let Some(type_name) = odin_value_xml_type(value) {
                    output.push_str(" odin:type=\"");
                    output.push_str(type_name);
                    output.push('"');
                }
                if let OdinValue::Currency { currency_code: Some(code), .. } = value {
                    output.push_str(" odin:currencyCode=\"");
                    output.push_str(code);
                    output.push('"');
                }

                if let Some(mods) = doc.modifiers.get(&full_path) {
                    if mods.required {
                        output.push_str(" odin:required=\"true\"");
                    }
                    if mods.confidential {
                        output.push_str(" odin:confidential=\"true\"");
                    }
                    if mods.deprecated {
                        output.push_str(" odin:deprecated=\"true\"");
                    }
                }

                output.push('>');

                let text_content = odin_value_to_xml_text(value);
                output.push_str(&quick_xml::escape::escape(&text_content));

                output.push_str("</");
                output.push_str(field_name);
                output.push_str(">\n");
            }

            output.push_str("  </");
            output.push_str(section_name);
            output.push_str(">\n");
        }

        output.push_str("</root>\n");
        output
    }

    fn odin_value_to_xml_text(value: &OdinValue) -> String {
        match value {
            OdinValue::Boolean { value: b, .. } => if *b { "true" } else { "false" }.to_string(),
            OdinValue::String { value: s, .. }
            | OdinValue::Time { value: s, .. }
            | OdinValue::Duration { value: s, .. } => s.clone(),
            OdinValue::Integer { value: n, .. } => {
                let mut buf = itoa::Buffer::new();
                buf.format(*n).to_string()
            }
            OdinValue::Number { raw: Some(r), .. } | OdinValue::Percent { raw: Some(r), .. } => {
                r.clone()
            }
            OdinValue::Currency { raw: Some(r), .. } => {
                r.split(':').next().unwrap_or(r).to_string()
            }
            OdinValue::Date { raw, .. } | OdinValue::Timestamp { raw, .. } => raw.clone(),
            OdinValue::Reference { path, .. } => format!("@{path}"),
            OdinValue::Binary { data, algorithm, .. } => {
                if let Some(algo) = algorithm {
                    let mut hex = String::with_capacity(data.len() * 2);
                    for b in data {
                        use std::fmt::Write;
                        let _ = write!(hex, "{b:02x}");
                    }
                    format!("^{algo}:{hex}")
                } else {
                    format!("^{}", crate::utils::base64::encode(data))
                }
            }
            _ => String::new(),
        }
    }

    fn odin_value_xml_type(value: &OdinValue) -> Option<&'static str> {
        match value {
            OdinValue::Integer { .. } => Some("integer"),
            OdinValue::Number { .. } => Some("number"),
            OdinValue::Currency { .. } => Some("currency"),
            OdinValue::Percent { .. } => Some("percent"),
            OdinValue::Boolean { .. } => Some("boolean"),
            OdinValue::Date { .. } => Some("date"),
            OdinValue::Timestamp { .. } => Some("timestamp"),
            OdinValue::Time { .. } => Some("time"),
            OdinValue::Duration { .. } => Some("duration"),
            OdinValue::Reference { .. } => Some("reference"),
            OdinValue::Binary { .. } => Some("binary"),
            _ => None,
        }
    }

    // ── CSV Export ──────────────────────────────────────────────────────────

    pub fn odin_doc_to_csv(doc: &OdinDocument, options: Option<&super::CsvExportOptions>) -> String {
        let delimiter = options.map_or(',', |o| o.delimiter);
        let include_header = options.map_or(true, |o| o.header);
        let array_path = options.and_then(|o| o.array_path.as_deref());

        // Find array data: either at specified path or auto-detect first array section
        let rows = find_array_rows(doc, array_path);
        if rows.is_empty() {
            // Treat entire document as single row
            return csv_single_row(doc, delimiter, include_header);
        }

        // Collect all column names from all rows (preserving order)
        let mut columns: Vec<String> = Vec::new();
        let mut col_set: Vec<String> = Vec::new();
        for row in &rows {
            for (key, _) in row {
                if !col_set.contains(key) {
                    col_set.push(key.clone());
                    columns.push(key.clone());
                }
            }
        }

        let mut output = String::new();

        // Header row
        if include_header {
            for (i, col) in columns.iter().enumerate() {
                if i > 0 { output.push(delimiter); }
                csv_escape_into(&mut output, col, delimiter);
            }
            output.push('\n');
        }

        // Data rows
        for row in &rows {
            for (i, col) in columns.iter().enumerate() {
                if i > 0 { output.push(delimiter); }
                let val = row.iter().find(|(k, _)| k == col).map(|(_, v)| v);
                if let Some(value) = val {
                    let text = odin_value_to_csv_text(value);
                    csv_escape_into(&mut output, &text, delimiter);
                }
            }
            output.push('\n');
        }

        output
    }

    fn find_array_rows<'a>(doc: &'a OdinDocument, array_path: Option<&str>) -> Vec<Vec<(String, &'a OdinValue)>> {
        let _sections = collect_sections(doc);

        if let Some(path) = array_path {
            // Look for array items at the specified path
            // Array items are indexed: path[0].field, path[1].field, etc.
            return collect_array_items_at(doc, path);
        }

        // Auto-detect: find section with indexed array items
        // Look for patterns like "section[0].field"
        let mut array_sections: Vec<String> = Vec::new();
        for (path, _) in doc.assignments.iter() {
            if let Some(bracket_pos) = path.find('[') {
                let section = &path[..bracket_pos];
                if !array_sections.contains(&section.to_string()) {
                    array_sections.push(section.to_string());
                }
            }
        }

        if let Some(first_array) = array_sections.first() {
            return collect_array_items_at(doc, first_array);
        }

        // Fall back to regular sections (no arrays found)
        Vec::new()
    }

    fn collect_array_items_at<'a>(doc: &'a OdinDocument, base: &str) -> Vec<Vec<(String, &'a OdinValue)>> {
        let prefix = format!("{base}[");
        let mut items: std::collections::BTreeMap<usize, Vec<(String, &'a OdinValue)>> = std::collections::BTreeMap::new();

        for (path, value) in doc.assignments.iter() {
            if path.starts_with(&prefix) {
                // Parse: base[N].field
                let rest = &path[prefix.len()..];
                if let Some(bracket_end) = rest.find(']') {
                    if let Ok(idx) = rest[..bracket_end].parse::<usize>() {
                        let field = if rest.len() > bracket_end + 1 && rest.as_bytes()[bracket_end + 1] == b'.' {
                            &rest[bracket_end + 2..]
                        } else {
                            continue;
                        };
                        items.entry(idx).or_default().push((field.to_string(), value));
                    }
                }
            }
        }

        items.into_values().collect()
    }

    fn csv_single_row(doc: &OdinDocument, delimiter: char, include_header: bool) -> String {
        let sections = collect_sections(doc);
        let mut output = String::new();

        // Flatten all fields
        let mut fields: Vec<(String, &OdinValue)> = Vec::new();
        for (section, section_fields) in &sections {
            for (field, value) in section_fields {
                fields.push((format!("{section}.{field}"), value));
            }
        }

        if include_header {
            for (i, (name, _)) in fields.iter().enumerate() {
                if i > 0 { output.push(delimiter); }
                csv_escape_into(&mut output, name, delimiter);
            }
            output.push('\n');
        }

        for (i, (_, value)) in fields.iter().enumerate() {
            if i > 0 { output.push(delimiter); }
            let text = odin_value_to_csv_text(value);
            csv_escape_into(&mut output, &text, delimiter);
        }
        output.push('\n');

        output
    }

    fn odin_value_to_csv_text(value: &OdinValue) -> String {
        match value {
            OdinValue::Boolean { value: b, .. } => b.to_string(),
            OdinValue::String { value: s, .. }
            | OdinValue::Time { value: s, .. }
            | OdinValue::Duration { value: s, .. } => s.clone(),
            OdinValue::Integer { value: n, .. } => n.to_string(),
            OdinValue::Number { raw: Some(r), .. } | OdinValue::Percent { raw: Some(r), .. } => r.clone(),
            OdinValue::Currency { raw: Some(r), .. } => r.split(':').next().unwrap_or(r).to_string(),
            OdinValue::Date { raw, .. } | OdinValue::Timestamp { raw, .. } => raw.clone(),
            OdinValue::Reference { path, .. } => format!("@{path}"),
            OdinValue::Binary { data, .. } => crate::utils::base64::encode(data),
            _ => String::new(),
        }
    }

    fn csv_escape_into(output: &mut String, s: &str, delimiter: char) {
        let needs_quoting = s.contains(delimiter) || s.contains('"') || s.contains('\n') || s.contains('\r');
        if needs_quoting {
            output.push('"');
            for ch in s.chars() {
                if ch == '"' { output.push('"'); }
                output.push(ch);
            }
            output.push('"');
        } else {
            output.push_str(s);
        }
    }

    // ── Fixed-Width Export ──────────────────────────────────────────────────

    pub fn odin_doc_to_fixed_width(doc: &OdinDocument, options: &super::FixedWidthExportOptions) -> String {
        let default_pad = options.pad_char;
        let mut line: Vec<char> = vec![default_pad; options.line_width];

        for field_spec in &options.fields {
            // Get value at path
            let value = doc.assignments.get(&field_spec.path)
                .map(odin_value_to_csv_text)
                .unwrap_or_default();

            let pad_char = field_spec.pad_char.unwrap_or(default_pad);
            let len = field_spec.len;
            let pos = field_spec.pos;

            // Truncate if too long
            let chars: Vec<char> = value.chars().take(len).collect();
            let value_len = chars.len();

            // Place with alignment
            match field_spec.align {
                super::FixedWidthAlign::Left => {
                    for (i, ch) in chars.iter().enumerate() {
                        if pos + i < options.line_width {
                            line[pos + i] = *ch;
                        }
                    }
                    // Pad remaining
                    for i in value_len..len {
                        if pos + i < options.line_width {
                            line[pos + i] = pad_char;
                        }
                    }
                }
                super::FixedWidthAlign::Right => {
                    let pad_count = len.saturating_sub(value_len);
                    for i in 0..pad_count {
                        if pos + i < options.line_width {
                            line[pos + i] = pad_char;
                        }
                    }
                    for (i, ch) in chars.iter().enumerate() {
                        if pos + pad_count + i < options.line_width {
                            line[pos + pad_count + i] = *ch;
                        }
                    }
                }
            }
        }

        line.into_iter().collect()
    }
}
