//! Golden canonical tests — reads test cases from sdk/golden/canonical/
//! and verifies that parse → canonicalize produces the expected output.
//!
//! Supports two expected formats:
//!   - String: compare decoded canonical text
//!   - Object { hex, sha256, byteLength }: compare raw binary output

use odin_core::Odin;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::path::PathBuf;

#[derive(Deserialize)]
struct TestSuite {
    suite: String,
    tests: Vec<TestCase>,
}

#[derive(Deserialize)]
struct TestCase {
    id: String,
    #[allow(dead_code)]
    description: Option<String>,
    input: String,
    expected: Expected,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Expected {
    Text(String),
    Binary(BinaryExpected),
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BinaryExpected {
    hex: String,
    sha256: String,
    byte_length: usize,
}

fn golden_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent().unwrap()  // sdk/rust/
        .parent().unwrap()  // sdk/
        .join("golden")
        .join("canonical")
}

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn run_canonical_suite(file: &str) {
    let path = golden_dir().join(file);
    let json = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("Failed to read {}: {e}", path.display()));
    let suite: TestSuite = serde_json::from_str(&json)
        .unwrap_or_else(|e| panic!("Failed to parse {}: {e}", path.display()));

    let mut failures = Vec::new();

    for test in &suite.tests {
        let doc = if test.input.is_empty() {
            Odin::empty()
        } else {
            match Odin::parse(&test.input) {
                Ok(d) => d,
                Err(e) => {
                    failures.push(format!("[{}] parse error: {e}", test.id));
                    continue;
                }
            }
        };

        let canonical = Odin::canonicalize(&doc);

        match &test.expected {
            Expected::Text(expected_text) => {
                let actual = String::from_utf8(canonical)
                    .unwrap_or_else(|e| {
                        panic!("[{}] canonical output not valid UTF-8: {e}", test.id)
                    });

                if actual != *expected_text {
                    failures.push(format!(
                        "[{}]\n  expected: {:?}\n  actual:   {:?}",
                        test.id, expected_text, actual
                    ));
                }
            }
            Expected::Binary(expected) => {
                // Check byte length
                if canonical.len() != expected.byte_length {
                    failures.push(format!(
                        "[{}] byteLength mismatch: expected {}, got {}",
                        test.id, expected.byte_length, canonical.len()
                    ));
                    continue;
                }

                // Check hex encoding
                let actual_hex = to_hex(&canonical);
                if actual_hex != expected.hex {
                    failures.push(format!(
                        "[{}] hex mismatch:\n  expected: {}\n  actual:   {}",
                        test.id, expected.hex, actual_hex
                    ));
                    continue;
                }

                // Check SHA-256
                let mut hasher = Sha256::new();
                hasher.update(&canonical);
                let actual_sha256 = format!("{:x}", hasher.finalize());
                if actual_sha256 != expected.sha256 {
                    failures.push(format!(
                        "[{}] sha256 mismatch:\n  expected: {}\n  actual:   {}",
                        test.id, expected.sha256, actual_sha256
                    ));
                }
            }
        }
    }

    if !failures.is_empty() {
        panic!(
            "\n{} canonical test failures in suite '{}':\n\n{}\n",
            failures.len(),
            suite.suite,
            failures.join("\n\n")
        );
    }

    eprintln!(
        "  ✓ {} canonical tests passed in '{}'",
        suite.tests.len(),
        suite.suite
    );
}

#[test]
fn golden_canonical_all_types() {
    run_canonical_suite("all-types.json");
}

#[test]
fn golden_canonical_binary_output() {
    run_canonical_suite("binary-output.json");
}

#[test]
fn golden_canonical_normalization() {
    run_canonical_suite("normalization.json");
}

#[test]
fn golden_canonical_tabular_expansion() {
    run_canonical_suite("tabular-expansion.json");
}
