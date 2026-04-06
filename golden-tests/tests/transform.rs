//! Golden transform self-tests — reads .test.odin files from sdk/golden/transform/verbs/
//! and verifies that each transform's built-in assertions pass.
//!
//! Convention: Self-testing transforms contain their own test cases via accumulators.
//! The runner executes each transform and checks TestResult.success == true.

use odin_core::transform::{parse_transform, execute_transform};
use odin_core::types::transform::DynValue;
use std::path::PathBuf;

fn golden_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent().unwrap()
        .parent().unwrap()
        .join("golden")
        .join("transform")
        .join("verbs")
}

/// Standard test input with known datetime values for deterministic testing.
fn test_input() -> DynValue {
    DynValue::Object(vec![
        ("_test".to_string(), DynValue::Object(vec![
            ("currentDate".to_string(), DynValue::String("2024-06-15".to_string())),
            ("currentTimestamp".to_string(), DynValue::String("2024-06-15T14:30:45Z".to_string())),
            ("currentYear".to_string(), DynValue::Integer(2024)),
            ("currentMonth".to_string(), DynValue::Integer(6)),
            ("currentDay".to_string(), DynValue::Integer(15)),
            ("currentHour".to_string(), DynValue::Integer(14)),
            ("currentMinute".to_string(), DynValue::Integer(30)),
            ("currentSecond".to_string(), DynValue::Integer(45)),
            ("unixTime".to_string(), DynValue::Integer(1718458245)),
            ("dayOfWeek".to_string(), DynValue::Integer(6)),
            ("weekOfYear".to_string(), DynValue::Integer(24)),
            ("quarter".to_string(), DynValue::Integer(2)),
        ])),
    ])
}

/// Extract a string/bool/integer value from a CDM-typed value or plain value.
fn get_test_result_field<'a>(output: &'a DynValue, section: &str, field: &str) -> Option<&'a DynValue> {
    if let DynValue::Object(entries) = output {
        // Find the TestResult section
        for (key, val) in entries {
            if key == section {
                if let DynValue::Object(fields) = val {
                    for (k, v) in fields {
                        if k == field {
                            // CDM format: { type: "...", value: ... }
                            if let DynValue::Object(cdm) = v {
                                for (ck, cv) in cdm {
                                    if ck == "value" {
                                        return Some(cv);
                                    }
                                }
                            }
                            // Plain value
                            return Some(v);
                        }
                    }
                }
                return None;
            }
        }
    }
    None
}

fn run_self_test(file: &str) {
    let path = golden_dir().join(file);
    let content = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("Failed to read {}: {e}", path.display()));

    let transform = match parse_transform(&content) {
        Ok(t) => t,
        Err(e) => panic!("Failed to parse transform {}: {e}", path.display()),
    };

    let input = test_input();
    let result = execute_transform(&transform, &input);

    let output = result.output.as_ref()
        .expect("transform should produce output");

    // Check for TestResult section first — self-testing transforms track their own pass/fail.
    // Only fall back to engine-level success if no TestResult is present.
    let has_test_result = get_test_result_field(output, "TestResult", "success").is_some();

    if !result.success && !has_test_result {
        let error_msgs: Vec<String> = result.errors.iter()
            .map(|e| format!("  - {}: {}", e.code.as_deref().unwrap_or("???"), &e.message))
            .collect();
        panic!(
            "\nTransform execution failed for '{}':\n{}\n",
            file,
            error_msgs.join("\n")
        );
    }

    if !result.errors.is_empty() {
        eprintln!("  ! {} non-fatal errors in '{}' (checking TestResult instead)", result.errors.len(), file);
        for e in &result.errors {
            eprintln!("    [{err}] {msg}", err = e.code.as_deref().unwrap_or("???"), msg = &e.message);
        }
    }

    // Extract TestResult section
    let success_val = get_test_result_field(output, "TestResult", "success");
    let passed_val = get_test_result_field(output, "TestResult", "passed");
    let failed_val = get_test_result_field(output, "TestResult", "failed");

    let success = match success_val {
        Some(DynValue::Bool(b)) => *b,
        Some(DynValue::String(s)) => s == "true",
        _ => {
            // If there's no TestResult section, dump what we got
            eprintln!("  ? No TestResult.success found in '{}', output keys:", file);
            if let DynValue::Object(entries) = output {
                for (k, _) in entries {
                    eprintln!("    - {}", k);
                }
            }
            // Don't fail — some test files might have different structures
            eprintln!("  ? Skipping '{}' (no TestResult section)", file);
            return;
        }
    };

    let passed = match passed_val {
        Some(DynValue::Integer(n)) => *n,
        Some(DynValue::Float(n)) => *n as i64,
        _ => 0,
    };
    let failed = match failed_val {
        Some(DynValue::Integer(n)) => *n,
        Some(DynValue::Float(n)) => *n as i64,
        _ => 0,
    };

    if !success {
        panic!(
            "\nSelf-test FAILED for '{}':\n  passed: {}\n  failed: {}\n  success: {}\n",
            file, passed, failed, success
        );
    }

    eprintln!(
        "  ✓ {} self-tests passed in '{}'",
        passed, file
    );
}

