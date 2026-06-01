//! Schema validation for ODIN documents.
//!
//! Validates an `OdinDocument` against an `OdinSchemaDefinition`,
//! producing a `ValidationResult` with errors and warnings.

mod format_validators;
mod invariant_evaluator;
mod schema_definition;
#[cfg(test)]
mod schema_enforcement_tests;
pub mod schema_parser;
pub mod schema_serializer;
pub mod validate_redos;

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use crate::types::document::OdinDocument;
use crate::types::schema::{
    OdinSchemaDefinition, SchemaConstraint, SchemaField,
    SchemaFieldType, SchemaObjectConstraint, SchemaType, ConditionalOperator, ConditionalValue,
    ValidationResult,
};
use crate::types::options::ValidateOptions;
use crate::types::errors::{ValidationError, ValidationErrorCode};
use crate::types::values::OdinValue;
use crate::resolver::TypeRegistry;

// ─────────────────────────────────────────────────────────────────────────────
// Schema-Only Memo
// ─────────────────────────────────────────────────────────────────────────────

/// Schema-only results, independent of any document: the composition-expanded
/// schema and the schema-level errors (V013 type references, V017 well-formedness).
///
/// Computed once per schema object and stored on it (see
/// [`OdinSchemaDefinition::validation_memo`]), then reused across every document
/// validated against that schema — no per-call key computation.
pub(crate) struct SchemaValidationMemo {
    expanded: OdinSchemaDefinition,
    /// Address of the registry this memo was computed against (`None` = no
    /// registry). Stored as a `usize` so the memo stays `Send + Sync`; used only
    /// to detect a registry change between calls, never dereferenced.
    registry_addr: Option<usize>,
    /// V013 schema-level type-reference errors (computed before V017).
    type_ref_errors: Vec<ValidationError>,
    /// V017 schema-definition well-formedness errors.
    definition_errors: Vec<ValidationError>,
    /// Document-independent field set: section `_composition` intersections plus
    /// the schema's explicit fields. Reused directly when the schema has no
    /// field-level `@TypeRef`/`@Reference` augmentation to apply.
    base_fields: Vec<(String, SchemaField)>,
    /// Whether any field carries a defined-type `@TypeRef`/`@Reference` whose
    /// fields must be enforced per-document (the only document-dependent part of
    /// field-composition expansion). When false, `base_fields` is the final set.
    has_typeref_augmentation: bool,
}

/// The address of an optional registry, used as an identity key.
fn registry_addr(registry: Option<&TypeRegistry>) -> Option<usize> {
    registry.map(|r| r as *const TypeRegistry as usize)
}

/// Build the schema-only memo for `schema`/`registry`.
fn build_schema_memo(
    schema: &OdinSchemaDefinition,
    registry: Option<&TypeRegistry>,
) -> SchemaValidationMemo {
    let expanded = expand_type_composition(schema).into_owned();
    let mut type_ref_errors = Vec::new();
    validate_schema_type_references(&expanded, registry, &mut type_ref_errors);
    let mut definition_errors = Vec::new();
    schema_definition::validate_schema_definition(&expanded, registry, &mut definition_errors);
    let base_fields = base_field_compositions(&expanded, registry);
    let has_typeref_augmentation = base_fields.iter().any(|(_, field)| {
        let name = match &field.field_type {
            SchemaFieldType::TypeRef(n) | SchemaFieldType::Reference(n) => n,
            _ => return false,
        };
        name.split('&')
            .map(str::trim)
            .filter(|m| !m.is_empty())
            .any(|m| lookup_type(&expanded, registry, m).is_some())
    });
    SchemaValidationMemo {
        registry_addr: registry_addr(registry),
        type_ref_errors,
        definition_errors,
        base_fields,
        has_typeref_augmentation,
        expanded,
    }
}

/// Get (computing once) the schema-only results for `schema`/`registry`.
///
/// The memo is stored on the schema object, so the schema-only walk runs exactly
/// once per schema and is read in O(1) thereafter, with no per-call hashing.
/// In the rare case the same schema is later validated against a *different*
/// registry, the memo is rebuilt transiently for that call without disturbing
/// the stored one.
fn get_schema_memo(
    schema: &OdinSchemaDefinition,
    registry: Option<&TypeRegistry>,
) -> Arc<SchemaValidationMemo> {
    let req_addr = registry_addr(registry);
    let memo = schema
        .validation_memo
        .get_or_init(|| Arc::new(build_schema_memo(schema, registry)));
    if memo.registry_addr == req_addr {
        return Arc::clone(memo);
    }
    // Registry differs from the one the stored memo was computed against.
    Arc::new(build_schema_memo(schema, registry))
}

/// Validate a document against a schema.
pub fn validate(
    doc: &OdinDocument,
    schema: &OdinSchemaDefinition,
    options: Option<&ValidateOptions>,
) -> ValidationResult {
    validate_with_registry(doc, schema, options, None)
}

/// Validate a document against a schema, resolving `@alias.typename` references via `registry`.
///
/// Pass `None` for the registry to get the same behavior as [`validate`].
pub fn validate_with_registry(
    doc: &OdinDocument,
    schema: &OdinSchemaDefinition,
    options: Option<&ValidateOptions>,
    registry: Option<&TypeRegistry>,
) -> ValidationResult {
    let opts = options.cloned().unwrap_or_default();
    let mut errors = Vec::new();

    // 0. Schema-only work (type composition expansion, V013 schema type refs,
    //    V017 well-formedness) is computed once per schema and reused.
    let memo = get_schema_memo(schema, registry);
    let schema = &memo.expanded;

    // Schema-level type references (V013): imported types resolve via registry.
    for e in &memo.type_ref_errors {
        errors.push(e.clone());
    }
    if opts.fail_fast && !errors.is_empty() {
        return ValidationResult::invalid(errors);
    }

    // Schema-definition well-formedness (V017): override/intersection/tabular/default.
    for e in &memo.definition_errors {
        errors.push(e.clone());
    }
    if opts.fail_fast && !errors.is_empty() {
        return ValidationResult::invalid(errors);
    }

    // 1. Validate fields defined in schema (including fields synthesized from
    //    section `_composition` intersections and field-level `@TypeRef`s). The
    //    document-independent base set is memoized; only schemas with field-level
    //    `@TypeRef`/`@Reference` types pay the per-document augmentation cost.
    let augmented;
    let expanded_fields: &[(String, SchemaField)] = if memo.has_typeref_augmentation {
        augmented = augment_typeref_fields(&memo.base_fields, doc, schema, registry);
        &augmented
    } else {
        &memo.base_fields
    };
    for (path, field) in expanded_fields {
        // Array item-field templates (`path[].field`) are not literal document
        // paths; they are validated per-row, not against the template path.
        if path.contains("[].") {
            continue;
        }
        validate_field(doc, path, field, &opts, &mut errors);
        if opts.fail_fast && !errors.is_empty() {
            return ValidationResult::invalid(errors);
        }
    }

    // 2. Validate type definitions' fields for sections that match
    for (type_name, schema_type) in &schema.types {
        // Type definitions apply to sections referencing @TypeName
        // For now, validate fields if they appear in the document at common paths
        for field in &schema_type.fields {
            // Check if this type is used directly as a section
            let path = format!("{}.{}", type_name, field.name);
            if doc.has(&path) {
                validate_field(doc, &path, field, &opts, &mut errors);
                if opts.fail_fast && !errors.is_empty() {
                    return ValidationResult::invalid(errors);
                }
            }
        }
    }

    // 3. Validate array constraints
    for (path, array_def) in &schema.arrays {
        validate_array(doc, path, array_def, &mut errors);
        if opts.fail_fast && !errors.is_empty() {
            return ValidationResult::invalid(errors);
        }
    }

    // 4. Validate object-level constraints
    for (path, obj_constraints) in &schema.constraints {
        for constraint in obj_constraints {
            validate_object_constraint(doc, path, constraint, &mut errors);
            if opts.fail_fast && !errors.is_empty() {
                return ValidationResult::invalid(errors);
            }
        }
    }

    // 5. Validate references (V012/V013)
    validate_references(doc, &mut errors);

    // 6. Strict mode: check for unknown fields
    if opts.strict {
        validate_strict(doc, schema, &mut errors);
    }

    if errors.is_empty() {
        ValidationResult::valid()
    } else {
        ValidationResult::invalid(errors)
    }
}

/// Parse an ODIN schema from text.
///
/// # Errors
///
/// Returns an error if the schema text is invalid.
pub fn parse_schema(input: &str) -> Result<OdinSchemaDefinition, crate::types::errors::ParseError> {
    schema_parser::parse_schema(input)
}

// ─────────────────────────────────────────────────────────────────────────────
// Field Validation
// ─────────────────────────────────────────────────────────────────────────────

fn validate_field(
    doc: &OdinDocument,
    path: &str,
    field: &SchemaField,
    _opts: &ValidateOptions,
    errors: &mut Vec<ValidationError>,
) {
    let value = doc.get(path);

    // Computed fields are produced downstream, not author-supplied.
    if field.computed && value.is_none() {
        return;
    }

    // Required check (V001 / V010)
    if field.required && value.is_none() {
        if field.conditionals.is_empty() {
            errors.push(ValidationError::new(
                ValidationErrorCode::RequiredFieldMissing,
                path,
                format!("Required field '{}' is missing", field.name),
            ));
        } else {
            // Conditional requirement: check if the condition is met
            let should_be_required = field.conditionals.iter().any(|cond| {
                let parent_path = path.rfind('.').map_or("", |p| &path[..p]);
                let cond_field_path = if cond.field.contains('.') || parent_path.is_empty() {
                    cond.field.clone()
                } else {
                    format!("{}.{}", parent_path, cond.field)
                };
                let cond_value = doc.get(&cond_field_path);
                let matches = matches_condition_value(cond_value, &cond.operator, &cond.value);
                if cond.unless { !matches } else { matches }
            });
            if should_be_required {
                errors.push(ValidationError::new(
                    ValidationErrorCode::ConditionalRequirementNotMet,
                    path,
                    format!("Conditional requirement not met: field '{}' is required", field.name),
                ));
            }
        }
        return;
    }

    let Some(value) = value else { return }; // Optional field not present — ok

    // Null check for required fields (V002)
    if field.required && matches!(value, OdinValue::Null { .. }) {
        errors.push(ValidationError::new(
            ValidationErrorCode::TypeMismatch,
            path,
            format!("Required field '{}' cannot be null", field.name),
        ));
        return;
    }

    // Type check (V002)
    if !check_type_match(value, &field.field_type) {
        errors.push(ValidationError {
            path: path.to_string(),
            error_code: ValidationErrorCode::TypeMismatch,
            message: format!(
                "Expected type {:?}, got {:?}",
                field.field_type,
                value_type_name(value)
            ),
            expected: Some(format!("{:?}", field.field_type)),
            actual: Some(value_type_name(value).to_string()),
            schema_path: None,
        });
        return;
    }

    // Decimal places: enforce exactly N places (#.N) from the raw string.
    if let SchemaFieldType::Decimal { decimal_places: Some(places) } = field.field_type {
        if let OdinValue::Number { value: num, raw, .. } = value {
            let raw_str = raw.clone().unwrap_or_else(|| num.to_string());
            let actual = raw_str.find('.').map_or(0, |dot| raw_str.len() - dot - 1);
            if actual != places as usize {
                errors.push(ValidationError {
                    path: path.to_string(),
                    error_code: ValidationErrorCode::ValueOutOfBounds,
                    message: format!("Decimal places mismatch: expected exactly {places}, got {actual}"),
                    expected: Some(places.to_string()),
                    actual: Some(actual.to_string()),
                    schema_path: None,
                });
            }
        }
    }

    // Currency places: enforce exactly N places (#$.N).
    if let SchemaFieldType::Currency { decimal_places: Some(places) } = field.field_type {
        if let OdinValue::Currency { decimal_places: actual, .. } = value {
            if *actual != places {
                errors.push(ValidationError {
                    path: path.to_string(),
                    error_code: ValidationErrorCode::ValueOutOfBounds,
                    message: format!("Currency decimal places mismatch: expected exactly {places}, got {actual}"),
                    expected: Some(places.to_string()),
                    actual: Some(actual.to_string()),
                    schema_path: None,
                });
            }
        }
    }

    // Constraint validation
    for constraint in &field.constraints {
        validate_constraint(value, path, constraint, errors);
    }
}

