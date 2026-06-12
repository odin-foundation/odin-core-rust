//! Golden format validation tests — reads test cases from sdk/golden/validate/formats/
//! and verifies format constraint validation.

use odin_core::Odin;
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Deserialize)]
struct TestSuite {
    suite: String,
    tests: Vec<TestCase>,
}

#[derive(Deserialize)]
struct TestCase {
    id: String,
    schema: String,
    input: String,
    expected: ExpectedValidation,
}

#[derive(Deserialize)]
struct ExpectedValidation {
    valid: bool,
    #[serde(default)]
    error: Option<String>,
}

fn golden_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent().unwrap()
        .parent().unwrap()
        .join("golden")
        .join("validate")
        .join("formats")
}

fn run_validate_suite(file: &str) {
    let path = golden_dir().join(file);
    let json = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("Failed to read {}: {e}", path.display()));
    let suite: TestSuite = serde_json::from_str(&json)
        .unwrap_or_else(|e| panic!("Failed to parse {}: {e}", path.display()));

    let mut failures = Vec::new();

    for test in &suite.tests {
        let doc = match Odin::parse(&test.input) {
            Ok(d) => d,
            Err(e) => {
                failures.push(format!("[{}] parse error: {e}", test.id));
                continue;
            }
        };

        let errors = odin_core::validator::validate_formats(&doc, &test.schema);

        if test.expected.valid {
            if !errors.is_empty() {
                failures.push(format!(
                    "[{}] expected valid but got errors: {:?}",
                    test.id, errors
                ));
            }
        } else {
            if errors.is_empty() {
                failures.push(format!(
                    "[{}] expected invalid but got valid",
                    test.id
                ));
                continue;
            }
            if let Some(ref expected_msg) = test.expected.error {
                let has_match = errors.iter().any(|(_, msg)| msg == expected_msg);
                if !has_match {
                    failures.push(format!(
                        "[{}] expected error {:?} but got {:?}",
                        test.id, expected_msg,
                        errors.iter().map(|(_, m)| m.as_str()).collect::<Vec<_>>()
                    ));
                }
            }
        }
    }

    if !failures.is_empty() {
        panic!(
            "\n{} validate test failures in suite '{}':\n\n{}\n",
            failures.len(),
            suite.suite,
            failures.join("\n\n")
        );
    }

    eprintln!(
        "  ✓ {} validate tests passed in '{}'",
        suite.tests.len(),
        suite.suite
    );
}

#[test]
fn golden_validate_format_validators() {
    run_validate_suite("format-validators.json");
}

#[test]
fn golden_validate_format_validators_extended() {
    run_format_code_suite("format-validators-extended.json");
}

// ── Format suite asserting error codes via the full validate() pipeline ───────

#[derive(Deserialize)]
struct CodeSuite {
    suite: String,
    tests: Vec<CodeCase>,
}

#[derive(Deserialize)]
struct CodeCase {
    id: String,
    schema: String,
    input: String,
    expected: CodeExpected,
}

#[derive(Deserialize)]
struct CodeExpected {
    valid: bool,
    #[serde(default)]
    errors: Vec<ExpectedError>,
}

/// Run a format suite through `validate()`, asserting the overall valid flag and
/// every declared `(code, path)` error.
fn run_format_code_suite(file: &str) {
    let path = golden_dir().join(file);
    let json = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("Failed to read {}: {e}", path.display()));
    let suite: CodeSuite = serde_json::from_str(&json)
        .unwrap_or_else(|e| panic!("Failed to parse {}: {e}", path.display()));

    let mut failures = Vec::new();

    for test in &suite.tests {
        let schema = match odin_core::validator::parse_schema(&test.schema) {
            Ok(s) => s,
            Err(e) => {
                failures.push(format!("[{}] schema parse error: {e}", test.id));
                continue;
            }
        };
        let doc = match Odin::parse(&test.input) {
            Ok(d) => d,
            Err(e) => {
                failures.push(format!("[{}] input parse error: {e}", test.id));
                continue;
            }
        };

        let result = odin_core::validator::validate(&doc, &schema, None);

        if result.valid != test.expected.valid {
            failures.push(format!(
                "[{}] expected valid={}, got valid={} (errors: {:?})",
                test.id,
                test.expected.valid,
                result.valid,
                result.errors.iter().map(|e| format!("{}@{}", e.code(), e.path)).collect::<Vec<_>>()
            ));
            continue;
        }

        for expected in &test.expected.errors {
            let found = result.errors.iter().any(|e| {
                e.code() == expected.code
                    && expected.path.as_ref().map_or(true, |p| &e.path == p)
            });
            if !found {
                failures.push(format!(
                    "[{}] missing error code={} path={:?} (got: {:?})",
                    test.id,
                    expected.code,
                    expected.path,
                    result.errors.iter().map(|e| format!("{}@{}", e.code(), e.path)).collect::<Vec<_>>()
                ));
            }
        }
    }

    if !failures.is_empty() {
        panic!(
            "\n{} format-code failures in suite '{}':\n\n{}\n",
            failures.len(),
            suite.suite,
            failures.join("\n\n")
        );
    }

    eprintln!("  ✓ {} format-code tests passed in '{}'", suite.tests.len(), suite.suite);
}

