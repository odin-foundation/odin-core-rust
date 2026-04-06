//! Transform specification types.
//!
//! Defines the structure of ODIN transform documents that specify
//! how to map data between formats.

use std::collections::HashMap;
use crate::types::values::OdinValue;

/// A parsed ODIN transform specification.
#[derive(Debug, Clone)]
pub struct OdinTransform {
    /// Transform metadata from the `{$}` header.
    pub metadata: TransformMetadata,
    /// Source format configuration.
    pub source: Option<SourceConfig>,
    /// Target format configuration.
    pub target: TargetConfig,
    /// Named constants.
    pub constants: HashMap<String, OdinValue>,
    /// Accumulator definitions.
    pub accumulators: HashMap<String, AccumulatorDef>,
    /// Lookup table definitions.
    pub tables: HashMap<String, LookupTable>,
    /// Transform segments (sections with field mappings).
    pub segments: Vec<TransformSegment>,
    /// Import references.
    pub imports: Vec<ImportRef>,
    /// Multi-pass execution order.
    pub passes: Vec<usize>,
    /// Confidential field enforcement mode.
    pub enforce_confidential: Option<ConfidentialMode>,
    /// Whether to enforce strict type checking.
    pub strict_types: bool,
}

/// Transform metadata from the `{$}` header.
#[derive(Debug, Clone, Default)]
pub struct TransformMetadata {
    /// ODIN specification version.
    pub odin_version: Option<String>,
    /// Transform specification version.
    pub transform_version: Option<String>,
    /// Transform direction (e.g., "json->odin", "odin->json").
    pub direction: Option<String>,
    /// Transform name/ID.
    pub name: Option<String>,
    /// Transform description.
    pub description: Option<String>,
}

/// Source format configuration.
#[derive(Debug, Clone)]
pub struct SourceConfig {
    /// Source format (json, xml, csv, fixed-width, flat, yaml, odin).
    pub format: String,
    /// Format-specific options.
    pub options: HashMap<String, String>,
    /// XML namespace prefix→URI mappings (from `{$source.namespace}`).
    /// Key `_` represents the default namespace.
    pub namespaces: HashMap<String, String>,
    /// Discriminator configuration for multi-record input.
    pub discriminator: Option<SourceDiscriminator>,
}

/// Target format configuration.
#[derive(Debug, Clone)]
pub struct TargetConfig {
    /// Target format (json, xml, csv, fixed-width, flat, odin).
    pub format: String,
    /// Format-specific options (e.g., root element for XML).
    pub options: HashMap<String, String>,
}

/// Accumulator definition for aggregation across records.
#[derive(Debug, Clone)]
pub struct AccumulatorDef {
    /// Accumulator name.
    pub name: String,
    /// Initial value.
    pub initial: OdinValue,
    /// Whether to persist value across passes (default false).
    pub persist: bool,
}

/// Lookup table for value mapping.
/// Supports multi-column tables with composite key lookups.
#[derive(Debug, Clone)]
pub struct LookupTable {
    /// Table name.
    pub name: String,
    /// Column names in order (key columns first, then result columns).
    pub columns: Vec<String>,
    /// Row data: each row is a Vec of `DynValue` matching column order.
    pub rows: Vec<Vec<DynValue>>,
    /// Default value when key is not found.
    pub default: Option<DynValue>,
}

/// A transform segment (section with field mappings).
#[derive(Debug, Clone)]
pub struct TransformSegment {
    /// Segment name (from header, e.g., "Customer", "Items").
    pub name: String,
    /// Full segment path (e.g., "segment.HEADER", "vehicles").
    pub path: String,
    /// Source path for iteration (for array mapping).
    pub source_path: Option<String>,
    /// Discriminator configuration for multi-record transforms.
    pub discriminator: Option<Discriminator>,
    /// Whether this segment accumulates into an array (path ends with `[]`).
    pub is_array: bool,
    /// Segment-level directives (e.g., `:type "01"`, `:pass 2`).
    pub directives: Vec<SegmentDirective>,
    /// Field mappings in this segment.
    pub mappings: Vec<FieldMapping>,
    /// Nested segments.
    pub children: Vec<TransformSegment>,
    /// Interleaved mappings and child segments in original order.
    /// When populated, the engine should process these instead of
    /// `mappings` then `children` separately, to preserve field ordering.
    pub items: Vec<SegmentItem>,
    /// Pass number (for multi-pass transforms).
    pub pass: Option<usize>,
    /// Condition for this segment.
    pub condition: Option<String>,
}