/// Check if an `OdinValue` matches a `SchemaFieldType`.
fn check_type_match(value: &OdinValue, expected: &SchemaFieldType) -> bool {
    match (value, expected) {
        // Null is allowed unless required; TypeRef can't be resolved without registry
        (OdinValue::Null { .. }, _)
        | (OdinValue::String { .. }, SchemaFieldType::String)
        | (OdinValue::Boolean { .. }, SchemaFieldType::Boolean)
        | (OdinValue::Integer { .. }, SchemaFieldType::Integer)
        | (OdinValue::Integer { .. }, SchemaFieldType::Number { .. }) // int is a number
        | (OdinValue::Number { .. }, SchemaFieldType::Number { .. })
        | (OdinValue::Number { .. }, SchemaFieldType::Decimal { .. })
        | (OdinValue::Currency { .. }, SchemaFieldType::Currency { .. })
        | (OdinValue::Percent { .. }, SchemaFieldType::Percent)
        | (OdinValue::Date { .. }, SchemaFieldType::Date)
        | (OdinValue::Timestamp { .. }, SchemaFieldType::Timestamp)
        | (OdinValue::Time { .. }, SchemaFieldType::Time)
        | (OdinValue::Duration { .. }, SchemaFieldType::Duration)
        | (OdinValue::Binary { .. }, SchemaFieldType::Binary)
        | (OdinValue::Reference { .. }, SchemaFieldType::Reference(_))
        | (OdinValue::String { .. }, SchemaFieldType::Enum(_)) // checked separately
        | (_, SchemaFieldType::TypeRef(_)) => true,
        // Union: any member matches
        (val, SchemaFieldType::Union(members)) => {
            members.iter().any(|m| check_type_match(val, m))
        }
        _ => false,
    }
}

