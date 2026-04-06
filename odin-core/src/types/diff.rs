//! Types for ODIN document diff and patch operations.

use crate::types::values::OdinValue;

/// A diff between two ODIN documents.
#[derive(Debug, Clone, PartialEq)]
pub struct OdinDiff {
    /// Paths that were added (exist in B but not A).
    pub added: Vec<PathValue>,
    /// Paths that were removed (exist in A but not B).
    pub removed: Vec<PathValue>,
    /// Paths where the value changed.
    pub changed: Vec<PathChange>,
    /// Paths that were moved (same value, different path).
    pub moved: Vec<PathMove>,
}

impl OdinDiff {
    /// Create an empty diff (no changes).
    pub fn empty() -> Self {
        Self {
            added: Vec::new(),
            removed: Vec::new(),
            changed: Vec::new(),
            moved: Vec::new(),
        }
    }

    /// Returns `true` if there are no differences.
    pub fn is_empty(&self) -> bool {
        self.added.is_empty()
            && self.removed.is_empty()
            && self.changed.is_empty()
            && self.moved.is_empty()
    }
}

/// A path-value pair (for additions and removals).
#[derive(Debug, Clone, PartialEq)]
pub struct PathValue {
    /// The field path.
    pub path: String,
    /// The value at that path.
    pub value: OdinValue,
}

/// A path with old and new values (for changes).
#[derive(Debug, Clone, PartialEq)]
pub struct PathChange {
    /// The field path.
    pub path: String,
    /// The old value.
    pub old: OdinValue,
    /// The new value.
    pub new: OdinValue,
}

/// A path movement (value moved from one path to another).
#[derive(Debug, Clone, PartialEq)]
pub struct PathMove {
    /// The original path.
    pub from: String,
    /// The new path.
    pub to: String,
    /// The value that was moved.
    pub value: OdinValue,
}
