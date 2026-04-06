//! Golden diff tests — reads test cases from sdk/golden/diff/
//! and verifies that diff(doc1, doc2) produces the expected result.

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
    doc1: String,
    doc2: String,
    expected: ExpectedDiff,
}

#[derive(Deserialize)]
struct ExpectedDiff {
    #[serde(rename = "isEmpty")]
    is_empty: bool,
    #[serde(default)]
    modifications: Vec<PathRef>,
    #[serde(default)]
    additions: Vec<PathRef>,
    #[serde(default)]
    deletions: Vec<PathRef>,
    #[serde(default)]
    moves: Vec<MoveRef>,
}

#[derive(Deserialize)]
struct PathRef {
    path: String,
}

#[derive(Deserialize)]
struct MoveRef {
    #[serde(rename = "fromPath")]
    from_path: String,
    #[serde(rename = "toPath")]
    to_path: String,
}

fn golden_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent().unwrap()
        .parent().unwrap()
        .join("golden")
        .join("diff")
}

fn run_diff_suite(file: &str) {
    let path = golden_dir().join(file);
    let json = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("Failed to read {}: {e}", path.display()));
    let suite: TestSuite = serde_json::from_str(&json)
        .unwrap_or_else(|e| panic!("Failed to parse {}: {e}", path.display()));

    let mut failures = Vec::new();

    for test in &suite.tests {
        let doc1 = match Odin::parse(&test.doc1) {
            Ok(d) => d,
            Err(e) => {
                failures.push(format!("[{}] doc1 parse error: {e}", test.id));
                continue;
            }
        };
        let doc2 = match Odin::parse(&test.doc2) {
            Ok(d) => d,
            Err(e) => {
                failures.push(format!("[{}] doc2 parse error: {e}", test.id));
                continue;
            }
        };

        let diff = Odin::diff(&doc1, &doc2);

        // Check isEmpty
        if diff.is_empty() != test.expected.is_empty {
            failures.push(format!(
                "[{}] isEmpty: expected {}, got {}",
                test.id, test.expected.is_empty, diff.is_empty()
            ));
            continue;
        }

        // Check modifications
        let actual_mod_paths: Vec<&str> = diff.changed.iter().map(|c| c.path.as_str()).collect();
        let expected_mod_paths: Vec<&str> = test.expected.modifications.iter().map(|m| m.path.as_str()).collect();
        if actual_mod_paths != expected_mod_paths {
            failures.push(format!(
                "[{}] modifications: expected {:?}, got {:?}",
                test.id, expected_mod_paths, actual_mod_paths
            ));
        }

        // Check additions
        let actual_add_paths: Vec<&str> = diff.added.iter().map(|a| a.path.as_str()).collect();
        let expected_add_paths: Vec<&str> = test.expected.additions.iter().map(|a| a.path.as_str()).collect();
        if actual_add_paths != expected_add_paths {
            failures.push(format!(
                "[{}] additions: expected {:?}, got {:?}",
                test.id, expected_add_paths, actual_add_paths
            ));
        }

        // Check deletions
        let actual_del_paths: Vec<&str> = diff.removed.iter().map(|r| r.path.as_str()).collect();
        let expected_del_paths: Vec<&str> = test.expected.deletions.iter().map(|d| d.path.as_str()).collect();
        if actual_del_paths != expected_del_paths {
            failures.push(format!(
                "[{}] deletions: expected {:?}, got {:?}",
                test.id, expected_del_paths, actual_del_paths
            ));
        }

        // Check moves
        let actual_moves: Vec<(&str, &str)> = diff.moved.iter().map(|m| (m.from.as_str(), m.to.as_str())).collect();
        let expected_moves: Vec<(&str, &str)> = test.expected.moves.iter().map(|m| (m.from_path.as_str(), m.to_path.as_str())).collect();
        if actual_moves != expected_moves {
            failures.push(format!(
                "[{}] moves: expected {:?}, got {:?}",
                test.id, expected_moves, actual_moves
            ));
        }
    }

    if !failures.is_empty() {
        panic!(
            "\n{} diff test failures in suite '{}':\n\n{}\n",
            failures.len(),
            suite.suite,
            failures.join("\n\n")
        );
    }

    eprintln!(
        "  ✓ {} diff tests passed in '{}'",
        suite.tests.len(),
        suite.suite
    );
}

#[test]
fn golden_diff_all_types() {
    run_diff_suite("all-types.json");
}

#[test]
fn golden_diff_modifiers_and_arrays() {
    run_diff_suite("modifiers-and-arrays.json");
}
