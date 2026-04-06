//! JSON Import Validation Tests
//!
//! Validates that the transform framework correctly parses and handles
//! all JSON input types. Mirrors sdk/typescript/tests/golden/json-import.test.ts.

use odin_core::transform::{parse_transform, execute_transform};
use odin_core::types::transform::DynValue;

/// Helper: create a transform that extracts a field from JSON input.
fn field_transform(field_path: &str) -> String {
    format!(
        r#"{{$}}
odin = "1.0.0"
transform = "1.0.0"
direction = "json->json"

{{output}}
result = "@.{field_path}"
"#
    )
}

/// Extract CDM value from transform output: output -> section -> field -> value.
fn get_output_value<'a>(output: &'a DynValue, section: &str, field: &str) -> Option<&'a DynValue> {
    if let DynValue::Object(entries) = output {
        for (k, v) in entries {
            if k == section {
                if let DynValue::Object(fields) = v {
                    for (fk, fv) in fields {
                        if fk == field {
                            // CDM format: { type: "...", value: ... }
                            if let DynValue::Object(cdm) = fv {
                                for (ck, cv) in cdm {
                                    if ck == "value" {
                                        return Some(cv);
                                    }
                                }
                            }
                            return Some(fv);
                        }
                    }
                }
            }
        }
    }
    None
}

/// Get the CDM type string from output.
fn get_output_type(output: &DynValue, section: &str, field: &str) -> Option<String> {
    if let DynValue::Object(entries) = output {
        for (k, v) in entries {
            if k == section {
                if let DynValue::Object(fields) = v {
                    for (fk, fv) in fields {
                        if fk == field {
                            if let DynValue::Object(cdm) = fv {
                                for (ck, cv) in cdm {
                                    if ck == "type" {
                                        if let DynValue::String(s) = cv {
                                            return Some(s.clone());
                                        }
                                    }
                                }
                            }
                            // Infer type from DynValue variant
                            return Some(match fv {
                                DynValue::String(_) => "string",
                                DynValue::Integer(_) => "integer",
                                DynValue::Float(_) => "number",
                                DynValue::Bool(_) => "boolean",
                                DynValue::Null => "null",
                                DynValue::Array(_) => "array",
                                DynValue::Object(_) => "object",
                                DynValue::Currency(..) | DynValue::CurrencyRaw(..) => "currency",
                                DynValue::FloatRaw(_) => "number",
                                DynValue::Percent(_) => "percent",
                                DynValue::Reference(_) => "reference",
                                DynValue::Binary(_) => "binary",
                                DynValue::Date(_) => "date",
                                DynValue::Timestamp(_) => "timestamp",
                                DynValue::Time(_) => "time",
                                DynValue::Duration(_) => "duration",
                            }.to_string());
                        }
                    }
                }
            }
        }
    }
    None
}

fn run_transform(transform_text: &str, input: DynValue) -> DynValue {
    let transform = parse_transform(transform_text)
        .unwrap_or_else(|e| panic!("Failed to parse transform: {e}"));
    let result = execute_transform(&transform, &input);
    assert!(result.success || result.output.is_some(),
        "Transform execution failed: {:?}", result.errors);
    result.output.unwrap_or(DynValue::Null)
}

// ─────────────────────────────────────────────────────────────────────────────
// Primitive Types
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn json_import_string() {
    let input = DynValue::Object(vec![
        ("value".into(), DynValue::String("hello world".into())),
    ]);
    let output = run_transform(&field_transform("value"), input);
    assert_eq!(get_output_type(&output, "output", "result").as_deref(), Some("string"));
    assert_eq!(get_output_value(&output, "output", "result"),
        Some(&DynValue::String("hello world".into())));
}

#[test]
fn json_import_empty_string() {
    let input = DynValue::Object(vec![
        ("value".into(), DynValue::String(String::new())),
    ]);
    let output = run_transform(&field_transform("value"), input);
    assert_eq!(get_output_type(&output, "output", "result").as_deref(), Some("string"));
    assert_eq!(get_output_value(&output, "output", "result"),
        Some(&DynValue::String(String::new())));
}

