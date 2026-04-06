//! ODIN Document types.
//!
//! An `OdinDocument` is an immutable container for ODIN assignments, metadata,
//! imports, schemas, and conditionals. All mutations return new documents.

use crate::types::ordered_map::OrderedMap;
use crate::types::values::{OdinModifiers, OdinValue, OdinValues};

// ─────────────────────────────────────────────────────────────────────────────
// Document Directives
// ─────────────────────────────────────────────────────────────────────────────

/// An `@import` directive in an ODIN document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OdinImport {
    /// Import path (relative, absolute, or URL).
    pub path: String,
    /// Optional namespace alias.
    pub alias: Option<String>,
    /// Source line number (1-based).
    pub line: usize,
}

/// A `@schema` directive in an ODIN document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OdinSchema {
    /// Schema URL or path.
    pub url: String,
    /// Source line number (1-based).
    pub line: usize,
}

/// A conditional directive in an ODIN document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OdinConditional {
    /// Condition expression.
    pub condition: String,
    /// Source line number (1-based).
    pub line: usize,
}

/// A section header in an ODIN document (e.g., `{Policy}`, `{$}`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OdinHeader {
    /// Header path (e.g., "$", "Policy", "Policy.Coverage").
    pub path: String,
    /// Whether this header uses tabular mode.
    pub tabular: bool,
    /// Column names for tabular headers.
    pub columns: Option<Vec<String>>,
    /// Source line number (1-based).
    pub line: usize,
}

/// Options for document flattening.
#[derive(Debug, Clone, Default)]
pub struct FlattenOptions {
    /// Include metadata (`{$}` section) in output.
    pub include_metadata: bool,
    /// Include null values in output.
    pub include_nulls: bool,
    /// Sort output keys alphabetically (default: true).
    pub sort: bool,
}

