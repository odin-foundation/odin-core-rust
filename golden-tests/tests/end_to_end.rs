//! End-to-End Golden Tests
//!
//! Tests format transformations through ODIN as the canonical data model.
//! Reads test definitions from manifest.json files in sdk/golden/end-to-end/.

use odin_core::transform::{parse_transform, execute_transform};
use odin_core::transform::source_parsers::parse_source;
use odin_core::types::transform::DynValue;
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Deserialize)]
struct MainManifest {
    suite: String,
    version: String,
    categories: Vec<CategoryRef>,
}

#[derive(Deserialize)]
struct CategoryRef {
    id: String,
    name: String,
    path: String,
}

#[derive(Deserialize)]
struct CategoryManifest {
    #[serde(default)]
    category: String,
    name: String,
    tests: Vec<TestDefinition>,
}

#[derive(Deserialize)]
struct TestDefinition {
    id: String,
    description: String,
    #[serde(default)]
    direction: Option<String>,
    input: String,
    #[serde(default)]
    transform: Option<String>,
    expected: String,
    #[serde(default, rename = "importTransform")]
    import_transform: Option<String>,
    #[serde(default, rename = "exportTransform")]
    export_transform: Option<String>,
    #[serde(default)]
    intermediate: Option<String>,
    #[serde(default)]
    method: Option<String>,
    #[serde(default)]
    options: Option<ExportOptions>,
}

#[derive(Deserialize, Default)]
struct ExportOptions {
    #[serde(default, rename = "preserveTypes")]
    preserve_types: bool,
    #[serde(default, rename = "preserveModifiers")]
    preserve_modifiers: bool,
}

fn golden_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent().unwrap()
        .parent().unwrap()
        .join("golden")
        .join("end-to-end")
}

fn read_file(path: &std::path::Path) -> String {
    std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("Failed to read {}: {e}", path.display()))
        .replace("\r\n", "\n")
}

/// Get source format from direction string (e.g., "json->odin" => "json")
fn source_format(direction: &str) -> &str {
    direction.split("->").next().unwrap_or("odin")
}

/// Parse input data based on source format
fn parse_input(raw: &str, format: &str) -> DynValue {
    match format {
        "json" => parse_source(raw, "json").unwrap_or(DynValue::Null),
        // For fixed-width, csv, and delimited: pass raw string — engine handles multi-record splitting
        "fixed-width" | "csv" | "delimited" => DynValue::String(raw.to_string()),
        "odin" => {
            // Parse ODIN document and convert to DynValue
            match odin_core::Odin::parse(raw) {
                Ok(doc) => odin_doc_to_dyn(&doc),
                Err(e) => panic!("Failed to parse ODIN input: {e}"),
            }
        }
        other => parse_source(raw, other).unwrap_or(DynValue::Null),
    }
}

/// Convert an OdinDocument to a nested DynValue structure.
///
/// ODIN documents store assignments as flat paths like `primitives.string_simple = "Hello"`.
/// The transform engine expects these to be nested objects like `{ primitives: { string_simple: "Hello" } }`.
fn odin_doc_to_dyn(doc: &odin_core::OdinDocument) -> DynValue {
    let mut root = DynValue::Object(Vec::new());
    for (path, value) in doc.assignments.iter() {
        // Skip metadata paths (prefixed with $)
        if path.starts_with('$') {
            continue;
        }
        let dyn_val = odin_value_to_dyn(value);
        set_nested_path(&mut root, path, dyn_val);
    }
    root
}

/// Set a value at a dotted path in a nested DynValue, creating intermediate objects as needed.
fn set_nested_path(root: &mut DynValue, path: &str, value: DynValue) {
    let segments: Vec<&str> = path.split('.').collect();
    set_nested_recursive(root, &segments, value);
}

