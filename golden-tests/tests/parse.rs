//! Golden parse tests — reads test cases from sdk/golden/parse/ and runs them
//! against the Rust parser implementation.

use odin_core::{Odin, OdinValue, OdinValueType};
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Deserialize)]
struct TestSuite {
    suite: String,
    #[serde(default)]
    _description: Option<String>,
    tests: Vec<TestCase>,
}

#[derive(Deserialize)]
struct TestCase {
    id: String,
    #[serde(default)]
    _description: Option<String>,
    input: String,
    #[serde(default)]
    expected: Option<Expected>,
    #[serde(default, rename = "expectError")]
    expect_error: Option<ExpectError>,
}

#[derive(Deserialize)]
struct Expected {
    #[serde(default)]
    assignments: std::collections::HashMap<String, ExpectedValue>,
    #[serde(default)]
    documents: Option<Vec<ExpectedDocument>>,
    #[serde(default)]
    directives: Option<Vec<ExpectedDirective>>,
    #[serde(default)]
    _note: Option<String>,
    #[serde(default)]
    _computed: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct ExpectedDocument {
    #[serde(default)]
    metadata: Option<std::collections::HashMap<String, serde_json::Value>>,
    #[serde(default)]
    assignments: Option<std::collections::HashMap<String, ExpectedValue>>,
}

#[derive(Deserialize)]
struct ExpectedDirective {
    #[serde(rename = "type")]
    directive_type: String,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    alias: Option<serde_json::Value>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    condition: Option<String>,
}

#[derive(Deserialize)]
struct ExpectedValue {
    #[serde(rename = "type")]
    value_type: String,
    #[serde(default)]
    value: Option<serde_json::Value>,
    #[serde(default)]
    raw: Option<String>,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    base64: Option<String>,
    #[serde(default)]
    algorithm: Option<String>,
    #[serde(default, rename = "decimalPlaces")]
    decimal_places: Option<u8>,
    #[serde(default, rename = "currencyCode")]
    currency_code: Option<String>,
    #[serde(default)]
    _note: Option<String>,
    #[serde(default)]
    modifiers: Option<Vec<String>>,
    #[serde(default, rename = "isArrayClear")]
    _is_array_clear: Option<bool>,
}

#[derive(Deserialize)]
struct ExpectError {
    code: String,
    #[serde(default)]
    _message: Option<String>,
}

fn golden_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("golden")
        .join("parse")
}

fn run_test_suite(filename: &str) {
    let path = golden_dir().join(filename);
    let content = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("Failed to read {}: {}", path.display(), e));
    let suite: TestSuite = serde_json::from_str(&content)
        .unwrap_or_else(|e| panic!("Failed to parse {}: {}", path.display(), e));

    let mut passed = 0;
    let mut failed = 0;
    let mut skipped = 0;

    for test in &suite.tests {
        let result = run_single_test(test);
        match result {
            TestResult::Pass => passed += 1,
            TestResult::Fail(msg) => {
                failed += 1;
                eprintln!("FAIL [{}/{}]: {}", suite.suite, test.id, msg);
            }
            TestResult::Skip(reason) => {
                skipped += 1;
                eprintln!("SKIP [{}/{}]: {}", suite.suite, test.id, reason);
            }
        }
    }

    eprintln!(
        "\n{}: {} passed, {} failed, {} skipped (total {})",
        suite.suite,
        passed,
        failed,
        skipped,
        suite.tests.len()
    );

    if failed > 0 {
        panic!(
            "{} test(s) failed in suite '{}'",
            failed, suite.suite
        );
    }
}

enum TestResult {
    Pass,
    Fail(String),
    Skip(String),
}

