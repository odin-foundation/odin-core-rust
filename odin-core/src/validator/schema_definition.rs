//! Schema-definition validation.
//!
//! Validates that the schema itself is well-formed, independent of any document:
//! override restrictiveness, intersection field conflicts, tabular column rules,
//! and default-value rules. Violations are reported as V017.

use crate::types::schema::{
    OdinSchemaDefinition, SchemaConstraint, SchemaDefault, SchemaField,
    SchemaFieldType, SchemaType,
};
use crate::types::errors::{ValidationError, ValidationErrorCode};
use crate::resolver::TypeRegistry;

/// Run all schema-definition validations, appending V017 errors.
pub fn validate_schema_definition(
    schema: &OdinSchemaDefinition,
    registry: Option<&TypeRegistry>,
    errors: &mut Vec<ValidationError>,
) {
    validate_type_definitions(schema, registry, errors);
    validate_path_compositions(schema, registry, errors);
    validate_tabular_columns(schema, registry, errors);
    validate_defaults(schema, errors);
}

fn add_error(errors: &mut Vec<ValidationError>, path: &str, message: &str, expected: &str, actual: &str) {
    errors.push(ValidationError {
        path: path.to_string(),
        error_code: ValidationErrorCode::SchemaDefinitionError,
        message: message.to_string(),
        expected: Some(expected.to_string()),
        actual: Some(actual.to_string()),
        schema_path: None,
    });
}

fn lookup_base<'a>(
    schema: &'a OdinSchemaDefinition,
    registry: Option<&'a TypeRegistry>,
    name: &str,
) -> Option<&'a SchemaType> {
    if let Some(reg) = registry {
        if let Some(t) = reg.lookup(name) {
            return Some(t);
        }
    }
    schema.types.get(name)
}

// ── Override and Intersection (type definitions) ─────────────────────────────

fn validate_type_definitions(
    schema: &OdinSchemaDefinition,
    registry: Option<&TypeRegistry>,
    errors: &mut Vec<ValidationError>,
) {
    for (type_name, ty) in &schema.types {
        if !ty.override_bases.is_empty() {
            validate_override(schema, registry, type_name, ty, &ty.override_bases, errors);
        } else if ty.parents.len() > 1 {
            validate_intersection_conflicts(schema, registry, type_name, &ty.parents, errors);
        }
    }
}

/// Override may only narrow: base type must match, optional→required allowed
/// (not reverse), nullability may be removed (not added), bounds may only narrow.
fn validate_override(
    schema: &OdinSchemaDefinition,
    registry: Option<&TypeRegistry>,
    type_name: &str,
    ty: &SchemaType,
    base_names: &[String],
    errors: &mut Vec<ValidationError>,
) {
    let mut base_fields: Vec<&SchemaField> = Vec::new();
    for base_name in base_names {
        if let Some(base) = lookup_base(schema, registry, base_name) {
            for f in &base.fields {
                if f.name != "_composition" {
                    base_fields.push(f);
                }
            }
        }
    }

    for over in &ty.fields {
        if over.name == "_composition" {
            continue;
        }
        if let Some(base) = base_fields.iter().find(|b| b.name == over.name) {
            check_override_field(&format!("@{type_name}.{}", over.name), base, over, errors);
        }
    }
}

fn check_override_field(
    label: &str,
    base: &SchemaField,
    over: &SchemaField,
    errors: &mut Vec<ValidationError>,
) {
    // Base type must match.
    if !same_base_type(&base.field_type, &over.field_type) {
        add_error(
            errors,
            label,
            "Override changes field type",
            type_kind_label(&base.field_type),
            type_kind_label(&over.field_type),
        );
    }

    // required: optional→required allowed, required→optional forbidden.
    if base.required && !over.required {
        add_error(errors, label, "Override relaxes required field to optional", "required", "optional");
    }

    // nullable: may remove, may not add.
    if !is_nullable(base) && is_nullable(over) {
        add_error(errors, label, "Override adds nullability", "non-nullable", "nullable");
    }

    // bounds: may only narrow.
    if let (Some(bb), Some(ob)) = (find_bounds(&base.constraints), find_bounds(&over.constraints)) {
        if widens_bounds(bb, ob) {
            add_error(
                errors,
                label,
                "Override widens constraint bounds",
                &bounds_label(bb),
                &bounds_label(ob),
            );
        }
    }
}