impl FlattenOptions {
    /// Create default flatten options (sorted, no metadata, no nulls).
    pub fn new() -> Self {
        Self {
            include_metadata: false,
            include_nulls: false,
            sort: true,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// OdinDocument
// ─────────────────────────────────────────────────────────────────────────────

/// A comment preserved from source ODIN text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OdinComment {
    /// The comment text (without the leading `;` or `; `).
    pub text: String,
    /// The path this comment is associated with (if any).
    /// Comments before a field associate with that field.
    /// Standalone comments have `None`.
    pub associated_path: Option<String>,
    /// Source line number (1-based).
    pub line: usize,
}

/// An immutable ODIN document.
///
/// Contains:
/// - `metadata` — key-value pairs from the `{$}` section
/// - `assignments` — all field assignments (using dot-separated paths as keys)
/// - `modifiers` — per-path modifiers (required, confidential, deprecated)
/// - `imports` — `@import` directives
/// - `schemas` — `@schema` directives
/// - `conditionals` — conditional directives
/// - `comments` — preserved comments (when `ParseOptions.preserve_comments` is true)
///
/// All mutations (`with`, `without`) return new documents.
#[derive(Debug, Clone, PartialEq)]
pub struct OdinDocument {
    /// Metadata assignments from the `{$}` header section.
    pub metadata: OrderedMap<String, OdinValue>,
    /// All field assignments (dot-separated paths as keys).
    pub assignments: OrderedMap<String, OdinValue>,
    /// Per-path modifiers.
    pub modifiers: OrderedMap<String, OdinModifiers>,
    /// Import directives.
    pub imports: Vec<OdinImport>,
    /// Schema directives.
    pub schemas: Vec<OdinSchema>,
    /// Conditional directives.
    pub conditionals: Vec<OdinConditional>,
    /// Preserved comments (populated when `ParseOptions.preserve_comments` is true).
    pub comments: Vec<OdinComment>,
}

impl OdinDocument {
    /// Create an empty document.
    pub fn empty() -> Self {
        Self {
            metadata: OrderedMap::new(),
            assignments: OrderedMap::new(),
            modifiers: OrderedMap::new(),
            imports: Vec::new(),
            schemas: Vec::new(),
            conditionals: Vec::new(),
            comments: Vec::new(),
        }
    }

    /// Get a value at the given path.
    ///
    /// Supports `$.key` prefix for metadata lookups.
    pub fn get(&self, path: &str) -> Option<&OdinValue> {
        if let Some(meta_key) = path.strip_prefix("$.") {
            self.metadata.get(&meta_key.to_string())
        } else {
            self.assignments.get(&path.to_string())
        }
    }

    /// Get a string value at the given path.
    pub fn get_string(&self, path: &str) -> Option<&str> {
        self.get(path).and_then(|v| v.as_str())
    }

    /// Get an integer value at the given path.
    pub fn get_integer(&self, path: &str) -> Option<i64> {
        self.get(path).and_then(super::values::OdinValue::as_i64)
    }

    /// Get a numeric value at the given path (works for any numeric type).
    pub fn get_number(&self, path: &str) -> Option<f64> {
        self.get(path).and_then(super::values::OdinValue::as_f64)
    }

    /// Get a boolean value at the given path.
    pub fn get_boolean(&self, path: &str) -> Option<bool> {
        self.get(path).and_then(super::values::OdinValue::as_bool)
    }

    /// Returns `true` if the given path has a value assigned.
    pub fn has(&self, path: &str) -> bool {
        self.get(path).is_some()
    }

    /// Resolve a value at the given path, following `@reference` chains.
    ///
    /// If the value is a reference, follows the chain until a non-reference
    /// value is found. Detects circular references and unresolved references.
    ///
    /// # Errors
    ///
    /// Returns an error string if a circular or unresolved reference is found.
    pub fn resolve(&self, path: &str) -> Result<Option<&OdinValue>, String> {
        let Some(value) = self.get(path) else { return Ok(None) };

        if let OdinValue::Reference { path: ref_path, .. } = value {
            let mut seen = std::collections::HashSet::new();
            seen.insert(path.to_string());

            let mut current_path = ref_path.clone();
            loop {
                if seen.contains(&current_path) {
                    return Err(format!("Circular reference detected: {path}"));
                }
                seen.insert(current_path.clone());

                match self.get(&current_path) {
                    None => return Err(format!("Unresolved reference: {current_path}")),
                    Some(OdinValue::Reference { path: next_path, .. }) => {
                        current_path = next_path.clone();
                    }
                    Some(resolved) => return Ok(Some(resolved)),
                }
            }
        } else {
            Ok(Some(value))
        }
    }

    /// Returns all assignment paths in insertion order.
    pub fn paths(&self) -> Vec<&String> {
        self.assignments.keys().collect()
    }

    /// Create a new document with the given path set to the given value.
    pub fn with(&self, path: &str, value: OdinValue) -> Self {
        let mut new_doc = self.clone();
        new_doc.assignments.insert(path.to_string(), value);
        new_doc
    }

    /// Create a new document with the given path removed.
    pub fn without(&self, path: &str) -> Self {
        let mut new_doc = self.clone();
        new_doc.assignments.remove(&path.to_string());
        new_doc
    }

    /// Flatten the document to a map of string key-value pairs.
    pub fn flatten(&self, options: Option<&FlattenOptions>) -> OrderedMap<String, String> {
        let opts = options.cloned().unwrap_or_default();
        let mut result = OrderedMap::new();

        if opts.include_metadata {
            for (key, value) in &self.metadata {
                result.insert(format!("$.{key}"), format_value_for_flatten(value));
            }
        }

        for (key, value) in &self.assignments {
            if !opts.include_nulls && value.is_null() {
                continue;
            }
            result.insert(key.clone(), format_value_for_flatten(value));
        }

        if opts.sort {
            let mut entries = result.into_vec();
            entries.sort_by(|(a, _), (b, _)| a.cmp(b));
            OrderedMap::from_vec(entries)
        } else {
            result
        }
    }

    /// Convert the document to a nested JSON-compatible structure.
    ///
    /// Returns a vector of key-value pairs representing the top-level fields.
    pub fn to_json(&self) -> Vec<(String, serde_compatible::JsonValue)> {
        let mut result = Vec::new();
        for (key, value) in &self.assignments {
            result.push((key.clone(), value_to_json(value)));
        }
        result
    }
}

/// Format a value for flatten output (string representation).
fn format_value_for_flatten(value: &OdinValue) -> String {
    match value {
        OdinValue::Null { .. } => "~".to_string(),
        OdinValue::Boolean { value, .. } => value.to_string(),
        OdinValue::String { value, .. }
        | OdinValue::Time { value, .. }
        | OdinValue::Duration { value, .. } => value.clone(),
        OdinValue::Integer { value, raw, .. } => {
            raw.as_deref().unwrap_or(&value.to_string()).to_string()
        }
        OdinValue::Number { value, raw, decimal_places, .. } => {
            if let Some(r) = raw {
                r.clone()
            } else if let Some(dp) = decimal_places {
                format!("{value:.prec$}", prec = *dp as usize)
            } else {
                value.to_string()
            }
        }
        OdinValue::Currency { value, raw, decimal_places, .. } => {
            if let Some(r) = raw {
                r.clone()
            } else {
                format!("{value:.prec$}", prec = *decimal_places as usize)
            }
        }
        OdinValue::Percent { value, raw, .. } => {
            raw.as_deref().unwrap_or(&value.to_string()).to_string()
        }
        OdinValue::Date { raw, .. } | OdinValue::Timestamp { raw, .. } => raw.clone(),
        OdinValue::Reference { path, .. } => format!("@{path}"),
        OdinValue::Binary { .. } => "<binary>".to_string(),
        OdinValue::Verb { verb, .. } => format!("%{verb}"),
        OdinValue::Array { items, .. } => format!("[{} items]", items.len()),
        OdinValue::Object { value, .. } => format!("{{{} fields}}", value.len()),
    }
}

/// JSON-compatible value types (used for `to_json()` without serde dependency).
pub mod serde_compatible {
    /// A JSON-compatible value.
    #[derive(Debug, Clone, PartialEq)]
    pub enum JsonValue {
        /// Null value.
        Null,
        /// Boolean value.
        Bool(bool),
        /// Integer value.
        Integer(i64),
        /// Floating-point value.
        Float(f64),
        /// String value.
        String(String),
        /// Array of values.
        Array(Vec<JsonValue>),
        /// Object with ordered key-value pairs.
        Object(Vec<(String, JsonValue)>),
    }
}

/// Convert an `OdinValue` to a JSON-compatible value.
fn value_to_json(value: &OdinValue) -> serde_compatible::JsonValue {
    use serde_compatible::JsonValue;
    match value {
        OdinValue::Null { .. } | OdinValue::Binary { .. } => JsonValue::Null,
        OdinValue::Boolean { value, .. } => JsonValue::Bool(*value),
        OdinValue::String { value, .. }
        | OdinValue::Time { value, .. }
        | OdinValue::Duration { value, .. } => JsonValue::String(value.clone()),
        OdinValue::Integer { value, .. } => JsonValue::Integer(*value),
        OdinValue::Number { value, .. }
        | OdinValue::Currency { value, .. }
        | OdinValue::Percent { value, .. } => JsonValue::Float(*value),
        OdinValue::Date { raw, .. }
        | OdinValue::Timestamp { raw, .. } => JsonValue::String(raw.clone()),
        OdinValue::Reference { path, .. } => JsonValue::String(format!("@{path}")),
        OdinValue::Verb { verb, .. } => JsonValue::String(format!("%{verb}")),
        OdinValue::Array { items, .. } => {
            let json_items: Vec<_> = items
                .iter()
                .map(|item| match item {
                    crate::types::values::OdinArrayItem::Record(fields) => {
                        let obj: Vec<_> = fields
                            .iter()
                            .map(|(k, v)| (k.clone(), value_to_json(v)))
                            .collect();
                        JsonValue::Object(obj)
                    }
                    crate::types::values::OdinArrayItem::Value(v) => value_to_json(v),
                })
                .collect();
            JsonValue::Array(json_items)
        }
        OdinValue::Object { value, .. } => {
            // Object values are opaque Records in TS; represent as null for now
            let _ = value;
            JsonValue::Null
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// OdinDocumentBuilder
// ─────────────────────────────────────────────────────────────────────────────

/// Fluent builder for constructing ODIN documents.
///
/// ```rust
/// use odin_core::{OdinDocumentBuilder, OdinValues};
///
/// let doc = OdinDocumentBuilder::new()
///     .metadata("odin", OdinValues::string("1.0.0"))
///     .set("name", OdinValues::string("Alice"))
///     .set("age", OdinValues::integer(30))
///     .build()
///     .unwrap();
/// ```
#[derive(Debug, Clone)]
pub struct OdinDocumentBuilder {
    metadata: OrderedMap<String, OdinValue>,
    assignments: OrderedMap<String, OdinValue>,
    modifiers: OrderedMap<String, OdinModifiers>,
    imports: Vec<OdinImport>,
    schemas: Vec<OdinSchema>,
    conditionals: Vec<OdinConditional>,
}

impl Default for OdinDocumentBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl OdinDocumentBuilder {
    /// Create a new empty builder.
    pub fn new() -> Self {
        Self {
            metadata: OrderedMap::new(),
            assignments: OrderedMap::new(),
            modifiers: OrderedMap::new(),
            imports: Vec::new(),
            schemas: Vec::new(),
            conditionals: Vec::new(),
        }
    }

    /// Set a metadata value (in the `{$}` section).
    pub fn metadata(mut self, key: &str, value: OdinValue) -> Self {
        self.metadata.insert(key.to_string(), value);
        self
    }

    /// Set a field value.
    pub fn set(mut self, path: &str, value: OdinValue) -> Self {
        self.assignments.insert(path.to_string(), value);
        self
    }

    /// Set a field value with modifiers.
    pub fn set_with_modifiers(
        mut self,
        path: &str,
        value: OdinValue,
        modifiers: OdinModifiers,
    ) -> Self {
        self.assignments.insert(path.to_string(), value);
        if modifiers.has_any() {
            self.modifiers.insert(path.to_string(), modifiers);
        }
        self
    }

    /// Set a string value (convenience method).
    pub fn set_string(self, path: &str, value: &str) -> Self {
        self.set(path, OdinValues::string(value))
    }

    /// Set an integer value (convenience method).
    pub fn set_integer(self, path: &str, value: i64) -> Self {
        self.set(path, OdinValues::integer(value))
    }

    /// Set a number value (convenience method).
    pub fn set_number(self, path: &str, value: f64) -> Self {
        self.set(path, OdinValues::number(value))
    }

    /// Set a boolean value (convenience method).
    pub fn set_boolean(self, path: &str, value: bool) -> Self {
        self.set(path, OdinValues::boolean(value))
    }

    /// Set a null value (convenience method).
    pub fn set_null(self, path: &str) -> Self {
        self.set(path, OdinValues::null())
    }

    /// Add an import directive.
    pub fn import(mut self, path: &str, alias: Option<&str>, line: usize) -> Self {
        self.imports.push(OdinImport {
            path: path.to_string(),
            alias: alias.map(std::string::ToString::to_string),
            line,
        });
        self
    }

    /// Add a schema directive.
    pub fn schema(mut self, url: &str, line: usize) -> Self {
        self.schemas.push(OdinSchema {
            url: url.to_string(),
            line,
        });
        self
    }

    /// Build the document.
    ///
    /// Validates array contiguity before returning.
    ///
    /// # Errors
    ///
    /// Returns an error if arrays have non-contiguous indices.
    pub fn build(self) -> Result<OdinDocument, String> {
        // Validate array contiguity
        self.validate_arrays()?;

        Ok(OdinDocument {
            metadata: self.metadata,
            assignments: self.assignments,
            modifiers: self.modifiers,
            imports: self.imports,
            schemas: self.schemas,
            conditionals: self.conditionals,
            comments: Vec::new(),
        })
    }

    /// Validate that all array indices are contiguous starting from 0.
    fn validate_arrays(&self) -> Result<(), String> {
        use std::collections::HashMap;

        // Collect array paths and their indices
        let mut arrays: HashMap<String, Vec<usize>> = HashMap::new();

        for (path, _) in &self.assignments {
            // Check for array index patterns like "items[0]", "items[1]"
            if let Some(bracket_pos) = path.rfind('[') {
                if let Some(end_bracket) = path[bracket_pos..].find(']') {
                    let array_path = &path[..bracket_pos];
                    let idx_str = &path[bracket_pos + 1..bracket_pos + end_bracket];
                    if let Ok(idx) = idx_str.parse::<usize>() {
                        arrays
                            .entry(array_path.to_string())
                            .or_default()
                            .push(idx);
                    }
                }
            }
        }

        for (path, mut indices) in arrays {
            indices.sort_unstable();
            indices.dedup();
            if indices.first() != Some(&0) && !indices.is_empty() {
                return Err(format!(
                    "Non-contiguous array indices in {path}: must start at 0"
                ));
            }
            for window in indices.windows(2) {
                if window[1] != window[0] + 1 {
                    return Err(format!(
                        "Non-contiguous array indices in {path}: gap between {} and {}",
                        window[0], window[1]
                    ));
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_document() {
        let doc = OdinDocument::empty();
        assert!(doc.assignments.is_empty());
        assert!(doc.metadata.is_empty());
        assert!(!doc.has("anything"));
    }

    #[test]
    fn test_builder_basic() {
        let doc = OdinDocumentBuilder::new()
            .set("name", OdinValues::string("Alice"))
            .set("age", OdinValues::integer(30))
            .build()
            .unwrap();

        assert_eq!(doc.get_string("name"), Some("Alice"));
        assert_eq!(doc.get_integer("age"), Some(30));
        assert!(doc.has("name"));
        assert!(!doc.has("missing"));
    }

    #[test]
    fn test_builder_metadata() {
        let doc = OdinDocumentBuilder::new()
            .metadata("odin", OdinValues::string("1.0.0"))
            .build()
            .unwrap();

        assert_eq!(doc.get_string("$.odin"), Some("1.0.0"));
    }

    #[test]
    fn test_document_with_without() {
        let doc = OdinDocumentBuilder::new()
            .set("a", OdinValues::integer(1))
            .set("b", OdinValues::integer(2))
            .build()
            .unwrap();

        let doc2 = doc.with("c", OdinValues::integer(3));
        assert!(doc2.has("c"));
        assert!(!doc.has("c")); // Original unchanged

        let doc3 = doc2.without("b");
        assert!(!doc3.has("b"));
        assert!(doc2.has("b")); // Previous unchanged
    }

    #[test]
    fn test_builder_validates_array_contiguity() {
        let result = OdinDocumentBuilder::new()
            .set("items[0]", OdinValues::string("a"))
            .set("items[2]", OdinValues::string("c")) // Gap: missing [1]
            .build();

        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Non-contiguous"));
    }

    #[test]
    fn test_paths() {
        let doc = OdinDocumentBuilder::new()
            .set("a", OdinValues::integer(1))
            .set("b.c", OdinValues::integer(2))
            .build()
            .unwrap();

        let paths = doc.paths();
        assert_eq!(paths.len(), 2);
    }

    #[test]
    fn test_flatten() {
        let doc = OdinDocumentBuilder::new()
            .set("name", OdinValues::string("Alice"))
            .set("age", OdinValues::integer(30))
            .set("active", OdinValues::boolean(true))
            .set("notes", OdinValues::null())
            .build()
            .unwrap();

        let flat = doc.flatten(None);
        assert_eq!(flat.len(), 3); // null excluded by default
        assert_eq!(flat.get(&"name".to_string()), Some(&"Alice".to_string()));

        let flat_with_nulls = doc.flatten(Some(&FlattenOptions {
            include_nulls: true,
            ..Default::default()
        }));
        assert_eq!(flat_with_nulls.len(), 4);
    }

    #[test]
    fn test_resolve_non_reference() {
        let doc = OdinDocumentBuilder::new()
            .set("name", OdinValues::string("Alice"))
            .build()
            .unwrap();

        let resolved = doc.resolve("name").unwrap();
        assert_eq!(resolved.unwrap().as_str(), Some("Alice"));
    }

    #[test]
    fn test_resolve_missing_path() {
        let doc = OdinDocument::empty();
        assert!(doc.resolve("missing").unwrap().is_none());
    }

    #[test]
    fn test_resolve_reference_chain() {
        let doc = OdinDocumentBuilder::new()
            .set("target", OdinValues::string("final_value"))
            .set("ref1", OdinValue::Reference {
                path: "target".to_string(),
                modifiers: None,
                directives: vec![],
            })
            .set("ref2", OdinValue::Reference {
                path: "ref1".to_string(),
                modifiers: None,
                directives: vec![],
            })
            .build()
            .unwrap();

        let resolved = doc.resolve("ref2").unwrap();
        assert_eq!(resolved.unwrap().as_str(), Some("final_value"));
    }

    #[test]
    fn test_resolve_circular_reference() {
        let doc = OdinDocumentBuilder::new()
            .set("a", OdinValue::Reference {
                path: "b".to_string(),
                modifiers: None,
                directives: vec![],
            })
            .set("b", OdinValue::Reference {
                path: "a".to_string(),
                modifiers: None,
                directives: vec![],
            })
            .build()
            .unwrap();

        let result = doc.resolve("a");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Circular reference"));
    }

    #[test]
    fn test_resolve_unresolved_reference() {
        let doc = OdinDocumentBuilder::new()
            .set("ref1", OdinValue::Reference {
                path: "nonexistent".to_string(),
                modifiers: None,
                directives: vec![],
            })
            .build()
            .unwrap();

        let result = doc.resolve("ref1");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Unresolved reference"));
    }

    // ─── Empty document tests ────────────────────────────────────────────

    #[test]
    fn test_empty_document_paths() {
        let doc = OdinDocument::empty();
        assert_eq!(doc.paths().len(), 0);
    }

    #[test]
    fn test_empty_document_get_returns_none() {
        let doc = OdinDocument::empty();
        assert!(doc.get("anything").is_none());
        assert!(doc.get_string("x").is_none());
        assert!(doc.get_integer("x").is_none());
        assert!(doc.get_number("x").is_none());
        assert!(doc.get_boolean("x").is_none());
    }

    #[test]
    fn test_empty_document_imports() {
        let doc = OdinDocument::empty();
        assert!(doc.imports.is_empty());
        assert!(doc.schemas.is_empty());
        assert!(doc.conditionals.is_empty());
        assert!(doc.comments.is_empty());
    }

    // ─── Builder convenience methods ─────────────────────────────────────

    #[test]
    fn test_builder_set_string() {
        let doc = OdinDocumentBuilder::new()
            .set_string("name", "Alice")
            .build()
            .unwrap();
        assert_eq!(doc.get_string("name"), Some("Alice"));
    }

    #[test]
    fn test_builder_set_integer() {
        let doc = OdinDocumentBuilder::new()
            .set_integer("count", 42)
            .build()
            .unwrap();
        assert_eq!(doc.get_integer("count"), Some(42));
    }

    #[test]
    fn test_builder_set_number() {
        let doc = OdinDocumentBuilder::new()
            .set_number("pi", 3.14)
            .build()
            .unwrap();
        assert!((doc.get_number("pi").unwrap() - 3.14).abs() < f64::EPSILON);
    }

    #[test]
    fn test_builder_set_boolean() {
        let doc = OdinDocumentBuilder::new()
            .set_boolean("active", true)
            .build()
            .unwrap();
        assert_eq!(doc.get_boolean("active"), Some(true));
    }

    #[test]
    fn test_builder_set_null() {
        let doc = OdinDocumentBuilder::new()
            .set_null("field")
            .build()
            .unwrap();
        assert!(doc.get("field").unwrap().is_null());
    }

    // ─── Metadata tests ─────────────────────────────────────────────────

    #[test]
    fn test_metadata_multiple_keys() {
        let doc = OdinDocumentBuilder::new()
            .metadata("odin", OdinValues::string("1.0.0"))
            .metadata("transform", OdinValues::string("1.0.0"))
            .build()
            .unwrap();
        assert_eq!(doc.get_string("$.odin"), Some("1.0.0"));
        assert_eq!(doc.get_string("$.transform"), Some("1.0.0"));
    }

    #[test]
    fn test_metadata_not_in_assignments() {
        let doc = OdinDocumentBuilder::new()
            .metadata("odin", OdinValues::string("1.0.0"))
            .set("name", OdinValues::string("Alice"))
            .build()
            .unwrap();
        // Metadata should not appear via regular path lookup
        assert!(doc.get("odin").is_none());
        // But should via $.prefix
        assert_eq!(doc.get_string("$.odin"), Some("1.0.0"));
        // Assignments count should not include metadata
        assert_eq!(doc.paths().len(), 1);
    }

    // ─── Key ordering tests ─────────────────────────────────────────────

    #[test]
    fn test_insertion_order_preserved() {
        let doc = OdinDocumentBuilder::new()
            .set("z", OdinValues::integer(3))
            .set("a", OdinValues::integer(1))
            .set("m", OdinValues::integer(2))
            .build()
            .unwrap();
        let paths: Vec<&str> = doc.paths().iter().map(|s| s.as_str()).collect();
        assert_eq!(paths, vec!["z", "a", "m"]);
    }

    // ─── Duplicate key behavior ──────────────────────────────────────────

    #[test]
    fn test_duplicate_key_overwrites() {
        let doc = OdinDocumentBuilder::new()
            .set("x", OdinValues::integer(1))
            .set("x", OdinValues::integer(2))
            .build()
            .unwrap();
        assert_eq!(doc.get_integer("x"), Some(2));
        // Should only have one path
        assert_eq!(doc.paths().len(), 1);
    }

    // ─── with/without immutability tests ─────────────────────────────────

    #[test]
    fn test_with_creates_new_doc() {
        let doc1 = OdinDocumentBuilder::new()
            .set("a", OdinValues::integer(1))
            .build()
            .unwrap();
        let doc2 = doc1.with("b", OdinValues::integer(2));
        // doc1 unchanged
        assert!(!doc1.has("b"));
        assert!(doc2.has("a"));
        assert!(doc2.has("b"));
    }

    #[test]
    fn test_without_creates_new_doc() {
        let doc1 = OdinDocumentBuilder::new()
            .set("a", OdinValues::integer(1))
            .set("b", OdinValues::integer(2))
            .build()
            .unwrap();
        let doc2 = doc1.without("a");
        // doc1 unchanged
        assert!(doc1.has("a"));
        assert!(!doc2.has("a"));
        assert!(doc2.has("b"));
    }

    #[test]
    fn test_with_overwrites_existing() {
        let doc1 = OdinDocumentBuilder::new()
            .set("x", OdinValues::integer(1))
            .build()
            .unwrap();
        let doc2 = doc1.with("x", OdinValues::integer(99));
        assert_eq!(doc2.get_integer("x"), Some(99));
    }

    #[test]
    fn test_without_nonexistent_key() {
        let doc = OdinDocumentBuilder::new()
            .set("a", OdinValues::integer(1))
            .build()
            .unwrap();
        let doc2 = doc.without("nonexistent");
        assert_eq!(doc2.paths().len(), 1);
    }

    // ─── Nested path tests ───────────────────────────────────────────────

    #[test]
    fn test_dotted_paths() {
        let doc = OdinDocumentBuilder::new()
            .set("person.name", OdinValues::string("Alice"))
            .set("person.age", OdinValues::integer(30))
            .build()
            .unwrap();
        assert_eq!(doc.get_string("person.name"), Some("Alice"));
        assert_eq!(doc.get_integer("person.age"), Some(30));
    }

    #[test]
    fn test_deeply_nested_paths() {
        let doc = OdinDocumentBuilder::new()
            .set("a.b.c.d.e", OdinValues::string("deep"))
            .build()
            .unwrap();
        assert_eq!(doc.get_string("a.b.c.d.e"), Some("deep"));
    }

    // ─── Array validation tests ──────────────────────────────────────────

    #[test]
    fn test_valid_array_indices() {
        let result = OdinDocumentBuilder::new()
            .set("items[0]", OdinValues::string("a"))
            .set("items[1]", OdinValues::string("b"))
            .set("items[2]", OdinValues::string("c"))
            .build();
        assert!(result.is_ok());
    }

    #[test]
    fn test_array_not_starting_at_zero() {
        let result = OdinDocumentBuilder::new()
            .set("items[1]", OdinValues::string("a"))
            .build();
        assert!(result.is_err());
    }

    #[test]
    fn test_array_gap_detected() {
        let result = OdinDocumentBuilder::new()
            .set("items[0]", OdinValues::string("a"))
            .set("items[3]", OdinValues::string("d"))
            .build();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Non-contiguous"));
    }

    // ─── Flatten tests ───────────────────────────────────────────────────

    #[test]
    fn test_flatten_sorted() {
        let doc = OdinDocumentBuilder::new()
            .set("z", OdinValues::string("last"))
            .set("a", OdinValues::string("first"))
            .build()
            .unwrap();
        let flat = doc.flatten(Some(&FlattenOptions::new()));
        let keys: Vec<_> = flat.keys().collect();
        // FlattenOptions::new() has sort: true
        assert_eq!(keys[0], "a");
        assert_eq!(keys[1], "z");
    }

    #[test]
    fn test_flatten_unsorted() {
        let doc = OdinDocumentBuilder::new()
            .set("z", OdinValues::string("last"))
            .set("a", OdinValues::string("first"))
            .build()
            .unwrap();
        let flat = doc.flatten(Some(&FlattenOptions {
            sort: false,
            ..Default::default()
        }));
        let keys: Vec<_> = flat.keys().collect();
        // Insertion order
        assert_eq!(keys[0], "z");
        assert_eq!(keys[1], "a");
    }

    #[test]
    fn test_flatten_with_metadata() {
        let doc = OdinDocumentBuilder::new()
            .metadata("odin", OdinValues::string("1.0.0"))
            .set("name", OdinValues::string("Alice"))
            .build()
            .unwrap();
        let flat = doc.flatten(Some(&FlattenOptions {
            include_metadata: true,
            ..Default::default()
        }));
        assert!(flat.contains_key(&"$.odin".to_string()));
    }

    #[test]
    fn test_flatten_excludes_nulls_by_default() {
        let doc = OdinDocumentBuilder::new()
            .set("a", OdinValues::string("val"))
            .set("b", OdinValues::null())
            .build()
            .unwrap();
        let flat = doc.flatten(None);
        assert_eq!(flat.len(), 1);
    }

    #[test]
    fn test_flatten_includes_nulls() {
        let doc = OdinDocumentBuilder::new()
            .set("a", OdinValues::string("val"))
            .set("b", OdinValues::null())
            .build()
            .unwrap();
        let flat = doc.flatten(Some(&FlattenOptions {
            include_nulls: true,
            ..Default::default()
        }));
        assert_eq!(flat.len(), 2);
        assert_eq!(flat.get(&"b".to_string()), Some(&"~".to_string()));
    }

    #[test]
    fn test_flatten_value_formatting() {
        let doc = OdinDocumentBuilder::new()
            .set("bool", OdinValues::boolean(true))
            .set("int", OdinValues::integer(42))
            .set("str", OdinValues::string("hello"))
            .set("ref", OdinValues::reference("x.y"))
            .build()
            .unwrap();
        let flat = doc.flatten(Some(&FlattenOptions { sort: false, ..Default::default() }));
        assert_eq!(flat.get(&"bool".to_string()), Some(&"true".to_string()));
        assert_eq!(flat.get(&"int".to_string()), Some(&"42".to_string()));
        assert_eq!(flat.get(&"str".to_string()), Some(&"hello".to_string()));
        assert_eq!(flat.get(&"ref".to_string()), Some(&"@x.y".to_string()));
    }

    // ─── to_json tests ───────────────────────────────────────────────────

    #[test]
    fn test_to_json_basic() {
        let doc = OdinDocumentBuilder::new()
            .set("name", OdinValues::string("Alice"))
            .set("age", OdinValues::integer(30))
            .set("active", OdinValues::boolean(true))
            .build()
            .unwrap();
        let json = doc.to_json();
        assert_eq!(json.len(), 3);
        assert_eq!(json[0].0, "name");
    }

    #[test]
    fn test_to_json_null_value() {
        let doc = OdinDocumentBuilder::new()
            .set("x", OdinValues::null())
            .build()
            .unwrap();
        let json = doc.to_json();
        assert_eq!(json[0].1, serde_compatible::JsonValue::Null);
    }

    // ─── Builder with modifiers tests ────────────────────────────────────

    #[test]
    fn test_builder_set_with_modifiers() {
        let mods = OdinModifiers { required: true, ..Default::default() };
        let doc = OdinDocumentBuilder::new()
            .set_with_modifiers("name", OdinValues::string("Alice"), mods)
            .build()
            .unwrap();
        assert_eq!(doc.get_string("name"), Some("Alice"));
        assert!(doc.modifiers.contains_key(&"name".to_string()));
    }

    #[test]
    fn test_builder_set_with_empty_modifiers() {
        let mods = OdinModifiers::default();
        let doc = OdinDocumentBuilder::new()
            .set_with_modifiers("name", OdinValues::string("Alice"), mods)
            .build()
            .unwrap();
        // Empty modifiers should not be stored
        assert!(!doc.modifiers.contains_key(&"name".to_string()));
    }

    // ─── Builder import/schema tests ─────────────────────────────────────

    #[test]
    fn test_builder_import() {
        let doc = OdinDocumentBuilder::new()
            .import("./base.odin", Some("base"), 1)
            .build()
            .unwrap();
        assert_eq!(doc.imports.len(), 1);
        assert_eq!(doc.imports[0].path, "./base.odin");
        assert_eq!(doc.imports[0].alias.as_deref(), Some("base"));
    }

    #[test]
    fn test_builder_schema() {
        let doc = OdinDocumentBuilder::new()
            .schema("https://example.com/schema.odin", 1)
            .build()
            .unwrap();
        assert_eq!(doc.schemas.len(), 1);
        assert_eq!(doc.schemas[0].url, "https://example.com/schema.odin");
    }

    // ─── Resolve tests ───────────────────────────────────────────────────

    #[test]
    fn test_resolve_direct_value() {
        let doc = OdinDocumentBuilder::new()
            .set("x", OdinValues::integer(42))
            .build()
            .unwrap();
        let resolved = doc.resolve("x").unwrap().unwrap();
        assert_eq!(resolved.as_i64(), Some(42));
    }

    #[test]
    fn test_resolve_single_reference() {
        let doc = OdinDocumentBuilder::new()
            .set("target", OdinValues::string("found"))
            .set("ref", OdinValue::Reference {
                path: "target".to_string(),
                modifiers: None,
                directives: vec![],
            })
            .build()
            .unwrap();
        let resolved = doc.resolve("ref").unwrap().unwrap();
        assert_eq!(resolved.as_str(), Some("found"));
    }

    // ─── Default builder tests ───────────────────────────────────────────

    #[test]
    fn test_builder_default() {
        let builder = OdinDocumentBuilder::default();
        let doc = builder.build().unwrap();
        assert!(doc.assignments.is_empty());
    }

    // ─── Clone document tests ────────────────────────────────────────────

    #[test]
    fn test_document_clone() {
        let doc = OdinDocumentBuilder::new()
            .set("x", OdinValues::integer(1))
            .metadata("odin", OdinValues::string("1.0.0"))
            .build()
            .unwrap();
        let cloned = doc.clone();
        assert_eq!(doc, cloned);
    }

    // ─── has() tests ─────────────────────────────────────────────────────

    #[test]
    fn test_has_metadata() {
        let doc = OdinDocumentBuilder::new()
            .metadata("odin", OdinValues::string("1.0.0"))
            .build()
            .unwrap();
        assert!(doc.has("$.odin"));
        assert!(!doc.has("$.missing"));
    }
}