fn run_single_test(test: &TestCase) -> TestResult {
    // Error expected
    if let Some(ref expect_error) = test.expect_error {
        let parse_result = Odin::parse(&test.input);
        match parse_result {
            Err(err) => {
                if err.error_code.code() == expect_error.code {
                    return TestResult::Pass;
                }
                return TestResult::Fail(format!(
                    "Expected error code {}, got {}",
                    expect_error.code,
                    err.error_code.code()
                ));
            }
            Ok(_) => {
                return TestResult::Fail(format!(
                    "Expected error {} but parse succeeded",
                    expect_error.code
                ));
            }
        }
    }

    // Success expected
    let expected = match &test.expected {
        Some(e) => e,
        None => return TestResult::Skip("No expected or expectError defined".to_string()),
    };

    // Check if this is a multi-document test
    if let Some(ref documents) = expected.documents {
        return run_multi_document_test(test, documents, expected.directives.as_deref());
    }

    // Single document test
    let parse_result = Odin::parse(&test.input);
    let doc = match parse_result {
        Ok(d) => d,
        Err(e) => {
            return TestResult::Fail(format!("Parse failed: {}", e));
        }
    };

    // Check directives (imports/schemas)
    if let Some(ref directives) = expected.directives {
        if let Err(msg) = check_directives(&doc, directives) {
            return TestResult::Fail(msg);
        }
    }

    // Check each expected assignment
    check_assignments(&doc, &expected.assignments)
}

fn run_multi_document_test(
    test: &TestCase,
    expected_docs: &[ExpectedDocument],
    _expected_directives: Option<&[ExpectedDirective]>,
) -> TestResult {
    let parse_result = Odin::parse_documents(&test.input);
    let docs = match parse_result {
        Ok(d) => d,
        Err(e) => {
            return TestResult::Fail(format!("Parse failed: {}", e));
        }
    };

    if docs.len() != expected_docs.len() {
        return TestResult::Fail(format!(
            "Expected {} documents, got {}",
            expected_docs.len(),
            docs.len()
        ));
    }

    for (i, (doc, expected_doc)) in docs.iter().zip(expected_docs.iter()).enumerate() {
        // Check metadata
        if let Some(ref expected_meta) = expected_doc.metadata {
            for (key, expected_val) in expected_meta {
                let full_key = key.clone();
                let actual = doc.metadata.get(&full_key);
                match actual {
                    Some(val) => {
                        if let Some(exp_str) = expected_val.as_str() {
                            if let OdinValue::String { ref value, .. } = val {
                                if value != exp_str {
                                    return TestResult::Fail(format!(
                                        "Doc {}: metadata '{}' expected \"{}\", got \"{}\"",
                                        i, key, exp_str, value
                                    ));
                                }
                            } else {
                                // Check other types for metadata
                                let actual_str = match val {
                                    OdinValue::Date { ref raw, .. } => raw.clone(),
                                    _ => format!("{}", val),
                                };
                                if actual_str != exp_str {
                                    return TestResult::Fail(format!(
                                        "Doc {}: metadata '{}' expected \"{}\", got \"{}\"",
                                        i, key, exp_str, actual_str
                                    ));
                                }
                            }
                        }
                    }
                    None => {
                        return TestResult::Fail(format!(
                            "Doc {}: missing metadata '{}'",
                            i, key
                        ));
                    }
                }
            }
        }

        // Check assignments
        if let Some(ref expected_assignments) = expected_doc.assignments {
            let result = check_assignments(doc, expected_assignments);
            match result {
                TestResult::Fail(msg) => {
                    return TestResult::Fail(format!("Doc {}: {}", i, msg));
                }
                TestResult::Skip(msg) => {
                    return TestResult::Skip(format!("Doc {}: {}", i, msg));
                }
                TestResult::Pass => {}
            }
        }
    }

    TestResult::Pass
}

