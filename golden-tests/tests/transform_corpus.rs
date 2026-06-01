//! Transform corpus golden tests.
//!
//! Loads the verified example corpus from sdk/golden/transform-corpus/<family>/*.json.
//! Each fixture carries a mapping block (transform), an ODIN source (input), and the
//! exact engine-produced output (expectedOutput). A standard odin->odin header is
//! prepended (honoring targetFormat/targetOptions/headerFields), the transform runs,
//! and result.formatted is compared to expectedOutput. Error fixtures assert that the
//! declared T-code surfaces in the errors/warnings (or thrown parse error).

use odin_core::transform::{parse_transform, execute_transform, document_to_dynvalue};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Fixture {
    id: String,
    family: String,
    #[serde(default)]
    transform: String,
    #[serde(default)]
    input: String,
    #[serde(default)]
    expected_output: Option<String>,
    #[serde(default)]
    target_format: Option<String>,
    #[serde(default)]
    target_options: Option<BTreeMap<String, serde_json::Value>>,
    #[serde(default)]
    header_fields: Option<BTreeMap<String, serde_json::Value>>,
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    enforced: Option<bool>,
    /// TS-specific fixtures (float-ULP, TS date format, seeded RNG) are pinned
    /// in TypeScript only; cross-language runners skip them. The verb's behavior
    /// stays covered by this SDK's own unit tests.
    #[serde(default)]
    ts_only: bool,
}

fn corpus_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent().unwrap()
        .parent().unwrap()
        .join("golden")
        .join("transform-corpus")
}

// Builds the transform header for a fixture. Source is always ODIN; the target
// format defaults to odin and may be overridden by format-conversion idioms.
// Render a JSON value the way the reference header builder interpolates it:
// strings yield their content, numbers/bools yield their literal text.
fn raw(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn build_header(
    target_format: &str,
    target_options: Option<&BTreeMap<String, serde_json::Value>>,
    header_fields: Option<&BTreeMap<String, serde_json::Value>>,
) -> String {
    let mut meta = vec![
        "odin = \"1.0.0\"".to_string(),
        "transform = \"1.0.0\"".to_string(),
        format!("direction = \"odin->{}\"", target_format),
    ];
    if let Some(fields) = header_fields {
        for (k, v) in fields {
            meta.push(format!("{} = {}", k, raw(v)));
        }
    }
    let mut target = vec![format!("format = \"{}\"", target_format)];
    if let Some(opts) = target_options {
        for (k, v) in opts {
            target.push(format!("{} = \"{}\"", k, raw(v)));
        }
    }
    format!(
        "{{$}}\n{}\n\n{{$source}}\nformat = \"odin\"\n\n{{$target}}\n{}\n\n",
        meta.join("\n"),
        target.join("\n"),
    )
}

fn header_for(f: &Fixture) -> String {
    build_header(
        f.target_format.as_deref().unwrap_or("odin"),
        f.target_options.as_ref(),
        f.header_fields.as_ref(),
    )
}

fn load_fixtures() -> Vec<(Fixture, PathBuf)> {
    let dir = corpus_dir();
    let mut out = Vec::new();
    let mut families: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("Failed to read {}: {e}", dir.display()))
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    families.sort();
    for family in families {
        let mut files: Vec<PathBuf> = std::fs::read_dir(&family)
            .unwrap_or_else(|e| panic!("Failed to read {}: {e}", family.display()))
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| {
                p.extension().map(|e| e == "json").unwrap_or(false)
                    && p.file_name().and_then(|f| f.to_str()) != Some("manifest.json")
            })
            .collect();
        files.sort();
        for file in files {
            let content = std::fs::read_to_string(&file)
                .unwrap_or_else(|e| panic!("Failed to read {}: {e}", file.display()));
            let fixture: Fixture = serde_json::from_str(&content)
                .unwrap_or_else(|e| panic!("Failed to parse {}: {e}", file.display()));
            out.push((fixture, file));
        }
    }
    out.sort_by(|a, b| a.1.cmp(&b.1));
    out
}

fn norm(s: &str) -> String {
    s.replace("\r\n", "\n").trim_end().to_string()
}