/// A directive on a transform segment.
#[derive(Debug, Clone)]
pub struct SegmentDirective {
    /// Directive type (e.g., "type", "pass", "when").
    pub directive_type: String,
    /// Directive value.
    pub value: Option<String>,
}

/// An item in a transform segment — either a field mapping or a child segment.
#[derive(Debug, Clone)]
pub enum SegmentItem {
    /// A field mapping entry.
    Mapping(FieldMapping),
    /// A nested child segment.
    Child(TransformSegment),
}

/// Discriminator for multi-record transforms.
#[derive(Debug, Clone)]
pub struct Discriminator {
    /// Source path to read the discriminator value.
    pub path: String,
    /// Expected value to match.
    pub value: String,
}

/// Source-level discriminator configuration for multi-record input.
#[derive(Debug, Clone)]
pub struct SourceDiscriminator {
    /// Discriminator type.
    pub disc_type: DiscriminatorType,
    /// Position-based: start position (0-indexed).
    pub pos: Option<usize>,
    /// Position-based: field length.
    pub len: Option<usize>,
    /// Field-based: field index (0-indexed).
    pub field: Option<usize>,
    /// Path-based: source path expression.
    pub path: Option<String>,
}

/// Discriminator extraction type.
#[derive(Debug, Clone, PartialEq)]
pub enum DiscriminatorType {
    /// Extract from fixed position in record.
    Position,
    /// Extract from delimited field index.
    Field,
    /// Extract from JSON/ODIN path.
    Path,
}

/// Input for multi-record transform processing.
#[derive(Debug, Clone)]
pub struct MultiRecordInput {
    /// Input records (lines for fixed-width, rows for delimited, etc.).
    pub records: Vec<String>,
    /// Delimiter for delimited format.
    pub delimiter: Option<String>,
}

/// Security limits for transform execution.
pub struct SecurityLimits;

impl SecurityLimits {
    /// Maximum number of records to process.
    pub const MAX_RECORDS: usize = 100_000;
}

/// A single field mapping in a transform.
#[derive(Debug, Clone)]
pub struct FieldMapping {
    /// Target field name.
    pub target: String,
    /// Source expression (copy, verb, literal, or object).
    pub expression: FieldExpression,
    /// Field modifiers to apply to output.
    pub modifiers: Option<crate::types::values::OdinModifiers>,
    /// Directives attached to this field (e.g., `:type integer`, `:date`, `:pos 3`).
    pub directives: Vec<crate::types::values::OdinDirective>,
}

impl FieldMapping {
    /// Create a new field mapping with no directives.
    pub fn new(target: String, expression: FieldExpression, modifiers: Option<crate::types::values::OdinModifiers>) -> Self {
        Self { target, expression, modifiers, directives: Vec::new() }
    }
}

/// A field mapping expression.
#[derive(Debug, Clone)]
pub enum FieldExpression {
    /// Copy from source path: `@path`.
    Copy(String),
    /// Transform with verb: `%verb args`.
    Transform(VerbCall),
    /// Literal value.
    Literal(OdinValue),
    /// Inline object construction.
    Object(Vec<FieldMapping>),
}

/// A verb call in a transform expression.
#[derive(Debug, Clone)]
pub struct VerbCall {
    /// Verb name.
    pub verb: String,
    /// Whether this is a custom verb.
    pub is_custom: bool,
    /// Arguments to the verb.
    pub args: Vec<VerbArg>,
}

/// An argument to a verb call.
#[derive(Debug, Clone)]
pub enum VerbArg {
    /// Reference to source path, with optional extraction directives (e.g., `:pos 3 :len 8`).
    Reference(String, Vec<crate::types::values::OdinDirective>),
    /// Literal value.
    Literal(OdinValue),
    /// Nested verb call.
    Verb(VerbCall),
}

