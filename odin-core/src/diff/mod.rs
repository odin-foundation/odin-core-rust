//! Document diff and patch operations.
//!
//! Compare two `OdinDocument`s to produce an `OdinDiff`, then apply
//! that diff to reconstruct the target document.

use crate::types::diff::{OdinDiff, PathValue, PathChange, PathMove};
use crate::types::document::OdinDocument;
use crate::types::errors::PatchError;

/// Compare two documents and produce a diff.
pub fn diff(a: &OdinDocument, b: &OdinDocument) -> OdinDiff {
    // Heuristic capacity: most diffs touch a fraction of total fields. Sizing
    // for ~1/4 of a covers typical change rates without large overshoot.
    let cap = (a.assignments.len() / 4).max(4);
    let mut added = Vec::with_capacity(cap);
    let mut removed = Vec::with_capacity(cap);
    let mut changed = Vec::with_capacity(cap);
    let mut moved: Vec<PathMove> = Vec::new();

    for (path, value_a) in &a.assignments {
        if let Some(value_b) = b.assignments.get(path) {
            if value_a != value_b {
                changed.push(PathChange {
                    path: path.clone(),
                    old: value_a.clone(),
                    new: value_b.clone(),
                });
            }
        } else {
            removed.push(PathValue {
                path: path.clone(),
                value: value_a.clone(),
            });
        }
    }

    for (path, value_b) in &b.assignments {
        if !a.assignments.contains_key(path) {
            added.push(PathValue {
                path: path.clone(),
                value: value_b.clone(),
            });
        }
    }

    // Move detection: only runs when both sides are non-empty (the common
    // case is changes, not moves). When it does run, swap-remove matched
    // entries in reverse order to preserve unmatched indices.
    if !removed.is_empty() && !added.is_empty() {
        let mut taken = vec![false; added.len()];
        let mut keep_removed = Vec::with_capacity(removed.len());
        for rem in removed.drain(..) {
            let mut matched = None;
            for (ai, add) in added.iter().enumerate() {
                if !taken[ai] && rem.value == add.value {
                    matched = Some(ai);
                    break;
                }
            }
            match matched {
                Some(ai) => {
                    taken[ai] = true;
                    moved.push(PathMove {
                        from: rem.path,
                        to: added[ai].path.clone(),
                        value: rem.value,
                    });
                }
                None => keep_removed.push(rem),
            }
        }
        removed = keep_removed;
        // Drop matched added entries in reverse order.
        for ai in (0..taken.len()).rev() {
            if taken[ai] {
                added.swap_remove(ai);
            }
        }
    }

    OdinDiff { added, removed, changed, moved }
}

