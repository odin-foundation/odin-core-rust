//! Insertion-order preserving map.
//!
//! Wraps `indexmap::IndexMap` with a fast non-cryptographic hasher.
//! Stores each key once (vs the previous double-storage design).

use std::fmt;

use indexmap::IndexMap;
use rustc_hash::FxBuildHasher;

/// An insertion-order preserving map with O(1) key lookups.
///
/// Iteration order equals insertion order (matches JavaScript `Map` semantics).
#[derive(Clone)]
pub struct OrderedMap<K: Clone + Eq + std::hash::Hash, V: Clone> {
    inner: IndexMap<K, V, FxBuildHasher>,
}

impl<K: Clone + Eq + std::hash::Hash + fmt::Debug, V: Clone + fmt::Debug> fmt::Debug
    for OrderedMap<K, V>
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_map().entries(self.inner.iter()).finish()
    }
}

impl<K: Clone + Eq + std::hash::Hash, V: Clone + PartialEq> PartialEq for OrderedMap<K, V> {
    fn eq(&self, other: &Self) -> bool {
        if self.inner.len() != other.inner.len() {
            return false;
        }
        // Insertion-order-sensitive comparison (matches previous semantics).
        self.inner
            .iter()
            .zip(other.inner.iter())
            .all(|((ak, av), (bk, bv))| ak == bk && av == bv)
    }
}

impl<K: Clone + Eq + std::hash::Hash, V: Clone + PartialEq> Eq for OrderedMap<K, V> {}

impl<K: Clone + Eq + std::hash::Hash, V: Clone> Default for OrderedMap<K, V> {
    fn default() -> Self {
        Self::new()
    }
}

impl<K: Clone + Eq + std::hash::Hash, V: Clone> OrderedMap<K, V> {
    pub fn new() -> Self {
        Self { inner: IndexMap::with_hasher(FxBuildHasher) }
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self { inner: IndexMap::with_capacity_and_hasher(capacity, FxBuildHasher) }
    }

    /// Insert or replace; preserves insertion position when replacing.
    pub fn insert(&mut self, key: K, value: V) -> Option<V> {
        self.inner.insert(key, value)
    }

    /// Entry API for fused contains+insert in a single hash lookup.
    pub fn entry(&mut self, key: K) -> indexmap::map::Entry<'_, K, V> {
        self.inner.entry(key)
    }

    pub fn get(&self, key: &K) -> Option<&V> {
        self.inner.get(key)
    }

    pub fn get_mut(&mut self, key: &K) -> Option<&mut V> {
        self.inner.get_mut(key)
    }

    pub fn contains_key(&self, key: &K) -> bool {
        self.inner.contains_key(key)
    }

    /// Remove a key, shifting later entries to maintain insertion order. O(n).
    pub fn remove(&mut self, key: &K) -> Option<V> {
        self.inner.shift_remove(key)
    }

    pub fn len(&self) -> usize {
        self.inner.len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    pub fn clear(&mut self) {
        self.inner.clear();
    }

    pub fn iter(&self) -> impl Iterator<Item = (&K, &V)> {
        self.inner.iter()
    }

    pub fn keys(&self) -> impl Iterator<Item = &K> {
        self.inner.keys()
    }

    pub fn values(&self) -> impl Iterator<Item = &V> {
        self.inner.values()
    }

    pub fn values_mut(&mut self) -> impl Iterator<Item = &mut V> {
        self.inner.values_mut()
    }

    pub fn into_vec(self) -> Vec<(K, V)> {
        self.inner.into_iter().collect()
    }

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
    type IntoIter = indexmap::map::IntoIter<K, V>;

    fn into_iter(self) -> Self::IntoIter {
        self.inner.into_iter()
    }
}

impl<'a, K: Clone + Eq + std::hash::Hash, V: Clone> IntoIterator for &'a OrderedMap<K, V> {
    type Item = (&'a K, &'a V);
    type IntoIter = indexmap::map::Iter<'a, K, V>;

    fn into_iter(self) -> Self::IntoIter {
        self.inner.iter()
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
        map.insert("b", 20);

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

    #[test]
    fn test_overwrite_preserves_position() {
        let mut map = OrderedMap::new();
        map.insert("a", 1);
        map.insert("b", 2);
        map.insert("c", 3);
        map.insert("a", 100);
        let keys: Vec<_> = map.keys().copied().collect();
        assert_eq!(keys, vec!["a", "b", "c"]);
        assert_eq!(map.get(&"a"), Some(&100));
    }

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
        assert_eq!(map.get(&"a"), Some(&1));
        assert_eq!(map.get(&"c"), Some(&3));
        assert_eq!(map.get(&"d"), Some(&4));
        assert!(!map.contains_key(&"b"));
    }

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

    #[test]
    fn test_with_capacity() {
        let mut map: OrderedMap<String, i32> = OrderedMap::with_capacity(100);
        assert!(map.is_empty());
        map.insert("a".to_string(), 1);
        assert_eq!(map.len(), 1);
    }

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

    #[test]
    fn test_string_keys() {
        let mut map: OrderedMap<String, String> = OrderedMap::new();
        map.insert("hello".to_string(), "world".to_string());
        assert_eq!(map.get(&"hello".to_string()), Some(&"world".to_string()));
    }

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

    #[test]
    fn test_debug_format() {
        let mut map = OrderedMap::new();
        map.insert("a", 1);
        let debug = format!("{:?}", map);
        assert!(debug.contains("a"));
        assert!(debug.contains("1"));
    }
}