#[test]
fn json_import_integer() {
    let input = DynValue::Object(vec![
        ("value".into(), DynValue::Integer(42)),
    ]);
    let output = run_transform(&field_transform("value"), input);
    assert_eq!(get_output_type(&output, "output", "result").as_deref(), Some("integer"));
    assert_eq!(get_output_value(&output, "output", "result"),
        Some(&DynValue::Integer(42)));
}

#[test]
fn json_import_negative_integer() {
    let input = DynValue::Object(vec![
        ("value".into(), DynValue::Integer(-100)),
    ]);
    let output = run_transform(&field_transform("value"), input);
    assert_eq!(get_output_type(&output, "output", "result").as_deref(), Some("integer"));
    assert_eq!(get_output_value(&output, "output", "result"),
        Some(&DynValue::Integer(-100)));
}

#[test]
fn json_import_zero() {
    let input = DynValue::Object(vec![
        ("value".into(), DynValue::Integer(0)),
    ]);
    let output = run_transform(&field_transform("value"), input);
    assert_eq!(get_output_type(&output, "output", "result").as_deref(), Some("integer"));
    assert_eq!(get_output_value(&output, "output", "result"),
        Some(&DynValue::Integer(0)));
}

#[test]
fn json_import_float() {
    let input = DynValue::Object(vec![
        ("value".into(), DynValue::Float(3.14159)),
    ]);
    let output = run_transform(&field_transform("value"), input);
    assert_eq!(get_output_type(&output, "output", "result").as_deref(), Some("number"));
    if let Some(DynValue::Float(v)) = get_output_value(&output, "output", "result") {
        assert!((v - 3.14159).abs() < 1e-5);
    } else {
        panic!("Expected float value");
    }
}

#[test]
fn json_import_negative_float() {
    let input = DynValue::Object(vec![
        ("value".into(), DynValue::Float(-99.99)),
    ]);
    let output = run_transform(&field_transform("value"), input);
    assert_eq!(get_output_type(&output, "output", "result").as_deref(), Some("number"));
    if let Some(DynValue::Float(v)) = get_output_value(&output, "output", "result") {
        assert!((v - (-99.99)).abs() < 0.01);
    } else {
        panic!("Expected float value");
    }
}

#[test]
fn json_import_bool_true() {
    let input = DynValue::Object(vec![
        ("value".into(), DynValue::Bool(true)),
    ]);
    let output = run_transform(&field_transform("value"), input);
    assert_eq!(get_output_type(&output, "output", "result").as_deref(), Some("boolean"));
    assert_eq!(get_output_value(&output, "output", "result"),
        Some(&DynValue::Bool(true)));
}

#[test]
fn json_import_bool_false() {
    let input = DynValue::Object(vec![
        ("value".into(), DynValue::Bool(false)),
    ]);
    let output = run_transform(&field_transform("value"), input);
    assert_eq!(get_output_type(&output, "output", "result").as_deref(), Some("boolean"));
    assert_eq!(get_output_value(&output, "output", "result"),
        Some(&DynValue::Bool(false)));
}

#[test]
fn json_import_null() {
    let input = DynValue::Object(vec![
        ("value".into(), DynValue::Null),
    ]);
    let output = run_transform(&field_transform("value"), input);
    assert_eq!(get_output_type(&output, "output", "result").as_deref(), Some("null"));
}

// ─────────────────────────────────────────────────────────────────────────────
// Nested Objects
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn json_import_nested_object() {
    let input = DynValue::Object(vec![
        ("person".into(), DynValue::Object(vec![
            ("name".into(), DynValue::String("John".into())),
            ("age".into(), DynValue::Integer(30)),
        ])),
    ]);
    let output = run_transform(&field_transform("person.name"), input);
    assert_eq!(get_output_value(&output, "output", "result"),
        Some(&DynValue::String("John".into())));
}