/// Import reference in a transform.
#[derive(Debug, Clone)]
pub struct ImportRef {
    /// Import file path.
    pub path: String,
    /// Optional alias.
    pub alias: Option<String>,
}

/// Confidential field enforcement mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfidentialMode {
    /// Replace confidential values with null.
    Redact,
    /// Replace strings with asterisks, numbers/booleans with null.
    Mask,
}

/// Result of executing a transform.
#[derive(Debug, Clone)]
pub struct TransformResult {
    /// Whether the transform succeeded.
    pub success: bool,
    /// Output data (as a dynamic value tree).
    pub output: Option<DynValue>,
    /// Formatted output string in target format.
    pub formatted: Option<String>,
    /// Errors encountered during transformation.
    pub errors: Vec<TransformError>,
    /// Warnings generated during transformation.
    pub warnings: Vec<TransformWarning>,
    /// Modifiers for output fields (path -> modifiers).
    pub modifiers: HashMap<String, crate::types::values::OdinModifiers>,
}

/// A dynamic value used in transform I/O (replaces `serde_json::Value`).
#[derive(Debug, Clone, PartialEq)]
pub enum DynValue {
    /// Null value.
    Null,
    /// Boolean value.
    Bool(bool),
    /// Integer value.
    Integer(i64),
    /// Floating-point value.
    Float(f64),
    /// Float with preserved raw string (for values exceeding f64 precision).
    FloatRaw(String),
    /// Currency value with decimal places and optional currency code.
    Currency(f64, u8, Option<String>),
    /// Currency with preserved raw string (for values exceeding f64 precision).
    CurrencyRaw(String, u8, Option<String>),
    /// Percent value.
    Percent(f64),
    /// Reference path (for ODIN `@path` output).
    Reference(String),
    /// Binary data as base64 string, with optional algorithm prefix.
    Binary(String),
    /// Date string (YYYY-MM-DD).
    Date(String),
    /// Timestamp string (ISO 8601).
    Timestamp(String),
    /// Time string (HH:MM:SS).
    Time(String),
    /// Duration string (ISO 8601 P...).
    Duration(String),
    /// String value.
    String(String),
    /// Array of values.
    Array(Vec<DynValue>),
    /// Object with ordered key-value pairs.
    Object(Vec<(String, DynValue)>),
}

impl DynValue {
    /// Returns `true` if this is null.
    pub fn is_null(&self) -> bool {
        matches!(self, Self::Null)
    }