/// Intersection field conflicts: a field defined in more than one member with a
/// differing definition is an error.
fn validate_intersection_conflicts(
    schema: &OdinSchemaDefinition,
    registry: Option<&TypeRegistry>,
    type_name: &str,
    member_names: &[String],
    errors: &mut Vec<ValidationError>,
) {
    let mut seen: Vec<(String, SchemaField)> = Vec::new();
    for member_name in member_names {
        let Some(member) = lookup_base(schema, registry, member_name) else { continue };
        for f in &member.fields {
            if f.name == "_composition" {
                continue;
            }
            if let Some((_, prior)) = seen.iter().find(|(n, _)| n == &f.name) {
                if !same_field_definition(prior, f) {
                    add_error(
                        errors,
                        &format!("@{type_name}.{}", f.name),
                        &format!("Intersection field conflict: '{}'", f.name),
                        "identical field definitions",
                        "conflicting definitions",
                    );
                }
            } else {
                seen.push((f.name.clone(), f.clone()));
            }
        }
    }
}

// ── Path-level compositions ({path} = @base :override) ───────────────────────

fn validate_path_compositions(
    schema: &OdinSchemaDefinition,
    registry: Option<&TypeRegistry>,
    errors: &mut Vec<ValidationError>,
) {
    for (path, field) in &schema.fields {
        let Some(parent_path) = path.strip_suffix("._composition") else { continue };
        let SchemaFieldType::TypeRef(name) = &field.field_type else { continue };

        let member_names: Vec<String> = name
            .split('&')
            .map(|n| n.trim().to_string())
            .filter(|n| !n.is_empty())
            .collect();

        // Section overrides are marked with description == "override".
        let is_override = field.description.as_deref() == Some("override");

        if is_override {
            let mut base_fields: Vec<&SchemaField> = Vec::new();
            for base_name in &member_names {
                if let Some(base) = lookup_base(schema, registry, base_name) {
                    for f in &base.fields {
                        if f.name != "_composition" {
                            base_fields.push(f);
                        }
                    }
                }
            }
            let prefix = format!("{parent_path}.");
            for (field_path, over) in &schema.fields {
                if !field_path.starts_with(&prefix) || field_path.ends_with("._composition") {
                    continue;
                }
                let local = &field_path[prefix.len()..];
                if local.contains('.') {
                    continue;
                }
                if let Some(base) = base_fields.iter().find(|b| &b.name == local) {
                    check_override_field(field_path, base, over, errors);
                }
            }
        } else if member_names.len() > 1 {
            validate_intersection_conflicts(schema, registry, parent_path, &member_names, errors);
        }
    }
}

// ── Tabular column rules ─────────────────────────────────────────────────────

fn validate_tabular_columns(
    schema: &OdinSchemaDefinition,
    registry: Option<&TypeRegistry>,
    errors: &mut Vec<ValidationError>,
) {
    for (array_path, array) in &schema.arrays {
        if array.columns.is_empty() {
            continue;
        }
        for column in &array.columns {
            let label = format!("{array_path}[].{column}");

            if is_multi_level_column(column) {
                add_error(errors, &label, "Tabular column uses multi-level path", "single-level column", column);
                continue;
            }

            let item_name = strip_index(column);
            let Some(field) = array.item_fields.get(item_name).or_else(|| array.item_fields.get(column.as_str())) else { continue };

            if !is_primitive_column_type(schema, registry, &field.field_type) {
                add_error(errors, &label, "Tabular column must be a primitive type", "primitive", type_kind_label(&field.field_type));
            }
        }
    }
}

fn is_multi_level_column(column: &str) -> bool {
    let dot_count = column.matches('.').count();
    let index_count = column.matches('[').count();
    if dot_count > 1 || index_count > 1 {
        return true;
    }
    dot_count == 1 && index_count == 1
}