fn set_nested_recursive(current: &mut DynValue, segments: &[&str], value: DynValue) {
    if segments.is_empty() {
        return;
    }

    let seg = segments[0];
    let is_last = segments.len() == 1;

    if let DynValue::Object(entries) = current {
        // Check for array index syntax: key[N]
        if let Some(bracket_pos) = seg.find('[') {
            let key = &seg[..bracket_pos];
            let idx_str = &seg[bracket_pos + 1..seg.len() - 1];
            if let Ok(idx) = idx_str.parse::<usize>() {
                // Find or create the array
                let arr_pos = entries.iter().position(|(k, _)| k == key);
                let arr_pos = if let Some(pos) = arr_pos {
                    pos
                } else {
                    entries.push((key.to_string(), DynValue::Array(Vec::new())));
                    entries.len() - 1
                };
                if let DynValue::Array(items) = &mut entries[arr_pos].1 {
                    let fill = if is_last { DynValue::Null } else { DynValue::Object(Vec::new()) };
                    while items.len() <= idx {
                        items.push(fill.clone());
                    }
                    if is_last {
                        items[idx] = value;
                    } else {
                        set_nested_recursive(&mut items[idx], &segments[1..], value);
                    }
                }
                return;
            }
        }

        if is_last {
            entries.push((seg.to_string(), value));
        } else {
            let existing = entries.iter().position(|(k, _)| k == seg);
            if let Some(pos) = existing {
                set_nested_recursive(&mut entries[pos].1, &segments[1..], value);
            } else {
                entries.push((seg.to_string(), DynValue::Object(Vec::new())));
                let last = entries.len() - 1;
                set_nested_recursive(&mut entries[last].1, &segments[1..], value);
            }
        }
    }
}

/// Convert OdinValue to DynValue
fn odin_value_to_dyn(val: &odin_core::OdinValue) -> DynValue {
    use odin_core::OdinValue;
    match val {
        OdinValue::Null { .. } => DynValue::Null,
        OdinValue::Boolean { value, .. } => DynValue::Bool(*value),
        OdinValue::String { value, .. } => DynValue::String(value.clone()),
        OdinValue::Integer { value, .. } => DynValue::Integer(*value),
        OdinValue::Number { value, .. } => DynValue::Float(*value),
        OdinValue::Currency { value, .. } => DynValue::Float(*value),
        OdinValue::Percent { value, .. } => DynValue::Float(*value),
        OdinValue::Date { raw, .. } => DynValue::String(raw.clone()),
        OdinValue::Timestamp { raw, .. } => DynValue::String(raw.clone()),
        OdinValue::Time { value, .. } => DynValue::String(value.clone()),
        OdinValue::Duration { value, .. } => DynValue::String(value.clone()),
        OdinValue::Reference { path, .. } => DynValue::String(path.clone()),
        OdinValue::Binary { .. } => DynValue::Null,
        OdinValue::Array { items, .. } => {
            let dyn_items: Vec<DynValue> = items.iter().map(|item| {
                match item {
                    odin_core::types::values::OdinArrayItem::Value(v) => odin_value_to_dyn(v),
                    odin_core::types::values::OdinArrayItem::Record(fields) => {
                        DynValue::Object(fields.iter().map(|(k, v)| (k.clone(), odin_value_to_dyn(v))).collect())
                    }
                }
            }).collect();
            DynValue::Array(dyn_items)
        }
        OdinValue::Object { value, .. } => {
            DynValue::Object(value.iter().map(|(k, v)| (k.clone(), odin_value_to_dyn(v))).collect())
        }
        OdinValue::Verb { .. } => DynValue::Null,
    }
}

/// Run a direct export test (toJSON / toXML)
fn run_direct_export_test(test: &TestDefinition, category_path: &str, method: &str) {
    let test_dir = golden_dir().join(category_path);
    let input_text = read_file(&test_dir.join(&test.input));
    let expected = read_file(&test_dir.join(&test.expected));

    let doc = odin_core::Odin::parse(&input_text)
        .unwrap_or_else(|e| panic!("[{}] Failed to parse ODIN input: {:?}", test.id, e));

    let opts = test.options.as_ref().map(|o| (o.preserve_types, o.preserve_modifiers)).unwrap_or((true, true));

    let actual = match method {
        "toJSON" => odin_core::Odin::to_json(&doc, opts.0, opts.1),
        "toXML" => odin_core::Odin::to_xml(&doc, opts.0, opts.1),
        _ => panic!("[{}] Unknown export method: {}", test.id, method),
    };

    assert_eq!(
        actual.trim(),
        expected.trim(),
        "[{}] {} output mismatch:\n--- expected ---\n{}\n--- actual ---\n{}",
        test.id,
        method,
        expected.trim(),
        actual.trim()
    );
}

