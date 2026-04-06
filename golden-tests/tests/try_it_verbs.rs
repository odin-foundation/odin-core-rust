//! Try-It Verb Golden Tests
//!
//! Discovers and runs all try-it verb golden tests from sdk/golden/transform/verbs/try-it/.
//! Each verb has a triplet: {verb}.input.json, {verb}.transform.odin, {verb}.expected.odin.

use odin_core::transform::{parse_transform, execute_transform};
use odin_core::transform::source_parsers::parse_source;
use odin_core::types::transform::DynValue;
use std::path::PathBuf;

fn try_it_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent().unwrap()
        .parent().unwrap()
        .join("golden")
        .join("transform")
        .join("verbs")
        .join("try-it")
}

fn read_file(path: &std::path::Path) -> String {
    std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("Failed to read {}: {e}", path.display()))
        .replace("\r\n", "\n")
}

#[test]
fn try_it_verbs() {
    let dir = try_it_dir();
    assert!(dir.exists(), "try-it directory not found: {}", dir.display());

    // Discover all verb tests by finding *.expected.odin files
    let mut expected_files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("Failed to read directory {}: {e}", dir.display()))
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|p| {
            p.extension().map(|e| e == "odin").unwrap_or(false)
                && p.file_name()
                    .and_then(|f| f.to_str())
                    .map(|f| f.ends_with(".expected.odin"))
                    .unwrap_or(false)
        })
        .collect();

    expected_files.sort();
    assert!(!expected_files.is_empty(), "No *.expected.odin files found in {}", dir.display());

    let mut passed = 0;
    let mut failed = 0;
    let mut failed_verbs: Vec<String> = Vec::new();

    for expected_path in &expected_files {
        let file_name = expected_path.file_name().unwrap().to_str().unwrap();
        let verb = file_name.strip_suffix(".expected.odin").unwrap();

        let input_path = dir.join(format!("{}.input.json", verb));
        let transform_path = dir.join(format!("{}.transform.odin", verb));

        let result = std::panic::catch_unwind(|| {
            let input_json = read_file(&input_path);
            let transform_text = read_file(&transform_path);
            let expected = read_file(&expected_path);

            let input: DynValue = parse_source(&input_json, "json")
                .unwrap_or_else(|_| panic!("[{}] Failed to parse JSON input", verb));

            let mut transform = parse_transform(&transform_text)
                .unwrap_or_else(|e| panic!("[{}] Failed to parse transform: {e}", verb));

            transform.target.format = "odin".to_string();

            let result = execute_transform(&transform, &input);

            if !result.success {
                let errors: Vec<String> = result.errors.iter()
                    .map(|e| format!("  [{err}] {msg}",
                        err = e.code.as_deref().unwrap_or("???"),
                        msg = &e.message))
                    .collect();
                panic!("[{}] Transform failed:\n{}", verb, errors.join("\n"));
            }

            let formatted = result.formatted.unwrap_or_default();
            let norm_actual = formatted.trim();
            let norm_expected = expected.trim();

            assert_eq!(
                norm_actual, norm_expected,
                "\n[{}] Output mismatch:\n--- expected ---\n{}\n--- actual ---\n{}",
                verb, norm_expected, norm_actual
            );
        });

        match result {
            Ok(()) => {
                passed += 1;
                eprintln!("  PASS [{}]", verb);
            }
            Err(e) => {
                failed += 1;
                let msg = e.downcast_ref::<String>()
                    .map(|s| s.as_str())
                    .or_else(|| e.downcast_ref::<&str>().copied())
                    .unwrap_or("unknown error");
                eprintln!("  FAIL [{}]: {}", verb, msg);
                failed_verbs.push(verb.to_string());
            }
        }
    }

    eprintln!("\n  Try-It Verbs: {} passed, {} failed out of {} total",
        passed, failed, expected_files.len());

    if failed > 0 {
        panic!("{} try-it verb test(s) failed: {}", failed, failed_verbs.join(", "));
    }
}
