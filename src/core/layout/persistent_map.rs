//! Small persistent ordered map for immutable measurement snapshots. Updating a
//! contribution copies only a balanced search path; worker captures share roots.
use std::cmp::Ordering;
use std::sync::Arc;

#[derive(Clone, Debug)]
pub(super) struct PersistentMap<K, V> {
    root: Link<K, V>,
}

type Link<K, V> = Option<Arc<Node<K, V>>>;

#[derive(Debug)]
struct Node<K, V> {
    key: K,
    value: V,
    left: Link<K, V>,
    right: Link<K, V>,
    height: u16,
    len: usize,
}

fn height<K, V>(node: &Link<K, V>) -> u16 {
    node.as_ref().map_or(0, |node| node.height)
}

fn len<K, V>(node: &Link<K, V>) -> usize {
    node.as_ref().map_or(0, |node| node.len)
}

impl<K, V> Node<K, V> {
    fn new(key: K, value: V, left: Link<K, V>, right: Link<K, V>) -> Arc<Self> {
        Arc::new(Self {
            height: 1 + height(&left).max(height(&right)),
            len: 1 + len(&left) + len(&right),
            key,
            value,
            left,
            right,
        })
    }
}

impl<K, V> Default for PersistentMap<K, V> {
    fn default() -> Self {
        Self { root: None }
    }
}

impl<K: Ord + Clone, V: Clone> PersistentMap<K, V> {
    pub(super) fn new() -> Self {
        Self::default()
    }

    pub(super) fn len(&self) -> usize {
        len(&self.root)
    }

    pub(super) fn clear(&mut self) {
        self.root = None;
    }

    pub(super) fn get(&self, key: &K) -> Option<&V> {
        let mut current = self.root.as_deref();
        while let Some(node) = current {
            current = match key.cmp(&node.key) {
                Ordering::Less => node.left.as_deref(),
                Ordering::Greater => node.right.as_deref(),
                Ordering::Equal => return Some(&node.value),
            };
        }
        None
    }

    pub(super) fn contains_key(&self, key: &K) -> bool {
        self.get(key).is_some()
    }

    pub(super) fn last_key_value(&self) -> Option<(&K, &V)> {
        let mut node = self.root.as_deref()?;
        while let Some(right) = node.right.as_deref() {
            node = right;
        }
        Some((&node.key, &node.value))
    }

    pub(super) fn insert(&mut self, key: K, value: V) -> Option<V> {
        let (root, previous) = insert(&self.root, key, value);
        self.root = Some(root);
        previous
    }

    pub(super) fn remove(&mut self, key: &K) -> Option<V> {
        let (root, previous) = remove(&self.root, key);
        self.root = root;
        previous
    }
}

fn rotate_left<K: Clone, V: Clone>(node: Arc<Node<K, V>>) -> Arc<Node<K, V>> {
    let pivot = node.right.as_ref().expect("right-heavy node");
    let left = Node::new(
        node.key.clone(),
        node.value.clone(),
        node.left.clone(),
        pivot.left.clone(),
    );
    Node::new(
        pivot.key.clone(),
        pivot.value.clone(),
        Some(left),
        pivot.right.clone(),
    )
}

fn rotate_right<K: Clone, V: Clone>(node: Arc<Node<K, V>>) -> Arc<Node<K, V>> {
    let pivot = node.left.as_ref().expect("left-heavy node");
    let right = Node::new(
        node.key.clone(),
        node.value.clone(),
        pivot.right.clone(),
        node.right.clone(),
    );
    Node::new(
        pivot.key.clone(),
        pivot.value.clone(),
        pivot.left.clone(),
        Some(right),
    )
}

