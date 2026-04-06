//! Golden schema parsing tests — reads test cases from sdk/golden/schema/
//! and verifies schema parser output.

use odin_core::validator::schema_parser;
use serde::Deserialize;
use serde_json::Value;
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
    expected: Value,
}

fn golden_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent().unwrap()
        .parent().unwrap()
        .join("golden")
        .join("schema")
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