    /// Try to get as a string reference.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(s) | Self::Reference(s) | Self::Binary(s)
            | Self::Date(s) | Self::Timestamp(s) | Self::Time(s) | Self::Duration(s)
            | Self::FloatRaw(s) | Self::CurrencyRaw(s, _, _) => Some(s),
            _ => None,
        }
    }

    /// Try to get as an i64.
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Self::Integer(n) => Some(*n),
            _ => None,
        }
    }

    /// Try to get as an f64.
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Self::Float(n) | Self::Currency(n, _, _) | Self::Percent(n) => Some(*n),
            Self::Integer(n) => Some(*n as f64),
            Self::FloatRaw(s) | Self::CurrencyRaw(s, _, _) => s.parse::<f64>().ok(),
            _ => None,
        }
    }

    /// Try to get as a bool.
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(b) => Some(*b),
            _ => None,
        }
    }

    /// Try to get as an array.
    pub fn as_array(&self) -> Option<&[DynValue]> {
        match self {
            Self::Array(arr) => Some(arr),
            _ => None,
        }
    }

    /// Extract an array from this value, parsing JSON-like string arrays.
    ///
    /// Handles:
    /// - Direct `DynValue::Array` — returns the array
    /// - `DynValue::String` starting with `[` — parses as JSON/ODIN array
    /// - Otherwise returns `None`
    pub fn extract_array(&self) -> Option<Vec<DynValue>> {
        match self {
            Self::Array(arr) => Some(arr.clone()),
            Self::String(s) => {
                let trimmed = s.trim();
                if trimmed.starts_with('[') && trimmed.ends_with(']') {
                    parse_array_string(trimmed)
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// Try to get as an object (ordered key-value pairs).
    pub fn as_object(&self) -> Option<&[(String, DynValue)]> {
        match self {
            Self::Object(obj) => Some(obj),
            _ => None,
        }
    }

    /// Extract an object from a `DynValue`, parsing string-encoded JSON/ODIN objects.
    pub fn extract_object(&self) -> Option<Vec<(String, DynValue)>> {
        match self {
            Self::Object(obj) => Some(obj.clone()),
            Self::String(s) => {
                let trimmed = s.trim();
                if trimmed.starts_with('{') && trimmed.ends_with('}') {
                    parse_object_string(trimmed)
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// Get a field from an object by key.
    pub fn get(&self, key: &str) -> Option<&DynValue> {
        match self {
            Self::Object(obj) => obj.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    /// Get an element from an array by index.
    pub fn get_index(&self, index: usize) -> Option<&DynValue> {
        match self {
            Self::Array(arr) => arr.get(index),
            _ => None,
        }
    }

    /// Convert a `serde_json::Value` into a `DynValue`.
    pub fn from_json_value(v: serde_json::Value) -> Self {
        match v {
            serde_json::Value::Null => Self::Null,
            serde_json::Value::Bool(b) => Self::Bool(b),
            serde_json::Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    Self::Integer(i)
                } else if let Some(f) = n.as_f64() {
                    Self::Float(f)
                } else {
                    // Fallback for very large numbers
                    Self::FloatRaw(n.to_string())
                }
            }
            serde_json::Value::String(s) => Self::String(s),
            serde_json::Value::Array(arr) => {
                Self::Array(arr.into_iter().map(Self::from_json_value).collect())
            }
            serde_json::Value::Object(map) => {
                Self::Object(map.into_iter().map(|(k, v)| (k, Self::from_json_value(v))).collect())
            }
        }
    }

    /// Convert this `DynValue` into a `serde_json::Value`.
    pub fn to_json_value(&self) -> serde_json::Value {
        match self {
            Self::Null => serde_json::Value::Null,
            Self::Bool(b) => serde_json::Value::Bool(*b),
            Self::Integer(n) => serde_json::Value::Number(serde_json::Number::from(*n)),
            Self::Float(n) | Self::Currency(n, _, _) | Self::Percent(n) => {
                serde_json::Number::from_f64(*n)
                    .map_or(serde_json::Value::Null, serde_json::Value::Number)
            }
            Self::FloatRaw(s) | Self::CurrencyRaw(s, _, _) => {
                if let Ok(f) = s.parse::<f64>() {
                    serde_json::Number::from_f64(f)
                        .map_or(serde_json::Value::Null, serde_json::Value::Number)
                } else {
                    serde_json::Value::String(s.clone())
                }
            }
            Self::String(s) | Self::Reference(s) | Self::Binary(s)
            | Self::Date(s) | Self::Timestamp(s) | Self::Time(s) | Self::Duration(s) => {
                serde_json::Value::String(s.clone())
            }
            Self::Array(items) => {
                serde_json::Value::Array(items.iter().map(DynValue::to_json_value).collect())
            }
            Self::Object(entries) => {
                let map: serde_json::Map<String, serde_json::Value> = entries
                    .iter()
                    .map(|(k, v)| (k.clone(), v.to_json_value()))
                    .collect();
                serde_json::Value::Object(map)
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Transform error codes
// ─────────────────────────────────────────────────────────────────────────────

/// Standard transform error codes, matching the cross-language specification.
///
/// T001–T010 are reserved for core transform errors.
/// T011+ are for verb-specific / implementation-specific errors.
pub mod transform_error_codes {
    /// Unknown verb — the specified verb does not exist.
    pub const T001_UNKNOWN_VERB: &str = "T001";
    /// Invalid verb arguments — wrong number or type of arguments.
    pub const T002_INVALID_VERB_ARGS: &str = "T002";
    /// Lookup table not found — referenced table doesn't exist.
    pub const T003_LOOKUP_TABLE_NOT_FOUND: &str = "T003";
    /// Lookup key not found — key doesn't exist in table.
    pub const T004_LOOKUP_KEY_NOT_FOUND: &str = "T004";
    /// Source path not found — cannot resolve source path.
    pub const T005_SOURCE_PATH_NOT_FOUND: &str = "T005";
    /// Invalid output format — unsupported or misconfigured format.
    pub const T006_INVALID_OUTPUT_FORMAT: &str = "T006";
    /// Invalid modifier for format — modifier not applicable to target format.
    pub const T007_INVALID_MODIFIER: &str = "T007";
    /// Accumulator overflow — accumulator value exceeds limits.
    pub const T008_ACCUMULATOR_OVERFLOW: &str = "T008";
    /// Loop source not array — `:loop` directive target is not an array.
    pub const T009_LOOP_SOURCE_NOT_ARRAY: &str = "T009";
    /// Position/length exceeds line width — fixed-width field extends past line.
    pub const T010_POSITION_OVERFLOW: &str = "T010";
    /// Incompatible or unknown conversion target (e.g., unknown unit in dateDiff/distance).
    pub const T011_INCOMPATIBLE_CONVERSION: &str = "T011";
}

/// An error during transformation.
#[derive(Debug, Clone)]
pub struct TransformError {
    /// Error message.
    pub message: String,
    /// Path where the error occurred.
    pub path: Option<String>,
    /// Error code.
    pub code: Option<String>,
}

/// Extract a `[TXXX]` error code prefix from an error message, if present.
///
/// Verbs encode structured error codes by prefixing their error strings with
/// `[T011]` (or similar). This helper strips the prefix and returns the code
/// and the remaining message separately.
///
/// # Example
/// ```
/// use odin_core::types::transform::extract_error_code;
/// let (code, msg) = extract_error_code("[T011] dateDiff: unknown unit 'foo'");
/// assert_eq!(code, Some("T011"));
/// assert_eq!(msg, "dateDiff: unknown unit 'foo'");
/// ```
pub fn extract_error_code(error_msg: &str) -> (Option<&str>, &str) {
    if error_msg.starts_with('[') {
        if let Some(end) = error_msg.find(']') {
            let code = &error_msg[1..end];
            // Validate it looks like a T-code (T followed by digits)
            if code.starts_with('T') && code[1..].chars().all(|c| c.is_ascii_digit()) {
                let rest = error_msg[end + 1..].trim_start();
                return (Some(code), rest);
            }
        }
    }
    (None, error_msg)
}

/// A warning during transformation.
#[derive(Debug, Clone)]
pub struct TransformWarning {
    /// Warning message.
    pub message: String,
    /// Path where the warning occurred.
    pub path: Option<String>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Array string parser — parses "[##1, ##2, ##3]" / "[1, 2, 3]" / nested arrays
// ─────────────────────────────────────────────────────────────────────────────

/// Parse a string like `[##1, ##2, "hello", ~, ?true]` into a `Vec<DynValue>`.
/// Supports ODIN type prefixes (`##`, `#`, `#$`, `?`, `~`) and plain JSON values.
fn parse_array_string(s: &str) -> Option<Vec<DynValue>> {
    let trimmed = s.trim();
    if !trimmed.starts_with('[') || !trimmed.ends_with(']') {
        return None;
    }
    let inner = &trimmed[1..trimmed.len() - 1];
    let items = split_array_items(inner);
    let mut result = Vec::new();
    for item in items {
        let item = item.trim();
        if item.is_empty() {
            continue;
        }
        result.push(parse_array_element(item));
    }
    Some(result)
}

/// Split array items, respecting nested brackets and quotes.
fn split_array_items(s: &str) -> Vec<&str> {
    let mut items = Vec::new();
    let mut depth = 0i32;
    let mut in_string = false;
    let mut escape = false;
    let mut start = 0;
    let bytes = s.as_bytes();

    for i in 0..bytes.len() {
        if escape {
            escape = false;
            continue;
        }
        match bytes[i] {
            b'\\' if in_string => escape = true,
            b'"' => in_string = !in_string,
            b'[' if !in_string => depth += 1,
            b']' if !in_string => depth -= 1,
            b',' if !in_string && depth == 0 => {
                items.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    if start < s.len() {
        items.push(&s[start..]);
    }
    items
}

/// Parse a single array element with ODIN type prefix support.
fn parse_array_element(s: &str) -> DynValue {
    let s = s.trim();
    // Null
    if s == "~" || s == "null" {
        return DynValue::Null;
    }
    // Boolean
    if s == "?true" || s == "true" {
        return DynValue::Bool(true);
    }
    if s == "?false" || s == "false" {
        return DynValue::Bool(false);
    }
    // ODIN integer: ##N
    if let Some(rest) = s.strip_prefix("##") {
        if let Ok(n) = rest.parse::<i64>() {
            return DynValue::Integer(n);
        }
    }
    // ODIN currency: #$N.NN
    if let Some(rest) = s.strip_prefix("#$") {
        if let Ok(n) = rest.parse::<f64>() {
            return DynValue::Float(n);
        }
    }
    // ODIN number: #N
    if let Some(rest) = s.strip_prefix('#') {
        if let Ok(n) = rest.parse::<f64>() {
            return DynValue::Float(n);
        }
    }
    // Quoted string
    if s.starts_with('"') && s.ends_with('"') && s.len() >= 2 {
        let inner = &s[1..s.len() - 1];
        // Unescape
        let unescaped = inner.replace("\\\"", "\"").replace("\\\\", "\\")
            .replace("\\n", "\n").replace("\\t", "\t").replace("\\r", "\r");
        return DynValue::String(unescaped);
    }
    // Nested array
    if s.starts_with('[') && s.ends_with(']') {
        if let Some(arr) = parse_array_string(s) {
            return DynValue::Array(arr);
        }
    }
    // Plain number
    if let Ok(n) = s.parse::<i64>() {
        return DynValue::Integer(n);
    }
    if let Ok(n) = s.parse::<f64>() {
        return DynValue::Float(n);
    }
    // Nested object
    if s.starts_with('{') && s.ends_with('}') {
        if let Some(obj) = parse_object_string(s) {
            return DynValue::Object(obj);
        }
    }
    // Fallback: bare string
    DynValue::String(s.to_string())
}

/// Parse a string-encoded JSON/ODIN object like `{"key": "value", "num": ##42}`.
fn parse_object_string(s: &str) -> Option<Vec<(String, DynValue)>> {
    let trimmed = s.trim();
    if !trimmed.starts_with('{') || !trimmed.ends_with('}') {
        return None;
    }
    let inner = trimmed[1..trimmed.len() - 1].trim();
    if inner.is_empty() {
        return Some(Vec::new());
    }
    // Split by commas at the top level (respecting nesting and strings)
    let pairs = split_array_items(inner);
    let mut result = Vec::new();
    for pair in pairs {
        let pair = pair.trim();
        if pair.is_empty() {
            continue;
        }
        // Find the colon separator (key: value)
        let colon_pos = find_colon_separator(pair)?;
        let key_str = pair[..colon_pos].trim();
        let val_str = pair[colon_pos + 1..].trim();
        // Parse key (must be a quoted string or bare word)
        let key = if key_str.starts_with('"') && key_str.ends_with('"') && key_str.len() >= 2 {
            key_str[1..key_str.len() - 1].replace("\\\"", "\"").replace("\\\\", "\\")
        } else {
            key_str.to_string()
        };
        // Parse value
        let value = parse_array_element(val_str);
        result.push((key, value));
    }
    Some(result)
}

/// Find the position of the colon separator in a key:value pair, skipping colons inside strings.
fn find_colon_separator(s: &str) -> Option<usize> {
    let mut in_string = false;
    let mut escape = false;
    for (i, b) in s.bytes().enumerate() {
        if escape {
            escape = false;
            continue;
        }
        match b {
            b'\\' if in_string => escape = true,
            b'"' => in_string = !in_string,
            b':' if !in_string => return Some(i),
            _ => {}
        }
    }
    None
}