fn balance<K: Clone, V: Clone>(mut node: Arc<Node<K, V>>) -> Arc<Node<K, V>> {
    if height(&node.left) > height(&node.right) + 1 {
        let left = node.left.as_ref().expect("left-heavy node");
        if height(&left.right) > height(&left.left) {
            node = Node::new(
                node.key.clone(),
                node.value.clone(),
                Some(rotate_left(left.clone())),
                node.right.clone(),
            );
        }
        rotate_right(node)
    } else if height(&node.right) > height(&node.left) + 1 {
        let right = node.right.as_ref().expect("right-heavy node");
        if height(&right.left) > height(&right.right) {
            node = Node::new(
                node.key.clone(),
                node.value.clone(),
                node.left.clone(),
                Some(rotate_right(right.clone())),
            );
        }
        rotate_left(node)
    } else {
        node
    }
}

fn insert<K: Ord + Clone, V: Clone>(
    root: &Link<K, V>,
    key: K,
    value: V,
) -> (Arc<Node<K, V>>, Option<V>) {
    let Some(node) = root else {
        return (Node::new(key, value, None, None), None);
    };
    let (next, previous) = match key.cmp(&node.key) {
        Ordering::Less => {
            let (left, previous) = insert(&node.left, key, value);
            (
                Node::new(
                    node.key.clone(),
                    node.value.clone(),
                    Some(left),
                    node.right.clone(),
                ),
                previous,
            )
        }
        Ordering::Greater => {
            let (right, previous) = insert(&node.right, key, value);
            (
                Node::new(
                    node.key.clone(),
                    node.value.clone(),
                    node.left.clone(),
                    Some(right),
                ),
                previous,
            )
        }
        Ordering::Equal => (
            Node::new(
                node.key.clone(),
                value,
                node.left.clone(),
                node.right.clone(),
            ),
            Some(node.value.clone()),
        ),
    };
    (balance(next), previous)
}

fn remove_first<K: Clone, V: Clone>(node: &Arc<Node<K, V>>) -> (Link<K, V>, K, V) {
    let Some(left) = &node.left else {
        return (node.right.clone(), node.key.clone(), node.value.clone());
    };
    let (left, key, value) = remove_first(left);
    (
        Some(balance(Node::new(
            node.key.clone(),
            node.value.clone(),
            left,
            node.right.clone(),
        ))),
        key,
        value,
    )
}

fn remove<K: Ord + Clone, V: Clone>(root: &Link<K, V>, key: &K) -> (Link<K, V>, Option<V>) {
    let Some(node) = root else {
        return (None, None);
    };
    let (next, previous) = match key.cmp(&node.key) {
        Ordering::Less => {
            let (left, previous) = remove(&node.left, key);
            if previous.is_none() {
                return (root.clone(), None);
            }
            (
                Node::new(
                    node.key.clone(),
                    node.value.clone(),
                    left,
                    node.right.clone(),
                ),
                previous,
            )
        }
        Ordering::Greater => {
            let (right, previous) = remove(&node.right, key);
            if previous.is_none() {
                return (root.clone(), None);
            }
            (
                Node::new(
                    node.key.clone(),
                    node.value.clone(),
                    node.left.clone(),
                    right,
                ),
                previous,
            )
        }
        Ordering::Equal => {
            let previous = Some(node.value.clone());
            if node.left.is_none() {
                return (node.right.clone(), previous);
            }
            let Some(right) = &node.right else {
                return (node.left.clone(), previous);
            };
            let (right, key, value) = remove_first(right);
            (Node::new(key, value, node.left.clone(), right), previous)
        }
    };
    (Some(balance(next)), previous)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn validate(node: &Link<u32, u32>, output: &mut BTreeMap<u32, u32>) {
        let Some(node) = node else {
            return;
        };
        assert!(height(&node.left).abs_diff(height(&node.right)) <= 1);
        assert_eq!(node.height, 1 + height(&node.left).max(height(&node.right)));
        assert_eq!(node.len, 1 + len(&node.left) + len(&node.right));
        validate(&node.left, output);
        assert!(output.insert(node.key, node.value).is_none());
        validate(&node.right, output);
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
            let mut actual = BTreeMap::new();
            validate(&map.root, &mut actual);
            assert_eq!(actual, expected);
        }
    }

    #[test]
    fn one_update_copies_only_a_balanced_path() {
        use std::sync::atomic::{AtomicUsize, Ordering};
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