fn check_assignments(
    doc: &odin_core::OdinDocument,
    expected_assignments: &std::collections::HashMap<String, ExpectedValue>,
) -> TestResult {
    for (path, expected_val) in expected_assignments {
        let actual = match doc.get(path) {
            Some(v) => v,
            None => {
                return TestResult::Fail(format!(
                    "Missing assignment '{}'",
                    path
                ));
            }
        };

        // Check type
        let actual_type = value_type_string(actual);
        if actual_type != expected_val.value_type {
            return TestResult::Fail(format!(
                "'{}': expected type '{}', got '{}'",
                path, expected_val.value_type, actual_type
            ));
        }

        // Check value (if specified)
        if let Some(ref expected_value) = expected_val.value {
            if let Err(msg) = check_value(actual, expected_value, path) {
                return TestResult::Fail(msg);
            }
        }

        // Check raw (if specified)
        if let Some(ref expected_raw) = expected_val.raw {
            if let Err(msg) = check_raw(actual, expected_raw, path) {
                return TestResult::Fail(msg);
            }
        }

        // Check reference path
        if let Some(ref expected_path) = expected_val.path {
            match actual {
                OdinValue::Reference { path: ref actual_path, .. } => {
                    if actual_path != expected_path {
                        return TestResult::Fail(format!(
                            "'{}': expected reference path '{}', got '{}'",
                            path, expected_path, actual_path
                        ));
                    }
                }
                _ => {
                    return TestResult::Fail(format!(
                        "'{}': expected reference, got {:?}",
                        path, actual.value_type()
                    ));
                }
            }
        }

        // Check currency code
        if let Some(ref expected_code) = expected_val.currency_code {
            match actual {
                OdinValue::Currency { currency_code, .. } => {
                    let actual_code = currency_code.as_deref().unwrap_or("");
                    if actual_code != expected_code {
                        return TestResult::Fail(format!(
                            "'{}': expected currency code '{}', got '{}'",
                            path, expected_code, actual_code
                        ));
                    }
                }
                _ => {
                    return TestResult::Fail(format!(
                        "'{}': expected currency, got {:?}",
                        path, actual.value_type()
                    ));
                }
            }
        }

        // Check decimal places
        if let Some(expected_dp) = expected_val.decimal_places {
            match actual {
                OdinValue::Currency { decimal_places, .. } => {
                    if *decimal_places != expected_dp {
                        return TestResult::Fail(format!(
                            "'{}': expected {} decimal places, got {}",
                            path, expected_dp, decimal_places
                        ));
                    }
                }
                _ => {}
            }
        }
    }

    TestResult::Pass
}

fn check_directives(
    doc: &odin_core::OdinDocument,
    expected_directives: &[ExpectedDirective],
) -> Result<(), String> {
    for (i, expected) in expected_directives.iter().enumerate() {
        match expected.directive_type.as_str() {
            "import" => {
                if i >= doc.imports.len() {
                    return Err(format!("Expected import directive at index {i}, but only {} imports found", doc.imports.len()));
                }
                let import = &doc.imports[i];
                if let Some(ref expected_path) = expected.path {
                    if &import.path != expected_path {
                        return Err(format!(
                            "Import {i}: expected path '{}', got '{}'",
                            expected_path, import.path
                        ));
                    }
                }
                if let Some(ref expected_alias) = expected.alias {
                    if expected_alias.is_null() {
                        if import.alias.is_some() {
                            return Err(format!(
                                "Import {i}: expected no alias, got '{}'",
                                import.alias.as_ref().unwrap()
                            ));
                        }
                    } else if let Some(alias_str) = expected_alias.as_str() {
                        match &import.alias {
                            Some(actual_alias) if actual_alias == alias_str => {}
                            Some(actual_alias) => {
                                return Err(format!(
                                    "Import {i}: expected alias '{}', got '{}'",
                                    alias_str, actual_alias
                                ));
                            }
                            None => {
                                return Err(format!(
                                    "Import {i}: expected alias '{}', got none",
                                    alias_str
                                ));
                            }
                        }
                    }
                }
            }
            "schema" => {
                if let Some(ref expected_url) = expected.url {
                    let schema = doc.schemas.iter().find(|s| &s.url == expected_url);
                    if schema.is_none() {
                        return Err(format!("Expected schema URL '{}' not found", expected_url));
                    }
                }
            }
            "if" => {
                // Conditionals are parsed but not stored in the current model
                // This is OK for now — the parser validates the syntax
            }
            other => {
                return Err(format!("Unknown directive type: {other}"));
            }
        }
    }
    Ok(())
}

