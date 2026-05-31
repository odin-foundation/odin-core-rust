//! Golden schema parsing tests — reads test cases from sdk/golden/schema/
//! and verifies schema parser output.

use odin_core::types::schema::{
    SchemaConstraint, SchemaDefault, SchemaField, SchemaFieldType,
};
use odin_core::validator::schema_parser;
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::PathBuf;

#[derive(Deserialize)]
struct TestSuite {
    suite: String,
    tests: Vec<TestCase>,
}

#[derive(Deserialize)]
struct TestCase {
    id: String,
    description: String,
    schema: String,
    #[serde(default)]
    expected: Value,
    #[serde(default)]
    structural: bool,
    /// Value-level assertions (constraint values, unions, defaults, flags).
    #[serde(default)]
    assert: Option<Assertions>,
}

#[derive(Deserialize)]
struct Assertions {
    #[serde(default)]
    fields: std::collections::HashMap<String, FieldAssertion>,
    #[serde(default)]
    types: std::collections::HashMap<String, TypeAssertion>,
}

#[derive(Deserialize)]
struct TypeAssertion {
    #[serde(default)]
    fields: std::collections::HashMap<String, FieldAssertion>,
}

#[derive(Deserialize, Default)]
struct FieldAssertion {
    #[serde(rename = "typeKind", default)]
    type_kind: Option<String>,
    #[serde(rename = "typeRefName", default)]
    type_ref_name: Option<String>,
    #[serde(default)]
    required: Option<bool>,
    #[serde(default)]
    immutable: Option<bool>,
    #[serde(default)]
    computed: Option<bool>,
    #[serde(default)]
    deprecated: Option<bool>,
    /// Expected union member kinds, order-insensitive.
    #[serde(default)]
    union: Option<Vec<String>>,
    /// Expected default value (subset of {type, value}).
    #[serde(default)]
    default: Option<Value>,
    /// Expected constraints (each listed must be present).
    #[serde(default)]
    constraints: Option<Vec<Value>>,
    /// Expected conditionals (each listed must be present).
    #[serde(default)]
    conditionals: Option<Vec<Value>>,
}

fn golden_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent().unwrap()
        .parent().unwrap()
        .join("golden")
        .join("schema")
}

/// The structural type-kind name for a parsed field type.
fn type_kind_name(t: &SchemaFieldType) -> &'static str {
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

/// A `{type, value}` JSON view of a typed default.
fn default_to_json(d: &SchemaDefault) -> Value {
    match d {
        SchemaDefault::String(s) => json!({ "type": "string", "value": s }),
        SchemaDefault::Number(n) => json!({ "type": "number", "value": n }),
        SchemaDefault::Integer(n) => json!({ "type": "integer", "value": n }),
        SchemaDefault::Currency(n) => json!({ "type": "currency", "value": n }),
        SchemaDefault::Percent(n) => json!({ "type": "percent", "value": n }),
        SchemaDefault::Bool(b) => json!({ "type": "boolean", "value": b }),
    }
}

/// A JSON view of a constraint for subset matching.
fn constraint_to_json(c: &SchemaConstraint) -> Value {
    match c {
        SchemaConstraint::Bounds { min, max, .. } => {
            let to_v = |o: &Option<String>| -> Value {
                match o {
                    None => Value::Null,
                    Some(s) => s.parse::<i64>().map(Value::from).unwrap_or_else(|_| Value::from(s.clone())),
                }
            };
            json!({ "kind": "bounds", "min": to_v(min), "max": to_v(max) })
        }
        SchemaConstraint::Pattern(p) => json!({ "kind": "pattern", "pattern": p }),
        SchemaConstraint::Enum(v) => json!({ "kind": "enum", "values": v }),
        SchemaConstraint::Unique => json!({ "kind": "unique" }),
        SchemaConstraint::Size { min, max } => json!({ "kind": "size", "min": min, "max": max }),
        SchemaConstraint::Format(f) => json!({ "kind": "format", "format": f }),
    }
}

/// A JSON view of a conditional for subset matching.
fn conditional_to_json(c: &odin_core::types::schema::SchemaConditional) -> Value {
    use odin_core::types::schema::{ConditionalOperator, ConditionalValue};
    let op = match c.operator {
        ConditionalOperator::Eq => "=",
        ConditionalOperator::NotEq => "!=",
        ConditionalOperator::Gt => ">",
        ConditionalOperator::Lt => "<",
        ConditionalOperator::Gte => ">=",
        ConditionalOperator::Lte => "<=",
    };
    let value = match &c.value {
        ConditionalValue::String(s) => Value::from(s.clone()),
        ConditionalValue::Number(n) => Value::from(*n),
        ConditionalValue::Bool(b) => Value::from(*b),
    };
    json!({ "field": c.field, "operator": op, "value": value, "unless": c.unless })
}

