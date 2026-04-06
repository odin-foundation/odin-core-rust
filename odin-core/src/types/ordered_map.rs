//! Insertion-order preserving map.
//!
//! `OrderedMap` maintains insertion order while providing O(1) key lookups.
//! This matches the behavior of JavaScript's `Map` which is used throughout
//! the TypeScript reference implementation.

use std::collections::HashMap;
use std::fmt;

/// An insertion-order preserving map with O(1) key lookups.
///
/// Uses a `Vec<(K, V)>` for ordered storage and a `HashMap<K, usize>` for
/// fast index lookups. This matches JavaScript's `Map` semantics where
/// iteration order equals insertion order.
#[derive(Clone)]
pub struct OrderedMap<K: Clone + Eq + std::hash::Hash, V: Clone> {
    entries: Vec<(K, V)>,
    index: HashMap<K, usize>,
}

impl<K: Clone + Eq + std::hash::Hash + fmt::Debug, V: Clone + fmt::Debug> fmt::Debug
    for OrderedMap<K, V>
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_map()
            .entries(self.entries.iter().map(|(k, v)| (k, v)))
            .finish()
    }
}

impl<K: Clone + Eq + std::hash::Hash + PartialEq, V: Clone + PartialEq> PartialEq
    for OrderedMap<K, V>
{
    fn eq(&self, other: &Self) -> bool {
        self.entries == other.entries
    }
}

impl<K: Clone + Eq + std::hash::Hash + PartialEq, V: Clone + PartialEq> Eq for OrderedMap<K, V> {}

impl<K: Clone + Eq + std::hash::Hash, V: Clone> Default for OrderedMap<K, V> {
    fn default() -> Self {
        Self::new()
    }
}

impl<K: Clone + Eq + std::hash::Hash, V: Clone> OrderedMap<K, V> {
    /// Create a new empty ordered map.
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            index: HashMap::new(),
        }
    }

    /// Create a new ordered map with the given capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            entries: Vec::with_capacity(capacity),
            index: HashMap::with_capacity(capacity),
        }
    }

    /// Insert a key-value pair. If the key already exists, updates the value
    /// and returns the old value. Insertion order of existing keys is preserved.
    pub fn insert(&mut self, key: K, value: V) -> Option<V> {
        if let Some(&idx) = self.index.get(&key) {
            let old = std::mem::replace(&mut self.entries[idx].1, value);
            Some(old)
        } else {
            let idx = self.entries.len();
            self.index.insert(key.clone(), idx);
            self.entries.push((key, value));
            None
        }
    }

    /// Get a reference to the value for a key.
    pub fn get(&self, key: &K) -> Option<&V> {
        self.index.get(key).map(|&idx| &self.entries[idx].1)
    }

    /// Get a mutable reference to the value for a key.
    pub fn get_mut(&mut self, key: &K) -> Option<&mut V> {
        self.index
            .get(key)
            .copied()
            .map(|idx| &mut self.entries[idx].1)
    }

    /// Returns `true` if the map contains the given key.
    pub fn contains_key(&self, key: &K) -> bool {
        self.index.contains_key(key)
    }

    /// Remove a key-value pair. Returns the value if it existed.
    ///
    /// Note: This is O(n) because it shifts elements to maintain order.
    pub fn remove(&mut self, key: &K) -> Option<V> {
        if let Some(&idx) = self.index.get(key) {
            let (_, value) = self.entries.remove(idx);
            self.index.remove(key);
            // Update indices for entries that shifted down
            for (k, existing_idx) in &mut self.index {
                if *existing_idx > idx {
                    *existing_idx -= 1;
                }
                let _ = k; // suppress unused warning
            }
            Some(value)
        } else {
            None
        }
    }

    /// Returns the number of entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns `true` if the map is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Clear all entries.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.index.clear();
    }

    /// Iterate over key-value pairs in insertion order.
    pub fn iter(&self) -> impl Iterator<Item = (&K, &V)> {
        self.entries.iter().map(|(k, v)| (k, v))
    }

    /// Iterate over keys in insertion order.
    pub fn keys(&self) -> impl Iterator<Item = &K> {
        self.entries.iter().map(|(k, _)| k)
    }

    /// Iterate over values in insertion order.
    pub fn values(&self) -> impl Iterator<Item = &V> {
        self.entries.iter().map(|(_, v)| v)
    }

    /// Iterate over mutable values in insertion order.
    pub fn values_mut(&mut self) -> impl Iterator<Item = &mut V> {
        self.entries.iter_mut().map(|(_, v)| v)
    }

    /// Convert to a vector of key-value pairs (consuming the map).
    pub fn into_vec(self) -> Vec<(K, V)> {
        self.entries
    }

    /// Create from a vector of key-value pairs.
    pub fn from_vec(entries: Vec<(K, V)>) -> Self {
        let mut map = Self::with_capacity(entries.len());
        for (k, v) in entries {
            map.insert(k, v);
        }
        map
    }
}