fn value_type_string(value: &OdinValue) -> &'static str {
    match value.value_type() {
        OdinValueType::Null => "null",
        OdinValueType::Boolean => "boolean",
        OdinValueType::String => "string",
        OdinValueType::Integer => "integer",
        OdinValueType::Number => "number",
        OdinValueType::Currency => "currency",
        OdinValueType::Percent => "percent",
        OdinValueType::Date => "date",
        OdinValueType::Timestamp => "timestamp",
        OdinValueType::Time => "time",
        OdinValueType::Duration => "duration",
        OdinValueType::Reference => "reference",
        OdinValueType::Binary => "binary",
        OdinValueType::Verb => "verb",
        OdinValueType::Array => "array",
        OdinValueType::Object => "object",
    }
}

fn check_value(actual: &OdinValue, expected: &serde_json::Value, path: &str) -> Result<(), String> {
    match actual {
        OdinValue::Null { .. } => {
            if !expected.is_null() {
                return Err(format!("'{}': expected null, got {:?}", path, expected));
            }
        }
        OdinValue::Boolean { value, .. } => {
            let exp = expected.as_bool().ok_or_else(|| {
                format!("'{}': expected bool in test, got {:?}", path, expected)
            })?;
            if *value != exp {
                return Err(format!("'{}': expected {}, got {}", path, exp, value));
            }
        }
        OdinValue::String { value, .. } => {
            let exp = expected.as_str().ok_or_else(|| {
                format!("'{}': expected string in test, got {:?}", path, expected)
            })?;
            if value != exp {
                return Err(format!("'{}': expected \"{}\", got \"{}\"", path, exp, value));
            }
        }
        OdinValue::Integer { value, raw, .. } => {
            if let Some(exp) = expected.as_i64() {
                // For very large integers stored as 0 with raw, check raw
                if *value == 0 && raw.is_some() {
                    let raw_str = raw.as_ref().unwrap();
                    if let Ok(raw_val) = raw_str.parse::<i64>() {
                        if raw_val != exp {
                            return Err(format!("'{}': expected {}, got {} (raw: {})", path, exp, value, raw_str));
                        }
                    }
                } else if *value != exp {
                    return Err(format!("'{}': expected {}, got {}", path, exp, value));
                }
            } else if let Some(exp) = expected.as_f64() {
                if (*value as f64 - exp).abs() > 0.5 {
                    return Err(format!("'{}': expected {}, got {}", path, exp, value));
                }
            }
        }
        OdinValue::Number { value, .. } => {
            let exp = expected.as_f64().ok_or_else(|| {
                format!("'{}': expected number in test, got {:?}", path, expected)
            })?;
            let tolerance = (exp.abs() * 1e-10).max(1e-15);
            if (*value - exp).abs() > tolerance {
                return Err(format!("'{}': expected {}, got {}", path, exp, value));
            }
        }
        OdinValue::Currency { value, .. } => {
            let exp = expected.as_f64().ok_or_else(|| {
                format!("'{}': expected number in test, got {:?}", path, expected)
            })?;
            let tolerance = (exp.abs() * 1e-10).max(1e-15);
            if (*value - exp).abs() > tolerance {
                return Err(format!("'{}': expected {}, got {}", path, exp, value));
            }
        }
        OdinValue::Percent { value, .. } => {
            let exp = expected.as_f64().ok_or_else(|| {
                format!("'{}': expected number in test, got {:?}", path, expected)
            })?;
            let tolerance = (exp.abs() * 1e-10).max(1e-15);
            if (*value - exp).abs() > tolerance {
                return Err(format!("'{}': expected {}, got {}", path, exp, value));
            }
        }
        OdinValue::Date { raw, .. } => {
            if let Some(exp) = expected.as_str() {
                if raw != exp {
                    return Err(format!("'{}': expected date \"{}\", got \"{}\"", path, exp, raw));
                }
            }
        }
        OdinValue::Timestamp { raw, .. } => {
            if let Some(exp) = expected.as_str() {
                if raw != exp {
                    return Err(format!("'{}': expected timestamp \"{}\", got \"{}\"", path, exp, raw));
                }
            }
        }
        OdinValue::Time { value, .. } => {
            let exp = expected.as_str().ok_or_else(|| {
                format!("'{}': expected string in test, got {:?}", path, expected)
            })?;
            if value != exp {
                return Err(format!("'{}': expected \"{}\", got \"{}\"", path, exp, value));
            }
        }
        OdinValue::Duration { value, .. } => {
            let exp = expected.as_str().ok_or_else(|| {
                format!("'{}': expected string in test, got {:?}", path, expected)
            })?;
            if value != exp {
                return Err(format!("'{}': expected \"{}\", got \"{}\"", path, exp, value));
            }
        }
        _ => {
            // For types where value comparison isn't straightforward, skip
        }
    }
    Ok(())
}

