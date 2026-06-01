//! Transform engine — executes ODIN transforms to map data between formats.
//!
//! The transform engine:
//! 1. Parses a transform specification (`.transform.odin`)
//! 2. Reads source data in the configured format
//! 3. Executes field mappings with verb expressions
//! 4. Produces output in the target format

pub mod verbs;
pub mod formatters;
pub mod source_parsers;
pub mod parser;
pub mod engine;

use crate::types::transform::{OdinTransform, TransformResult, DynValue, ExecuteOptions};
use crate::types::document::OdinDocument;
use crate::types::values::{OdinValue, OdinArrayItem};
use crate::types::errors::ParseError;

/// Parse a transform specification from ODIN text.
///
/// Parses the input as an ODIN document first, then extracts transform
/// metadata, segments, field mappings, lookup tables, and other transform
/// configuration from the document structure.
///
/// # Errors
///
/// Returns `ParseError` if the transform text is not valid ODIN.
pub fn parse_transform(input: &str) -> Result<OdinTransform, ParseError> {
    let preprocessed = rewrite_section_verb_assignments(input);
    let doc = crate::parser::parse(&preprocessed, None)?;
    Ok(parser::parse_transform_doc(doc))
}

/// Rewrite section-verb assignments (`{.name} = %verb …`) into ordinary field
/// mappings (`name = %verb …`). An object-returning verb assigned to a section
/// header populates that section's fields; expressed as a normal mapping, the
/// resulting object serializes back to the same `{.name}` sub-block.
fn rewrite_section_verb_assignments(input: &str) -> std::borrow::Cow<'_, str> {
    if !input.contains("} =") {
        return std::borrow::Cow::Borrowed(input);
    }
    let mut out = String::with_capacity(input.len());
    let mut changed = false;
    for line in input.split_inclusive('\n') {
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix('{') {
            if let Some(close) = rest.find('}') {
                let header = &rest[..close];
                let after = rest[close + 1..].trim_start();
                // Only a header immediately followed by `=` (not `[`/`$`/`@`
                // headers, not tabular `[]`).
                if let Some(expr) = after.strip_prefix('=') {
                    if !header.contains('[') && !header.starts_with('$') && !header.starts_with('@') {
                        let name = header.trim_start_matches('.').rsplit('.').next().unwrap_or(header);
                        if !name.is_empty() {
                            let indent = &line[..line.len() - trimmed.len()];
                            out.push_str(indent);
                            out.push_str(name);
                            out.push_str(" = ");
                            out.push_str(expr.trim_start());
                            if !out.ends_with('\n') && line.ends_with('\n') {
                                out.push('\n');
                            }
                            changed = true;
                            continue;
                        }
                    }
                }
            }
        }
        out.push_str(line);
    }
    if changed { std::borrow::Cow::Owned(out) } else { std::borrow::Cow::Borrowed(input) }
}

/// Execute a transform against source data.
///
/// Delegates to the [`engine`] module which handles segment ordering,
/// expression evaluation, verb dispatch, and output formatting.
pub fn execute_transform(
    transform: &OdinTransform,
    source: &DynValue,
) -> TransformResult {
    engine::execute(transform, source)
}

/// Execute a transform against source data with explicit execution options.
///
/// Use this to supply an `@import` resolver. Without a resolver, `@import`
/// references stay unresolved.
pub fn execute_transform_with_options(
    transform: &OdinTransform,
    source: &DynValue,
    options: &ExecuteOptions,
) -> TransformResult {
    engine::execute_with_options(transform, source, options)
}

/// Execute a multi-record transform against raw input records.
///
/// Each record is routed to a segment based on a discriminator value extracted
/// from the record. Array segments accumulate records into arrays.
pub fn execute_multi_record(
    transform: &OdinTransform,
    input: &crate::types::transform::MultiRecordInput,
) -> TransformResult {
    // Join records into raw input for the existing engine
    let raw = input.records.join("\n");
    let source = DynValue::String(raw);
    engine::execute(transform, &source)
}

/// Execute a transform against an `OdinDocument`.
///
/// Converts the document to a `DynValue` object and then executes the transform.
pub fn transform_document(
    transform: &OdinTransform,
    doc: &OdinDocument,
) -> TransformResult {
    let source = document_to_dynvalue(doc);
    engine::execute(transform, &source)
}

/// Convert an `OdinDocument` into a `DynValue` for transform processing.
///
/// Builds a nested object structure from the document's flat dot-path assignments.
pub fn document_to_dynvalue(doc: &OdinDocument) -> DynValue {
    let mut root = Vec::<(String, DynValue)>::new();

    for (path, value) in doc.assignments.iter() {
        let dyn_val = odin_value_to_dynvalue(value);
        insert_at_path(&mut root, path, dyn_val);
    }

    DynValue::Object(root)
}

