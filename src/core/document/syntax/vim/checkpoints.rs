//! Persistent treap of snapshot-local indexes. Values retain stable leaf anchors;
//! subtree shifts rebase indexes without walking later checkpoint values.
use super::Restart;
use std::sync::Arc;
type Link = Option<Arc<Node>>;
#[derive(Clone, Debug)]
struct Node {
    key: usize,
    priority: u64,
    value: Arc<Restart>,
    left: Link,
    right: Link,
    shift: isize,
    count: usize,
    bytes: usize,
}
#[derive(Clone, Debug, Default)]
pub(super) struct Checkpoints {
    root: Link,
    pub visits: usize,
}
fn count(n: &Link) -> usize {
    n.as_ref().map_or(0, |n| n.count)
}
fn shifted(n: &Link, delta: isize) -> Link {
    n.as_ref().map(|n| {
        let mut n = (**n).clone();
        n.key = n
            .key
            .checked_add_signed(delta)
            .expect("validated checkpoint shift");
        n.shift += delta;
        Arc::new(n)
    })
}
fn push(n: &Arc<Node>) -> Node {
    let mut n = (**n).clone();
    if n.shift != 0 {
        n.left = shifted(&n.left, n.shift);
        n.right = shifted(&n.right, n.shift);
        n.shift = 0;
    }
    n
}
fn bytes(n: &Link) -> usize {
    n.as_ref().map_or(0, |n| n.bytes)
}
fn node(mut n: Node) -> Link {
    n.count = 1 + count(&n.left) + count(&n.right);
    n.bytes =
        std::mem::size_of::<Node>() + n.value.memory_usage() + bytes(&n.left) + bytes(&n.right);
    Some(Arc::new(n))
}
fn split(root: Link, key: usize, visits: &mut usize) -> (Link, Link) {
    let Some(root) = root else {
        return (None, None);
    };
    *visits += 1;
    let mut n = push(&root);
    if n.key < key {
        let (a, b) = split(n.right.take(), key, visits);
        n.right = a;
        (node(n), b)
    } else {
        let (a, b) = split(n.left.take(), key, visits);
        n.left = b;
        (a, node(n))
    }
}
fn merge(a: Link, b: Link, visits: &mut usize) -> Link {
    match (a, b) {
        (None, b) => b,
        (a, None) => a,
        (Some(a), Some(b)) => {
            *visits += 1;
            if a.priority > b.priority {
                let mut n = push(&a);
                n.right = merge(n.right.take(), Some(b), visits);
                node(n)
            } else {
                let mut n = push(&b);
                n.left = merge(Some(a), n.left.take(), visits);
                node(n)
            }
        }
    }
}
impl Checkpoints {
    pub fn len(&self) -> usize {
        count(&self.root)
    }
    pub fn memory_usage(&self) -> usize {
        bytes(&self.root)
    }
    pub fn before(&mut self, at: usize) -> Option<(usize, Arc<Restart>)> {
        let mut current = self.root.as_ref();
        let mut shift = 0isize;
        let mut result = None;
        while let Some(n) = current {
            self.visits += 1;
            let key = n.key.checked_add_signed(shift)?;
            if key <= at {
                result = Some((key, n.value.clone()));
                shift += n.shift;
                current = n.right.as_ref();
            } else {
                shift += n.shift;
                current = n.left.as_ref();
            }
        }
        result
    }
    pub fn exact(&mut self, at: usize) -> Option<Arc<Restart>> {
        self.before(at).filter(|(k, _)| *k == at).map(|(_, v)| v)
    }
    pub fn insert(&mut self, key: usize, value: Restart) {
        let (a, b) = split(self.root.take(), key, &mut self.visits);
        let (_, b) = split(b, key.saturating_add(1), &mut self.visits);
        let mut priority = (key as u64).wrapping_add(0x9e3779b97f4a7c15);
        priority = (priority ^ (priority >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        priority = (priority ^ (priority >> 27)).wrapping_mul(0x94d049bb133111eb);
        priority ^= priority >> 31;
        let n = node(Node {
            key,
            priority,
            value: Arc::new(value),
            left: None,
            right: None,
            shift: 0,
            count: 1,
            bytes: 0,
        });
        self.root = merge(merge(a, n, &mut self.visits), b, &mut self.visits);
    }
    pub fn edit(&mut self, start: usize, old_end: usize, new_end: usize) {
        let (a, b) = split(self.root.take(), start, &mut self.visits);
        let (_, b) = split(b, old_end.saturating_add(1), &mut self.visits);
        let delta = new_end as isize - old_end as isize;
        self.root = merge(a, shifted(&b, delta), &mut self.visits);
    }
    pub fn evict_first(&mut self) {
        let mut n = self.root.as_ref();
        let mut shift = 0isize;
        let mut key = None;
        while let Some(v) = n {
            key = v.key.checked_add_signed(shift);
            shift += v.shift;
            n = v.left.as_ref();
        }
        if let Some(k) = key {
            let (_, b) = split(self.root.take(), k.saturating_add(1), &mut self.visits);
            self.root = b;
        }
    }
}