fn strip_index(column: &str) -> &str {
    if let Some(pos) = column.find('[') {
        if column.ends_with(']') {
            return &column[..pos];
        }
    }
    column
}

fn is_primitive_column_type(
    schema: &OdinSchemaDefinition,
    registry: Option<&TypeRegistry>,
    ty: &SchemaFieldType,
) -> bool {
    match ty {
        SchemaFieldType::TypeRef(_) => false,
        SchemaFieldType::Union(members) => {
            members.iter().all(|m| is_primitive_column_type(schema, registry, m))
        }
        SchemaFieldType::Reference(_) => false,
        SchemaFieldType::String
        | SchemaFieldType::Boolean
        | SchemaFieldType::Number { .. }
        | SchemaFieldType::Integer
        | SchemaFieldType::Decimal { .. }
        | SchemaFieldType::Currency { .. }
        | SchemaFieldType::Percent
        | SchemaFieldType::Date
        | SchemaFieldType::Timestamp
        | SchemaFieldType::Time
        | SchemaFieldType::Duration
        | SchemaFieldType::Enum(_)
        | SchemaFieldType::Binary
        | SchemaFieldType::Null => true,
    }
}

// ── Default value rules ──────────────────────────────────────────────────────

fn validate_defaults(schema: &OdinSchemaDefinition, errors: &mut Vec<ValidationError>) {
    for (path, field) in &schema.fields {
        if path.ends_with("._composition") {
            continue;
        }
        check_default(path, field, errors);
    }
    for ty in schema.types.values() {
        for field in &ty.fields {
            if field.name == "_composition" {
                continue;
            }
            check_default(&format!("@{}.{}", ty.name, field.name), field, errors);
        }
    }
    for (array_path, array) in &schema.arrays {
        for field in array.item_fields.values() {
            check_default(&format!("{array_path}[].{}", field.name), field, errors);
        }
    }
}

fn check_default(label: &str, field: &SchemaField, errors: &mut Vec<ValidationError>) {
    let Some(default) = &field.default_value else { return };

    if field.required {
        add_error(errors, label, "Required field cannot have a default value", "no default", "default present");
        return;
    }

    if !default_satisfies_constraints(field, default) {
        add_error(errors, label, "Default value violates field constraints", "value within constraints", &describe_default(default));
    }
}

fn default_satisfies_constraints(field: &SchemaField, default: &SchemaDefault) -> bool {
    for constraint in &field.constraints {
        match constraint {
            SchemaConstraint::Bounds { min, max, .. } => {
                if !bounds_satisfied(min, max, default) {
                    return false;
                }
            }
            SchemaConstraint::Enum(values) => {
                if let SchemaDefault::String(s) = default {
                    if !values.iter().any(|v| v == s) {
                        return false;
                    }
                } else {
                    return false;
                }
            }
            _ => {}
        }
    }
    // Enum-typed field.
    if let SchemaFieldType::Enum(values) = &field.field_type {
        if let SchemaDefault::String(s) = default {
            if !values.iter().any(|v| v == s) {
                return false;
            }
        } else {
            return false;
        }
    }
    true
}