/// Apply a diff to a document, producing a new document.
///
/// # Errors
///
/// Returns `PatchError` if a path to be changed or removed doesn't exist.
pub fn patch(doc: &OdinDocument, diff: &OdinDiff) -> Result<OdinDocument, PatchError> {
    let mut result = doc.clone();

    // Apply removals
    for removal in &diff.removed {
        if !result.assignments.contains_key(&removal.path) {
            return Err(PatchError::new(
                "path does not exist for removal",
                &removal.path,
            ));
        }
        result.assignments.remove(&removal.path);
    }

    // Apply changes
    for change in &diff.changed {
        if !result.assignments.contains_key(&change.path) {
            return Err(PatchError::new(
                "path does not exist for change",
                &change.path,
            ));
        }
        result.assignments.insert(change.path.clone(), change.new.clone());
    }

    // Apply additions
    for addition in &diff.added {
        result.assignments.insert(addition.path.clone(), addition.value.clone());
    }

    // Apply moves
    for mv in &diff.moved {
        if let Some(val) = result.assignments.remove(&mv.from) {
            result.assignments.insert(mv.to.clone(), val);
        } else {
            return Err(PatchError::new(
                "source path does not exist for move",
                &mv.from,
            ));
        }
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::values::{OdinValues, OdinModifiers};
    use crate::OdinDocumentBuilder;

    // ── Identical documents ────────────────────────────────────────────────

    #[test]
    fn test_diff_no_changes() {
        let doc = OdinDocumentBuilder::new()
            .set("x", OdinValues::integer(1))
            .build()
            .unwrap();
        let d = diff(&doc, &doc);
        assert!(d.is_empty());
    }

    #[test]
    fn diff_identical_empty_documents() {
        let a = OdinDocument::empty();
        let b = OdinDocument::empty();
        let d = diff(&a, &b);
        assert!(d.is_empty());
    }

    #[test]
    fn diff_identical_complex_documents() {
        let doc = OdinDocumentBuilder::new()
            .set("name", OdinValues::string("Alice"))
            .set("age", OdinValues::integer(30))
            .set("active", OdinValues::boolean(true))
            .build()
            .unwrap();
        let d = diff(&doc, &doc);
        assert!(d.is_empty());
        assert!(d.added.is_empty());
        assert!(d.removed.is_empty());
        assert!(d.changed.is_empty());
        assert!(d.moved.is_empty());
    }

    // ── Additions ───────────────────────────────────────────────────────────

    #[test]
    fn test_diff_additions() {
        let a = OdinDocumentBuilder::new()
            .set("x", OdinValues::integer(1))
            .build()
            .unwrap();
        let b = OdinDocumentBuilder::new()
            .set("x", OdinValues::integer(1))
            .set("y", OdinValues::integer(2))
            .build()
            .unwrap();
        let d = diff(&a, &b);
        assert_eq!(d.added.len(), 1);
        assert_eq!(d.added[0].path, "y");
    }

    #[test]
    fn diff_add_multiple_fields() {
        let a = OdinDocumentBuilder::new()
            .set("x", OdinValues::integer(1))
            .build()
            .unwrap();
        let b = OdinDocumentBuilder::new()
            .set("x", OdinValues::integer(1))
            .set("y", OdinValues::integer(2))
            .set("z", OdinValues::integer(3))
            .build()
            .unwrap();
        let d = diff(&a, &b);
        assert_eq!(d.added.len(), 2);
    }

    #[test]
    fn diff_add_to_empty_document() {
        let a = OdinDocument::empty();
        let b = OdinDocumentBuilder::new()
            .set("name", OdinValues::string("Alice"))
            .build()
            .unwrap();
        let d = diff(&a, &b);
        assert_eq!(d.added.len(), 1);
        assert_eq!(d.added[0].path, "name");
        assert!(d.removed.is_empty());
        assert!(d.changed.is_empty());
    }

    #[test]
    fn diff_add_string_field() {
        let a = OdinDocumentBuilder::new()
            .set("x", OdinValues::integer(1))
            .build()
            .unwrap();
        let b = OdinDocumentBuilder::new()
            .set("x", OdinValues::integer(1))
            .set("name", OdinValues::string("Bob"))
            .build()
            .unwrap();
        let d = diff(&a, &b);
        assert_eq!(d.added.len(), 1);
        assert_eq!(d.added[0].value.as_str(), Some("Bob"));
    }

    #[test]
    fn diff_add_boolean_field() {
        let a = OdinDocumentBuilder::new()
            .set("x", OdinValues::integer(1))
            .build()
            .unwrap();
        let b = OdinDocumentBuilder::new()
            .set("x", OdinValues::integer(1))
            .set("flag", OdinValues::boolean(true))
            .build()
            .unwrap();
        let d = diff(&a, &b);
        assert_eq!(d.added.len(), 1);
        assert_eq!(d.added[0].value.as_bool(), Some(true));
    }

    #[test]
    fn diff_add_null_field() {
        let a = OdinDocumentBuilder::new()
            .set("x", OdinValues::integer(1))
            .build()
            .unwrap();
        let b = OdinDocumentBuilder::new()
            .set("x", OdinValues::integer(1))
            .set("missing", OdinValues::null())
            .build()
            .unwrap();
        let d = diff(&a, &b);
        assert_eq!(d.added.len(), 1);
        assert!(d.added[0].value.is_null());
    }

    // ── Removals ────────────────────────────────────────────────────────────

    #[test]
    fn diff_remove_single_field() {
        let a = OdinDocumentBuilder::new()
            .set("x", OdinValues::integer(1))
            .set("y", OdinValues::integer(2))
            .build()
            .unwrap();
        let b = OdinDocumentBuilder::new()
            .set("x", OdinValues::integer(1))
            .build()
            .unwrap();
        let d = diff(&a, &b);
        assert_eq!(d.removed.len(), 1);
        assert_eq!(d.removed[0].path, "y");
    }

    #[test]
    fn diff_remove_all_fields() {
        let a = OdinDocumentBuilder::new()
            .set("x", OdinValues::integer(1))
            .set("y", OdinValues::integer(2))
            .build()
            .unwrap();
        let b = OdinDocument::empty();
        let d = diff(&a, &b);
        assert_eq!(d.removed.len(), 2);
        assert!(d.added.is_empty());
    }

    #[test]
    fn diff_remove_multiple_fields() {
        let a = OdinDocumentBuilder::new()
            .set("a", OdinValues::integer(1))
            .set("b", OdinValues::integer(2))
            .set("c", OdinValues::integer(3))
            .build()
            .unwrap();
        let b = OdinDocumentBuilder::new()
            .set("b", OdinValues::integer(2))
            .build()
            .unwrap();
        let d = diff(&a, &b);
        assert_eq!(d.removed.len(), 2);
    }

    // ── Changes ─────────────────────────────────────────────────────────────

    #[test]
    fn diff_change_string_value() {
        let a = OdinDocumentBuilder::new()
            .set("name", OdinValues::string("old"))
            .build()
            .unwrap();
        let b = OdinDocumentBuilder::new()
            .set("name", OdinValues::string("new"))
            .build()
            .unwrap();
        let d = diff(&a, &b);
        assert_eq!(d.changed.len(), 1);
        assert_eq!(d.changed[0].path, "name");
        assert_eq!(d.changed[0].old.as_str(), Some("old"));
        assert_eq!(d.changed[0].new.as_str(), Some("new"));
    }

    #[test]
    fn diff_change_integer_value() {
        let a = OdinDocumentBuilder::new()
            .set("count", OdinValues::integer(10))
            .build()
            .unwrap();
        let b = OdinDocumentBuilder::new()
            .set("count", OdinValues::integer(20))
            .build()
            .unwrap();
        let d = diff(&a, &b);
        assert_eq!(d.changed.len(), 1);
        assert_eq!(d.changed[0].old.as_i64(), Some(10));
        assert_eq!(d.changed[0].new.as_i64(), Some(20));
    }

    #[test]
    fn diff_change_boolean_value() {
        let a = OdinDocumentBuilder::new()
            .set("active", OdinValues::boolean(true))
            .build()
            .unwrap();
        let b = OdinDocumentBuilder::new()
            .set("active", OdinValues::boolean(false))
            .build()
            .unwrap();
        let d = diff(&a, &b);
        assert_eq!(d.changed.len(), 1);
        assert_eq!(d.changed[0].old.as_bool(), Some(true));
        assert_eq!(d.changed[0].new.as_bool(), Some(false));
    }

    #[test]
    fn diff_change_type_string_to_integer() {
        let a = OdinDocumentBuilder::new()
            .set("val", OdinValues::string("42"))
            .build()
            .unwrap();
        let b = OdinDocumentBuilder::new()
            .set("val", OdinValues::integer(42))
            .build()
            .unwrap();
        let d = diff(&a, &b);
        assert_eq!(d.changed.len(), 1);
        assert!(d.changed[0].old.is_string());
        assert!(d.changed[0].new.is_integer());
    }

    #[test]
    fn diff_change_type_integer_to_boolean() {
        let a = OdinDocumentBuilder::new()
            .set("val", OdinValues::integer(1))
            .build()
            .unwrap();
        let b = OdinDocumentBuilder::new()
            .set("val", OdinValues::boolean(true))
            .build()
            .unwrap();
        let d = diff(&a, &b);
        assert_eq!(d.changed.len(), 1);
        assert!(d.changed[0].old.is_integer());
        assert!(d.changed[0].new.is_boolean());
    }

    #[test]
    fn diff_change_type_string_to_null() {
        let a = OdinDocumentBuilder::new()
            .set("val", OdinValues::string("something"))
            .build()
            .unwrap();
        let b = OdinDocumentBuilder::new()
            .set("val", OdinValues::null())
            .build()
            .unwrap();
        let d = diff(&a, &b);
        assert_eq!(d.changed.len(), 1);
        assert!(d.changed[0].old.is_string());
        assert!(d.changed[0].new.is_null());
    }

    #[test]
    fn diff_change_type_null_to_string() {
        let a = OdinDocumentBuilder::new()
            .set("val", OdinValues::null())
            .build()
            .unwrap();
        let b = OdinDocumentBuilder::new()
            .set("val", OdinValues::string("filled"))
            .build()
            .unwrap();
        let d = diff(&a, &b);
        assert_eq!(d.changed.len(), 1);
        assert!(d.changed[0].old.is_null());
        assert!(d.changed[0].new.is_string());
    }

    #[test]
    fn diff_change_multiple_fields() {
        let a = OdinDocumentBuilder::new()
            .set("x", OdinValues::integer(1))
            .set("y", OdinValues::integer(2))
            .set("z", OdinValues::integer(3))
            .build()
            .unwrap();
        let b = OdinDocumentBuilder::new()
            .set("x", OdinValues::integer(10))
            .set("y", OdinValues::integer(20))
            .set("z", OdinValues::integer(3))  // unchanged
            .build()
            .unwrap();
        let d = diff(&a, &b);
        assert_eq!(d.changed.len(), 2);
        assert!(d.added.is_empty());
        assert!(d.removed.is_empty());
    }

    // ── Moves ───────────────────────────────────────────────────────────────

    #[test]
    fn diff_move_field() {
        let a = OdinDocumentBuilder::new()
            .set("old_path", OdinValues::string("moved_value"))
            .build()
            .unwrap();
        let b = OdinDocumentBuilder::new()
            .set("new_path", OdinValues::string("moved_value"))
            .build()
            .unwrap();
        let d = diff(&a, &b);
        assert_eq!(d.moved.len(), 1);
        assert_eq!(d.moved[0].from, "old_path");
        assert_eq!(d.moved[0].to, "new_path");
        assert_eq!(d.moved[0].value.as_str(), Some("moved_value"));
        // Moved items should not appear in added/removed
        assert!(d.added.is_empty());
        assert!(d.removed.is_empty());
    }

    #[test]
    fn diff_move_between_sections() {
        let a = OdinDocumentBuilder::new()
            .set("Policy.name", OdinValues::string("test"))
            .build()
            .unwrap();
        let b = OdinDocumentBuilder::new()
            .set("Agent.name", OdinValues::string("test"))
            .build()
            .unwrap();
        let d = diff(&a, &b);
        assert_eq!(d.moved.len(), 1);
        assert_eq!(d.moved[0].from, "Policy.name");
        assert_eq!(d.moved[0].to, "Agent.name");
    }

    #[test]
    fn diff_move_integer_value() {
        let a = OdinDocumentBuilder::new()
            .set("src", OdinValues::integer(42))
            .build()
            .unwrap();
        let b = OdinDocumentBuilder::new()
            .set("dst", OdinValues::integer(42))
            .build()
            .unwrap();
        let d = diff(&a, &b);
        assert_eq!(d.moved.len(), 1);
    }

    // ── Complex diffs ───────────────────────────────────────────────────────

    #[test]
    fn diff_mixed_add_remove_change() {
        let a = OdinDocumentBuilder::new()
            .set("keep", OdinValues::integer(1))
            .set("change", OdinValues::string("old"))
            .set("remove", OdinValues::boolean(true))
            .build()
            .unwrap();
        let b = OdinDocumentBuilder::new()
            .set("keep", OdinValues::integer(1))
            .set("change", OdinValues::string("new"))
            .set("add", OdinValues::integer(99))
            .build()
            .unwrap();
        let d = diff(&a, &b);
        assert_eq!(d.changed.len(), 1);
        assert_eq!(d.removed.len(), 1);
        assert_eq!(d.added.len(), 1);
        assert_eq!(d.changed[0].path, "change");
        assert_eq!(d.removed[0].path, "remove");
        assert_eq!(d.added[0].path, "add");
    }

    #[test]
    fn diff_section_additions() {
        let a = OdinDocumentBuilder::new()
            .set("Policy.number", OdinValues::string("POL-001"))
            .build()
            .unwrap();
        let b = OdinDocumentBuilder::new()
            .set("Policy.number", OdinValues::string("POL-001"))
            .set("Agent.name", OdinValues::string("Smith"))
            .build()
            .unwrap();
        let d = diff(&a, &b);
        assert_eq!(d.added.len(), 1);
        assert_eq!(d.added[0].path, "Agent.name");
    }

    #[test]
    fn diff_section_removals() {
        let a = OdinDocumentBuilder::new()
            .set("Policy.number", OdinValues::string("POL-001"))
            .set("Agent.name", OdinValues::string("Smith"))
            .build()
            .unwrap();
        let b = OdinDocumentBuilder::new()
            .set("Policy.number", OdinValues::string("POL-001"))
            .build()
            .unwrap();
        let d = diff(&a, &b);
        assert_eq!(d.removed.len(), 1);
        assert_eq!(d.removed[0].path, "Agent.name");
    }

    #[test]
    fn diff_nested_section_changes() {
        let a = OdinDocumentBuilder::new()
            .set("Policy.Coverage.limit", OdinValues::integer(100000))
            .build()
            .unwrap();
        let b = OdinDocumentBuilder::new()
            .set("Policy.Coverage.limit", OdinValues::integer(200000))
            .build()
            .unwrap();
        let d = diff(&a, &b);
        assert_eq!(d.changed.len(), 1);
        assert_eq!(d.changed[0].path, "Policy.Coverage.limit");
    }

    #[test]
    fn diff_with_modifier_changes() {
        let mods_a = OdinModifiers { required: true, ..Default::default() };
        let mods_b = OdinModifiers { required: true, confidential: true, ..Default::default() };
        let a = OdinDocumentBuilder::new()
            .set("field", OdinValues::string("val").with_modifiers(mods_a))
            .build()
            .unwrap();
        let b = OdinDocumentBuilder::new()
            .set("field", OdinValues::string("val").with_modifiers(mods_b))
            .build()
            .unwrap();
        let d = diff(&a, &b);
        // Modifiers are part of the value, so this counts as a change
        assert_eq!(d.changed.len(), 1);
    }

    // ── Roundtrips (patch) ──────────────────────────────────────────────────

    #[test]
    fn test_diff_and_patch_roundtrip() {
        let a = OdinDocumentBuilder::new()
            .set("x", OdinValues::integer(1))
            .set("y", OdinValues::string("old"))
            .build()
            .unwrap();
        let b = OdinDocumentBuilder::new()
            .set("x", OdinValues::integer(1))
            .set("y", OdinValues::string("new"))
            .set("z", OdinValues::boolean(true))
            .build()
            .unwrap();
        let d = diff(&a, &b);
        let patched = patch(&a, &d).unwrap();
        assert_eq!(patched.get_string("y"), Some("new"));
        assert_eq!(patched.get_boolean("z"), Some(true));
    }

    #[test]
    fn patch_additions_only() {
        let a = OdinDocumentBuilder::new()
            .set("x", OdinValues::integer(1))
            .build()
            .unwrap();
        let b = OdinDocumentBuilder::new()
            .set("x", OdinValues::integer(1))
            .set("y", OdinValues::integer(2))
            .set("z", OdinValues::integer(3))
            .build()
            .unwrap();
        let d = diff(&a, &b);
        let patched = patch(&a, &d).unwrap();
        assert_eq!(patched.get_integer("x"), Some(1));
        assert_eq!(patched.get_integer("y"), Some(2));
        assert_eq!(patched.get_integer("z"), Some(3));
    }

    #[test]
    fn patch_removals_only() {
        let a = OdinDocumentBuilder::new()
            .set("x", OdinValues::integer(1))
            .set("y", OdinValues::integer(2))
            .set("z", OdinValues::integer(3))
            .build()
            .unwrap();
        let b = OdinDocumentBuilder::new()
            .set("x", OdinValues::integer(1))
            .build()
            .unwrap();
        let d = diff(&a, &b);
        let patched = patch(&a, &d).unwrap();
        assert_eq!(patched.get_integer("x"), Some(1));
        assert!(!patched.has("y"));
        assert!(!patched.has("z"));
    }

    #[test]
    fn patch_changes_only() {
        let a = OdinDocumentBuilder::new()
            .set("x", OdinValues::integer(1))
            .set("y", OdinValues::string("old"))
            .build()
            .unwrap();
        let b = OdinDocumentBuilder::new()
            .set("x", OdinValues::integer(99))
            .set("y", OdinValues::string("new"))
            .build()
            .unwrap();
        let d = diff(&a, &b);
        let patched = patch(&a, &d).unwrap();
        assert_eq!(patched.get_integer("x"), Some(99));
        assert_eq!(patched.get_string("y"), Some("new"));
    }

    #[test]
    fn patch_complex_roundtrip() {
        let a = OdinDocumentBuilder::new()
            .set("Policy.number", OdinValues::string("POL-001"))
            .set("Policy.status", OdinValues::string("active"))
            .set("Policy.premium", OdinValues::integer(1500))
            .set("Agent.name", OdinValues::string("Smith"))
            .build()
            .unwrap();
        let b = OdinDocumentBuilder::new()
            .set("Policy.number", OdinValues::string("POL-001"))  // unchanged
            .set("Policy.status", OdinValues::string("cancelled"))  // changed
            .set("Agent.name", OdinValues::string("Jones"))  // changed
            .set("Agent.code", OdinValues::string("AG-99"))  // added
            .build()
            .unwrap();
        let d = diff(&a, &b);
        let patched = patch(&a, &d).unwrap();
        assert_eq!(patched.get_string("Policy.number"), Some("POL-001"));
        assert_eq!(patched.get_string("Policy.status"), Some("cancelled"));
        assert!(!patched.has("Policy.premium"));
        assert_eq!(patched.get_string("Agent.name"), Some("Jones"));
        assert_eq!(patched.get_string("Agent.code"), Some("AG-99"));
    }

    #[test]
    fn patch_with_moves() {
        let a = OdinDocumentBuilder::new()
            .set("old_name", OdinValues::string("value"))
            .set("keep", OdinValues::integer(1))
            .build()
            .unwrap();
        let b = OdinDocumentBuilder::new()
            .set("new_name", OdinValues::string("value"))
            .set("keep", OdinValues::integer(1))
            .build()
            .unwrap();
        let d = diff(&a, &b);
        let patched = patch(&a, &d).unwrap();
        assert!(!patched.has("old_name"));
        assert_eq!(patched.get_string("new_name"), Some("value"));
        assert_eq!(patched.get_integer("keep"), Some(1));
    }

    // ── Patch error cases ───────────────────────────────────────────────────

    #[test]
    fn patch_error_remove_nonexistent() {
        let doc = OdinDocumentBuilder::new()
            .set("x", OdinValues::integer(1))
            .build()
            .unwrap();
        let bad_diff = OdinDiff {
            added: vec![],
            removed: vec![PathValue {
                path: "nonexistent".to_string(),
                value: OdinValues::integer(1),
            }],
            changed: vec![],
            moved: vec![],
        };
        let result = patch(&doc, &bad_diff);
        assert!(result.is_err());
    }

    #[test]
    fn patch_error_change_nonexistent() {
        let doc = OdinDocumentBuilder::new()
            .set("x", OdinValues::integer(1))
            .build()
            .unwrap();
        let bad_diff = OdinDiff {
            added: vec![],
            removed: vec![],
            changed: vec![PathChange {
                path: "nonexistent".to_string(),
                old: OdinValues::integer(1),
                new: OdinValues::integer(2),
            }],
            moved: vec![],
        };
        let result = patch(&doc, &bad_diff);
        assert!(result.is_err());
    }

    #[test]
    fn patch_error_move_nonexistent_source() {
        let doc = OdinDocumentBuilder::new()
            .set("x", OdinValues::integer(1))
            .build()
            .unwrap();
        let bad_diff = OdinDiff {
            added: vec![],
            removed: vec![],
            changed: vec![],
            moved: vec![PathMove {
                from: "nonexistent".to_string(),
                to: "dest".to_string(),
                value: OdinValues::integer(1),
            }],
        };
        let result = patch(&doc, &bad_diff);
        assert!(result.is_err());
    }

    // ── Empty diff ──────────────────────────────────────────────────────────

    #[test]
    fn patch_empty_diff_is_identity() {
        let doc = OdinDocumentBuilder::new()
            .set("x", OdinValues::integer(1))
            .set("y", OdinValues::string("hello"))
            .build()
            .unwrap();
        let d = OdinDiff::empty();
        let patched = patch(&doc, &d).unwrap();
        assert_eq!(patched.get_integer("x"), Some(1));
        assert_eq!(patched.get_string("y"), Some("hello"));
    }

    // ── Diff with null values ───────────────────────────────────────────────

    #[test]
    fn diff_null_to_value() {
        let a = OdinDocumentBuilder::new()
            .set("field", OdinValues::null())
            .build()
            .unwrap();
        let b = OdinDocumentBuilder::new()
            .set("field", OdinValues::string("filled"))
            .build()
            .unwrap();
        let d = diff(&a, &b);
        assert_eq!(d.changed.len(), 1);
    }

    #[test]
    fn diff_value_to_null() {
        let a = OdinDocumentBuilder::new()
            .set("field", OdinValues::string("exists"))
            .build()
            .unwrap();
        let b = OdinDocumentBuilder::new()
            .set("field", OdinValues::null())
            .build()
            .unwrap();
        let d = diff(&a, &b);
        assert_eq!(d.changed.len(), 1);
    }

    #[test]
    fn diff_null_to_null_no_change() {
        let a = OdinDocumentBuilder::new()
            .set("field", OdinValues::null())
            .build()
            .unwrap();
        let b = OdinDocumentBuilder::new()
            .set("field", OdinValues::null())
            .build()
            .unwrap();
        let d = diff(&a, &b);
        assert!(d.is_empty());
    }

    // ── Array path changes ──────────────────────────────────────────────────

    #[test]
    fn diff_array_item_added() {
        let a = OdinDocumentBuilder::new()
            .set("items[0]", OdinValues::string("first"))
            .build()
            .unwrap();
        let b = OdinDocumentBuilder::new()
            .set("items[0]", OdinValues::string("first"))
            .set("items[1]", OdinValues::string("second"))
            .build()
            .unwrap();
        let d = diff(&a, &b);
        assert_eq!(d.added.len(), 1);
        assert_eq!(d.added[0].path, "items[1]");
    }

    #[test]
    fn diff_array_item_removed() {
        let a = OdinDocumentBuilder::new()
            .set("items[0]", OdinValues::string("first"))
            .set("items[1]", OdinValues::string("second"))
            .build()
            .unwrap();
        let b = OdinDocumentBuilder::new()
            .set("items[0]", OdinValues::string("first"))
            .build()
            .unwrap();
        let d = diff(&a, &b);
        assert_eq!(d.removed.len(), 1);
        assert_eq!(d.removed[0].path, "items[1]");
    }

    #[test]
    fn diff_array_item_changed() {
        let a = OdinDocumentBuilder::new()
            .set("items[0]", OdinValues::string("old"))
            .build()
            .unwrap();
        let b = OdinDocumentBuilder::new()
            .set("items[0]", OdinValues::string("new"))
            .build()
            .unwrap();
        let d = diff(&a, &b);
        assert_eq!(d.changed.len(), 1);
        assert_eq!(d.changed[0].path, "items[0]");
    }

    // ── OdinDiff helper methods ─────────────────────────────────────────────

    #[test]
    fn odin_diff_empty_constructor() {
        let d = OdinDiff::empty();
        assert!(d.is_empty());
        assert_eq!(d.added.len(), 0);
        assert_eq!(d.removed.len(), 0);
        assert_eq!(d.changed.len(), 0);
        assert_eq!(d.moved.len(), 0);
    }

    #[test]
    fn odin_diff_is_empty_false_when_has_additions() {
        let d = OdinDiff {
            added: vec![PathValue { path: "x".to_string(), value: OdinValues::integer(1) }],
            removed: vec![],
            changed: vec![],
            moved: vec![],
        };
        assert!(!d.is_empty());
    }

    #[test]
    fn odin_diff_is_empty_false_when_has_removals() {
        let d = OdinDiff {
            added: vec![],
            removed: vec![PathValue { path: "x".to_string(), value: OdinValues::integer(1) }],
            changed: vec![],
            moved: vec![],
        };
        assert!(!d.is_empty());
    }

    // ── Multi-section diff roundtrip ────────────────────────────────────────

    #[test]
    fn multi_section_roundtrip() {
        let a = OdinDocumentBuilder::new()
            .set("Policy.number", OdinValues::string("POL-001"))
            .set("Policy.status", OdinValues::string("active"))
            .set("Vehicle.vin", OdinValues::string("VIN123"))
            .set("Vehicle.make", OdinValues::string("Honda"))
            .set("Driver.name", OdinValues::string("Alice"))
            .build()
            .unwrap();
        let b = OdinDocumentBuilder::new()
            .set("Policy.number", OdinValues::string("POL-002"))
            .set("Policy.status", OdinValues::string("active"))
            .set("Vehicle.vin", OdinValues::string("VIN456"))
            .set("Vehicle.model", OdinValues::string("Civic"))
            .set("Driver.name", OdinValues::string("Bob"))
            .set("Driver.age", OdinValues::integer(30))
            .build()
            .unwrap();
        let d = diff(&a, &b);
        let patched = patch(&a, &d).unwrap();
        assert_eq!(patched.get_string("Policy.number"), Some("POL-002"));
        assert_eq!(patched.get_string("Vehicle.vin"), Some("VIN456"));
        assert!(!patched.has("Vehicle.make"));
        assert_eq!(patched.get_string("Vehicle.model"), Some("Civic"));
        assert_eq!(patched.get_string("Driver.name"), Some("Bob"));
        assert_eq!(patched.get_integer("Driver.age"), Some(30));
    }

    // ── Type change roundtrips ──────────────────────────────────────────────

    #[test]
    fn patch_type_change_string_to_integer() {
        let a = OdinDocumentBuilder::new()
            .set("val", OdinValues::string("42"))
            .build()
            .unwrap();
        let b = OdinDocumentBuilder::new()
            .set("val", OdinValues::integer(42))
            .build()
            .unwrap();
        let d = diff(&a, &b);
        let patched = patch(&a, &d).unwrap();
        assert_eq!(patched.get_integer("val"), Some(42));
    }

    #[test]
    fn patch_type_change_boolean_to_string() {
        let a = OdinDocumentBuilder::new()
            .set("val", OdinValues::boolean(true))
            .build()
            .unwrap();
        let b = OdinDocumentBuilder::new()
            .set("val", OdinValues::string("yes"))
            .build()
            .unwrap();
        let d = diff(&a, &b);
        let patched = patch(&a, &d).unwrap();
        assert_eq!(patched.get_string("val"), Some("yes"));
    }

    // ── Diff preserves value types correctly ────────────────────────────────

    #[test]
    fn diff_preserves_currency_type() {
        let a = OdinDocumentBuilder::new()
            .set("price", OdinValues::currency(100.0, 2))
            .build()
            .unwrap();
        let b = OdinDocumentBuilder::new()
            .set("price", OdinValues::currency(200.0, 2))
            .build()
            .unwrap();
        let d = diff(&a, &b);
        assert_eq!(d.changed.len(), 1);
        assert!(d.changed[0].old.is_currency());
        assert!(d.changed[0].new.is_currency());
    }

    #[test]
    fn diff_preserves_reference_type() {
        let a = OdinDocumentBuilder::new()
            .set("ref", OdinValues::reference("old.path"))
            .build()
            .unwrap();
        let b = OdinDocumentBuilder::new()
            .set("ref", OdinValues::reference("new.path"))
            .build()
            .unwrap();
        let d = diff(&a, &b);
        assert_eq!(d.changed.len(), 1);
        assert!(d.changed[0].old.is_reference());
        assert!(d.changed[0].new.is_reference());
    }
}