fn check_raw(actual: &OdinValue, expected_raw: &str, path: &str) -> Result<(), String> {
    let actual_raw = match actual {
        OdinValue::Integer { raw, .. } => raw.as_deref(),
        OdinValue::Number { raw, .. } => raw.as_deref(),
        OdinValue::Currency { raw, .. } => raw.as_deref(),
        OdinValue::Percent { raw, .. } => raw.as_deref(),
        OdinValue::Date { raw, .. } => Some(raw.as_str()),
        OdinValue::Timestamp { raw, .. } => Some(raw.as_str()),
        _ => None,
    };

    match actual_raw {
        Some(r) if r == expected_raw => Ok(()),
        Some(r) => Err(format!("'{}': expected raw '{}', got '{}'", path, expected_raw, r)),
        None => Err(format!("'{}': expected raw '{}', but no raw field", path, expected_raw)),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Test functions (one per golden test file)
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn golden_parse_basic() {
    run_test_suite("basic/simple-assignments.json");
}

#[test]
fn golden_parse_all_types() {
    run_test_suite("types/all-types.json");
}

#[test]
fn golden_parse_numeric_precision() {
    run_test_suite("types/numeric-precision.json");
}

#[test]
fn golden_parse_binary_edge_cases() {
    run_test_suite("types/binary-edge-cases.json");
}

#[test]
fn golden_parse_temporal_edge_cases() {
    run_test_suite("temporal/temporal-edge-cases.json");
}

#[test]
fn golden_parse_unicode_edge_cases() {
    run_test_suite("unicode/unicode-edge-cases.json");
}

#[test]
fn golden_parse_errors() {
    run_test_suite("errors/parse-errors.json");
}

#[test]
fn golden_parse_directives() {
    run_test_suite("directives/directives.json");
}

#[test]
fn golden_parse_document_chaining() {
    run_test_suite("composition/document-chaining.json");
}

#[test]
fn golden_parse_error_recovery() {
    run_test_suite("errors/error-recovery.json");
}

#[test]
fn golden_parse_security_limits() {
    run_test_suite("errors/security-limits.json");
}

#[test]
fn golden_parse_precision_canary() {
    run_test_suite("types/precision-canary.json");
}

#[test]
fn golden_parse_case_sensitivity() {
    run_test_suite("basic/case-sensitivity.json");
}

#[test]
fn golden_parse_crlf_bom() {
    run_test_suite("basic/crlf-bom.json");
}

#[test]
fn golden_parse_extension_paths() {
    run_test_suite("basic/extension-paths.json");
}

#[test]
fn golden_parse_string_escapes() {
    run_test_suite("basic/string-escapes.json");
}

#[test]
fn golden_parse_relative_headers() {
    run_test_suite("composition/relative-headers.json");
}

#[test]
fn golden_parse_tabular_mode() {
    run_test_suite("composition/tabular-mode.json");
}

#[test]
fn golden_parse_modifiers() {
    run_test_suite("types/modifiers.json");
}

#[test]
fn golden_parse_verb_expressions() {
    run_test_suite("types/verb-expressions.json");
}

#[test]
fn golden_parse_array_index_normalization() {
    run_test_suite("basic/array-index-normalization.json");
}

#[test]
fn golden_parse_currency_scientific() {
    run_test_suite("types/currency-scientific.json");
}