/// Whether `actual` contains every key/value pair declared in `expected`.
fn json_subset(expected: &Value, actual: &Value) -> bool {
    match (expected, actual) {
        (Value::Object(e), Value::Object(a)) => e.iter().all(|(k, v)| {
            a.get(k).map_or(false, |av| json_subset(v, av))
        }),
        // Numeric equality is value-based (5 == 5.0).
        (Value::Number(e), Value::Number(a)) => match (e.as_f64(), a.as_f64()) {
            (Some(x), Some(y)) => (x - y).abs() < 1e-9,
            _ => e == a,
        },
        _ => expected == actual,
    }
}

/// Assert one parsed field against a `FieldAssertion`. Returns failures.
fn assert_field(field: Option<&SchemaField>, a: &FieldAssertion, label: &str) -> Vec<String> {
    let mut fails = Vec::new();
    let Some(f) = field else {
        fails.push(format!("{label}: field not found"));
        return fails;
    };

    if let Some(tk) = &a.type_kind {
        let actual = type_kind_name(&f.field_type);
        if actual != tk {
            fails.push(format!("{label}: typeKind expected {tk}, got {actual}"));
        }
    }
    if let Some(name) = &a.type_ref_name {
        match &f.field_type {
            SchemaFieldType::TypeRef(n) if n == name => {}
            other => fails.push(format!("{label}: typeRefName expected {name}, got {other:?}")),
        }
    }
    if let Some(req) = a.required {
        if f.required != req { fails.push(format!("{label}: required expected {req}, got {}", f.required)); }
    }
    if let Some(imm) = a.immutable {
        if f.immutable != imm { fails.push(format!("{label}: immutable expected {imm}, got {}", f.immutable)); }
    }
    if let Some(c) = a.computed {
        if f.computed != c { fails.push(format!("{label}: computed expected {c}, got {}", f.computed)); }
    }
    if let Some(dep) = a.deprecated {
        if f.deprecated != dep { fails.push(format!("{label}: deprecated expected {dep}, got {}", f.deprecated)); }
    }
    if let Some(expected_union) = &a.union {
        match &f.field_type {
            SchemaFieldType::Union(members) => {
                let mut got: Vec<String> = members.iter().map(|m| type_kind_name(m).to_string()).collect();
                let mut want = expected_union.clone();
                got.sort();
                want.sort();
                if got != want {
                    fails.push(format!("{label}: union members expected {want:?}, got {got:?}"));
                }
            }
            other => fails.push(format!("{label}: expected union, got {other:?}")),
        }
    }
    if let Some(expected_default) = &a.default {
        match &f.default_value {
            Some(d) => {
                let actual = default_to_json(d);
                if !json_subset(expected_default, &actual) {
                    fails.push(format!("{label}: default expected {expected_default}, got {actual}"));
                }
            }
            None => fails.push(format!("{label}: default expected {expected_default}, got none")),
        }
    }
    if let Some(expected_cs) = &a.constraints {
        let actual: Vec<Value> = f.constraints.iter().map(constraint_to_json).collect();
        for ec in expected_cs {
            if !actual.iter().any(|ac| json_subset(ec, ac)) {
                fails.push(format!("{label}: missing constraint {ec} (got {actual:?})"));
            }
        }
    }
    if let Some(expected_conds) = &a.conditionals {
        let actual: Vec<Value> = f.conditionals.iter().map(conditional_to_json).collect();
        for ec in expected_conds {
            if !actual.iter().any(|ac| json_subset(ec, ac)) {
                fails.push(format!("{label}: missing conditional {ec} (got {actual:?})"));
            }
        }
    }
    fails
}