impl<K: Clone + Eq + std::hash::Hash, V: Clone> FromIterator<(K, V)> for OrderedMap<K, V> {
    fn from_iter<I: IntoIterator<Item = (K, V)>>(iter: I) -> Self {
        let iter = iter.into_iter();
        let (lower, _) = iter.size_hint();
        let mut map = Self::with_capacity(lower);
        for (k, v) in iter {
            map.insert(k, v);
        }
        map
    }
}

impl<K: Clone + Eq + std::hash::Hash, V: Clone> IntoIterator for OrderedMap<K, V> {
    type Item = (K, V);
    type IntoIter = std::vec::IntoIter<(K, V)>;

    fn into_iter(self) -> Self::IntoIter {
        self.entries.into_iter()
    }
}

impl<'a, K: Clone + Eq + std::hash::Hash, V: Clone> IntoIterator for &'a OrderedMap<K, V> {
    type Item = (&'a K, &'a V);
    type IntoIter = std::iter::Map<
        std::slice::Iter<'a, (K, V)>,
        fn(&'a (K, V)) -> (&'a K, &'a V),
    >;

    fn into_iter(self) -> Self::IntoIter {
        self.entries.iter().map(|(k, v)| (k, v))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_insertion_order() {
        let mut map = OrderedMap::new();
        map.insert("c", 3);
        map.insert("a", 1);
        map.insert("b", 2);

        let keys: Vec<_> = map.keys().copied().collect();
        assert_eq!(keys, vec!["c", "a", "b"]);
    }

    #[test]
    fn test_update_preserves_order() {
        let mut map = OrderedMap::new();
        map.insert("a", 1);
        map.insert("b", 2);
        map.insert("c", 3);
        map.insert("b", 20); // Update existing key

        let pairs: Vec<_> = map.iter().map(|(&k, &v)| (k, v)).collect();
        assert_eq!(pairs, vec![("a", 1), ("b", 20), ("c", 3)]);
    }

    #[test]
    fn test_get_and_contains() {
        let mut map = OrderedMap::new();
        map.insert("x", 42);

        assert_eq!(map.get(&"x"), Some(&42));
        assert_eq!(map.get(&"y"), None);
        assert!(map.contains_key(&"x"));
        assert!(!map.contains_key(&"y"));
    }

    #[test]
    fn test_remove() {
        let mut map = OrderedMap::new();
        map.insert("a", 1);
        map.insert("b", 2);
        map.insert("c", 3);

        assert_eq!(map.remove(&"b"), Some(2));
        assert_eq!(map.len(), 2);

        let keys: Vec<_> = map.keys().copied().collect();
        assert_eq!(keys, vec!["a", "c"]);
    }

    #[test]
    fn test_from_iterator() {
        let map: OrderedMap<&str, i32> = vec![("a", 1), ("b", 2)].into_iter().collect();
        assert_eq!(map.len(), 2);
        assert_eq!(map.get(&"a"), Some(&1));
    }

    // ─── Empty map tests ─────────────────────────────────────────────────

    #[test]
    fn test_empty_map() {
        let map: OrderedMap<String, i32> = OrderedMap::new();
        assert!(map.is_empty());
        assert_eq!(map.len(), 0);
        assert_eq!(map.get(&"x".to_string()), None);
    }

    #[test]
    fn test_default_is_empty() {
        let map: OrderedMap<String, i32> = OrderedMap::default();
        assert!(map.is_empty());
    }

    // ─── Insert and get tests ────────────────────────────────────────────

    #[test]
    fn test_insert_returns_none_for_new() {
        let mut map = OrderedMap::new();
        let old = map.insert("a", 1);
        assert_eq!(old, None);
    }

    #[test]
    fn test_insert_returns_old_for_existing() {
        let mut map = OrderedMap::new();
        map.insert("a", 1);
        let old = map.insert("a", 2);
        assert_eq!(old, Some(1));
    }

    #[test]
    fn test_get_mut() {
        let mut map = OrderedMap::new();
        map.insert("a", 1);
        if let Some(val) = map.get_mut(&"a") {
            *val = 99;
        }
        assert_eq!(map.get(&"a"), Some(&99));
    }

    // ─── Insertion order tests ───────────────────────────────────────────

    #[test]
    fn test_insertion_order_many_items() {
        let mut map = OrderedMap::new();
        for i in 0..100 {
            map.insert(format!("key_{:03}", i), i);
        }
        let keys: Vec<_> = map.keys().collect();
        for (i, key) in keys.iter().enumerate() {
            assert_eq!(**key, format!("key_{:03}", i));
        }
    }

    #[test]
    fn test_values_in_insertion_order() {
        let mut map = OrderedMap::new();
        map.insert("c", 3);
        map.insert("a", 1);
        map.insert("b", 2);
        let values: Vec<_> = map.values().copied().collect();
        assert_eq!(values, vec![3, 1, 2]);
    }

    #[test]
    fn test_iter_in_insertion_order() {
        let mut map = OrderedMap::new();
        map.insert("z", 26);
        map.insert("a", 1);
        let pairs: Vec<_> = map.iter().map(|(&k, &v)| (k, v)).collect();
        assert_eq!(pairs, vec![("z", 26), ("a", 1)]);
    }

    // ─── Update preserves order tests ────────────────────────────────────

    #[test]
    fn test_overwrite_preserves_position() {
        let mut map = OrderedMap::new();
        map.insert("a", 1);
        map.insert("b", 2);
        map.insert("c", 3);
        map.insert("a", 100); // Update existing
        let keys: Vec<_> = map.keys().copied().collect();
        assert_eq!(keys, vec!["a", "b", "c"]);
        assert_eq!(map.get(&"a"), Some(&100));
    }

    // ─── Remove tests ────────────────────────────────────────────────────

    #[test]
    fn test_remove_returns_none_for_missing() {
        let mut map: OrderedMap<&str, i32> = OrderedMap::new();
        assert_eq!(map.remove(&"missing"), None);
    }

    #[test]
    fn test_remove_first_element() {
        let mut map = OrderedMap::new();
        map.insert("a", 1);
        map.insert("b", 2);
        map.insert("c", 3);
        assert_eq!(map.remove(&"a"), Some(1));
        let keys: Vec<_> = map.keys().copied().collect();
        assert_eq!(keys, vec!["b", "c"]);
        // Verify get still works after removal
        assert_eq!(map.get(&"b"), Some(&2));
        assert_eq!(map.get(&"c"), Some(&3));
    }

    #[test]
    fn test_remove_last_element() {
        let mut map = OrderedMap::new();
        map.insert("a", 1);
        map.insert("b", 2);
        assert_eq!(map.remove(&"b"), Some(2));
        assert_eq!(map.len(), 1);
        assert_eq!(map.get(&"a"), Some(&1));
    }

    #[test]
    fn test_remove_all_elements() {
        let mut map = OrderedMap::new();
        map.insert("a", 1);
        map.insert("b", 2);
        map.remove(&"a");
        map.remove(&"b");
        assert!(map.is_empty());
    }

    #[test]
    fn test_remove_middle_then_get() {
        let mut map = OrderedMap::new();
        map.insert("a", 1);
        map.insert("b", 2);
        map.insert("c", 3);
        map.insert("d", 4);
        map.remove(&"b");
        // After removing "b", indices should be correct
        assert_eq!(map.get(&"a"), Some(&1));
        assert_eq!(map.get(&"c"), Some(&3));
        assert_eq!(map.get(&"d"), Some(&4));
        assert!(!map.contains_key(&"b"));
    }

    // ─── Contains key tests ──────────────────────────────────────────────

    #[test]
    fn test_contains_key_after_insert() {
        let mut map = OrderedMap::new();
        map.insert("x", 1);
        assert!(map.contains_key(&"x"));
    }

    #[test]
    fn test_contains_key_after_remove() {
        let mut map = OrderedMap::new();
        map.insert("x", 1);
        map.remove(&"x");
        assert!(!map.contains_key(&"x"));
    }

    // ─── Clear tests ─────────────────────────────────────────────────────

    #[test]
    fn test_clear() {
        let mut map = OrderedMap::new();
        map.insert("a", 1);
        map.insert("b", 2);
        map.clear();
        assert!(map.is_empty());
        assert_eq!(map.len(), 0);
        assert!(!map.contains_key(&"a"));
    }

    // ─── with_capacity tests ─────────────────────────────────────────────

    #[test]
    fn test_with_capacity() {
        let mut map: OrderedMap<String, i32> = OrderedMap::with_capacity(100);
        assert!(map.is_empty());
        map.insert("a".to_string(), 1);
        assert_eq!(map.len(), 1);
    }

    // ─── from_vec / into_vec tests ───────────────────────────────────────

    #[test]
    fn test_into_vec() {
        let mut map = OrderedMap::new();
        map.insert("a", 1);
        map.insert("b", 2);
        let vec = map.into_vec();
        assert_eq!(vec, vec![("a", 1), ("b", 2)]);
    }

    #[test]
    fn test_from_vec() {
        let map = OrderedMap::from_vec(vec![("a", 1), ("b", 2), ("c", 3)]);
        assert_eq!(map.len(), 3);
        assert_eq!(map.get(&"b"), Some(&2));
    }

    #[test]
    fn test_from_vec_deduplicates() {
        let map = OrderedMap::from_vec(vec![("a", 1), ("a", 2)]);
        assert_eq!(map.len(), 1);
        assert_eq!(map.get(&"a"), Some(&2));
    }

    // ─── Clone / Equality tests ──────────────────────────────────────────

    #[test]
    fn test_clone() {
        let mut map = OrderedMap::new();
        map.insert("a", 1);
        map.insert("b", 2);
        let cloned = map.clone();
        assert_eq!(map, cloned);
    }

    #[test]
    fn test_equality_same_order() {
        let mut m1 = OrderedMap::new();
        m1.insert("a", 1);
        m1.insert("b", 2);
        let mut m2 = OrderedMap::new();
        m2.insert("a", 1);
        m2.insert("b", 2);
        assert_eq!(m1, m2);
    }

    #[test]
    fn test_inequality_different_order() {
        let mut m1 = OrderedMap::new();
        m1.insert("a", 1);
        m1.insert("b", 2);
        let mut m2 = OrderedMap::new();
        m2.insert("b", 2);
        m2.insert("a", 1);
        // Different order means not equal (ordered map)
        assert_ne!(m1, m2);
    }

    #[test]
    fn test_inequality_different_values() {
        let mut m1 = OrderedMap::new();
        m1.insert("a", 1);
        let mut m2 = OrderedMap::new();
        m2.insert("a", 2);
        assert_ne!(m1, m2);
    }

    // ─── IntoIterator tests ──────────────────────────────────────────────

    #[test]
    fn test_into_iterator_owned() {
        let mut map = OrderedMap::new();
        map.insert("a", 1);
        map.insert("b", 2);
        let collected: Vec<_> = map.into_iter().collect();
        assert_eq!(collected, vec![("a", 1), ("b", 2)]);
    }

    #[test]
    fn test_into_iterator_ref() {
        let mut map = OrderedMap::new();
        map.insert("a", 1);
        map.insert("b", 2);
        let collected: Vec<_> = (&map).into_iter().map(|(&k, &v)| (k, v)).collect();
        assert_eq!(collected, vec![("a", 1), ("b", 2)]);
    }

    // ─── values_mut tests ────────────────────────────────────────────────

    #[test]
    fn test_values_mut() {
        let mut map = OrderedMap::new();
        map.insert("a", 1);
        map.insert("b", 2);
        for v in map.values_mut() {
            *v *= 10;
        }
        assert_eq!(map.get(&"a"), Some(&10));
        assert_eq!(map.get(&"b"), Some(&20));
    }

    // ─── String key tests ────────────────────────────────────────────────

    #[test]
    fn test_string_keys() {
        let mut map: OrderedMap<String, String> = OrderedMap::new();
        map.insert("hello".to_string(), "world".to_string());
        assert_eq!(map.get(&"hello".to_string()), Some(&"world".to_string()));
    }

    // ─── Insert after remove tests ───────────────────────────────────────

    #[test]
    fn test_insert_after_remove() {
        let mut map = OrderedMap::new();
        map.insert("a", 1);
        map.insert("b", 2);
        map.remove(&"a");
        map.insert("c", 3);
        let keys: Vec<_> = map.keys().copied().collect();
        assert_eq!(keys, vec!["b", "c"]);
    }

    // ─── Debug format test ───────────────────────────────────────────────

    #[test]
    fn test_debug_format() {
        let mut map = OrderedMap::new();
        map.insert("a", 1);
        let debug = format!("{:?}", map);
        assert!(debug.contains("a"));
        assert!(debug.contains("1"));
    }
}