fn bounds_satisfied(min: &Option<String>, max: &Option<String>, default: &SchemaDefault) -> bool {
    let target: Option<f64> = match default {
        SchemaDefault::Number(n) | SchemaDefault::Currency(n) | SchemaDefault::Percent(n) => Some(*n),
        SchemaDefault::Integer(n) => Some(*n as f64),
        SchemaDefault::String(s) => Some(s.chars().count() as f64),
        SchemaDefault::Bool(_) => None,
    };
    let Some(target) = target else { return true };

    if let Some(min_str) = min {
        if let Ok(min_val) = min_str.parse::<f64>() {
            if target < min_val {
                return false;
            }
        }
    }
    if let Some(max_str) = max {
        if let Ok(max_val) = max_str.parse::<f64>() {
            if target > max_val {
                return false;
            }
        }
    }
    true
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn same_base_type(a: &SchemaFieldType, b: &SchemaFieldType) -> bool {
    type_kind_label(non_null_base(a)) == type_kind_label(non_null_base(b))
}

/// The underlying type of a nullable field: a `null | T` union reduces to `T`.
fn non_null_base(t: &SchemaFieldType) -> &SchemaFieldType {
    if let SchemaFieldType::Union(members) = t {
        let non_null: Vec<&SchemaFieldType> =
            members.iter().filter(|m| !matches!(m, SchemaFieldType::Null)).collect();
        if non_null.len() == 1 {
            return non_null[0];
        }
    }
    t
}

fn type_kind_label(t: &SchemaFieldType) -> &'static str {
    match t {
        SchemaFieldType::String => "string",
        SchemaFieldType::Boolean => "boolean",
        SchemaFieldType::Null => "null",
        SchemaFieldType::Number { .. } => "number",
        SchemaFieldType::Integer => "integer",
        SchemaFieldType::Decimal { .. } => "decimal",
        SchemaFieldType::Currency { .. } => "currency",
        SchemaFieldType::Date => "date",
        SchemaFieldType::Timestamp => "timestamp",
        SchemaFieldType::Time => "time",
        SchemaFieldType::Duration => "duration",
        SchemaFieldType::Percent => "percent",
        SchemaFieldType::Enum(_) => "enum",
        SchemaFieldType::Union(_) => "union",
        SchemaFieldType::Reference(_) => "reference",
        SchemaFieldType::Binary => "binary",
        SchemaFieldType::TypeRef(_) => "typeRef",
    }
}

/// A field is nullable when its type is `Null` or a union that admits `Null`.
fn is_nullable(field: &SchemaField) -> bool {
    match &field.field_type {
        SchemaFieldType::Null => true,
        SchemaFieldType::Union(members) => {
            members.iter().any(|m| matches!(m, SchemaFieldType::Null))
        }
        _ => false,
    }
}

fn find_bounds(constraints: &[SchemaConstraint]) -> Option<&SchemaConstraint> {
    constraints.iter().find(|c| matches!(c, SchemaConstraint::Bounds { .. }))
}

fn bounds_label(c: &SchemaConstraint) -> String {
    if let SchemaConstraint::Bounds { min, max, .. } = c {
        format!("({}..{})", min.clone().unwrap_or_default(), max.clone().unwrap_or_default())
    } else {
        String::new()
    }
}

/// A bounds override widens if it loosens either end relative to the base.
fn widens_bounds(base: &SchemaConstraint, over: &SchemaConstraint) -> bool {
    let (SchemaConstraint::Bounds { min: bmin, max: bmax, .. }, SchemaConstraint::Bounds { min: omin, max: omax, .. }) = (base, over) else {
        return false;
    };
    if let Some(bmin) = bmin.as_ref().and_then(|s| s.parse::<f64>().ok()) {
        match omin.as_ref().and_then(|s| s.parse::<f64>().ok()) {
            Some(o) if o >= bmin => {}
            _ => return true,
        }
    }
    if let Some(bmax) = bmax.as_ref().and_then(|s| s.parse::<f64>().ok()) {
        match omax.as_ref().and_then(|s| s.parse::<f64>().ok()) {
            Some(o) if o <= bmax => {}
            _ => return true,
        }
    }
    false
}

fn same_field_definition(a: &SchemaField, b: &SchemaField) -> bool {
    if type_kind_label(&a.field_type) != type_kind_label(&b.field_type) {
        return false;
    }
    if a.required != b.required {
        return false;
    }
    if is_nullable(a) != is_nullable(b) {
        return false;
    }
    constraints_repr(&a.constraints) == constraints_repr(&b.constraints)
}

fn constraints_repr(constraints: &[SchemaConstraint]) -> Vec<String> {
    constraints.iter().map(|c| format!("{c:?}")).collect()
}

fn describe_default(default: &SchemaDefault) -> String {
    match default {
        SchemaDefault::String(s) => s.clone(),
        SchemaDefault::Number(n) | SchemaDefault::Currency(n) | SchemaDefault::Percent(n) => n.to_string(),
        SchemaDefault::Integer(n) => n.to_string(),
        SchemaDefault::Bool(b) => b.to_string(),
    }
}