fn odin_value_to_dynvalue(value: &OdinValue) -> DynValue {
    match value {
        OdinValue::Null { .. } | OdinValue::Binary { .. } => DynValue::Null,
        OdinValue::Boolean { value: b, .. } => DynValue::Bool(*b),
        // An integer literal too large for i64 is stored with value 0 but keeps
        // its raw text; preserve the magnitude as a float so downstream numeric
        // logic (e.g. accumulator overflow) sees the real value.
        OdinValue::Integer { value: 0, raw: Some(r), .. } if r != "0" => {
            r.parse::<f64>().map_or(DynValue::Integer(0), DynValue::Float)
        }
        OdinValue::Integer { value: n, .. } => DynValue::Integer(*n),
        OdinValue::Number { value: f, .. }
        | OdinValue::Percent { value: f, .. } => DynValue::Float(*f),
        OdinValue::Currency { value, decimal_places, currency_code, .. } => {
            DynValue::Currency(*value, *decimal_places, currency_code.clone())
        }
        OdinValue::String { value: s, .. }
        | OdinValue::Time { value: s, .. }
        | OdinValue::Duration { value: s, .. } => DynValue::String(s.clone()),
        OdinValue::Date { raw, .. }
        | OdinValue::Timestamp { raw, .. } => DynValue::String(raw.clone()),
        OdinValue::Reference { path, .. } => DynValue::String(format!("@{path}")),
        OdinValue::Verb { verb, .. } => DynValue::String(format!("%{verb}")),
        OdinValue::Array { items, .. } => {
            let arr: Vec<DynValue> = items.iter().map(|item| match item {
                OdinArrayItem::Value(v) => odin_value_to_dynvalue(v),
                OdinArrayItem::Record(fields) => {
                    let obj: Vec<(String, DynValue)> = fields.iter()
                        .map(|(k, v)| (k.clone(), odin_value_to_dynvalue(v)))
                        .collect();
                    DynValue::Object(obj)
                }
            }).collect();
            DynValue::Array(arr)
        }
        OdinValue::Object { value: fields, .. } => {
            let obj: Vec<(String, DynValue)> = fields.iter()
                .map(|(k, v)| (k.clone(), odin_value_to_dynvalue(v)))
                .collect();
            DynValue::Object(obj)
        }
    }
}

/// Insert a value at a dot-separated path into a nested object structure.
fn insert_at_path(root: &mut Vec<(String, DynValue)>, path: &str, value: DynValue) {
    // Split on first dot or bracket to find top-level key
    let (head, rest) = split_first_segment(path);

    if rest.is_empty() {
        // Leaf: insert directly
        if let Some(existing) = root.iter_mut().find(|(k, _)| k == &head) {
            existing.1 = value;
        } else {
            root.push((head, value));
        }
        return;
    }

    // Check if next segment is an array index
    let next_is_array = rest.starts_with('[');

    // Find or create the intermediate container
    let container = if let Some(existing) = root.iter_mut().find(|(k, _)| k == &head) {
        &mut existing.1
    } else {
        let placeholder = if next_is_array {
            DynValue::Array(Vec::new())
        } else {
            DynValue::Object(Vec::new())
        };
        root.push((head.clone(), placeholder));
        // Safe: we just pushed an element, so last_mut() is always Some.
        let last = root.last_mut();
        debug_assert!(last.is_some(), "Vec must be non-empty after push");
        match last {
            Some(entry) => &mut entry.1,
            // This branch is unreachable after a push, but avoids unwrap.
            None => return,
        }
    };

    if next_is_array {
        // Array: parse index, ensure array exists
        if let DynValue::Array(ref mut arr) = container {
            if let Some(bracket_end) = rest.find(']') {
                if let Ok(idx) = rest[1..bracket_end].parse::<usize>() {
                    let after = &rest[bracket_end + 1..];
                    let after = after.strip_prefix('.').unwrap_or(after);

                    // Extend array if needed
                    while arr.len() <= idx {
                        arr.push(DynValue::Null);
                    }

                    if after.is_empty() {
                        arr[idx] = value;
                    } else if after.starts_with('[') {
                        // Nested array index: the element is itself an array.
                        if !matches!(arr[idx], DynValue::Array(_)) {
                            arr[idx] = DynValue::Array(Vec::new());
                        }
                        insert_into_array(&mut arr[idx], after, value);
                    } else {
                        // Recurse into array element as an object.
                        if !matches!(arr[idx], DynValue::Object(_)) {
                            arr[idx] = DynValue::Object(Vec::new());
                        }
                        if let DynValue::Object(ref mut obj) = arr[idx] {
                            insert_at_path(obj, after, value);
                        }
                    }
                }
            }
        }
    } else {
        // Object: recurse
        if let DynValue::Object(ref mut obj) = container {
            insert_at_path(obj, &rest, value);
        }
    }
}

/// Insert `value` into an array-typed container along a path that begins with
/// `[idx]` (possibly followed by further `[idx]` indices or `.field`).
fn insert_into_array(container: &mut DynValue, path: &str, value: DynValue) {
    let DynValue::Array(arr) = container else { return };
    let Some(bracket_end) = path.find(']') else { return };
    let Ok(idx) = path[1..bracket_end].parse::<usize>() else { return };
    let after = &path[bracket_end + 1..];
    let after = after.strip_prefix('.').unwrap_or(after);

    while arr.len() <= idx {
        arr.push(DynValue::Null);
    }

    if after.is_empty() {
        arr[idx] = value;
    } else if after.starts_with('[') {
        if !matches!(arr[idx], DynValue::Array(_)) {
            arr[idx] = DynValue::Array(Vec::new());
        }
        insert_into_array(&mut arr[idx], after, value);
    } else {
        if !matches!(arr[idx], DynValue::Object(_)) {
            arr[idx] = DynValue::Object(Vec::new());
        }
        if let DynValue::Object(ref mut obj) = arr[idx] {
            insert_at_path(obj, after, value);
        }
    }
}

/// Split a path into its first segment and the rest.
/// "a.b.c" → ("a", "b.c")
/// "a[0].b" → ("a", "[0].b")
fn split_first_segment(path: &str) -> (String, String) {
    let bytes = path.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        if b == b'.' {
            return (path[..i].to_string(), path[i + 1..].to_string());
        }
        if b == b'[' {
            return (path[..i].to_string(), path[i..].to_string());
        }
    }
    (path.to_string(), String::new())
}