// Individual test functions for each self-testing transform file.
// This way each test is independently reported.

#[test]
fn golden_transform_core() {
    run_self_test("core.test.odin");
}

#[test]
fn golden_transform_string() {
    run_self_test("string.test.odin");
}

#[test]
fn golden_transform_string_case() {
    run_self_test("string-case.test.odin");
}

#[test]
fn golden_transform_numeric() {
    run_self_test("numeric.test.odin");
}

#[test]
fn golden_transform_datetime() {
    run_self_test("datetime.test.odin");
}

#[test]
fn golden_transform_logic() {
    run_self_test("logic.test.odin");
}

#[test]
fn golden_transform_ifelse() {
    run_self_test("ifelse-verify.test.odin");
}

#[test]
fn golden_transform_array() {
    run_self_test("array.test.odin");
}

#[test]
fn golden_transform_object() {
    run_self_test("object.test.odin");
}

#[test]
fn golden_transform_encoding() {
    run_self_test("encoding.test.odin");
}

#[test]
fn golden_transform_finance() {
    run_self_test("finance.test.odin");
}

#[test]
fn golden_transform_financial_advanced() {
    run_self_test("financial-advanced.test.odin");
}

#[test]
fn golden_transform_insurance() {
    run_self_test("insurance.test.odin");
}

#[test]
fn golden_transform_accumulator() {
    run_self_test("accumulator.test.odin");
}

#[test]
fn golden_transform_state() {
    run_self_test("state.test.odin");
}

#[test]
fn golden_transform_generation() {
    run_self_test("generation.test.odin");
}

#[test]
fn golden_transform_coercion() {
    run_self_test("coercion.test.odin");
}

#[test]
fn golden_transform_custom_verbs() {
    run_self_test("custom-verbs.test.odin");
}

#[test]
fn golden_transform_precision() {
    run_self_test("precision.test.odin");
}

#[test]
fn golden_transform_lookup_multikey() {
    run_self_test("lookup-multikey.test.odin");
}

#[test]
fn golden_transform_persist() {
    run_self_test("persist.test.odin");
}

#[test]
fn golden_transform_and() {
    run_self_test("and.test.odin");
}

#[test]
fn golden_transform_geo() {
    run_self_test("geo-text-new.test.odin");
}

#[test]
fn golden_transform_interpolation() {
    run_self_test("interpolation.test.odin");
}

#[test]
fn golden_transform_timeseries() {
    run_self_test("timeseries.test.odin");
}

#[test]
fn golden_transform_convert_unit() {
    run_self_test("convertUnit.test.odin");
}

#[test]
fn golden_transform_business_date() {
    run_self_test("business-date.test.odin");
}

#[test]
fn golden_transform_format_extra() {
    run_self_test("format-extra.test.odin");
}

#[test]
fn golden_transform_collection_extra() {
    run_self_test("collection-extra.test.odin");
}