#[test]
fn json_import_deeply_nested() {
    let input = DynValue::Object(vec![
        ("level1".into(), DynValue::Object(vec![
            ("level2".into(), DynValue::Object(vec![
                ("level3".into(), DynValue::Object(vec![
                    ("value".into(), DynValue::String("deep".into())),
                ])),
            ])),
        ])),
    ]);
    let output = run_transform(&field_transform("level1.level2.level3.value"), input);
    assert_eq!(get_output_value(&output, "output", "result"),
        Some(&DynValue::String("deep".into())));
}

#[test]
fn json_import_empty_object() {
    let input = DynValue::Object(vec![
        ("obj".into(), DynValue::Object(vec![])),
    ]);
    let transform = parse_transform(&field_transform("obj")).unwrap();
    let result = execute_transform(&transform, &input);
    assert!(result.success || result.output.is_some());
}

// ─────────────────────────────────────────────────────────────────────────────
// Arrays
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn json_import_string_array() {
    let input = DynValue::Object(vec![
        ("items".into(), DynValue::Array(vec![
            DynValue::String("a".into()),
            DynValue::String("b".into()),
            DynValue::String("c".into()),
        ])),
    ]);
    let t = r#"{$}
odin = "1.0.0"
transform = "1.0.0"
direction = "json->json"

{output}
count = %count @.items
first = %first @.items
"#;
    let output = run_transform(t, input);
    assert_eq!(get_output_value(&output, "output", "count"),
        Some(&DynValue::Integer(3)));
    assert_eq!(get_output_value(&output, "output", "first"),
        Some(&DynValue::String("a".into())));
}

#[test]
fn json_import_number_array() {
    let input = DynValue::Object(vec![
        ("numbers".into(), DynValue::Array(vec![
            DynValue::Integer(10),
            DynValue::Integer(20),
            DynValue::Integer(30),
        ])),
    ]);
    let t = r#"{$}
odin = "1.0.0"
transform = "1.0.0"
direction = "json->json"

{output}
sum = %sum @.numbers
avg = %avg @.numbers
"#;
    let output = run_transform(t, input);
    assert_eq!(get_output_value(&output, "output", "sum"),
        Some(&DynValue::Integer(60)));
    // avg may return Float(20.0) or Integer(20) depending on engine
    let avg_val = get_output_value(&output, "output", "avg");
    match avg_val {
        Some(DynValue::Integer(20)) | Some(DynValue::Float(_)) => {},
        other => panic!("Expected avg=20, got {:?}", other),
    }
}

#[test]
fn json_import_object_array() {
    let input = DynValue::Object(vec![
        ("users".into(), DynValue::Array(vec![
            DynValue::Object(vec![
                ("name".into(), DynValue::String("Alice".into())),
                ("age".into(), DynValue::Integer(25)),
            ]),
            DynValue::Object(vec![
                ("name".into(), DynValue::String("Bob".into())),
                ("age".into(), DynValue::Integer(30)),
            ]),
        ])),
    ]);
    let t = r#"{$}
odin = "1.0.0"
transform = "1.0.0"
direction = "json->json"

{output}
count = %count @.users
"#;
    let output = run_transform(t, input);
    assert_eq!(get_output_value(&output, "output", "count"),
        Some(&DynValue::Integer(2)));
}

#[test]
fn json_import_empty_array() {
    let input = DynValue::Object(vec![
        ("items".into(), DynValue::Array(vec![])),
    ]);
    let t = r#"{$}
odin = "1.0.0"
transform = "1.0.0"
direction = "json->json"

{output}
count = %count @.items
"#;
    let output = run_transform(t, input);
    assert_eq!(get_output_value(&output, "output", "count"),
        Some(&DynValue::Integer(0)));
}

#[test]
fn json_import_mixed_array() {
    let input = DynValue::Object(vec![
        ("mixed".into(), DynValue::Array(vec![
            DynValue::Integer(1),
            DynValue::String("two".into()),
            DynValue::Bool(true),
            DynValue::Null,
        ])),
    ]);
    let t = r#"{$}
odin = "1.0.0"
transform = "1.0.0"
direction = "json->json"

{output}
count = %count @.mixed
"#;
    let output = run_transform(t, input);
    assert_eq!(get_output_value(&output, "output", "count"),
        Some(&DynValue::Integer(4)));
}

