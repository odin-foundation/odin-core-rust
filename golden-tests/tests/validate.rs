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