// Parse the fixture's ODIN source into the engine's input value (mirrors toJSON).
fn parse_input(input: &str) -> odin_core::types::transform::DynValue {
    let doc = odin_core::Odin::parse(input)
        .unwrap_or_else(|e| panic!("Failed to parse ODIN input: {e}"));
    document_to_dynvalue(&doc)
}

enum Outcome {
    Pass,
    Fail { expected: String, actual: String, errors: String },
}

fn run_fixture(f: &Fixture) -> Outcome {
    let transform_text = header_for(f) + &f.transform;
    let source = parse_input(&f.input);

    let parsed = match parse_transform(&transform_text) {
        Ok(t) => t,
        Err(e) => {
            // Error fixtures may surface the code via a parse failure.
            if f.family == "error" {
                let surfaced = format!("{e}");
                let code = f.code.clone().unwrap_or_default();
                return if surfaced.contains(&code) {
                    Outcome::Pass
                } else {
                    Outcome::Fail { expected: code, actual: surfaced, errors: String::new() }
                };
            }
            return Outcome::Fail {
                expected: f.expected_output.clone().unwrap_or_default(),
                actual: String::new(),
                errors: format!("parse error: {e}"),
            };
        }
    };

    let result = execute_transform(&parsed, &source);

    if f.family == "error" {
        // Documented-but-not-enforced fixtures: skip assertion.
        if f.enforced == Some(false) {
            return Outcome::Pass;
        }
        let code = f.code.clone().unwrap_or_default();
        let mut surfaced = String::new();
        for e in &result.errors {
            surfaced.push_str(e.code.as_deref().unwrap_or(""));
            surfaced.push(' ');
            surfaced.push_str(&e.message);
            surfaced.push('\n');
        }
        for w in &result.warnings {
            surfaced.push_str(&w.message);
            surfaced.push('\n');
        }
        return if surfaced.contains(&code) {
            Outcome::Pass
        } else {
            Outcome::Fail { expected: code, actual: surfaced, errors: String::new() }
        };
    }

    let errors: String = result.errors.iter()
        .map(|e| format!("[{}] {}", e.code.as_deref().unwrap_or("???"), e.message))
        .collect::<Vec<_>>().join("\n");

    let actual = norm(&result.formatted.clone().unwrap_or_default());
    let expected = norm(f.expected_output.as_deref().unwrap_or(""));

    if actual == expected && result.errors.is_empty() {
        Outcome::Pass
    } else {
        Outcome::Fail { expected, actual, errors }
    }
}

#[test]
fn transform_corpus() {
    let fixtures = load_fixtures();
    assert!(!fixtures.is_empty(), "no corpus fixtures found");

    let mut passed = 0;
    let mut failed = 0;
    let mut skipped = 0;
    let mut failures: Vec<String> = Vec::new();

    for (f, file) in &fixtures {
        // TS-only fixtures are verified in TypeScript; skip them here.
        if f.ts_only {
            skipped += 1;
            continue;
        }
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run_fixture(f)));
        match outcome {
            Ok(Outcome::Pass) => passed += 1,
            Ok(Outcome::Fail { expected, actual, errors }) => {
                failed += 1;
                failures.push(format!(
                    "FAIL {}/{}\n  file: {}\n  --- expected ---\n{}\n  --- actual ---\n{}\n  --- errors ---\n{}",
                    f.family, f.id, file.display(), expected, actual, errors,
                ));
            }
            Err(e) => {
                failed += 1;
                let msg = e.downcast_ref::<String>().map(|s| s.as_str())
                    .or_else(|| e.downcast_ref::<&str>().copied())
                    .unwrap_or("unknown panic");
                failures.push(format!("PANIC {}/{}: {}", f.family, f.id, msg));
            }
        }
    }

    eprintln!("\nTransform Corpus: {} passed, {} failed, {} skipped (tsOnly) out of {} total",
        passed, failed, skipped, fixtures.len());
    for fail in &failures {
        eprintln!("\n{}", fail);
    }

    if failed > 0 {
        panic!("{} corpus fixture(s) failed", failed);
    }
}