fn value_type_name(value: &OdinValue) -> &'static str {
    match value {
        OdinValue::Null { .. } => "null",
        OdinValue::Boolean { .. } => "boolean",
        OdinValue::String { .. } => "string",
        OdinValue::Integer { .. } => "integer",
        OdinValue::Number { .. } => "number",
        OdinValue::Currency { .. } => "currency",
        OdinValue::Percent { .. } => "percent",
        OdinValue::Date { .. } => "date",
        OdinValue::Timestamp { .. } => "timestamp",
        OdinValue::Time { .. } => "time",
        OdinValue::Duration { .. } => "duration",
        OdinValue::Reference { .. } => "reference",
        OdinValue::Binary { .. } => "binary",
        OdinValue::Verb { .. } => "verb",
        OdinValue::Array { .. } => "array",
        OdinValue::Object { .. } => "object",
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Constraint Validation
// ─────────────────────────────────────────────────────────────────────────────

fn validate_constraint(
    value: &OdinValue,
    path: &str,
    constraint: &SchemaConstraint,
    errors: &mut Vec<ValidationError>,
) {
    match constraint {
        SchemaConstraint::Bounds { min, max, min_exclusive, max_exclusive } => {
            validate_bounds(value, path, min, max, *min_exclusive, *max_exclusive, errors);
        }
        SchemaConstraint::Pattern(pattern) => {
            validate_pattern(value, path, pattern, errors);
        }
        SchemaConstraint::Enum(allowed) => {
            validate_enum(value, path, allowed, errors);
        }
        SchemaConstraint::Format(format_name) => {
            validate_format(value, path, format_name, errors);
        }
        SchemaConstraint::Unique => {
            // Unique is validated at array level, not individual field
        }
        SchemaConstraint::Size { min, max } => {
            validate_size(value, path, min, max, errors);
        }
    }
}

fn validate_bounds(
    value: &OdinValue,
    path: &str,
    min: &Option<String>,
    max: &Option<String>,
    min_exclusive: bool,
    max_exclusive: bool,
    errors: &mut Vec<ValidationError>,
) {
    // For numeric types, compare numerically
    if let Some(num) = value.as_f64() {
        if let Some(min_str) = min {
            if let Ok(min_val) = min_str.parse::<f64>() {
                let fail = if min_exclusive { num <= min_val } else { num < min_val };
                if fail {
                    errors.push(ValidationError::new(
                        ValidationErrorCode::ValueOutOfBounds,
                        path,
                        format!("Value {num} is below minimum {min_str}"),
                    ));
                    return;
                }
            }
        }
        if let Some(max_str) = max {
            if let Ok(max_val) = max_str.parse::<f64>() {
                let fail = if max_exclusive { num >= max_val } else { num > max_val };
                if fail {
                    errors.push(ValidationError::new(
                        ValidationErrorCode::ValueOutOfBounds,
                        path,
                        format!("Value {num} is above maximum {max_str}"),
                    ));
                }
            }
        }
        return;
    }

    // For date/timestamp types, compare chronologically against temporal bounds.
    if let Some(actual) = temporal_key(value) {
        if let Some(min_str) = min {
            if let Some(min_key) = parse_temporal_bound(min_str) {
                if actual < min_key {
                    errors.push(ValidationError::new(
                        ValidationErrorCode::ValueOutOfBounds,
                        path,
                        format!("Date {value:?} is before minimum {min_str}"),
                    ));
                    return;
                }
            }
        }
        if let Some(max_str) = max {
            if let Some(max_key) = parse_temporal_bound(max_str) {
                if actual > max_key {
                    errors.push(ValidationError::new(
                        ValidationErrorCode::ValueOutOfBounds,
                        path,
                        format!("Date {value:?} is after maximum {max_str}"),
                    ));
                }
            }
        }
        return;
    }

    // For string types, compare length
    if let Some(s) = value.as_str() {
        let len = s.len();
        if let Some(min_str) = min {
            if let Ok(min_val) = min_str.parse::<usize>() {
                let fail = if min_exclusive { len <= min_val } else { len < min_val };
                if fail {
                    errors.push(ValidationError::new(
                        ValidationErrorCode::ValueOutOfBounds,
                        path,
                        format!("String length {len} is below minimum {min_str}"),
                    ));
                    return;
                }
            }
        }
        if let Some(max_str) = max {
            if let Ok(max_val) = max_str.parse::<usize>() {
                let fail = if max_exclusive { len >= max_val } else { len > max_val };
                if fail {
                    errors.push(ValidationError::new(
                        ValidationErrorCode::ValueOutOfBounds,
                        path,
                        format!("String length {len} is above maximum {max_str}"),
                    ));
                }
            }
        }
    }
}

/// Days since the civil epoch (1970-01-01) for a proleptic Gregorian date.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// A comparable millisecond key for a temporal `OdinValue`, or `None` if the
/// value is not temporal.
fn temporal_key(value: &OdinValue) -> Option<i64> {
    match value {
        OdinValue::Date { year, month, day, .. } => {
            Some(days_from_civil(*year as i64, *month as i64, *day as i64) * 86_400_000)
        }
        OdinValue::Timestamp { epoch_ms, .. } => Some(*epoch_ms),
        _ => None,
    }
}

/// Parse a temporal bound literal (`YYYY-MM-DD`, optionally with a time part)
/// into a comparable millisecond key.
fn parse_temporal_bound(s: &str) -> Option<i64> {
    let date_part = s.split(['T', ' ']).next().unwrap_or(s);
    let mut it = date_part.split('-');
    let y: i64 = it.next()?.parse().ok()?;
    let m: i64 = it.next()?.parse().ok()?;
    let d: i64 = it.next()?.parse().ok()?;
    Some(days_from_civil(y, m, d) * 86_400_000)
}

/// Cached compilation of a distinct pattern string: the ReDoS safety verdict
/// and, when the `regex` feature is on, the compiled regex.
#[derive(Clone)]
enum CompiledPattern {
    /// ReDoS analysis rejected the pattern.
    Unsafe(String),
    /// Pattern is safe but failed to compile.
    Invalid,
    /// Pattern is safe and compiled (only tracked under the `regex` feature).
    #[cfg(feature = "regex")]
    Compiled(Arc<regex::Regex>),
    /// Pattern is safe; matching is unavailable without the `regex` feature.
    #[cfg(not(feature = "regex"))]
    Safe,
}

fn pattern_cache() -> &'static Mutex<HashMap<String, CompiledPattern>> {
    static CACHE: OnceLock<Mutex<HashMap<String, CompiledPattern>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Compile a pattern once per distinct string and reuse: runs the ReDoS safety
/// check and (under the `regex` feature) compiles the regex.
fn compile_pattern(pattern: &str) -> CompiledPattern {
    if let Ok(cache) = pattern_cache().lock() {
        if let Some(c) = cache.get(pattern) {
            return c.clone();
        }
    }

    let redos_check = validate_redos::analyze_pattern(pattern);
    let compiled = if !redos_check.safe {
        CompiledPattern::Unsafe(redos_check.reason.unwrap_or_default())
    } else {
        #[cfg(feature = "regex")]
        {
            match regex::Regex::new(pattern) {
                Ok(re) => CompiledPattern::Compiled(Arc::new(re)),
                Err(_) => CompiledPattern::Invalid,
            }
        }
        #[cfg(not(feature = "regex"))]
        {
            CompiledPattern::Safe
        }
    };

    if let Ok(mut cache) = pattern_cache().lock() {
        return cache.entry(pattern.to_string()).or_insert(compiled).clone();
    }
    compiled
}

fn validate_pattern(
    value: &OdinValue,
    path: &str,
    pattern: &str,
    errors: &mut Vec<ValidationError>,
) {
    let compiled = compile_pattern(pattern);

    // ReDoS safety check
    if let CompiledPattern::Unsafe(reason) = &compiled {
        errors.push(ValidationError::new(
            ValidationErrorCode::PatternMismatch,
            path,
            format!("Unsafe regex pattern: {reason}"),
        ));
        return;
    }

    if let Some(s) = value.as_str() {
        // Use regex crate if available, otherwise basic matching
        #[cfg(feature = "regex")]
        {
            match &compiled {
                CompiledPattern::Compiled(re) => {
                    if !re.is_match(s) {
                        errors.push(ValidationError::new(
                            ValidationErrorCode::PatternMismatch,
                            path,
                            format!("Value '{}' does not match pattern '{}'", s, pattern),
                        ));
                    }
                }
                CompiledPattern::Invalid => {
                    errors.push(ValidationError::new(
                        ValidationErrorCode::PatternMismatch,
                        path,
                        format!("Invalid regex pattern: '{}'", pattern),
                    ));
                }
                CompiledPattern::Unsafe(_) => {}
            }
        }
        #[cfg(not(feature = "regex"))]
        {
            // Without regex feature, skip pattern validation
            let _ = (s, pattern);
        }
    }
}

fn validate_enum(
    value: &OdinValue,
    path: &str,
    allowed: &[String],
    errors: &mut Vec<ValidationError>,
) {
    if let Some(s) = value.as_str() {
        if !allowed.iter().any(|a| a == s) {
            errors.push(ValidationError::new(
                ValidationErrorCode::InvalidEnumValue,
                path,
                format!(
                    "Value '{}' is not one of allowed values: [{}]",
                    s,
                    allowed.join(", ")
                ),
            ));
        }
    }
}

fn validate_format(
    value: &OdinValue,
    path: &str,
    format_name: &str,
    errors: &mut Vec<ValidationError>,
) {
    if let Some(s) = value.as_str() {
        if let Some(Err(msg)) = format_validators::validate_format(s, format_name) {
            errors.push(ValidationError::new(
                ValidationErrorCode::PatternMismatch,
                path,
                msg,
            ));
        }
    }
    // Date values are valid for date-iso format
    if matches!(value, OdinValue::Date { .. }) && format_name == "date-iso" {
        // Already a date — valid
    }
}

fn validate_size(
    value: &OdinValue,
    path: &str,
    min: &Option<u64>,
    max: &Option<u64>,
    errors: &mut Vec<ValidationError>,
) {
    if let OdinValue::Binary { data, .. } = value {
        let size = data.len() as u64;
        if let Some(min_val) = min {
            if size < *min_val {
                errors.push(ValidationError::new(
                    ValidationErrorCode::ValueOutOfBounds,
                    path,
                    format!("Binary size {size} is below minimum {min_val}"),
                ));
            }
        }
        if let Some(max_val) = max {
            if size > *max_val {
                errors.push(ValidationError::new(
                    ValidationErrorCode::ValueOutOfBounds,
                    path,
                    format!("Binary size {size} is above maximum {max_val}"),
                ));
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Array Validation
// ─────────────────────────────────────────────────────────────────────────────

fn validate_array(
    doc: &OdinDocument,
    path: &str,
    array_def: &crate::types::schema::SchemaArray,
    errors: &mut Vec<ValidationError>,
) {
    // Count items matching the pattern path[N]
    let prefix = format!("{path}[");
    let mut max_index: Option<usize> = None;

    for key in doc.assignments.keys() {
        if key.starts_with(&prefix) {
            if let Some(bracket_end) = key[prefix.len()..].find(']') {
                if let Ok(idx) = key[prefix.len()..prefix.len() + bracket_end].parse::<usize>() {
                    max_index = Some(max_index.map_or(idx, |m: usize| m.max(idx)));
                }
            }
        }
    }

    let count = max_index.map_or(0, |m| m + 1);

    // Min items (V006)
    if let Some(min) = array_def.min_items {
        if count < min {
            errors.push(ValidationError::new(
                ValidationErrorCode::ArrayLengthViolation,
                path,
                format!("Array has {count} items, minimum is {min}"),
            ));
        }
    }

    // Max items (V006)
    if let Some(max) = array_def.max_items {
        if count > max {
            errors.push(ValidationError::new(
                ValidationErrorCode::ArrayLengthViolation,
                path,
                format!("Array has {count} items, maximum is {max}"),
            ));
        }
    }

    // Unique check (V007) — compare serialized values
    if array_def.unique && count > 1 {
        let mut seen = std::collections::HashSet::new();
        for i in 0..count {
            let item_path = format!("{path}[{i}]");
            if let Some(val) = doc.get(&item_path) {
                let serialized = format!("{val:?}");
                if !seen.insert(serialized) {
                    errors.push(ValidationError::new(
                        ValidationErrorCode::UniqueConstraintViolation,
                        &item_path,
                        format!("Duplicate item at index {i}"),
                    ));
                    break;
                }
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Object-Level Constraints
// ─────────────────────────────────────────────────────────────────────────────

fn validate_object_constraint(
    doc: &OdinDocument,
    path: &str,
    constraint: &SchemaObjectConstraint,
    errors: &mut Vec<ValidationError>,
) {
    match constraint {
        SchemaObjectConstraint::Invariant(expr) => {
            validate_invariant(doc, path, expr, errors);
        }
        SchemaObjectConstraint::Cardinality { fields, min, max } => {
            validate_cardinality(doc, path, fields, min, max, errors);
        }
    }
}

fn validate_invariant(
    doc: &OdinDocument,
    path: &str,
    expr: &str,
    errors: &mut Vec<ValidationError>,
) {
    let expr = expr.trim();

    let resolve = |name: &str| -> Option<OdinValue> {
        let full = if path.is_empty() { name.to_string() } else { format!("{path}.{name}") };
        doc.get(&full).cloned()
    };

    let result = match invariant_evaluator::evaluate_invariant(expr, resolve) {
        Ok(r) => r,
        Err(()) => {
            errors.push(ValidationError::new(
                ValidationErrorCode::InvariantViolation,
                path,
                format!("Invalid invariant expression: {expr}"),
            ));
            return;
        }
    };

    // Absent operands: invariant does not apply.
    if result.value.is_none() && !result.null_operand {
        return;
    }

    if result.value == Some(false) {
        errors.push(ValidationError::new(
            ValidationErrorCode::InvariantViolation,
            path,
            format!("Invariant '{expr}' violated"),
        ));
    }
}

fn validate_cardinality(
    doc: &OdinDocument,
    path: &str,
    fields: &[String],
    min: &Option<usize>,
    max: &Option<usize>,
    errors: &mut Vec<ValidationError>,
) {
    let present_count = fields.iter().filter(|f| {
        let full_path = if path.is_empty() {
            (*f).clone()
        } else {
            format!("{path}.{f}")
        };
        doc.has(&full_path)
    }).count();

    if let Some(min_val) = min {
        if present_count < *min_val {
            errors.push(ValidationError::new(
                ValidationErrorCode::CardinalityConstraintViolation,
                path,
                format!(
                    "At least {} of [{}] must be present, found {}",
                    min_val,
                    fields.join(", "),
                    present_count
                ),
            ));
        }
    }

    if let Some(max_val) = max {
        if present_count > *max_val {
            errors.push(ValidationError::new(
                ValidationErrorCode::CardinalityConstraintViolation,
                path,
                format!(
                    "At most {} of [{}] may be present, found {}",
                    max_val,
                    fields.join(", "),
                    present_count
                ),
            ));
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Type Composition Expansion
// ─────────────────────────────────────────────────────────────────────────────

/// Expand type composition by merging parent type fields into derived types.
///
/// For `@Child : @Parent`, the child type inherits all fields from the parent
/// that it doesn't already define (child fields take precedence).
fn expand_type_composition(schema: &OdinSchemaDefinition) -> Cow<'_, OdinSchemaDefinition> {
    // Skip the schema clone when no types declare parents.
    if schema.types.values().all(|t| t.parents.is_empty()) {
        return Cow::Borrowed(schema);
    }

    let mut expanded = schema.clone();

    let type_names: Vec<String> = expanded.types.keys().cloned().collect();
    for type_name in &type_names {
        let parents = {
            let t = &expanded.types[type_name];
            if t.parents.is_empty() {
                continue;
            }
            t.parents.clone()
        };

        let mut inherited_fields = Vec::new();
        for parent_name in &parents {
            let clean_name = parent_name.strip_prefix('@').unwrap_or(parent_name);
            if let Some(parent_type) = schema.types.get(clean_name) {
                for field in &parent_type.fields {
                    inherited_fields.push(field.clone());
                }
            }
        }

        let Some(child_type) = expanded.types.get_mut(type_name) else { continue; };
        let existing_names: Vec<String> = child_type.fields.iter().map(|f| f.name.clone()).collect();
        for field in inherited_fields {
            if !existing_names.contains(&field.name) {
                child_type.fields.push(field);
            }
        }
    }

    Cow::Owned(expanded)
}

// ─────────────────────────────────────────────────────────────────────────────
// Conditional Evaluation
// ─────────────────────────────────────────────────────────────────────────────

fn matches_condition_value(
    value: Option<&OdinValue>,
    operator: &ConditionalOperator,
    expected: &ConditionalValue,
) -> bool {
    let Some(value) = value else { return false };

    match expected {
        ConditionalValue::Bool(expected_bool) => {
            if let Some(actual) = value.as_bool() {
                match operator {
                    ConditionalOperator::Eq => actual == *expected_bool,
                    ConditionalOperator::NotEq => actual != *expected_bool,
                    _ => false,
                }
            } else {
                false
            }
        }
        ConditionalValue::Number(expected_num) => {
            if let Some(actual) = value.as_f64() {
                match operator {
                    ConditionalOperator::Eq => (actual - expected_num).abs() < f64::EPSILON,
                    ConditionalOperator::NotEq => (actual - expected_num).abs() >= f64::EPSILON,
                    ConditionalOperator::Gt => actual > *expected_num,
                    ConditionalOperator::Lt => actual < *expected_num,
                    ConditionalOperator::Gte => actual >= *expected_num,
                    ConditionalOperator::Lte => actual <= *expected_num,
                }
            } else {
                false
            }
        }
        ConditionalValue::String(expected_str) => {
            if let Some(actual) = value.as_str() {
                match operator {
                    ConditionalOperator::Eq => actual == expected_str,
                    ConditionalOperator::NotEq => actual != expected_str,
                    ConditionalOperator::Gt => actual > expected_str.as_str(),
                    ConditionalOperator::Lt => actual < expected_str.as_str(),
                    ConditionalOperator::Gte => actual >= expected_str.as_str(),
                    ConditionalOperator::Lte => actual <= expected_str.as_str(),
                }
            } else {
                false
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Schema Type Reference Validation (V013)
// ─────────────────────────────────────────────────────────────────────────────

/// Whether an object at `path` is present — the path itself or any descendant
/// holds a value.
fn is_object_present(doc: &OdinDocument, path: &str) -> bool {
    if doc.has(path) {
        return true;
    }
    let prefix = format!("{path}.");
    doc.assignments.keys().any(|p| p.starts_with(&prefix))
}

/// Build the effective field set for validation: the schema's own fields plus
/// fields synthesized from section `_composition` intersections and field-level
/// `@TypeRef`s (which enforce the referenced type's fields under the sub-path).
/// Build the document-independent field set: section `_composition`
/// intersections plus the schema's explicit fields. Computed once per schema
/// (stored in the memo) and reused across documents.
fn base_field_compositions(
    schema: &OdinSchemaDefinition,
    registry: Option<&TypeRegistry>,
) -> Vec<(String, SchemaField)> {
    use std::collections::HashMap as Map;
    let mut result: Map<String, SchemaField> = Map::new();

    // Section `_composition` intersections: merge every member type's fields
    // under the parent path.
    for (path, field) in &schema.fields {
        if let Some(parent) = path.strip_suffix("._composition") {
            if let SchemaFieldType::TypeRef(name) = &field.field_type {
                for member in name.split('&').map(str::trim).filter(|m| !m.is_empty()) {
                    if let Some(type_def) = lookup_type(schema, registry, member) {
                        for tf in &type_def.fields {
                            if tf.name == "_composition" {
                                continue;
                            }
                            let full = format!("{parent}.{}", tf.name);
                            result.insert(full.clone(), rename_field(tf, &full));
                        }
                    }
                }
            }
        }
    }

    // The schema's explicit fields (override synthesized ones).
    for (path, field) in &schema.fields {
        if path.ends_with("._composition") {
            continue;
        }
        result.insert(path.clone(), field.clone());
    }

    result.into_iter().collect()
}

/// Apply field-level `@TypeRef`/`@Reference` augmentation to a base field set:
/// enforce the referenced type's fields under the field path when the sub-object
/// is present or the field is required. This is the only document-dependent part
/// of field-composition expansion, so it runs per call only when such fields exist.
fn augment_typeref_fields(
    base: &[(String, SchemaField)],
    doc: &OdinDocument,
    schema: &OdinSchemaDefinition,
    registry: Option<&TypeRegistry>,
) -> Vec<(String, SchemaField)> {
    use std::collections::HashMap as Map;
    let mut result: Map<String, SchemaField> =
        base.iter().map(|(p, f)| (p.clone(), f.clone())).collect();

    for (path, field) in base {
        let name = match &field.field_type {
            SchemaFieldType::TypeRef(n) => n.clone(),
            SchemaFieldType::Reference(n) => n.clone(),
            _ => continue,
        };
        let members: Vec<&str> = name.split('&').map(str::trim).filter(|m| !m.is_empty()).collect();
        let type_defs: Vec<&SchemaType> = members
            .iter()
            .filter_map(|m| lookup_type(schema, registry, m))
            .collect();
        if type_defs.is_empty() {
            continue; // runtime reference, not a defined type
        }
        if !is_object_present(doc, path) && !field.required {
            continue;
        }
        for type_def in type_defs {
            for tf in &type_def.fields {
                if tf.name == "_composition" {
                    continue;
                }
                let full = format!("{path}.{}", tf.name);
                result.entry(full.clone()).or_insert_with(|| rename_field(tf, &full));
            }
        }
    }

    result.into_iter().collect()
}

/// Clone a field, replacing its `name` with `name` (the synthesized full path's
/// leaf is kept as the original field name for messages; here we keep the leaf).
fn rename_field(field: &SchemaField, full_path: &str) -> SchemaField {
    let leaf = full_path.rsplit('.').next().unwrap_or(full_path);
    let mut f = field.clone();
    f.name = leaf.to_string();
    f
}

/// Look up a named type, checking the import registry first then local types.
fn lookup_type<'a>(
    schema: &'a OdinSchemaDefinition,
    registry: Option<&'a TypeRegistry>,
    name: &str,
) -> Option<&'a crate::types::schema::SchemaType> {
    if let Some(reg) = registry {
        if let Some(t) = reg.lookup(name) {
            return Some(t);
        }
    }
    schema.types.get(name)
}

/// Validate that top-level `@typeRef` fields resolve to a defined type (V013).
fn validate_schema_type_references(
    schema: &OdinSchemaDefinition,
    registry: Option<&TypeRegistry>,
    errors: &mut Vec<ValidationError>,
) {
    for (path, field) in &schema.fields {
        if let SchemaFieldType::TypeRef(name) = &field.field_type {
            // An intersection typeRef (`@a & @b`) carries `&`-joined member names.
            for member in name.split('&').map(str::trim).filter(|m| !m.is_empty()) {
                if lookup_type(schema, registry, member).is_none() {
                    errors.push(ValidationError::new(
                        ValidationErrorCode::UnresolvedReference,
                        path,
                        format!("Unresolved type reference: @{member}"),
                    ));
                }
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Reference Validation (V012/V013)
// ─────────────────────────────────────────────────────────────────────────────

const MAX_CIRCULAR_REF_DEPTH: usize = 100;

fn validate_references(
    doc: &OdinDocument,
    errors: &mut Vec<ValidationError>,
) {
    for (path, value) in doc.assignments.iter() {
        if let OdinValue::Reference { path: ref_path, .. } = value {
            // V013: Check target exists
            if doc.get(ref_path).is_none() {
                errors.push(ValidationError::new(
                    ValidationErrorCode::UnresolvedReference,
                    path,
                    format!("Unresolved reference: @{ref_path}"),
                ));
                continue;
            }

            // V012: Check for circular references
            let mut visited = std::collections::HashSet::new();
            visited.insert(path.clone());
            if detect_circular_ref(doc, ref_path, &mut visited, 0) {
                errors.push(ValidationError::new(
                    ValidationErrorCode::CircularReference,
                    path,
                    format!("Circular reference detected: @{ref_path}"),
                ));
            }
        }
    }
}

fn detect_circular_ref(
    doc: &OdinDocument,
    target_path: &str,
    visited: &mut std::collections::HashSet<String>,
    depth: usize,
) -> bool {
    if depth > MAX_CIRCULAR_REF_DEPTH {
        return true; // Treat deep chains as circular
    }
    if visited.contains(target_path) {
        return true;
    }
    visited.insert(target_path.to_string());

    if let Some(OdinValue::Reference { path: next_path, .. }) = doc.get(target_path) {
        return detect_circular_ref(doc, next_path, visited, depth + 1);
    }

    false
}

// ─────────────────────────────────────────────────────────────────────────────
// Strict Mode
// ─────────────────────────────────────────────────────────────────────────────

fn validate_strict(
    doc: &OdinDocument,
    schema: &OdinSchemaDefinition,
    errors: &mut Vec<ValidationError>,
) {
    // Pre-compute prefixes/suffixes once instead of allocating inside the
    // per-path loop.
    let array_prefixes: Vec<String> = schema.arrays.keys()
        .map(|p| format!("{p}["))
        .collect();
    let type_field_suffixes: Vec<String> = schema.types.values()
        .flat_map(|t| t.fields.iter().map(|f| format!(".{}", f.name)))
        .collect();

    for path in doc.assignments.keys() {
        if schema.fields.contains_key(path) {
            continue;
        }
        if array_prefixes.iter().any(|p| path.starts_with(p)) {
            continue;
        }
        if type_field_suffixes.iter().any(|s| path.ends_with(s)) {
            continue;
        }

        errors.push(ValidationError::new(
            ValidationErrorCode::UnknownField,
            path,
            format!("Unknown field '{path}' not defined in schema"),
        ));
    }
}

/// A single format constraint parsed from a schema line.
#[derive(Debug, Clone)]
struct FormatConstraint {
    /// The field name (e.g., "email", "ssn").
    field: String,
    /// The format name (e.g., "email", "ssn", "date-iso").
    format: String,
}

/// Parse a simple format schema text into a list of format constraints.
///
/// Each line must match the pattern: `fieldName = :format formatName`
/// Lines that do not match this pattern are silently skipped.
fn parse_format_schema(schema_text: &str) -> Vec<FormatConstraint> {
    let mut constraints = Vec::new();

    for line in schema_text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with(';') {
            continue;
        }

        // Look for ":format" in the line
        if let Some(format_pos) = line.find(":format ") {
            // Extract field name: everything before '=' trimmed
            if let Some(eq_pos) = line.find('=') {
                let field = line[..eq_pos].trim().to_string();
                // Extract format name: everything after ":format " trimmed
                let format = line[format_pos + 8..].trim().to_string();

                if !field.is_empty() && !format.is_empty() {
                    constraints.push(FormatConstraint { field, format });
                }
            }
        }
    }

    constraints
}

/// Validate a document against a simple format schema.
///
/// Schema format: `fieldName = :format formatName` per line.
///
/// For each constraint, the function looks up the field in the document's
/// assignments and validates the value against the named format:
/// - If the value is an `OdinValue::String`, the string content is validated
///   using `format_validators::validate_format`.
/// - If the value is an `OdinValue::Date` and the format is `"date-iso"`,
///   the value is considered valid (already parsed as a date).
/// - Missing fields are silently skipped (format validation does not
///   enforce presence).
///
/// Returns a `Vec` of `(field_path, error_message)` tuples for any
/// validation failures. An empty vec means all checks passed.
pub fn validate_formats(doc: &OdinDocument, schema_text: &str) -> Vec<(String, String)> {
    let constraints = parse_format_schema(schema_text);
    let mut errors = Vec::new();

    for constraint in &constraints {
        let value = doc.assignments.get(&constraint.field);

        match value {
            Some(OdinValue::String { value: s, .. }) => {
                let result = format_validators::validate_format(s, &constraint.format);
                if let Some(Err(msg)) = result {
                    errors.push((constraint.field.clone(), msg));
                }
            }
            Some(OdinValue::Date { .. }) if constraint.format == "date-iso" => {
                // Date values are inherently valid for date-iso format
            }
            _ => {
                // Field not present, or other value types with a format
                // constraint — skip silently to match golden test expectations.
            }
        }
    }

    errors
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use crate::types::values::OdinValues;
    use crate::types::document::OdinDocumentBuilder;
    use crate::types::schema::SchemaMetadata;

    #[test]
    fn test_parse_format_schema_single_line() {
        let schema = "email = :format email";
        let constraints = parse_format_schema(schema);
        assert_eq!(constraints.len(), 1);
        assert_eq!(constraints[0].field, "email");
        assert_eq!(constraints[0].format, "email");
    }

    #[test]
    fn test_parse_format_schema_multiple_lines() {
        let schema = "email = :format email\nssn = :format ssn\nvin = :format vin";
        let constraints = parse_format_schema(schema);
        assert_eq!(constraints.len(), 3);
        assert_eq!(constraints[0].format, "email");
        assert_eq!(constraints[1].format, "ssn");
        assert_eq!(constraints[2].format, "vin");
    }

    #[test]
    fn test_parse_format_schema_skips_blanks_and_comments() {
        let schema = "; comment\n\nemail = :format email\n";
        let constraints = parse_format_schema(schema);
        assert_eq!(constraints.len(), 1);
    }

    #[test]
    fn test_validate_formats_valid_email() {
        let doc = OdinDocumentBuilder::new()
            .set("email", OdinValues::string("user@example.com"))
            .build()
            .unwrap();

        let errors = validate_formats(&doc, "email = :format email");
        assert!(errors.is_empty(), "Expected no errors, got: {:?}", errors);
    }

    #[test]
    fn test_validate_formats_invalid_email() {
        let doc = OdinDocumentBuilder::new()
            .set("email", OdinValues::string("userexample.com"))
            .build()
            .unwrap();

        let errors = validate_formats(&doc, "email = :format email");
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].0, "email");
        assert!(errors[0].1.contains("email"), "Error message: {}", errors[0].1);
    }

    #[test]
    fn test_validate_formats_date_iso_with_date_value() {
        let doc = OdinDocumentBuilder::new()
            .set("date", OdinValue::Date {
                year: 2024,
                month: 6,
                day: 15,
                raw: "2024-06-15".to_string(),
                modifiers: None,
                directives: vec![],
            })
            .build()
            .unwrap();

        let errors = validate_formats(&doc, "date = :format date-iso");
        assert!(errors.is_empty(), "Expected no errors, got: {:?}", errors);
    }

    #[test]
    fn test_validate_formats_missing_field_is_ok() {
        let doc = OdinDocumentBuilder::new()
            .set("other", OdinValues::string("value"))
            .build()
            .unwrap();

        let errors = validate_formats(&doc, "email = :format email");
        assert!(errors.is_empty());
    }

    // ── Reference Validation Tests ───────────────────────────────────────

    fn make_empty_schema() -> OdinSchemaDefinition {
        OdinSchemaDefinition {
            metadata: SchemaMetadata::default(),
            imports: vec![],
            types: HashMap::new(),
            fields: HashMap::new(),
            arrays: HashMap::new(),
            constraints: HashMap::new(),
            validation_memo: Default::default(),
        }
    }

    #[test]
    fn test_v013_unresolved_reference() {
        let doc = OdinDocumentBuilder::new()
            .set("ref_field", OdinValue::Reference {
                path: "nonexistent".to_string(),
                modifiers: None,
                directives: vec![],
            })
            .build()
            .unwrap();

        let schema = make_empty_schema();
        let result = validate(&doc, &schema, None);
        assert!(!result.valid);
        assert!(result.errors.iter().any(|e|
            e.error_code == ValidationErrorCode::UnresolvedReference
        ));
    }

    #[test]
    fn test_v012_circular_reference() {
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

        let schema = make_empty_schema();
        let result = validate(&doc, &schema, None);
        assert!(!result.valid);
        assert!(result.errors.iter().any(|e|
            e.error_code == ValidationErrorCode::CircularReference
        ));
    }

    #[test]
    fn test_valid_reference() {
        let doc = OdinDocumentBuilder::new()
            .set("target", OdinValues::string("hello"))
            .set("ref_field", OdinValue::Reference {
                path: "target".to_string(),
                modifiers: None,
                directives: vec![],
            })
            .build()
            .unwrap();

        let schema = make_empty_schema();
        let result = validate(&doc, &schema, None);
        assert!(result.valid);
    }

    // ── Fail-Fast Tests ──────────────────────────────────────────────────

    #[test]
    fn test_fail_fast_stops_at_first_error() {
        use crate::types::schema::SchemaFieldType;

        let doc = OdinDocumentBuilder::new()
            .build()
            .unwrap();

        let mut fields = HashMap::new();
        fields.insert("a".to_string(), SchemaField {
            name: "a".to_string(),
            field_type: SchemaFieldType::String,
            required: true,
            confidential: false,
            deprecated: false,
            immutable: false,
            computed: false,
            description: None,
            constraints: vec![],
            default_value: None,
            conditionals: vec![],
        });
        fields.insert("b".to_string(), SchemaField {
            name: "b".to_string(),
            field_type: SchemaFieldType::String,
            required: true,
            confidential: false,
            deprecated: false,
            immutable: false,
            computed: false,
            description: None,
            constraints: vec![],
            default_value: None,
            conditionals: vec![],
        });

        let mut schema = make_empty_schema();
        schema.fields = fields;

        let opts = ValidateOptions {
            fail_fast: true,
            ..Default::default()
        };
        let result = validate(&doc, &schema, Some(&opts));
        assert!(!result.valid);
        assert_eq!(result.errors.len(), 1, "fail_fast should stop at first error");
    }

    // ── Type Composition Tests ───────────────────────────────────────────

    #[test]
    fn test_type_composition_inherits_parent_fields() {
        use crate::types::schema::{SchemaType, SchemaFieldType};

        let mut schema = make_empty_schema();

        schema.types.insert("Parent".to_string(), SchemaType {
            name: "Parent".to_string(),
            description: None,
            fields: vec![
                SchemaField {
                    name: "inherited_field".to_string(),
                    field_type: SchemaFieldType::String,
                    required: true,
                    confidential: false,
                    deprecated: false,
                    immutable: false,
                    computed: false,
                    description: None,
                    constraints: vec![],
                    default_value: None,
                    conditionals: vec![],
                },
            ],
            parents: vec![],
            override_bases: vec![],
        });

        schema.types.insert("Child".to_string(), SchemaType {
            name: "Child".to_string(),
            description: None,
            fields: vec![
                SchemaField {
                    name: "own_field".to_string(),
                    field_type: SchemaFieldType::Integer,
                    required: false,
                    confidential: false,
                    deprecated: false,
                    immutable: false,
                    computed: false,
                    description: None,
                    constraints: vec![],
                    default_value: None,
                    conditionals: vec![],
                },
            ],
            parents: vec!["@Parent".to_string()],
            override_bases: vec![],
        });

        let expanded = expand_type_composition(&schema);
        let child = expanded.types.get("Child").unwrap();
        assert_eq!(child.fields.len(), 2, "Child should have own + inherited fields");
        assert!(child.fields.iter().any(|f| f.name == "inherited_field"));
        assert!(child.fields.iter().any(|f| f.name == "own_field"));
    }

    // ── Helper to create a schema field ─────────────────────────────────

    fn make_field(name: &str, field_type: SchemaFieldType, required: bool) -> SchemaField {
        SchemaField {
            name: name.to_string(),
            field_type,
            required,
            confidential: false,
            deprecated: false,
            immutable: false,
            computed: false,
            description: None,
            constraints: vec![],
            default_value: None,
            conditionals: vec![],
        }
    }

    fn make_field_with_constraints(name: &str, field_type: SchemaFieldType, required: bool, constraints: Vec<SchemaConstraint>) -> SchemaField {
        SchemaField {
            name: name.to_string(),
            field_type,
            required,
            confidential: false,
            deprecated: false,
            immutable: false,
            computed: false,
            description: None,
            constraints,
            default_value: None,
            conditionals: vec![],
        }
    }

    // ── V001: Required field missing ────────────────────────────────────

    #[test]
    fn test_v001_required_string_missing() {
        let doc = OdinDocumentBuilder::new().build().unwrap();
        let mut schema = make_empty_schema();
        schema.fields.insert("name".to_string(), make_field("name", SchemaFieldType::String, true));
        let result = validate(&doc, &schema, None);
        assert!(!result.valid);
        assert!(result.errors.iter().any(|e| e.error_code == ValidationErrorCode::RequiredFieldMissing));
    }

    #[test]
    fn test_v001_required_field_present() {
        let doc = OdinDocumentBuilder::new()
            .set("name", OdinValues::string("Alice"))
            .build().unwrap();
        let mut schema = make_empty_schema();
        schema.fields.insert("name".to_string(), make_field("name", SchemaFieldType::String, true));
        let result = validate(&doc, &schema, None);
        assert!(result.valid);
    }

    #[test]
    fn test_v001_optional_field_missing_ok() {
        let doc = OdinDocumentBuilder::new().build().unwrap();
        let mut schema = make_empty_schema();
        schema.fields.insert("name".to_string(), make_field("name", SchemaFieldType::String, false));
        let result = validate(&doc, &schema, None);
        assert!(result.valid);
    }

    #[test]
    fn test_v001_required_null_is_type_mismatch() {
        let doc = OdinDocumentBuilder::new()
            .set("name", OdinValues::null())
            .build().unwrap();
        let mut schema = make_empty_schema();
        schema.fields.insert("name".to_string(), make_field("name", SchemaFieldType::String, true));
        let result = validate(&doc, &schema, None);
        assert!(!result.valid);
        assert!(result.errors.iter().any(|e| e.error_code == ValidationErrorCode::TypeMismatch));
    }

    // ── V002: Type mismatch ─────────────────────────────────────────────

    #[test]
    fn test_v002_string_where_integer_expected() {
        let doc = OdinDocumentBuilder::new()
            .set("count", OdinValues::string("not a number"))
            .build().unwrap();
        let mut schema = make_empty_schema();
        schema.fields.insert("count".to_string(), make_field("count", SchemaFieldType::Integer, false));
        let result = validate(&doc, &schema, None);
        assert!(!result.valid);
        assert!(result.errors.iter().any(|e| e.error_code == ValidationErrorCode::TypeMismatch));
    }

    #[test]
    fn test_v002_integer_where_string_expected() {
        let doc = OdinDocumentBuilder::new()
            .set("name", OdinValues::integer(42))
            .build().unwrap();
        let mut schema = make_empty_schema();
        schema.fields.insert("name".to_string(), make_field("name", SchemaFieldType::String, false));
        let result = validate(&doc, &schema, None);
        assert!(!result.valid);
        assert!(result.errors.iter().any(|e| e.error_code == ValidationErrorCode::TypeMismatch));
    }

    #[test]
    fn test_v002_string_where_boolean_expected() {
        let doc = OdinDocumentBuilder::new()
            .set("flag", OdinValues::string("yes"))
            .build().unwrap();
        let mut schema = make_empty_schema();
        schema.fields.insert("flag".to_string(), make_field("flag", SchemaFieldType::Boolean, false));
        let result = validate(&doc, &schema, None);
        assert!(!result.valid);
    }

    #[test]
    fn test_v002_boolean_matches_boolean() {
        let doc = OdinDocumentBuilder::new()
            .set("flag", OdinValues::boolean(true))
            .build().unwrap();
        let mut schema = make_empty_schema();
        schema.fields.insert("flag".to_string(), make_field("flag", SchemaFieldType::Boolean, false));
        let result = validate(&doc, &schema, None);
        assert!(result.valid);
    }

    #[test]
    fn test_v002_integer_matches_number() {
        // Integer should be accepted where Number is expected
        let doc = OdinDocumentBuilder::new()
            .set("val", OdinValues::integer(42))
            .build().unwrap();
        let mut schema = make_empty_schema();
        schema.fields.insert("val".to_string(), make_field("val", SchemaFieldType::Number { decimal_places: None }, false));
        let result = validate(&doc, &schema, None);
        assert!(result.valid);
    }

    #[test]
    fn test_v002_number_matches_number() {
        let doc = OdinDocumentBuilder::new()
            .set("val", OdinValues::number(3.14))
            .build().unwrap();
        let mut schema = make_empty_schema();
        schema.fields.insert("val".to_string(), make_field("val", SchemaFieldType::Number { decimal_places: None }, false));
        let result = validate(&doc, &schema, None);
        assert!(result.valid);
    }

    #[test]
    fn test_v002_currency_matches_currency() {
        let doc = OdinDocumentBuilder::new()
            .set("price", OdinValues::currency(99.99, 2))
            .build().unwrap();
        let mut schema = make_empty_schema();
        schema.fields.insert("price".to_string(), make_field("price", SchemaFieldType::Currency { decimal_places: None }, false));
        let result = validate(&doc, &schema, None);
        assert!(result.valid);
    }

    #[test]
    fn test_v002_null_passes_type_check_for_optional() {
        // Null is allowed for any optional field
        let doc = OdinDocumentBuilder::new()
            .set("val", OdinValues::null())
            .build().unwrap();
        let mut schema = make_empty_schema();
        schema.fields.insert("val".to_string(), make_field("val", SchemaFieldType::Integer, false));
        let result = validate(&doc, &schema, None);
        assert!(result.valid);
    }

    // ── V003/V004: String too short / too long ──────────────────────────

    #[test]
    fn test_v003_string_too_short() {
        let doc = OdinDocumentBuilder::new()
            .set("name", OdinValues::string(""))
            .build().unwrap();
        let mut schema = make_empty_schema();
        schema.fields.insert("name".to_string(), make_field_with_constraints(
            "name", SchemaFieldType::String, false,
            vec![SchemaConstraint::Bounds {
                min: Some("1".to_string()),
                max: Some("100".to_string()),
                min_exclusive: false,
                max_exclusive: false,
            }],
        ));
        let result = validate(&doc, &schema, None);
        assert!(!result.valid);
        assert!(result.errors.iter().any(|e| e.error_code == ValidationErrorCode::ValueOutOfBounds));
    }

    #[test]
    fn test_v004_string_too_long() {
        let doc = OdinDocumentBuilder::new()
            .set("code", OdinValues::string("ABCDE"))
            .build().unwrap();
        let mut schema = make_empty_schema();
        schema.fields.insert("code".to_string(), make_field_with_constraints(
            "code", SchemaFieldType::String, false,
            vec![SchemaConstraint::Bounds {
                min: Some("2".to_string()),
                max: Some("2".to_string()),
                min_exclusive: false,
                max_exclusive: false,
            }],
        ));
        let result = validate(&doc, &schema, None);
        assert!(!result.valid);
        assert!(result.errors.iter().any(|e| e.error_code == ValidationErrorCode::ValueOutOfBounds));
    }

    #[test]
    fn test_string_within_bounds() {
        let doc = OdinDocumentBuilder::new()
            .set("code", OdinValues::string("AB"))
            .build().unwrap();
        let mut schema = make_empty_schema();
        schema.fields.insert("code".to_string(), make_field_with_constraints(
            "code", SchemaFieldType::String, false,
            vec![SchemaConstraint::Bounds {
                min: Some("2".to_string()),
                max: Some("2".to_string()),
                min_exclusive: false,
                max_exclusive: false,
            }],
        ));
        let result = validate(&doc, &schema, None);
        assert!(result.valid);
    }

    // ── V005/V006: Number below min / above max ─────────────────────────

    #[test]
    fn test_v005_number_below_min() {
        let doc = OdinDocumentBuilder::new()
            .set("age", OdinValues::integer(-1))
            .build().unwrap();
        let mut schema = make_empty_schema();
        schema.fields.insert("age".to_string(), make_field_with_constraints(
            "age", SchemaFieldType::Integer, false,
            vec![SchemaConstraint::Bounds {
                min: Some("0".to_string()),
                max: Some("150".to_string()),
                min_exclusive: false,
                max_exclusive: false,
            }],
        ));
        let result = validate(&doc, &schema, None);
        assert!(!result.valid);
        assert!(result.errors.iter().any(|e| e.error_code == ValidationErrorCode::ValueOutOfBounds));
    }

    #[test]
    fn test_v006_number_above_max() {
        let doc = OdinDocumentBuilder::new()
            .set("age", OdinValues::integer(200))
            .build().unwrap();
        let mut schema = make_empty_schema();
        schema.fields.insert("age".to_string(), make_field_with_constraints(
            "age", SchemaFieldType::Integer, false,
            vec![SchemaConstraint::Bounds {
                min: Some("0".to_string()),
                max: Some("150".to_string()),
                min_exclusive: false,
                max_exclusive: false,
            }],
        ));
        let result = validate(&doc, &schema, None);
        assert!(!result.valid);
        assert!(result.errors.iter().any(|e| e.error_code == ValidationErrorCode::ValueOutOfBounds));
    }

    #[test]
    fn test_number_at_exact_min() {
        let doc = OdinDocumentBuilder::new()
            .set("age", OdinValues::integer(0))
            .build().unwrap();
        let mut schema = make_empty_schema();
        schema.fields.insert("age".to_string(), make_field_with_constraints(
            "age", SchemaFieldType::Integer, false,
            vec![SchemaConstraint::Bounds {
                min: Some("0".to_string()),
                max: Some("150".to_string()),
                min_exclusive: false,
                max_exclusive: false,
            }],
        ));
        let result = validate(&doc, &schema, None);
        assert!(result.valid);
    }

    #[test]
    fn test_number_at_exact_max() {
        let doc = OdinDocumentBuilder::new()
            .set("age", OdinValues::integer(150))
            .build().unwrap();
        let mut schema = make_empty_schema();
        schema.fields.insert("age".to_string(), make_field_with_constraints(
            "age", SchemaFieldType::Integer, false,
            vec![SchemaConstraint::Bounds {
                min: Some("0".to_string()),
                max: Some("150".to_string()),
                min_exclusive: false,
                max_exclusive: false,
            }],
        ));
        let result = validate(&doc, &schema, None);
        assert!(result.valid);
    }

    #[test]
    fn test_exclusive_min_at_boundary() {
        let doc = OdinDocumentBuilder::new()
            .set("val", OdinValues::number(0.0))
            .build().unwrap();
        let mut schema = make_empty_schema();
        schema.fields.insert("val".to_string(), make_field_with_constraints(
            "val", SchemaFieldType::Number { decimal_places: None }, false,
            vec![SchemaConstraint::Bounds {
                min: Some("0".to_string()),
                max: None,
                min_exclusive: true,
                max_exclusive: false,
            }],
        ));
        let result = validate(&doc, &schema, None);
        assert!(!result.valid);
    }

    #[test]
    fn test_exclusive_max_at_boundary() {
        let doc = OdinDocumentBuilder::new()
            .set("val", OdinValues::number(100.0))
            .build().unwrap();
        let mut schema = make_empty_schema();
        schema.fields.insert("val".to_string(), make_field_with_constraints(
            "val", SchemaFieldType::Number { decimal_places: None }, false,
            vec![SchemaConstraint::Bounds {
                min: None,
                max: Some("100".to_string()),
                min_exclusive: false,
                max_exclusive: true,
            }],
        ));
        let result = validate(&doc, &schema, None);
        assert!(!result.valid);
    }

    // ── V008: Enum value not in list ────────────────────────────────────

    #[test]
    fn test_v008_enum_invalid_value() {
        let doc = OdinDocumentBuilder::new()
            .set("color", OdinValues::string("purple"))
            .build().unwrap();
        let mut schema = make_empty_schema();
        schema.fields.insert("color".to_string(), make_field_with_constraints(
            "color", SchemaFieldType::String, false,
            vec![SchemaConstraint::Enum(vec!["red".to_string(), "green".to_string(), "blue".to_string()])],
        ));
        let result = validate(&doc, &schema, None);
        assert!(!result.valid);
        assert!(result.errors.iter().any(|e| e.error_code == ValidationErrorCode::InvalidEnumValue));
    }

    #[test]
    fn test_v008_enum_valid_value() {
        let doc = OdinDocumentBuilder::new()
            .set("color", OdinValues::string("red"))
            .build().unwrap();
        let mut schema = make_empty_schema();
        schema.fields.insert("color".to_string(), make_field_with_constraints(
            "color", SchemaFieldType::String, false,
            vec![SchemaConstraint::Enum(vec!["red".to_string(), "green".to_string(), "blue".to_string()])],
        ));
        let result = validate(&doc, &schema, None);
        assert!(result.valid);
    }

    // ── V011: Format validation ─────────────────────────────────────────

    #[test]
    fn test_v011_format_email_invalid() {
        let doc = OdinDocumentBuilder::new()
            .set("email", OdinValues::string("notanemail"))
            .build().unwrap();
        let mut schema = make_empty_schema();
        schema.fields.insert("email".to_string(), make_field_with_constraints(
            "email", SchemaFieldType::String, false,
            vec![SchemaConstraint::Format("email".to_string())],
        ));
        let result = validate(&doc, &schema, None);
        assert!(!result.valid);
    }

    #[test]
    fn test_v011_format_email_valid() {
        let doc = OdinDocumentBuilder::new()
            .set("email", OdinValues::string("user@example.com"))
            .build().unwrap();
        let mut schema = make_empty_schema();
        schema.fields.insert("email".to_string(), make_field_with_constraints(
            "email", SchemaFieldType::String, false,
            vec![SchemaConstraint::Format("email".to_string())],
        ));
        let result = validate(&doc, &schema, None);
        assert!(result.valid);
    }

    // ── Multiple errors ─────────────────────────────────────────────────

    #[test]
    fn test_multiple_errors_collected() {
        let doc = OdinDocumentBuilder::new().build().unwrap();
        let mut schema = make_empty_schema();
        schema.fields.insert("a".to_string(), make_field("a", SchemaFieldType::String, true));
        schema.fields.insert("b".to_string(), make_field("b", SchemaFieldType::String, true));
        schema.fields.insert("c".to_string(), make_field("c", SchemaFieldType::Integer, true));
        let result = validate(&doc, &schema, None);
        assert!(!result.valid);
        assert_eq!(result.errors.len(), 3);
    }

    // ── Nested section validation ───────────────────────────────────────

    #[test]
    fn test_nested_field_required() {
        let doc = OdinDocumentBuilder::new()
            .set("person.name", OdinValues::string("Alice"))
            .build().unwrap();
        let mut schema = make_empty_schema();
        schema.fields.insert("person.name".to_string(), make_field("name", SchemaFieldType::String, true));
        schema.fields.insert("person.email".to_string(), make_field("email", SchemaFieldType::String, true));
        let result = validate(&doc, &schema, None);
        assert!(!result.valid);
        assert!(result.errors.iter().any(|e| e.path == "person.email"));
    }

    #[test]
    fn test_nested_field_all_present() {
        let doc = OdinDocumentBuilder::new()
            .set("person.name", OdinValues::string("Alice"))
            .set("person.email", OdinValues::string("alice@example.com"))
            .build().unwrap();
        let mut schema = make_empty_schema();
        schema.fields.insert("person.name".to_string(), make_field("name", SchemaFieldType::String, true));
        schema.fields.insert("person.email".to_string(), make_field("email", SchemaFieldType::String, true));
        let result = validate(&doc, &schema, None);
        assert!(result.valid);
    }

    // ── Strict mode ─────────────────────────────────────────────────────

    #[test]
    fn test_strict_mode_rejects_unknown_fields() {
        let doc = OdinDocumentBuilder::new()
            .set("name", OdinValues::string("Alice"))
            .set("unknown_field", OdinValues::string("oops"))
            .build().unwrap();
        let mut schema = make_empty_schema();
        schema.fields.insert("name".to_string(), make_field("name", SchemaFieldType::String, false));
        let opts = ValidateOptions { strict: true, ..Default::default() };
        let result = validate(&doc, &schema, Some(&opts));
        assert!(!result.valid);
        assert!(result.errors.iter().any(|e| e.error_code == ValidationErrorCode::UnknownField));
    }

    #[test]
    fn test_strict_mode_no_unknown_fields() {
        let doc = OdinDocumentBuilder::new()
            .set("name", OdinValues::string("Alice"))
            .build().unwrap();
        let mut schema = make_empty_schema();
        schema.fields.insert("name".to_string(), make_field("name", SchemaFieldType::String, false));
        let opts = ValidateOptions { strict: true, ..Default::default() };
        let result = validate(&doc, &schema, Some(&opts));
        assert!(result.valid);
    }

    // ── Empty schema validates anything ─────────────────────────────────

    #[test]
    fn test_empty_schema_validates_any_doc() {
        let doc = OdinDocumentBuilder::new()
            .set("anything", OdinValues::string("goes"))
            .set("number", OdinValues::integer(42))
            .build().unwrap();
        let schema = make_empty_schema();
        let result = validate(&doc, &schema, None);
        assert!(result.valid);
    }

    #[test]
    fn test_empty_doc_empty_schema() {
        let doc = OdinDocumentBuilder::new().build().unwrap();
        let schema = make_empty_schema();
        let result = validate(&doc, &schema, None);
        assert!(result.valid);
    }

    // ── Complex document with multiple sections ─────────────────────────

    #[test]
    fn test_complex_multi_section_validation() {
        let doc = OdinDocumentBuilder::new()
            .set("person.name", OdinValues::string("Alice"))
            .set("person.age", OdinValues::integer(30))
            .set("address.street", OdinValues::string("123 Main St"))
            .set("address.city", OdinValues::string("Springfield"))
            .set("address.state", OdinValues::string("IL"))
            .build().unwrap();

        let mut schema = make_empty_schema();
        schema.fields.insert("person.name".to_string(), make_field("name", SchemaFieldType::String, true));
        schema.fields.insert("person.age".to_string(), make_field_with_constraints(
            "age", SchemaFieldType::Integer, true,
            vec![SchemaConstraint::Bounds {
                min: Some("0".to_string()),
                max: Some("150".to_string()),
                min_exclusive: false,
                max_exclusive: false,
            }],
        ));
        schema.fields.insert("address.street".to_string(), make_field("street", SchemaFieldType::String, true));
        schema.fields.insert("address.city".to_string(), make_field("city", SchemaFieldType::String, true));
        schema.fields.insert("address.state".to_string(), make_field_with_constraints(
            "state", SchemaFieldType::String, true,
            vec![SchemaConstraint::Bounds {
                min: Some("2".to_string()),
                max: Some("2".to_string()),
                min_exclusive: false,
                max_exclusive: false,
            }],
        ));
        let result = validate(&doc, &schema, None);
        assert!(result.valid);
    }

    #[test]
    fn test_complex_validation_with_errors() {
        let doc = OdinDocumentBuilder::new()
            .set("person.name", OdinValues::string("A"))
            .set("person.age", OdinValues::integer(200))
            .build().unwrap();

        let mut schema = make_empty_schema();
        schema.fields.insert("person.name".to_string(), make_field_with_constraints(
            "name", SchemaFieldType::String, true,
            vec![SchemaConstraint::Bounds {
                min: Some("2".to_string()),
                max: Some("50".to_string()),
                min_exclusive: false,
                max_exclusive: false,
            }],
        ));
        schema.fields.insert("person.age".to_string(), make_field_with_constraints(
            "age", SchemaFieldType::Integer, true,
            vec![SchemaConstraint::Bounds {
                min: Some("0".to_string()),
                max: Some("150".to_string()),
                min_exclusive: false,
                max_exclusive: false,
            }],
        ));
        let result = validate(&doc, &schema, None);
        assert!(!result.valid);
        assert_eq!(result.errors.len(), 2);
    }

    // ── Valid reference chain ────────────────────────────────────────────

    #[test]
    fn test_reference_chain_valid() {
        let doc = OdinDocumentBuilder::new()
            .set("a", OdinValues::string("hello"))
            .set("b", OdinValue::Reference {
                path: "a".to_string(),
                modifiers: None,
                directives: vec![],
            })
            .set("c", OdinValue::Reference {
                path: "b".to_string(),
                modifiers: None,
                directives: vec![],
            })
            .build().unwrap();
        let schema = make_empty_schema();
        let result = validate(&doc, &schema, None);
        assert!(result.valid);
    }

    // ── Cardinality constraint tests ────────────────────────────────────

    #[test]
    fn test_cardinality_one_of_met() {
        let doc = OdinDocumentBuilder::new()
            .set("contact.email", OdinValues::string("a@b.com"))
            .build().unwrap();
        let mut schema = make_empty_schema();
        schema.constraints.insert("contact".to_string(), vec![
            SchemaObjectConstraint::Cardinality {
                fields: vec!["email".to_string(), "phone".to_string()],
                min: Some(1),
                max: None,
            },
        ]);
        let result = validate(&doc, &schema, None);
        assert!(result.valid);
    }

    #[test]
    fn test_cardinality_one_of_not_met() {
        let doc = OdinDocumentBuilder::new().build().unwrap();
        let mut schema = make_empty_schema();
        schema.constraints.insert("contact".to_string(), vec![
            SchemaObjectConstraint::Cardinality {
                fields: vec!["email".to_string(), "phone".to_string()],
                min: Some(1),
                max: None,
            },
        ]);
        let result = validate(&doc, &schema, None);
        assert!(!result.valid);
        assert!(result.errors.iter().any(|e| e.error_code == ValidationErrorCode::CardinalityConstraintViolation));
    }

    #[test]
    fn test_cardinality_at_most_one_violated() {
        let doc = OdinDocumentBuilder::new()
            .set("contact.email", OdinValues::string("a@b.com"))
            .set("contact.phone", OdinValues::string("555-1234"))
            .build().unwrap();
        let mut schema = make_empty_schema();
        schema.constraints.insert("contact".to_string(), vec![
            SchemaObjectConstraint::Cardinality {
                fields: vec!["email".to_string(), "phone".to_string()],
                min: None,
                max: Some(1),
            },
        ]);
        let result = validate(&doc, &schema, None);
        assert!(!result.valid);
        assert!(result.errors.iter().any(|e| e.error_code == ValidationErrorCode::CardinalityConstraintViolation));
    }

    #[test]
    fn test_cardinality_exactly_one_met() {
        let doc = OdinDocumentBuilder::new()
            .set("contact.email", OdinValues::string("a@b.com"))
            .build().unwrap();
        let mut schema = make_empty_schema();
        schema.constraints.insert("contact".to_string(), vec![
            SchemaObjectConstraint::Cardinality {
                fields: vec!["email".to_string(), "phone".to_string()],
                min: Some(1),
                max: Some(1),
            },
        ]);
        let result = validate(&doc, &schema, None);
        assert!(result.valid);
    }

    // ── Invariant constraint tests ──────────────────────────────────────

    #[test]
    fn test_invariant_passes() {
        let doc = OdinDocumentBuilder::new()
            .set("data.age", OdinValues::integer(25))
            .build().unwrap();
        let mut schema = make_empty_schema();
        schema.constraints.insert("data".to_string(), vec![
            SchemaObjectConstraint::Invariant("age >= 0".to_string()),
        ]);
        let result = validate(&doc, &schema, None);
        assert!(result.valid);
    }

    #[test]
    fn test_invariant_fails() {
        let doc = OdinDocumentBuilder::new()
            .set("data.age", OdinValues::integer(-5))
            .build().unwrap();
        let mut schema = make_empty_schema();
        schema.constraints.insert("data".to_string(), vec![
            SchemaObjectConstraint::Invariant("age >= 0".to_string()),
        ]);
        let result = validate(&doc, &schema, None);
        assert!(!result.valid);
        assert!(result.errors.iter().any(|e| e.error_code == ValidationErrorCode::InvariantViolation));
    }

    // ── Fail-fast with multiple field types ─────────────────────────────

    #[test]
    fn test_fail_fast_with_type_mismatch_and_missing() {
        let doc = OdinDocumentBuilder::new()
            .set("name", OdinValues::integer(42))
            .build().unwrap();
        let mut schema = make_empty_schema();
        schema.fields.insert("name".to_string(), make_field("name", SchemaFieldType::String, false));
        schema.fields.insert("email".to_string(), make_field("email", SchemaFieldType::String, true));
        let opts = ValidateOptions { fail_fast: true, ..Default::default() };
        let result = validate(&doc, &schema, Some(&opts));
        assert!(!result.valid);
        assert_eq!(result.errors.len(), 1);
    }

    // ── Date type field validation ──────────────────────────────────────

    #[test]
    fn test_date_value_matches_date_type() {
        let doc = OdinDocumentBuilder::new()
            .set("born", OdinValue::Date {
                year: 1990, month: 6, day: 15,
                raw: "1990-06-15".to_string(),
                modifiers: None, directives: vec![],
            })
            .build().unwrap();
        let mut schema = make_empty_schema();
        schema.fields.insert("born".to_string(), make_field("born", SchemaFieldType::Date, false));
        let result = validate(&doc, &schema, None);
        assert!(result.valid);
    }

    #[test]
    fn test_string_does_not_match_date_type() {
        let doc = OdinDocumentBuilder::new()
            .set("born", OdinValues::string("1990-06-15"))
            .build().unwrap();
        let mut schema = make_empty_schema();
        schema.fields.insert("born".to_string(), make_field("born", SchemaFieldType::Date, false));
        let result = validate(&doc, &schema, None);
        assert!(!result.valid);
    }

    // ── Timestamp type ──────────────────────────────────────────────────

    #[test]
    fn test_timestamp_value_matches() {
        let doc = OdinDocumentBuilder::new()
            .set("created", OdinValue::Timestamp {
                epoch_ms: 1718451600000,
                raw: "2024-06-15T14:00:00Z".to_string(),
                modifiers: None, directives: vec![],
            })
            .build().unwrap();
        let mut schema = make_empty_schema();
        schema.fields.insert("created".to_string(), make_field("created", SchemaFieldType::Timestamp, false));
        let result = validate(&doc, &schema, None);
        assert!(result.valid);
    }

    // ── Binary type ─────────────────────────────────────────────────────

    #[test]
    fn test_binary_value_matches() {
        let doc = OdinDocumentBuilder::new()
            .set("data", OdinValue::Binary {
                data: vec![1, 2, 3],
                algorithm: None,
                modifiers: None,
                directives: vec![],
            })
            .build().unwrap();
        let mut schema = make_empty_schema();
        schema.fields.insert("data".to_string(), make_field("data", SchemaFieldType::Binary, false));
        let result = validate(&doc, &schema, None);
        assert!(result.valid);
    }

    // ── Size constraint on binary ───────────────────────────────────────

    #[test]
    fn test_binary_size_too_small() {
        let doc = OdinDocumentBuilder::new()
            .set("data", OdinValue::Binary {
                data: vec![1],
                algorithm: None,
                modifiers: None,
                directives: vec![],
            })
            .build().unwrap();
        let mut schema = make_empty_schema();
        schema.fields.insert("data".to_string(), make_field_with_constraints(
            "data", SchemaFieldType::Binary, false,
            vec![SchemaConstraint::Size { min: Some(10), max: None }],
        ));
        let result = validate(&doc, &schema, None);
        assert!(!result.valid);
    }

    #[test]
    fn test_binary_size_too_large() {
        let doc = OdinDocumentBuilder::new()
            .set("data", OdinValue::Binary {
                data: vec![0; 100],
                algorithm: None,
                modifiers: None,
                directives: vec![],
            })
            .build().unwrap();
        let mut schema = make_empty_schema();
        schema.fields.insert("data".to_string(), make_field_with_constraints(
            "data", SchemaFieldType::Binary, false,
            vec![SchemaConstraint::Size { min: None, max: Some(50) }],
        ));
        let result = validate(&doc, &schema, None);
        assert!(!result.valid);
    }

    // ── Percent type ────────────────────────────────────────────────────

    #[test]
    fn test_percent_matches() {
        let doc = OdinDocumentBuilder::new()
            .set("rate", OdinValue::Percent {
                value: 0.15,
                raw: None,
                modifiers: None,
                directives: vec![],
            })
            .build().unwrap();
        let mut schema = make_empty_schema();
        schema.fields.insert("rate".to_string(), make_field("rate", SchemaFieldType::Percent, false));
        let result = validate(&doc, &schema, None);
        assert!(result.valid);
    }

    // ── Duration type ───────────────────────────────────────────────────

    #[test]
    fn test_duration_matches() {
        let doc = OdinDocumentBuilder::new()
            .set("period", OdinValue::Duration {
                value: "P1Y6M".to_string(),
                modifiers: None,
                directives: vec![],
            })
            .build().unwrap();
        let mut schema = make_empty_schema();
        schema.fields.insert("period".to_string(), make_field("period", SchemaFieldType::Duration, false));
        let result = validate(&doc, &schema, None);
        assert!(result.valid);
    }

    // ── Time type ───────────────────────────────────────────────────────

    #[test]
    fn test_time_matches() {
        let doc = OdinDocumentBuilder::new()
            .set("start", OdinValue::Time {
                value: "T14:30:00".to_string(),
                modifiers: None,
                directives: vec![],
            })
            .build().unwrap();
        let mut schema = make_empty_schema();
        schema.fields.insert("start".to_string(), make_field("start", SchemaFieldType::Time, false));
        let result = validate(&doc, &schema, None);
        assert!(result.valid);
    }

    // ── Reference type matches ──────────────────────────────────────────

    #[test]
    fn test_reference_type_matches() {
        let doc = OdinDocumentBuilder::new()
            .set("target", OdinValues::string("hello"))
            .set("ptr", OdinValue::Reference {
                path: "target".to_string(),
                modifiers: None,
                directives: vec![],
            })
            .build().unwrap();
        let mut schema = make_empty_schema();
        schema.fields.insert("ptr".to_string(), make_field("ptr", SchemaFieldType::Reference("target".to_string()), false));
        let result = validate(&doc, &schema, None);
        assert!(result.valid);
    }

    // ── TypeRef value-type check + schema type reference (V013) ──────────

    #[test]
    fn test_typeref_accepts_any_value() {
        use crate::types::schema::SchemaType;
        // A defined type ref passes both the value type check and the V013
        // schema-type-reference check.
        let doc = OdinDocumentBuilder::new()
            .set("val", OdinValues::string("anything"))
            .build().unwrap();
        let mut schema = make_empty_schema();
        schema.types.insert("SomeType".to_string(), SchemaType {
            name: "SomeType".to_string(), description: None, fields: vec![], parents: vec![], override_bases: vec![],
        });
        schema.fields.insert("val".to_string(), make_field("val", SchemaFieldType::TypeRef("SomeType".to_string()), false));
        let result = validate(&doc, &schema, None);
        assert!(result.valid);
    }

    #[test]
    fn test_typeref_unresolved_is_v013() {
        // An undefined type reference yields V013 (unresolved type).
        let doc = OdinDocumentBuilder::new()
            .set("val", OdinValues::string("anything"))
            .build().unwrap();
        let mut schema = make_empty_schema();
        schema.fields.insert("val".to_string(), make_field("val", SchemaFieldType::TypeRef("Undefined".to_string()), false));
        let result = validate(&doc, &schema, None);
        assert!(!result.valid);
        assert!(result.errors.iter().any(|e|
            e.error_code == ValidationErrorCode::UnresolvedReference && e.path == "val"
        ), "expected V013 for unresolved type reference");
    }

    // ── validate_formats additional tests ───────────────────────────────

    #[test]
    fn test_validate_formats_multiple() {
        let doc = OdinDocumentBuilder::new()
            .set("email", OdinValues::string("bad"))
            .set("ssn", OdinValues::string("not-ssn"))
            .build().unwrap();
        let errors = validate_formats(&doc, "email = :format email\nssn = :format ssn");
        assert_eq!(errors.len(), 2);
    }

    #[test]
    fn test_validate_formats_all_valid() {
        let doc = OdinDocumentBuilder::new()
            .set("email", OdinValues::string("user@example.com"))
            .set("zip", OdinValues::string("90210"))
            .build().unwrap();
        let errors = validate_formats(&doc, "email = :format email\nzip = :format zip");
        assert!(errors.is_empty());
    }

    // ── Error code correctness ──────────────────────────────────────────

    #[test]
    fn test_error_codes_match_v_codes() {
        assert_eq!(ValidationErrorCode::RequiredFieldMissing.code(), "V001");
        assert_eq!(ValidationErrorCode::TypeMismatch.code(), "V002");
        assert_eq!(ValidationErrorCode::ValueOutOfBounds.code(), "V003");
        assert_eq!(ValidationErrorCode::PatternMismatch.code(), "V004");
        assert_eq!(ValidationErrorCode::InvalidEnumValue.code(), "V005");
        assert_eq!(ValidationErrorCode::ArrayLengthViolation.code(), "V006");
        assert_eq!(ValidationErrorCode::UniqueConstraintViolation.code(), "V007");
        assert_eq!(ValidationErrorCode::InvariantViolation.code(), "V008");
        assert_eq!(ValidationErrorCode::CardinalityConstraintViolation.code(), "V009");
        assert_eq!(ValidationErrorCode::ConditionalRequirementNotMet.code(), "V010");
        assert_eq!(ValidationErrorCode::UnknownField.code(), "V011");
        assert_eq!(ValidationErrorCode::CircularReference.code(), "V012");
        assert_eq!(ValidationErrorCode::UnresolvedReference.code(), "V013");
    }

    // ── ValidationResult helper tests ───────────────────────────────────

    #[test]
    fn test_validation_result_valid() {
        let result = ValidationResult::valid();
        assert!(result.valid);
        assert!(result.errors.is_empty());
    }

    #[test]
    fn test_validation_result_invalid() {
        let errors = vec![ValidationError::new(
            ValidationErrorCode::RequiredFieldMissing,
            "test",
            "missing",
        )];
        let result = ValidationResult::invalid(errors);
        assert!(!result.valid);
        assert_eq!(result.errors.len(), 1);
    }

    // ── Registry-aware schema type references (V013) ─────────────────────

    use crate::resolver::{FileReader, ImportResolver, ResolverOptions};

    /// In-memory reader serving inline schema sources by path, keeping these
    /// tests self-contained.
    struct MemReader(std::collections::HashMap<String, String>);
    impl FileReader for MemReader {
        fn read_file(&self, path: &str) -> Result<String, String> {
            self.0.get(path).cloned().ok_or_else(|| format!("not found: {path}"))
        }
        fn resolve_path(&self, _base: &str, import: &str) -> Result<String, String> {
            Ok(import.to_string())
        }
    }

    /// An imported `@alias.typename` reference is unresolved (V013) without a
    /// registry, and resolves once the import registry is supplied.
    #[test]
    fn test_imported_typeref_resolved_with_registry() {
        let mut files = std::collections::HashMap::new();
        files.insert(
            "main.odin".to_string(),
            "@import \"types.odin\" as types\n\
             {$}\nodin = \"1.0.0\"\nschema = \"1.0.0\"\n\
             {policy}\nstatus_ref = @types.policy_status\n"
                .to_string(),
        );
        files.insert(
            "types.odin".to_string(),
            "{@policy_status}\nvalue = !\n".to_string(),
        );

        let mut resolver =
            ImportResolver::new(Box::new(MemReader(files)), ResolverOptions::default());
        let resolved = resolver.resolve_schema("main.odin").unwrap();
        assert!(
            resolved.type_registry.lookup("types.policy_status").is_some(),
            "registry should resolve types.policy_status"
        );

        let empty = crate::Odin::parse("").unwrap();

        // Without a registry the imported type reference is unresolved (V013).
        let baseline = validate(&empty, &resolved.schema, None);
        let v013_baseline = baseline
            .errors
            .iter()
            .filter(|e| e.error_code == ValidationErrorCode::UnresolvedReference)
            .count();
        assert!(v013_baseline >= 1, "expected V013 without registry");

        // With the registry the reference resolves — no V013.
        let result = validate_with_registry(
            &empty,
            &resolved.schema,
            None,
            Some(&resolved.type_registry),
        );
        let v013 = result
            .errors
            .iter()
            .filter(|e| e.error_code == ValidationErrorCode::UnresolvedReference)
            .count();
        assert_eq!(v013, 0, "registry must resolve @types.policy_status");
    }

    // ── Conformance fixes (full validate pipeline) ──────────────────────

    fn validate_text(schema_text: &str, input: &str) -> ValidationResult {
        let schema = schema_parser::parse_schema(schema_text).unwrap();
        let doc = crate::Odin::parse(input).unwrap();
        validate(&doc, &schema, None)
    }

    #[test]
    fn test_intersection_all_present_valid() {
        let schema = "{@hasName}\nname = !\n\n{@hasAge}\nage = !##\n\n{customer}\n= @hasName & @hasAge";
        let r = validate_text(schema, "{customer}\nname = \"Bob\"\nage = ##5");
        assert!(r.valid, "errors: {:?}", r.errors);
    }

    #[test]
    fn test_intersection_missing_member_field_v001() {
        let schema = "{@hasName}\nname = !\n\n{@hasAge}\nage = !##\n\n{customer}\n= @hasName & @hasAge";
        let r = validate_text(schema, "{customer}\nname = \"Bob\"");
        assert!(!r.valid);
        assert!(r.errors.iter().any(|e|
            e.error_code == ValidationErrorCode::RequiredFieldMissing && e.path == "customer.age"));
    }

    #[test]
    fn test_intersection_unresolved_member_v013() {
        let schema = "{@hasName}\nname = !\n\n{customer}\n= @hasName & @doesNotExist";
        let r = validate_text(schema, "{customer}\nname = \"Bob\"");
        assert!(!r.valid);
        assert!(r.errors.iter().any(|e| e.error_code == ValidationErrorCode::UnresolvedReference));
    }

    #[test]
    fn test_temporal_bounds_chronological() {
        let schema = "{root}\nd = date:(2020-06-15..2020-06-20)";
        assert!(validate_text(schema, "{root}\nd = 2020-06-17").valid);
        assert!(!validate_text(schema, "{root}\nd = 2020-06-10").valid);
        assert!(!validate_text(schema, "{root}\nd = 2020-06-25").valid);
    }

    #[test]
    fn test_percent_type_validation() {
        let schema = "{root}\ntax = #%";
        assert!(validate_text(schema, "{root}\ntax = #%0.15").valid);
        let bad = validate_text(schema, "{root}\ntax = \"fifteen\"");
        assert!(!bad.valid);
        assert!(bad.errors.iter().any(|e| e.error_code == ValidationErrorCode::TypeMismatch));
    }

    #[test]
    fn test_union_null_member_accepts_null() {
        assert!(validate_text("{root}\nn = #|~", "{root}\nn = ~").valid);
    }

    #[test]
    fn test_union_date_timestamp_accepts_timestamp() {
        assert!(validate_text("{root}\nu = date|timestamp", "{root}\nu = 2020-06-17T10:00:00Z").valid);
    }

    #[test]
    fn test_pattern_conditional_required_when_met() {
        let schema = "{root}\nfield = !:/^[a-z]+$/:if method = paypal\nmethod = ";
        let r = validate_text(schema, "{root}\nmethod = \"paypal\"");
        assert!(!r.valid);
        assert!(r.errors.iter().any(|e|
            e.error_code == ValidationErrorCode::ConditionalRequirementNotMet && e.path == "root.field"));
    }

    #[test]
    fn test_pattern_conditional_optional_when_unmet() {
        let schema = "{root}\nfield = !:/^[a-z]+$/:if method = paypal\nmethod = ";
        assert!(validate_text(schema, "{root}\nmethod = \"stripe\"").valid);
    }

    #[cfg(feature = "regex")]
    #[test]
    fn test_pattern_still_enforced_on_present_value() {
        let schema = "{root}\nfield = !:/^[a-z]+$/:if method = paypal\nmethod = ";
        let r = validate_text(schema, "{root}\nfield = \"ABC123\"\nmethod = \"paypal\"");
        assert!(!r.valid);
        assert!(r.errors.iter().any(|e|
            e.error_code == ValidationErrorCode::PatternMismatch && e.path == "root.field"));
    }

    #[test]
    fn test_field_typeref_enforces_nested_required() {
        let schema = "{@address}\nstreet = !\ncity = !\n\n{customer}\nname = !\nbilling = @address";
        let r = validate_text(schema, "{customer}\nname = \"X\"\nbilling.street = \"Main\"");
        assert!(!r.valid);
        assert!(r.errors.iter().any(|e|
            e.error_code == ValidationErrorCode::RequiredFieldMissing && e.path == "customer.billing.city"));
    }

    #[test]
    fn test_field_typeref_absent_optional_ok() {
        let schema = "{@address}\nstreet = !\ncity = !\n\n{customer}\nname = !\nbilling = @address";
        assert!(validate_text(schema, "{customer}\nname = \"X\"").valid);
    }

    #[test]
    fn test_field_typeref_complete_valid() {
        let schema = "{@address}\nstreet = !\ncity = !\n\n{customer}\nname = !\nbilling = @address";
        assert!(validate_text(schema,
            "{customer}\nname = \"X\"\nbilling.street = \"Main\"\nbilling.city = \"NYC\"").valid);
    }

    #[test]
    fn test_invariant_null_operand_arithmetic_v008() {
        let schema = "{order}\ntotal = #$\nsubtotal = #$\ntax = ~#$\n:invariant total = subtotal + tax";
        let r = validate_text(schema, "{order}\ntotal = #$10.00\nsubtotal = #$10.00\ntax = ~");
        assert!(!r.valid);
        assert!(r.errors.iter().any(|e|
            e.error_code == ValidationErrorCode::InvariantViolation && e.path == "order"));
    }

    #[test]
    fn test_invariant_arithmetic_all_present_valid() {
        let schema = "{order}\ntotal = #$\nsubtotal = #$\ntax = #$\n:invariant total = subtotal + tax";
        assert!(validate_text(schema, "{order}\ntotal = #$12.00\nsubtotal = #$10.00\ntax = #$2.00").valid);
    }

    #[test]
    fn test_invariant_comparison_null_operand_v008() {
        let schema = "{range}\nstart = ~#\nend = ~#\n:invariant end >= start";
        let r = validate_text(schema, "{range}\nend = #5\nstart = ~");
        assert!(!r.valid);
        assert!(r.errors.iter().any(|e| e.error_code == ValidationErrorCode::InvariantViolation));
    }

    /// A relative `{.sub}` header inside a `{@type}` nests its fields into that
    /// type, not the schema root.
    #[test]
    fn test_relative_subsection_nests_into_type() {
        let schema = schema_parser::parse_schema(
            "{@policy}\nnumber = !\n{.term}\neffective = !date\nexpiration = !date\n",
        )
        .unwrap();
        let policy = schema
            .types
            .get("policy")
            .expect("policy type should be defined");
        let has = |n: &str| policy.fields.iter().any(|f| f.name == n);
        assert!(
            has("term.effective") && has("term.expiration"),
            "term.* should nest into the policy type"
        );
        assert!(
            !schema.fields.contains_key("term.effective"),
            "term.* must not leak to the schema root"
        );
    }

    // ── Conformance: conditional, computed, binary size, decimal places ──────

    fn has_code_at(result: &ValidationResult, code: &str, path: &str) -> bool {
        result.errors.iter().any(|e| e.code() == code && e.path == path)
    }

    #[test]
    fn unless_required_when_condition_false() {
        let schema = "{$}\nodin = \"1.0.0\"\nschema = \"1.0.0\"\n\n{Person}\nstatus =\nphone = ! :unless status = \"inactive\"";
        let ok = validate_text(schema, "{Person}\nstatus = \"inactive\"");
        assert!(ok.valid);
        let bad = validate_text(schema, "{Person}\nstatus = \"active\"");
        assert!(!bad.valid);
        assert!(has_code_at(&bad, "V010", "Person.phone"));
    }

    #[test]
    fn computed_absent_not_required() {
        let schema = "{$}\nodin = \"1.0.0\"\nschema = \"1.0.0\"\n\n{Order}\ntotal = !# :computed";
        let r = validate_text(schema, "{Order}\nname = \"x\"");
        assert!(r.valid, "errors: {:?}", r.errors);
    }

    #[test]
    fn binary_size_byte_length() {
        let schema = "{$}\nodin = \"1.0.0\"\nschema = \"1.0.0\"\n\n{R}\nhash = ^:(4)";
        assert!(validate_text(schema, "{R}\nhash = ^AAAAAA==").valid);
        let small = validate_text(schema, "{R}\nhash = ^AAAA");
        assert!(has_code_at(&small, "V003", "R.hash"));
        let large = validate_text(schema, "{R}\nhash = ^AAAAAAA=");
        assert!(has_code_at(&large, "V003", "R.hash"));
    }

    #[test]
    fn binary_sha256_size_wrong() {
        let schema = "{$}\nodin = \"1.0.0\"\nschema = \"1.0.0\"\n\n{R}\nhash = ^sha256:(32)";
        let r = validate_text(schema, "{R}\nhash = ^sha256:AAAAAAAAAAAAAAAAAAAAAA==");
        assert!(has_code_at(&r, "V003", "R.hash"));
    }

    #[test]
    fn decimal_places_exact() {
        let schema = "{$}\nodin = \"1.0.0\"\nschema = \"1.0.0\"\n\n{R}\nrate = #.4";
        assert!(validate_text(schema, "{R}\nrate = #1.2345").valid);
        assert!(has_code_at(&validate_text(schema, "{R}\nrate = #1.23"), "V003", "R.rate"));
        assert!(has_code_at(&validate_text(schema, "{R}\nrate = #1.23456"), "V003", "R.rate"));
    }
}