/// Run a single import/export test
fn run_transform_test(test: &TestDefinition, category_path: &str) {
    let test_dir = golden_dir().join(category_path);

    let input_raw = read_file(&test_dir.join(&test.input));
    let transform_text = read_file(&test_dir.join(test.transform.as_ref().unwrap()));
    let expected = read_file(&test_dir.join(&test.expected));

    let transform = parse_transform(&transform_text)
        .unwrap_or_else(|e| panic!("[{}] Failed to parse transform: {e}", test.id));


    let direction = test.direction.as_deref().unwrap_or("odin->odin");
    let src_fmt = source_format(direction);
    let input = parse_input(&input_raw, src_fmt);

    let result = execute_transform(&transform, &input);

    if !result.success {
        let errors: Vec<String> = result.errors.iter()
            .map(|e| format!("  [{err}] {msg}", err = e.code.as_deref().unwrap_or("???"), msg = &e.message))
            .collect();
        panic!("[{}] Transform failed:\n{}", test.id, errors.join("\n"));
    }

    let formatted = result.formatted.unwrap_or_default();
    let norm_expected = expected.trim();
    let norm_actual = formatted.trim();

    // Write actual output to file for diffing
    if norm_actual != norm_expected {
        let dump_dir = golden_dir().join("..").join("..").join("rust").join("test-output");
        let _ = std::fs::create_dir_all(&dump_dir);
        let _ = std::fs::write(dump_dir.join(format!("{}.actual.txt", test.id)), norm_actual);
        let _ = std::fs::write(dump_dir.join(format!("{}.expected.txt", test.id)), norm_expected);
    }

    assert_eq!(norm_actual, norm_expected,
        "\n[{}] Formatted output mismatch:\n--- expected ---\n{}\n--- actual ---\n{}",
        test.id, norm_expected, norm_actual);
}

/// Run a roundtrip test (import + export)
fn run_roundtrip_test(test: &TestDefinition, category_path: &str) {
    let test_dir = golden_dir().join(category_path);

    let input_raw = read_file(&test_dir.join(&test.input));
    let import_transform_text = read_file(&test_dir.join(test.import_transform.as_ref().unwrap()));
    let export_transform_text = read_file(&test_dir.join(test.export_transform.as_ref().unwrap()));
    let expected = read_file(&test_dir.join(&test.expected));

    // Step 1: Import
    let import_transform = parse_transform(&import_transform_text)
        .unwrap_or_else(|e| panic!("[{}] Failed to parse import transform: {e}", test.id));

    let direction = test.direction.as_deref().unwrap_or("fixed-width->fixed-width");
    let src_fmt = source_format(direction);
    let input = parse_input(&input_raw, src_fmt);

    let import_result = execute_transform(&import_transform, &input);
    assert!(import_result.success, "[{}] Import transform failed", test.id);

    let import_output = import_result.output.expect("Import should produce output");

    // Step 2: Export
    let export_transform = parse_transform(&export_transform_text)
        .unwrap_or_else(|e| panic!("[{}] Failed to parse export transform: {e}", test.id));

    let export_result = execute_transform(&export_transform, &import_output);
    if !export_result.success {
        for err in &export_result.errors {
            eprintln!("  EXPORT ERR: [{}] {}", err.code.as_deref().unwrap_or("???"), err.message);
        }
    }
    assert!(export_result.success, "[{}] Export transform failed", test.id);

    let formatted = export_result.formatted.unwrap_or_default();
    let norm_expected = expected.trim();
    let norm_actual = formatted.trim();

    assert_eq!(norm_actual, norm_expected,
        "\n[{}] Roundtrip output mismatch", test.id);
}

