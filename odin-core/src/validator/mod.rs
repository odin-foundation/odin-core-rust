//! Schema validation for ODIN documents.
//!
//! Validates an `OdinDocument` against an `OdinSchemaDefinition`,
//! producing a `ValidationResult` with errors and warnings.

mod format_validators;
pub mod schema_parser;
pub mod schema_serializer;
pub mod validate_redos;

use std::borrow::Cow;

use crate::types::document::OdinDocument;
use crate::types::schema::{
    OdinSchemaDefinition, SchemaConstraint, SchemaField,
    SchemaFieldType, SchemaObjectConstraint, ConditionalOperator, ConditionalValue,
    ValidationResult,
};
use crate::types::options::ValidateOptions;
use crate::types::errors::{ValidationError, ValidationErrorCode};
use crate::types::values::OdinValue;

/// Validate a document against a schema.
pub fn validate(
    doc: &OdinDocument,
    schema: &OdinSchemaDefinition,
    options: Option<&ValidateOptions>,
) -> ValidationResult {
    let opts = options.cloned().unwrap_or_default();
    let mut errors = Vec::new();

    // 0. Expand type composition (merge base type fields into derived types)
    let schema = expand_type_composition(schema);

    // 1. Validate fields defined in schema
    for (path, field) in &schema.fields {
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
        validate_strict(doc, &schema, &mut errors);
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

fn validate_pattern(
    value: &OdinValue,
    path: &str,
    pattern: &str,
    errors: &mut Vec<ValidationError>,
) {
    // ReDoS safety check
    let redos_check = validate_redos::analyze_pattern(pattern);
    if !redos_check.safe {
        errors.push(ValidationError::new(
            ValidationErrorCode::PatternMismatch,
            path,
            format!("Unsafe regex pattern: {}", redos_check.reason.unwrap_or_default()),
        ));
        return;
    }

    if let Some(s) = value.as_str() {
        // Use regex crate if available, otherwise basic matching
        #[cfg(feature = "regex")]
        {
            match regex::Regex::new(pattern) {
                Ok(re) => {
                    if !re.is_match(s) {
                        errors.push(ValidationError::new(
                            ValidationErrorCode::PatternMismatch,
                            path,
                            format!("Value '{}' does not match pattern '{}'", s, pattern),
                        ));
                    }
                }
                Err(_) => {
                    errors.push(ValidationError::new(
                        ValidationErrorCode::PatternMismatch,
                        path,
                        format!("Invalid regex pattern: '{}'", pattern),
                    ));
                }
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
    // Simple expression evaluator: field op value
    // Supports: >, <, >=, <=, ==, !=
    let ops = [">=", "<=", "!=", "==", ">", "<"];
    for op in &ops {
        if let Some(pos) = expr.find(op) {
            let field_name = expr[..pos].trim();
            let compare_val = expr[pos + op.len()..].trim();

            let full_path = if path.is_empty() {
                field_name.to_string()
            } else {
                format!("{path}.{field_name}")
            };

            if let Some(val) = doc.get(&full_path) {
                let passes = evaluate_comparison(val, op, compare_val);
                if !passes {
                    errors.push(ValidationError::new(
                        ValidationErrorCode::InvariantViolation,
                        path,
                        format!("Invariant '{expr}' violated"),
                    ));
                }
            }
            return;
        }
    }
}

fn evaluate_comparison(value: &OdinValue, op: &str, compare: &str) -> bool {
    // Try numeric comparison
    if let Some(num) = value.as_f64() {
        if let Ok(cmp) = compare.parse::<f64>() {
            return match op {
                ">" => num > cmp,
                "<" => num < cmp,
                ">=" => num >= cmp,
                "<=" => num <= cmp,
                "==" => (num - cmp).abs() < f64::EPSILON,
                "!=" => (num - cmp).abs() >= f64::EPSILON,
                _ => true,
            };
        }
    }
    // Try string comparison
    if let Some(s) = value.as_str() {
        let cmp = compare.trim_matches('"');
        return match op {
            "==" => s == cmp,
            "!=" => s != cmp,
            ">" => s > cmp,
            "<" => s < cmp,
            ">=" => s >= cmp,
            "<=" => s <= cmp,
            _ => true,
        };
    }
    true // Can't evaluate — skip
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
                    description: None,
                    constraints: vec![],
                    default_value: None,
                    conditionals: vec![],
                },
            ],
            parents: vec![],
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
                    description: None,
                    constraints: vec![],
                    default_value: None,
                    conditionals: vec![],
                },
            ],
            parents: vec!["@Parent".to_string()],
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

    // ── TypeRef always passes type check ────────────────────────────────

    #[test]
    fn test_typeref_accepts_any_value() {
        let doc = OdinDocumentBuilder::new()
            .set("val", OdinValues::string("anything"))
            .build().unwrap();
        let mut schema = make_empty_schema();
        schema.fields.insert("val".to_string(), make_field("val", SchemaFieldType::TypeRef("SomeType".to_string()), false));
        let result = validate(&doc, &schema, None);
        assert!(result.valid);
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
}
