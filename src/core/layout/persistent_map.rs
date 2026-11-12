//! Small persistent ordered map for immutable measurement snapshots. Updating a
//! contribution copies only a balanced search path; worker captures share roots.
use crate::persistent::{Item, LeafSplit, Seek, Side, Summary, Tree, Work};

#[derive(Clone, Debug)]
pub(super) struct PersistentMap<K: Ord + Clone, V> {
    entries: Tree<Entry<K, V>>,
}

#[derive(Debug)]
struct Entry<K, V> {
    key: K,
    value: V,
}

#[derive(Clone, Debug, PartialEq)]
struct Keys<K> {
    count: usize,
    last: Option<K>,
}

impl<K> Default for Keys<K> {
    fn default() -> Self {
        Self { count: 0, last: None }
    }
}

impl<K: Clone> Summary for Keys<K> {
    fn add(&mut self, right: &Self) {
        self.count += right.count;
        if right.last.is_some() {
            self.last.clone_from(&right.last);
        }
    }
}

impl<K: Ord + Clone, V> Item for Entry<K, V> {
    type Summary = Keys<K>;
    type Tag = ();

    fn summary(&self) -> Keys<K> {
        Keys { count: 1, last: Some(self.key.clone()) }
    }
}

/// The first entry whose key is not less than the target.
struct NotBefore<'a, K>(&'a K);

impl<K: Ord> Seek<Keys<K>> for NotBefore<'_, K> {
    fn side(&self, left: &Keys<K>) -> Side {
        if left.last.as_ref().is_some_and(|last| self.0 <= last) { Side::Left } else { Side::Right }
    }

    fn advance(&mut self, _left: &Keys<K>) {}
}

impl<K: Ord + Clone, V> Default for PersistentMap<K, V> {
    fn default() -> Self {
        Self { entries: Tree::new() }
    }
}

impl<K: Ord + Clone, V: Clone> PersistentMap<K, V> {
    pub(super) fn new() -> Self {
        Self::default()
    }

    pub(super) fn len(&self) -> usize {
        self.entries.measure(|keys| keys.count)
    }

    pub(super) fn clear(&mut self) {
        self.entries = Tree::new();
    }

    pub(super) fn get(&self, key: &K) -> Option<&V> {
        let (entry, _) = self.entries.find(&mut NotBefore(key), &mut Work::default())?;
        (entry.key == *key).then_some(&entry.value)
    }

    pub(super) fn contains_key(&self, key: &K) -> bool {
        self.get(key).is_some()
    }

    pub(super) fn last_key_value(&self) -> Option<(&K, &V)> {
        self.entries.last().map(|entry| (&entry.key, &entry.value))
    }

    pub(super) fn insert(&mut self, key: K, value: V) -> Option<V> {
        let work = &mut Work::default();
        let (before, previous, after) = self.without(&key, work);
        let entry = Tree::from_items([Entry { key, value }], work);
        self.entries = before.concat(&entry, work).concat(&after, work);
        previous
    }

    pub(super) fn remove(&mut self, key: &K) -> Option<V> {
        let work = &mut Work::default();
        let (before, previous, after) = self.without(key, work);
        if previous.is_some() {
            self.entries = before.concat(&after, work);
        }
        previous
    }

    /// The entries before and after `key`, and the value stored at it.
    #[allow(clippy::type_complexity)]
    fn without(&self, key: &K, work: &mut Work) -> (Tree<Entry<K, V>>, Option<V>, Tree<Entry<K, V>>) {
        let (before, from) = self.entries.split(
            &mut NotBefore(key),
            |entry, seek| if entry.key < *seek.0 { LeafSplit::After } else { LeafSplit::Before },
            work,
        );
        match from.first() {
            Some(entry) if entry.key == *key => {
                let previous = Some(entry.value.clone());
                let (_, after) =
                    from.split_at(1, |keys| keys.count, |_, _| unreachable!("entries are whole"), work);
                (before, previous, after)
            }
            _ => (before, None, from),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn validate(map: &PersistentMap<u32, u32>) -> BTreeMap<u32, u32> {
        assert!(map.entries.invariant_holds());
        let entries = map.entries.iter().map(|entry| (entry.key, entry.value)).collect::<Vec<_>>();
        assert!(entries.windows(2).all(|pair| pair[0].0 < pair[1].0));
        entries.into_iter().collect()
    }

    #[test]
    fn rotations_removals_and_retained_versions_match_ordered_map() {
        let mut map = PersistentMap::new();
        let mut expected = BTreeMap::new();
        let mut retained = Vec::new();
        let mut seed = 13u32;
        for value in 0..10_000 {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            let key = (seed >> 8) % 1024;
            if value % 3 == 0 {
                assert_eq!(map.remove(&key), expected.remove(&key));
            } else {
                assert_eq!(map.insert(key, value), expected.insert(key, value));
            }
            assert_eq!(map.len(), expected.len());
            assert_eq!(map.get(&key), expected.get(&key));
            assert_eq!(map.contains_key(&key), expected.contains_key(&key));
            assert_eq!(map.last_key_value(), expected.last_key_value());
            if value % 127 == 0 {
                retained.push((map.clone(), expected.clone()));
            }
        }
        retained.push((map.clone(), expected));
        map.clear();
        assert_eq!(map.len(), 0);
        for (map, expected) in retained {
            assert_eq!(validate(&map), expected);
        }
    }

    #[test]
    fn one_update_copies_only_a_balanced_path() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;
        #[derive(Debug)]
        struct Counted(Arc<AtomicUsize>);
        impl Clone for Counted {
            fn clone(&self) -> Self {
                self.0.fetch_add(1, Ordering::Relaxed);
                Self(self.0.clone())
            }
        }
        let copies = Arc::new(AtomicUsize::new(0));
        let mut map = PersistentMap::new();
        for key in 0..65_536 {
            map.insert(key, Counted(copies.clone()));
        }
        let retained = map.clone();
        copies.store(0, Ordering::Relaxed);
        map.insert(32_000, Counted(copies.clone()));
        map.remove(&48_000);
        assert!(copies.load(Ordering::Relaxed) < 100);
        assert_eq!(retained.len(), 65_536);
        assert_eq!(map.len(), 65_535);
        assert!(retained.get(&48_000).is_some());
    }
}