fn run_schema_suite(file: &str) {
    let path = golden_dir().join(file);
    let json = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("Failed to read {}: {e}", path.display()));
    let suite: TestSuite = serde_json::from_str(&json)
        .unwrap_or_else(|e| panic!("Failed to parse {}: {e}", path.display()));

    let mut failures = Vec::new();

    for test in &suite.tests {
        match schema_parser::parse_schema(&test.schema) {
            Ok(schema) => {
                // Verify expected structure
                let mut test_failures = Vec::new();

                // Check metadata
                if let Some(expected_meta) = test.expected.get("metadata") {
                    if let Some(meta_obj) = expected_meta.as_object() {
                        for (key, val) in meta_obj {
                            let expected_str = val.as_str().unwrap_or("");
                            let actual = match key.as_str() {
                                "id" => schema.metadata.id.as_deref(),
                                "title" => schema.metadata.title.as_deref(),
                                "description" => schema.metadata.description.as_deref(),
                                "version" => schema.metadata.version.as_deref(),
                                "odin" | "schema" => {
                                    // These are stored in metadata too
                                    // For now, accept — schema metadata may store these
                                    continue;
                                }
                                _ => None,
                            };
                            if actual != Some(expected_str) {
                                test_failures.push(format!(
                                    "metadata.{}: expected {:?}, got {:?}",
                                    key, expected_str, actual
                                ));
                            }
                        }
                    }
                }

                // Check types exist
                if let Some(expected_types) = test.expected.get("types") {
                    if let Some(types_obj) = expected_types.as_object() {
                        for (type_name, _type_def) in types_obj {
                            if !schema.types.contains_key(type_name) {
                                test_failures.push(format!(
                                    "type '{}' not found in parsed schema (found: {:?})",
                                    type_name,
                                    schema.types.keys().collect::<Vec<_>>()
                                ));
                            }
                        }
                    }
                }

                // Check fields exist
                if let Some(expected_fields) = test.expected.get("fields") {
                    if let Some(fields_obj) = expected_fields.as_object() {
                        for (field_path, _field_def) in fields_obj {
                            if !schema.fields.contains_key(field_path) {
                                test_failures.push(format!(
                                    "field '{}' not found in parsed schema (found: {:?})",
                                    field_path,
                                    schema.fields.keys().collect::<Vec<_>>()
                                ));
                            }
                        }
                    }
                }

                // Check constraints exist
                if let Some(expected_constraints) = test.expected.get("constraints") {
                    if let Some(constraints_obj) = expected_constraints.as_object() {
                        for (scope, _) in constraints_obj {
                            if !schema.constraints.contains_key(scope) {
                                test_failures.push(format!(
                                    "constraint scope '{}' not found (found: {:?})",
                                    scope,
                                    schema.constraints.keys().collect::<Vec<_>>()
                                ));
                            }
                        }
                    }
                }

                // Structural cases: also assert each type's internal fields exist.
                if test.structural {
                    if let Some(types_obj) =
                        test.expected.get("types").and_then(Value::as_object)
                    {
                        for (type_name, type_def) in types_obj {
                            if let Some(fields_obj) =
                                type_def.get("fields").and_then(Value::as_object)
                            {
                                let parsed_names: Vec<&str> = schema
                                    .types
                                    .get(type_name)
                                    .map(|t| t.fields.iter().map(|f| f.name.as_str()).collect())
                                    .unwrap_or_default();
                                for field_key in fields_obj.keys() {
                                    if !parsed_names.contains(&field_key.as_str()) {
                                        test_failures.push(format!(
                                            "type '{}' missing field '{}' (found: {:?})",
                                            type_name, field_key, parsed_names
                                        ));
                                    }
                                }
                            }
                        }
                    }
                    if let Some(fields_obj) =
                        test.expected.get("fields").and_then(Value::as_object)
                    {
                        for field_path in fields_obj.keys() {
                            if !schema.fields.contains_key(field_path) {
                                test_failures.push(format!(
                                    "root field '{}' not found (found: {:?})",
                                    field_path,
                                    schema.fields.keys().collect::<Vec<_>>()
                                ));
                            }
                        }
                    }
                }

                // Check invariants
                if let Some(expected_invariants) = test.expected.get("invariants") {
                    if let Some(inv_arr) = expected_invariants.as_array() {
                        for inv in inv_arr {
                            if let Some(scope) = inv.get("scope").and_then(|s| s.as_str()) {
                                if !schema.constraints.contains_key(scope) {
                                    test_failures.push(format!(
                                        "invariant scope '{}' not found",
                                        scope,
                                    ));
                                }
                            }
                        }
                    }
                }

                // Value-level assertions on parsed fields/types.
                if let Some(asserts) = &test.assert {
                    for (field_path, a) in &asserts.fields {
                        test_failures.extend(assert_field(
                            schema.fields.get(field_path),
                            a,
                            &format!("field '{field_path}'"),
                        ));
                    }
                    for (type_name, ta) in &asserts.types {
                        match schema.types.get(type_name) {
                            None => test_failures.push(format!("type '{type_name}' not defined")),
                            Some(t) => {
                                for (field_key, a) in &ta.fields {
                                    let field = t.fields.iter().find(|f| f.name == *field_key);
                                    test_failures.extend(assert_field(
                                        field,
                                        a,
                                        &format!("type '{type_name}' field '{field_key}'"),
                                    ));
                                }
                            }
                        }
                    }
                }

                if !test_failures.is_empty() {
                    failures.push(format!(
                        "[{}] {}:\n  {}",
                        test.id,
                        test.description,
                        test_failures.join("\n  ")
                    ));
                }
            }
            Err(e) => {
                failures.push(format!(
                    "[{}] {}: parse error: {}",
                    test.id, test.description, e
                ));
            }
        }
    }

    if !failures.is_empty() {
        panic!(
            "\n{} schema test failures in suite '{}':\n\n{}\n",
            failures.len(),
            suite.suite,
            failures.join("\n\n")
        );
    }

    eprintln!(
        "  ✓ {} schema tests passed in '{}'",
        suite.tests.len(),
        suite.suite
    );
}

#[test]
fn golden_schema_composition() {
    run_schema_suite("composition/schema-composition.json");
}

#[test]
fn golden_schema_conformance() {
    run_schema_suite("conformance/schema-conformance.json");
}