// ── Full-validate conformance suite ──────────────────────────────────────────

#[derive(Deserialize)]
struct ConformanceSuite {
    suite: String,
    tests: Vec<ConformanceCase>,
}

#[derive(Deserialize)]
struct ConformanceCase {
    id: String,
    schema: String,
    input: String,
    expected: ConformanceExpected,
}

#[derive(Deserialize)]
struct ConformanceExpected {
    valid: bool,
    #[serde(default)]
    errors: Vec<ExpectedError>,
}

#[derive(Deserialize)]
struct ExpectedError {
    code: String,
    #[serde(default)]
    path: Option<String>,
}

fn validate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent().unwrap()
        .parent().unwrap()
        .join("golden")
        .join("validate")
}

/// Run a suite that drives the full `validate()` pipeline, asserting both the
/// overall valid flag and that every declared error (code, optional path) is
/// present.
fn run_conformance_suite(rel: &str) {
    let path = validate_dir().join(rel);
    let json = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("Failed to read {}: {e}", path.display()));
    let suite: ConformanceSuite = serde_json::from_str(&json)
        .unwrap_or_else(|e| panic!("Failed to parse {}: {e}", path.display()));

    let mut failures = Vec::new();

    for test in &suite.tests {
        let schema = match odin_core::validator::parse_schema(&test.schema) {
            Ok(s) => s,
            Err(e) => {
                failures.push(format!("[{}] schema parse error: {e}", test.id));
                continue;
            }
        };
        let doc = match Odin::parse(&test.input) {
            Ok(d) => d,
            Err(e) => {
                failures.push(format!("[{}] input parse error: {e}", test.id));
                continue;
            }
        };

        let result = odin_core::validator::validate(&doc, &schema, None);

        if result.valid != test.expected.valid {
            failures.push(format!(
                "[{}] expected valid={}, got valid={} (errors: {:?})",
                test.id,
                test.expected.valid,
                result.valid,
                result.errors.iter().map(|e| format!("{}@{}", e.code(), e.path)).collect::<Vec<_>>()
            ));
            continue;
        }

        for expected in &test.expected.errors {
            let found = result.errors.iter().any(|e| {
                e.code() == expected.code
                    && expected.path.as_ref().map_or(true, |p| &e.path == p)
            });
            if !found {
                failures.push(format!(
                    "[{}] missing error code={} path={:?} (got: {:?})",
                    test.id,
                    expected.code,
                    expected.path,
                    result.errors.iter().map(|e| format!("{}@{}", e.code(), e.path)).collect::<Vec<_>>()
                ));
            }
        }
    }

    if !failures.is_empty() {
        panic!(
            "\n{} conformance failures in suite '{}':\n\n{}\n",
            failures.len(),
            suite.suite,
            failures.join("\n\n")
        );
    }

    eprintln!("  ✓ {} conformance tests passed in '{}'", suite.tests.len(), suite.suite);
}

#[test]
fn golden_validate_conformance() {
    run_conformance_suite("conformance/validate-conformance.json");
}

#[test]
fn golden_validate_conditional_binary_decimal() {
    run_conformance_suite("conformance/conditional-binary-decimal.json");
}

#[test]
fn golden_validate_schema_definition() {
    run_conformance_suite("conformance/schema-definition.json");
}

#[test]
fn golden_validate_array_of_type_required() {
    run_conformance_suite("conformance/array-of-type-required.json");
}