/// Run all tests in a category
fn run_category(category_id: &str, category_path: &str) {
    let manifest_path = golden_dir().join(category_path).join("manifest.json");
    if !manifest_path.exists() {
        eprintln!("  SKIP category '{}': manifest not found", category_id);
        return;
    }

    let content = read_file(&manifest_path);
    let manifest: CategoryManifest = serde_json::from_str(&content)
        .unwrap_or_else(|e| panic!("Failed to parse category manifest: {e}"));

    let mut passed = 0;
    let mut failed = 0;
    let mut skipped = 0;

    for test in &manifest.tests {
        // Handle direct export tests (toJSON/toXML methods)
        if let Some(ref method) = test.method {
            let result = std::panic::catch_unwind(|| {
                run_direct_export_test(test, category_path, method);
            });
            match result {
                Ok(()) => {
                    passed += 1;
                    eprintln!("  PASS [{}]: {}", test.id, test.description);
                }
                Err(e) => {
                    failed += 1;
                    let msg = e.downcast_ref::<String>()
                        .map(|s| s.as_str())
                        .or_else(|| e.downcast_ref::<&str>().copied())
                        .unwrap_or("unknown error");
                    eprintln!("  FAIL [{}]: {}", test.id, msg);
                }
            }
            continue;
        }

        let result = std::panic::catch_unwind(|| {
            if category_id == "roundtrip" {
                run_roundtrip_test(test, category_path);
            } else {
                run_transform_test(test, category_path);
            }
        });

        match result {
            Ok(()) => {
                passed += 1;
                eprintln!("  PASS [{}]: {}", test.id, test.description);
            }
            Err(e) => {
                failed += 1;
                let msg = e.downcast_ref::<String>()
                    .map(|s| s.as_str())
                    .or_else(|| e.downcast_ref::<&str>().copied())
                    .unwrap_or("unknown error");
                eprintln!("  FAIL [{}]: {}", test.id, msg);
            }
        }
    }

    eprintln!("\n  {}: {} passed, {} failed, {} skipped",
        manifest.name, passed, failed, skipped);

    if failed > 0 {
        panic!("{} test(s) failed in category '{}'", failed, category_id);
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Test functions — one per category
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn debug_yaml_transform() {
    let dir = golden_dir().join("import").join("yaml-to-odin");
    let transform_text = read_file(&dir.join("config.transform.odin"));
    let doc = odin_core::Odin::parse(&transform_text).unwrap();
    eprintln!("\n=== ODIN DOC ASSIGNMENTS ===");
    for (key, _val) in &doc.assignments {
        eprintln!("  key: {}", key);
    }
    let transform = odin_core::transform::parse_transform(&transform_text).unwrap();
    eprintln!("\n=== PARSED SEGMENTS ===");
    for seg in &transform.segments {
        eprintln!("SEG: {:?} source_path={:?} mappings={} items={} children={}",
            seg.name, seg.source_path, seg.mappings.len(), seg.items.len(), seg.children.len());
        for m in &seg.mappings {
            eprintln!("  MAP: {} = {:?} mods={:?}", m.target, m.expression, m.modifiers);
        }
        for c in &seg.children {
            eprintln!("  CHILD: {:?} source_path={:?} mappings={}", c.name, c.source_path, c.mappings.len());
            for m in &c.mappings {
                eprintln!("    MAP: {} = {:?}", m.target, m.expression);
            }
        }
    }
}

#[test]
fn end_to_end_import() {
    run_category("import", "import");
}

#[test]
fn end_to_end_export() {
    run_category("export", "export");
}

#[test]
fn end_to_end_roundtrip() {
    run_category("roundtrip", "roundtrip");
}

#[test]
fn end_to_end_odin_export() {
    run_category("odin-export", "odin-export");
}