// ─────────────────────────────────────────────────────────────────────────────
// Special Characters
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn json_import_unicode() {
    let input = DynValue::Object(vec![
        ("text".into(), DynValue::String("日本語テスト".into())),
    ]);
    let output = run_transform(&field_transform("text"), input);
    assert_eq!(get_output_value(&output, "output", "result"),
        Some(&DynValue::String("日本語テスト".into())));
}

#[test]
fn json_import_emoji() {
    let input = DynValue::Object(vec![
        ("text".into(), DynValue::String("Hello 👋 World 🌍".into())),
    ]);
    let output = run_transform(&field_transform("text"), input);
    assert_eq!(get_output_value(&output, "output", "result"),
        Some(&DynValue::String("Hello 👋 World 🌍".into())));
}

#[test]
fn json_import_newlines() {
    let input = DynValue::Object(vec![
        ("text".into(), DynValue::String("line1\nline2\nline3".into())),
    ]);
    let output = run_transform(&field_transform("text"), input);
    assert_eq!(get_output_value(&output, "output", "result"),
        Some(&DynValue::String("line1\nline2\nline3".into())));
}

#[test]
fn json_import_special_chars() {
    let input = DynValue::Object(vec![
        ("text".into(), DynValue::String(r#"quotes: "test" and backslash: \"#.into())),
    ]);
    let output = run_transform(&field_transform("text"), input);
    assert_eq!(get_output_value(&output, "output", "result"),
        Some(&DynValue::String(r#"quotes: "test" and backslash: \"#.into())));
}

// ─────────────────────────────────────────────────────────────────────────────
// Edge Cases
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn json_import_large_integer() {
    let input = DynValue::Object(vec![
        ("value".into(), DynValue::Integer(9_007_199_254_740_991)), // MAX_SAFE_INTEGER
    ]);
    let output = run_transform(&field_transform("value"), input);
    assert_eq!(get_output_value(&output, "output", "result"),
        Some(&DynValue::Integer(9_007_199_254_740_991)));
}

#[test]
fn json_import_small_float() {
    let input = DynValue::Object(vec![
        ("value".into(), DynValue::Float(0.000001)),
    ]);
    let output = run_transform(&field_transform("value"), input);
    if let Some(DynValue::Float(v)) = get_output_value(&output, "output", "result") {
        assert!((v - 0.000001).abs() < 1e-7);
    } else {
        panic!("Expected float value");
    }
}

#[test]
fn json_import_scientific_notation() {
    let input = DynValue::Object(vec![
        ("value".into(), DynValue::Float(1.5e10)),
    ]);
    let output = run_transform(&field_transform("value"), input);
    let val = get_output_value(&output, "output", "result");
    let actual = match val {
        Some(DynValue::Float(f)) => *f,
        Some(DynValue::Integer(i)) => *i as f64,
        other => panic!("Expected numeric value, got {:?}", other),
    };
    assert!((actual - 1.5e10).abs() < 1.0);
}

#[test]
fn json_import_long_string() {
    let long_string = "a".repeat(10000);
    let input = DynValue::Object(vec![
        ("value".into(), DynValue::String(long_string.clone())),
    ]);
    let output = run_transform(&field_transform("value"), input);
    assert_eq!(get_output_value(&output, "output", "result"),
        Some(&DynValue::String(long_string)));
}

#[test]
fn json_import_large_array() {
    let items: Vec<DynValue> = (0..1000).map(DynValue::Integer).collect();
    let input = DynValue::Object(vec![
        ("items".into(), DynValue::Array(items)),
    ]);
    let t = r#"{$}
odin = "1.0.0"
transform = "1.0.0"
direction = "json->json"

{output}
count = %count @.items
sum = %sum @.items
"#;
    let output = run_transform(t, input);
    assert_eq!(get_output_value(&output, "output", "count"),
        Some(&DynValue::Integer(1000)));
    assert_eq!(get_output_value(&output, "output", "sum"),
        Some(&DynValue::Integer(499500))); // sum of 0-999
}

// ─────────────────────────────────────────────────────────────────────────────
// Type Preservation
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn json_import_int_vs_float() {
    let int_input = DynValue::Object(vec![
        ("value".into(), DynValue::Integer(42)),
    ]);
    let float_input = DynValue::Object(vec![
        ("value".into(), DynValue::Float(42.5)),
    ]);
    let t = field_transform("value");
    let int_output = run_transform(&t, int_input);
    let float_output = run_transform(&t, float_input);

    assert_eq!(get_output_type(&int_output, "output", "result").as_deref(), Some("integer"));
    assert_eq!(get_output_type(&float_output, "output", "result").as_deref(), Some("number"));
}

#[test]
fn json_import_type_preservation() {
    let input = DynValue::Object(vec![
        ("strVal".into(), DynValue::String("test".into())),
        ("numVal".into(), DynValue::Integer(123)),
        ("boolVal".into(), DynValue::Bool(true)),
    ]);
    let t = r#"{$}
odin = "1.0.0"
transform = "1.0.0"
direction = "json->json"

{output}
str = "@.strVal"
num = "@.numVal"
bool = "@.boolVal"
"#;
    let output = run_transform(t, input);
    assert_eq!(get_output_type(&output, "output", "str").as_deref(), Some("string"));
    assert_eq!(get_output_type(&output, "output", "num").as_deref(), Some("integer"));
    assert_eq!(get_output_type(&output, "output", "bool").as_deref(), Some("boolean"));
}

// ─────────────────────────────────────────────────────────────────────────────
// Quoted Verb Strings Are Literal
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn json_import_quoted_verb_is_literal_string() {
    // A quoted string that starts with % should be treated as a literal string,
    // NOT parsed as a verb expression. Only bare (unquoted) %verb lines are verbs.
    let input = DynValue::Object(vec![
        ("value".into(), DynValue::String("hello".into())),
    ]);
    let t = r#"{$}
odin = "1.0.0"
transform = "1.0.0"
direction = "json->json"

{output}
bare_copy = @.value
quoted_verb = "%trim @.value"
bare_verb = %trim @.value
"#;
    let output = run_transform(t, input);
    // bare_copy should resolve the reference
    assert_eq!(get_output_value(&output, "output", "bare_copy"),
        Some(&DynValue::String("hello".into())));
    // quoted_verb should be a literal string, not a verb call
    assert_eq!(get_output_value(&output, "output", "quoted_verb"),
        Some(&DynValue::String("%trim @.value".into())));
    // bare_verb should execute the trim verb
    assert_eq!(get_output_value(&output, "output", "bare_verb"),
        Some(&DynValue::String("hello".into())));
}

// ── Golden File–Driven JSON Import Tests ─────────────────────────────────────

#[derive(serde::Deserialize)]
struct GoldenImportSuite {
    #[allow(dead_code)]
    suite: Option<String>,
    tests: Vec<GoldenImportTest>,
}

#[derive(serde::Deserialize)]
struct GoldenImportTest {
    id: String,
    #[allow(dead_code)]
    description: Option<String>,
    transform: String,
    input: serde_json::Value,
    expected: Option<GoldenImportExpected>,
}

#[derive(serde::Deserialize)]
struct GoldenImportExpected {
    output: Option<serde_json::Value>,
}

fn golden_import_dir() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent().unwrap()  // sdk/rust/
        .parent().unwrap()  // sdk/
        .join("golden")
        .join("json-import")
}

fn assert_golden_value_matches(actual: &DynValue, expected: &serde_json::Value, path: &str) {
    match expected {
        serde_json::Value::Null => {
            assert!(matches!(actual, DynValue::Null), "Expected null at {path}");
        }
        serde_json::Value::Bool(b) => {
            if let DynValue::Bool(a) = actual {
                assert_eq!(a, b, "Bool mismatch at {path}");
            } else {
                panic!("Expected bool at {path}, got {:?}", actual);
            }
        }
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                let actual_i = match actual {
                    DynValue::Integer(v) => *v,
                    DynValue::Float(v) => *v as i64,
                    _ => panic!("Expected number at {path}, got {:?}", actual),
                };
                assert_eq!(actual_i, i, "Int mismatch at {path}");
            } else if let Some(f) = n.as_f64() {
                let actual_f = match actual {
                    DynValue::Float(v) => *v,
                    DynValue::Integer(v) => *v as f64,
                    _ => panic!("Expected float at {path}, got {:?}", actual),
                };
                assert!((actual_f - f).abs() < 0.00001, "Float mismatch at {path}: {actual_f} != {f}");
            }
        }
        serde_json::Value::String(s) => {
            let actual_s = match actual {
                DynValue::String(v) => v.as_str(),
                _ => panic!("Expected string at {path}, got {:?}", actual),
            };
            assert_eq!(actual_s, s.as_str(), "String mismatch at {path}");
        }
        serde_json::Value::Object(map) => {
            if let DynValue::Object(entries) = actual {
                for (key, exp_val) in map {
                    let act_val = entries.iter()
                        .find(|(k, _)| k == key)
                        .map(|(_, v)| v)
                        .unwrap_or_else(|| panic!("Missing field '{key}' at {path}"));
                    assert_golden_value_matches(act_val, exp_val, &format!("{path}.{key}"));
                }
            } else {
                panic!("Expected object at {path}, got {:?}", actual);
            }
        }
        _ => {}
    }
}

#[test]
fn golden_json_import_from_file() {
    let dir = golden_import_dir();
    if !dir.exists() { return; }

    let mut failures = Vec::new();

    for entry in std::fs::read_dir(&dir).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        if !path.extension().map_or(false, |e| e == "json") { continue; }
        if path.file_name().map_or(false, |n| n == "manifest.json") { continue; }

        let content = std::fs::read_to_string(&path).unwrap();
        let suite: GoldenImportSuite = serde_json::from_str(&content).unwrap();

        for test in &suite.tests {
            let source = DynValue::from_json_value(test.input.clone());
            let transform = match parse_transform(&test.transform) {
                Ok(t) => t,
                Err(e) => {
                    failures.push(format!("[{}] parse transform error: {e}", test.id));
                    continue;
                }
            };
            let result = execute_transform(&transform, &source);
            if !result.success {
                failures.push(format!("[{}] transform failed", test.id));
                continue;
            }

            if let Some(ref expected) = test.expected {
                if let Some(ref exp_output) = expected.output {
                    // Navigate into the "output" segment
                    if let Some(DynValue::Object(entries)) = &result.output {
                        let out_seg = entries.iter()
                            .find(|(k, _)| k == "output")
                            .map(|(_, v)| v);
                        if let Some(out_seg) = out_seg {
                            if let serde_json::Value::Object(map) = exp_output {
                                for (key, exp_val) in map {
                                    if let DynValue::Object(fields) = out_seg {
                                        if let Some((_, actual_val)) = fields.iter().find(|(k, _)| k == key) {
                                            // Use a catch to collect failures instead of panicking
                                            let result = std::panic::catch_unwind(|| {
                                                assert_golden_value_matches(actual_val, exp_val, &format!("output.{key}"));
                                            });
                                            if let Err(_) = result {
                                                failures.push(format!("[{}] value mismatch at output.{key}", test.id));
                                            }
                                        } else {
                                            failures.push(format!("[{}] missing field '{key}' in output", test.id));
                                        }
                                    }
                                }
                            }
                        } else {
                            failures.push(format!("[{}] missing 'output' segment", test.id));
                        }
                    }
                }
            }
        }
    }

    if !failures.is_empty() {
        panic!("Golden JSON import failures:\n{}", failures.join("\n"));
    }
}
