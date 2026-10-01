//! Native-window geometry, independent of document layout and command grammar.
//! Coordinates are device-independent pixels with a top-left origin.
use std::collections::HashMap;

pub const MIN_WIDTH: f64 = 100.0;
pub const SPLITTER_WIDTH: f64 = 5.0;
pub const MAX_PANES: usize = 256;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    Height,
    Width,
}
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}
impl Rect {
    fn extent(self, axis: Axis) -> f64 {
        match axis {
            Axis::Height => self.height,
            Axis::Width => self.width,
        }
    }
}
#[derive(Clone, Debug)]
struct Node {
    id: u64,
    rect: Rect,
    kind: Kind,
}
#[derive(Clone, Debug)]
enum Kind {
    Pane,
    Group { axis: Axis, children: Vec<Node> },
}
impl Node {
    fn minimum(&self, chrome: &HashMap<u64, f64>) -> (f64, f64) {
        match &self.kind {
            Kind::Pane => (MIN_WIDTH, *chrome.get(&self.id).unwrap_or(&0.0)),
            Kind::Group { axis, children } => {
                let sizes: Vec<_> = children.iter().map(|n| n.minimum(chrome)).collect();
                match axis {
                    Axis::Height => (
                        sizes.iter().map(|s| s.0).fold(0.0, f64::max),
                        sizes.iter().map(|s| s.1).sum(),
                    ),
                    Axis::Width => (
                        sizes.iter().map(|s| s.0).sum::<f64>()
                            + SPLITTER_WIDTH * (children.len() - 1) as f64,
                        sizes.iter().map(|s| s.1).fold(0.0, f64::max),
                    ),
                }
            }
        }
    }
    fn find(&self, id: u64) -> Option<&Node> {
        if self.id == id {
            return Some(self);
        }
        match &self.kind {
            Kind::Pane => None,
            Kind::Group { children, .. } => children.iter().find_map(|n| n.find(id)),
        }
    }
    fn contains(&self, id: u64) -> bool {
        self.find(id).is_some()
    }
    fn place(&mut self, rect: Rect, chrome: &HashMap<u64, f64>) {
        self.rect = rect;
        let Kind::Group { axis, children } = &mut self.kind else {
            return;
        };
        let spacing = if *axis == Axis::Width {
            SPLITTER_WIDTH
        } else {
            0.0
        };
        let minimums: Vec<_> = children
            .iter()
            .map(|n| {
                let (w, h) = n.minimum(chrome);
                if *axis == Axis::Width {
                    w
                } else {
                    h
                }
            })
            .collect();
        let available = (rect.extent(*axis)
            - spacing * (children.len() - 1) as f64
            - minimums.iter().sum::<f64>())
        .max(0.0);
        let gaps: Vec<_> = children
            .iter()
            .zip(&minimums)
            .map(|(n, m)| (n.rect.extent(*axis) - m).max(0.0))
            .collect();
        let sum = gaps.iter().sum::<f64>();
        let count = children.len();
        let mut offset = 0.0;
        for (i, child) in children.iter_mut().enumerate() {
            let extent = minimums[i]
                + if sum > 0.000001 {
                    available * gaps[i] / sum
                } else {
                    available / count as f64
                };
            let r = match axis {
                Axis::Height => Rect {
                    y: rect.y + offset,
                    height: extent,
                    ..rect
                },
                Axis::Width => Rect {
                    x: rect.x + offset,
                    width: extent,
                    ..rect
                },
            };
            child.place(r, chrome);
            offset += extent + spacing;
        }
    }
    fn split(
        &mut self,
        pane: u64,
        new: Node,
        group_id: u64,
        axis: Axis,
        chrome: &HashMap<u64, f64>,
    ) -> bool {
        if self.id == pane && matches!(self.kind, Kind::Pane) {
            let rect = self.rect;
            let mut old = self.clone();
            let mut new = new;
            let spacing = if axis == Axis::Width {
                SPLITTER_WIDTH
            } else {
                0.0
            };
            let (ow, oh) = old.minimum(chrome);
            let (nw, nh) = new.minimum(chrome);
            let (a, b) = if axis == Axis::Width {
                (ow, nw)
            } else {
                (oh, nh)
            };
            let gap = (rect.extent(axis) - spacing - a - b).max(0.0) / 2.0;
            match axis {
                Axis::Height => {
                    old.rect.height = a + gap;
                    new.rect.height = b + gap;
                }
                Axis::Width => {
                    old.rect.width = a + gap;
                    new.rect.width = b + gap;
                }
            }
            *self = Node {
                id: group_id,
                rect,
                kind: Kind::Group {
                    axis,
                    children: vec![old, new],
                },
            };
            return true;
        }
        if let Kind::Group { children, .. } = &mut self.kind {
            for child in children {
                if child.contains(pane) {
                    return child.split(pane, new, group_id, axis, chrome);
                }
            }
        }
        false
    }
    fn normalize(&mut self) {
        let Kind::Group { axis, children } = &mut self.kind else {
            return;
        };
        for child in children.iter_mut() {
            child.normalize();
        }
        let mut next = Vec::new();
        for child in std::mem::take(children) {
            match child.kind {
                Kind::Group {
                    axis: child_axis,
                    children: grandchildren,
                } if child_axis == *axis => next.extend(grandchildren),
                _ => next.push(child),
            }
        }
        *children = next;
        if children.len() == 1 {
            let rect = self.rect;
            *self = children.remove(0);
            self.rect = rect;
        }
    }
    fn remove(&mut self, pane: u64) -> Option<Node> {
        let Kind::Group { children, .. } = &mut self.kind else {
            return None;
        };
        if let Some(i) = children.iter().position(|n| n.id == pane) {
            return Some(children.remove(i));
        }
        children.iter_mut().find_map(|n| n.remove(pane))
    }
    fn parent_mut(&mut self, pane: u64, axis_filter: Option<Axis>) -> Option<(&mut Node, usize)> {
        let Kind::Group { axis, children } = &self.kind else {
            return None;
        };
        let i = children.iter().position(|n| n.contains(pane))?;
        let deeper = matches!(children[i].kind, Kind::Group { .. })
            && children[i].has_parent(pane, axis_filter);
        if deeper {
            let Kind::Group { children, .. } = &mut self.kind else {
                unreachable!()
            };
            return children[i].parent_mut(pane, axis_filter);
        }
        if axis_filter.is_none_or(|a| a == *axis) {
            Some((self, i))
        } else {
            None
        }
    }
    fn has_parent(&self, pane: u64, filter: Option<Axis>) -> bool {
        match &self.kind {
            Kind::Pane => false,
            Kind::Group { axis, children } => {
                children.iter().any(|n| n.contains(pane))
                    && (filter.is_none_or(|a| a == *axis)
                        || children.iter().any(|n| n.has_parent(pane, filter)))
            }
        }
    }
    fn boundary_for_status(&self, pane: u64) -> Option<u64> {
        let Kind::Group { axis, children } = &self.kind else {
            return None;
        };
        let i = children.iter().position(|n| n.contains(pane))?;
        if let Some(id) = children[i].boundary_for_status(pane) {
            return Some(id);
        }
        if *axis == Axis::Height && i + 1 < children.len() {
            let leaf = children[i].find(pane)?;
            if (leaf.rect.y + leaf.rect.height - children[i].rect.y - children[i].rect.height).abs()
                < 0.01
            {
                return Some(children[i + 1].id);
            }
        }
        None
    }
    fn drag(&mut self, boundary: u64, delta: f64, chrome: &HashMap<u64, f64>) -> bool {
        let Kind::Group { axis, children } = &mut self.kind else {
            return false;
        };
        if let Some(right) = children
            .iter()
            .position(|n| n.id == boundary)
            .filter(|i| *i > 0)
        {
            let left = right - 1;
            let receiver = if delta > 0.0 { left } else { right };
            let donors: Vec<_> = if delta > 0.0 {
                (right..children.len()).collect()
            } else {
                (0..=left).rev().collect()
            };
            let mut remaining = delta.abs();
            for i in donors {
                let (w, h) = children[i].minimum(chrome);
                let minimum = if *axis == Axis::Width { w } else { h };
                let change = remaining.min((children[i].rect.extent(*axis) - minimum).max(0.0));
                match axis {
                    Axis::Height => {
                        children[i].rect.height -= change;
                        children[receiver].rect.height += change;
                    }
                    Axis::Width => {
                        children[i].rect.width -= change;
                        children[receiver].rect.width += change;
                    }
                }
                remaining -= change;
                if remaining < 0.000001 {
                    break;
                }
            }
            return true;
        }
        children.iter_mut().any(|n| n.drag(boundary, delta, chrome))
    }
    fn equalize(&mut self, filter: Option<Axis>, chrome: &HashMap<u64, f64>) {
        let Kind::Group { axis, children } = &mut self.kind else {
            return;
        };
        if filter.is_none_or(|a| a == *axis) {
            let total = self.rect.extent(*axis)
                - if *axis == Axis::Width {
                    SPLITTER_WIDTH * (children.len() - 1) as f64
                } else {
                    0.0
                };
            let minimums: Vec<_> = children
                .iter()
                .map(|n| {
                    let (w, h) = n.minimum(chrome);
                    if *axis == Axis::Width {
                        w
                    } else {
                        h
                    }
                })
                .collect();
            let gap = (total - minimums.iter().sum::<f64>()).max(0.0) / children.len() as f64;
            for (n, m) in children.iter_mut().zip(minimums) {
                match axis {
                    Axis::Height => n.rect.height = m + gap,
                    Axis::Width => n.rect.width = m + gap,
                }
            }
        }
        for child in children {
            child.equalize(filter, chrome);
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    InvalidPane,
    NoRoom,
    TooManyPanes,
    Unsupported,
}
#[derive(Clone, Copy, Debug)]
pub struct Frame {
    pub id: u64,
    pub rect: Rect,
    pub splitter: bool,
    pub status_draggable: bool,
}
#[derive(Clone, Debug)]
pub struct PaneLayout {
    root: Node,
    next_id: u64,
    chrome: HashMap<u64, f64>,
    size: Rect,
}
impl Default for PaneLayout {
    fn default() -> Self {
        Self {
            root: Node {
                id: 1,
                rect: Rect::default(),
                kind: Kind::Pane,
            },
            next_id: 2,
            chrome: HashMap::from([(1, 0.0)]),
            size: Rect::default(),
        }
    }
}
impl PaneLayout {
    pub fn pane_rect(&self, pane: u64) -> Result<Rect, Error> {
        self.root
            .find(pane)
            .filter(|n| matches!(n.kind, Kind::Pane))
            .map(|n| n.rect)
            .ok_or(Error::InvalidPane)
    }
    pub fn minimum(&self) -> (f64, f64) {
        self.root.minimum(&self.chrome)
    }
    pub fn update(&mut self, width: f64, height: f64, chrome: &[(u64, f64)]) -> Result<(), Error> {
        if !width.is_finite()
            || !height.is_finite()
            || width < 0.0
            || height < 0.0
            || chrome.len() > MAX_PANES
        {
            return Err(Error::InvalidPane);
        }
        let panes = self.frames().into_iter().filter(|f| !f.splitter).count();
        if chrome.len() != panes
            || chrome.iter().any(|(id, h)| {
                !h.is_finite()
                    || *h < 0.0
                    || *h > 1_000_000.0
                    || !self
                        .root
                        .find(*id)
                        .is_some_and(|n| matches!(n.kind, Kind::Pane))
            })
        {
            return Err(Error::InvalidPane);
        }
        let next: HashMap<_, _> = chrome.iter().copied().collect();
        if next.len() != panes {
            return Err(Error::InvalidPane);
        }
        self.chrome = next;
        self.size = Rect {
            width,
            height,
            ..Rect::default()
        };
        self.place();
        Ok(())
    }
    fn place(&mut self) {
        let (w, h) = self.minimum();
        self.root.place(
            Rect {
                width: self.size.width.max(w),
                height: self.size.height.max(h),
                ..self.size
            },
            &self.chrome,
        );
    }
    pub fn can_split(&self, pane: u64, axis: Axis, status_height: f64) -> Result<(), Error> {
        let n = self
            .root
            .find(pane)
            .filter(|n| matches!(n.kind, Kind::Pane))
            .ok_or(Error::InvalidPane)?;
        if !status_height.is_finite() || status_height < 0.0 || status_height > 1_000_000.0 {
            return Err(Error::InvalidPane);
        }
        if self.chrome.len() >= MAX_PANES || self.next_id > u64::MAX - 2 {
            return Err(Error::TooManyPanes);
        }
        let required = if axis == Axis::Width {
            MIN_WIDTH * 2.0 + SPLITTER_WIDTH
        } else {
            self.chrome.get(&pane).copied().unwrap_or(0.0) + status_height
        };
        if n.rect.extent(axis) + 0.000001 < required {
            Err(Error::NoRoom)
        } else {
            Ok(())
        }
    }
    pub fn split(&mut self, pane: u64, axis: Axis, status_height: f64) -> Result<u64, Error> {
        self.can_split(pane, axis, status_height)?;
        let id = self.next_id;
        self.next_id += 2;
        self.chrome.insert(id, status_height);
        let leaf = Node {
            id,
            rect: Rect::default(),
            kind: Kind::Pane,
        };
        self.root.split(pane, leaf, id + 1, axis, &self.chrome);
        self.root.normalize();
        self.place();
        Ok(id)
    }
    pub fn remove(&mut self, pane: u64) -> Result<(), Error> {
        self.pane_rect(pane)?;
        if self.chrome.len() <= 1 || self.root.remove(pane).is_none() {
            return Err(Error::InvalidPane);
        }
        self.chrome.remove(&pane);
        self.root.normalize();
        self.place();
        Ok(())
    }
    pub fn frames(&self) -> Vec<Frame> {
        fn walk(n: &Node, root: &Node, out: &mut Vec<Frame>) {
            match &n.kind {
                Kind::Pane => out.push(Frame {
                    id: n.id,
                    rect: n.rect,
                    splitter: false,
                    status_draggable: root.boundary_for_status(n.id).is_some(),
                }),
                Kind::Group { axis, children } => {
                    for (i, c) in children.iter().enumerate() {
                        if *axis == Axis::Width && i > 0 {
                            out.push(Frame {
                                id: c.id,
                                rect: Rect {
                                    x: c.rect.x - SPLITTER_WIDTH,
                                    y: n.rect.y,
                                    width: SPLITTER_WIDTH,
                                    height: n.rect.height,
                                },
                                splitter: true,
                                status_draggable: false,
                            });
                        }
                        walk(c, root, out);
                    }
                }
            }
        }
        let mut out = Vec::new();
        walk(&self.root, &self.root, &mut out);
        out
    }
    pub fn drag(&mut self, id: u64, status: bool, delta: f64) -> Result<(), Error> {
        if status {
            self.pane_rect(id)?;
        } else if !self.frames().iter().any(|f| f.splitter && f.id == id) {
            return Err(Error::InvalidPane);
        }
        if !delta.is_finite() {
            return Err(Error::InvalidPane);
        }
        let boundary = if status {
            self.root.boundary_for_status(id)
        } else {
            Some(id)
        };
        if let Some(boundary) = boundary {
            if !self.root.drag(boundary, delta, &self.chrome) {
                return Err(Error::InvalidPane);
            }
            self.place();
        }
        Ok(())
    }
    pub fn equalize(&mut self, axis: Option<Axis>) {
        self.root.equalize(axis, &self.chrome);
        self.place();
    }
    pub fn resize(
        &mut self,
        pane: u64,
        axis: Axis,
        extent: Option<f64>,
        relative: bool,
    ) -> Result<(), Error> {
        let current = self
            .root
            .find(pane)
            .ok_or(Error::InvalidPane)?
            .rect
            .extent(axis);
        let extra = if axis == Axis::Height {
            self.chrome.get(&pane).copied().unwrap_or(0.0)
        } else {
            0.0
        };
        let Some((parent, i)) = self.root.parent_mut(pane, Some(axis)) else {
            return Ok(());
        };
        let Kind::Group { children, .. } = &mut parent.kind else {
            unreachable!()
        };
        let minimums: Vec<_> = children
            .iter()
            .map(|n| {
                let (w, h) = n.minimum(&self.chrome);
                if axis == Axis::Width {
                    w
                } else {
                    h
                }
            })
            .collect();
        let old = children[i].rect.extent(axis);
        let target = extent
            .map(|e| if relative { current + e } else { e + extra })
            .unwrap_or(f64::MAX);
        if target.is_nan() {
            return Err(Error::InvalidPane);
        }
        let available = children
            .iter()
            .enumerate()
            .filter(|(j, _)| *j != i)
            .map(|(j, n)| n.rect.extent(axis) - minimums[j])
            .sum::<f64>();
        let mut remaining = target.clamp(minimums[i], old + available.max(0.0)) - old;
        for distance in 1..children.len() {
            for j in [
                i.checked_add(distance).filter(|j| *j < children.len()),
                i.checked_sub(distance),
            ]
            .into_iter()
            .flatten()
            {
                let change = if remaining > 0.0 {
                    remaining.min((children[j].rect.extent(axis) - minimums[j]).max(0.0))
                } else {
                    remaining
                };
                match axis {
                    Axis::Height => {
                        children[j].rect.height -= change;
                        children[i].rect.height += change;
                    }
                    Axis::Width => {
                        children[j].rect.width -= change;
                        children[i].rect.width += change;
                    }
                }
                remaining -= change;
            }
        }
        self.place();
        Ok(())
    }
    /// Direction: down=0, up=1, left=2, right=3. Orthogonal caret position
    /// chooses the matching neighbour when a row or column has nested splits.
    pub fn focus(
        &self,
        pane: u64,
        direction: u32,
        count: usize,
        point: (f64, f64),
    ) -> Result<u64, Error> {
        let mut id = pane;
        let frames: Vec<_> = self.frames().into_iter().filter(|f| !f.splitter).collect();
        for _ in 0..count.min(frames.len()) {
            let r = frames
                .iter()
                .find(|f| f.id == id)
                .ok_or(Error::InvalidPane)?
                .rect;
            let mut candidates: Vec<_> = frames
                .iter()
                .filter(|f| f.id != id)
                .filter_map(|f| {
                    let t = f.rect;
                    let (distance, miss) = match direction {
                        0 if t.y >= r.y + r.height - 0.01 => (
                            t.y - r.y - r.height,
                            (t.x - point.0).max(0.0).max(point.0 - t.x - t.width),
                        ),
                        1 if t.y + t.height <= r.y + 0.01 => (
                            r.y - t.y - t.height,
                            (t.x - point.0).max(0.0).max(point.0 - t.x - t.width),
                        ),
                        2 if t.x + t.width <= r.x + 0.01 => (
                            r.x - t.x - t.width,
                            (t.y - point.1).max(0.0).max(point.1 - t.y - t.height),
                        ),
                        3 if t.x >= r.x + r.width - 0.01 => (
                            t.x - r.x - r.width,
                            (t.y - point.1).max(0.0).max(point.1 - t.y - t.height),
                        ),
                        _ => return None,
                    };
                    Some((f.id, distance, miss))
                })
                .collect();
            candidates.sort_by(|a, b| a.2.total_cmp(&b.2).then(a.1.total_cmp(&b.1)));
            if let Some(next) = candidates.first() {
                id = next.0;
            } else {
                break;
            }
        }
        Ok(id)
    }
    pub fn rotate(&mut self, pane: u64, steps: u64, forward: bool) -> Result<(), Error> {
        self.pane_rect(pane)?;
        let Some((parent, _)) = self.root.parent_mut(pane, None) else {
            return Ok(());
        };
        let Kind::Group { children, .. } = &parent.kind else {
            unreachable!()
        };
        let steps = (steps % children.len() as u64) as i64;
        self.reorder(pane, Some(if forward { steps } else { -steps }), None)
    }
    pub fn reorder(
        &mut self,
        pane: u64,
        rotate: Option<i64>,
        index: Option<usize>,
    ) -> Result<(), Error> {
        let Some((parent, i)) = self.root.parent_mut(pane, None) else {
            return Ok(());
        };
        let Kind::Group { children, .. } = &mut parent.kind else {
            unreachable!()
        };
        if children.iter().any(|n| !matches!(n.kind, Kind::Pane)) {
            return Err(Error::Unsupported);
        }
        let rects: Vec<_> = children.iter().map(|n| n.rect).collect();
        if let Some(steps) = rotate {
            let shift = steps.rem_euclid(children.len() as i64) as usize;
            children.rotate_right(shift);
        } else {
            let other = index
                .map(|n| n.saturating_sub(1).min(children.len() - 1))
                .unwrap_or(if i + 1 < children.len() { i + 1 } else { i - 1 });
            children.swap(i, other);
        }
        for (n, r) in children.iter_mut().zip(rects) {
            n.rect = r;
        }
        self.place();
        Ok(())
    }
    pub fn move_edge(&mut self, pane: u64, direction: u32) -> Result<(), Error> {
        self.pane_rect(pane)?;
        if self.chrome.len() <= 1 {
            return Ok(());
        }
        if self.next_id == u64::MAX {
            return Err(Error::TooManyPanes);
        }
        let mut next = self.clone();
        let leaf = next.root.remove(pane).ok_or(Error::InvalidPane)?;
        next.root.normalize();
        let axis = if direction < 2 {
            Axis::Height
        } else {
            Axis::Width
        };
        let rect = next.root.rect;
        let children = if direction == 1 || direction == 2 {
            vec![leaf, next.root]
        } else {
            vec![next.root, leaf]
        };
        next.root = Node {
            id: next.next_id,
            rect,
            kind: Kind::Group { axis, children },
        };
        next.next_id += 1;
        next.root.normalize();
        let (w, h) = next.minimum();
        if w > self.size.width + 0.001 || h > self.size.height + 0.001 {
            return Err(Error::NoRoom);
        }
        next.equalize(Some(axis));
        *self = next;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn layout(w: f64, h: f64) -> PaneLayout {
        let mut p = PaneLayout::default();
        p.update(w, h, &[(1, 25.0)]).unwrap();
        p
    }
    fn panes(p: &PaneLayout) -> Vec<Frame> {
        p.frames().into_iter().filter(|f| !f.splitter).collect()
    }
    #[test]
    fn width_admission_and_failed_split_are_atomic() {
        let mut p = layout(204.0, 500.0);
        let before = p.frames();
        assert_eq!(p.split(1, Axis::Width, 25.0), Err(Error::NoRoom));
        assert_eq!(p.frames()[0].rect, before[0].rect);
        p.update(205.0, 500.0, &[(1, 25.0)]).unwrap();
        p.split(1, Axis::Width, 25.0).unwrap();
        assert!(panes(&p).iter().all(|f| f.rect.width == 100.0));
        assert_eq!(
            p.frames().iter().find(|f| f.splitter).unwrap().rect.width,
            5.0
        );
    }
    #[test]
    fn pushing_collects_widths_and_reversal_releases_at_edge() {
        let mut p = layout(700.0, 600.0);
        let b = p.split(1, Axis::Width, 25.0).unwrap();
        let c = p.split(b, Axis::Width, 25.0).unwrap();
        p.equalize(None);
        let boundary = p.frames().iter().find(|f| f.splitter).unwrap().id;
        p.drag(boundary, false, 10000.0).unwrap();
        let f = panes(&p);
        assert_eq!(f[1].rect.width, 100.0);
        assert_eq!(f[2].rect.width, 100.0);
        let last = f[2].rect;
        p.drag(boundary, false, -12.0).unwrap();
        let f = panes(&p);
        assert_eq!(f[1].rect.width, 112.0);
        assert_eq!(f[2].rect, last);
        assert_eq!(f[2].id, c);
    }
    #[test]
    fn mixed_tree_status_handles_and_directional_focus() {
        let mut p = layout(605.0, 500.0);
        let right = p.split(1, Axis::Width, 25.0).unwrap();
        let bottom = p.split(right, Axis::Height, 25.0).unwrap();
        let f = panes(&p);
        assert!(!f[0].status_draggable);
        assert!(f[1].status_draggable);
        assert!(!f[2].status_draggable);
        assert_eq!(p.focus(1, 3, 1, (100.0, 400.0)), Ok(bottom));
        assert_eq!(p.focus(bottom, 1, 1, (450.0, 400.0)), Ok(right));
        p.drag(right, true, 10000.0).unwrap();
        assert_eq!(panes(&p)[2].rect.height, 25.0);
        p.drag(right, true, -10.0).unwrap();
        assert_eq!(panes(&p)[2].rect.height, 35.0);
        assert_eq!(p.minimum(), (205.0, 50.0));
    }
    #[test]
    fn root_moves_collapse_and_axis_resize_preserve_minimums() {
        let mut p = layout(605.0, 600.0);
        let b = p.split(1, Axis::Height, 25.0).unwrap();
        let c = p.split(b, Axis::Width, 25.0).unwrap();
        p.move_edge(c, 2).unwrap();
        assert_eq!(panes(&p)[0].id, c);
        assert_eq!(panes(&p)[0].rect.height, 600.0);
        p.resize(c, Axis::Width, Some(10000.0), true).unwrap();
        assert_eq!(p.minimum().0, 205.0);
        assert!(panes(&p).iter().all(|f| f.rect.width >= 100.0));
        p.remove(c).unwrap();
        assert!(p.frames().iter().all(|f| !f.splitter));
        assert_eq!(p.focus(1, 0, usize::MAX, (100.0, 100.0)), Ok(b));
    }
    #[test]
    fn nested_rotations_use_sibling_counts_and_invalid_group_ids_are_atomic() {
        let mut p = layout(600.0, 600.0);
        let right = p.split(1, Axis::Width, 25.0).unwrap();
        let bottom = p.split(right, Axis::Height, 25.0).unwrap();
        let before = panes(&p).iter().map(|f| f.id).collect::<Vec<_>>();
        p.rotate(right, 6, true).unwrap();
        assert_eq!(panes(&p).iter().map(|f| f.id).collect::<Vec<_>>(), before);
        p.rotate(right, u64::MAX, true).unwrap();
        assert_eq!(panes(&p)[1].id, bottom);
        let before = p
            .frames()
            .iter()
            .map(|f| (f.id, f.rect))
            .collect::<Vec<_>>();
        assert_eq!(p.remove(bottom + 1), Err(Error::InvalidPane));
        assert_eq!(p.move_edge(999, 2), Err(Error::InvalidPane));
        assert_eq!(
            p.frames()
                .iter()
                .map(|f| (f.id, f.rect))
                .collect::<Vec<_>>(),
            before
        );
    }
    #[test]
    fn pane_budget_is_finite_and_layout_never_touches_document_caches() {
        let mut p = layout(100.0, 256.0 * 25.0);
        for _ in 1..MAX_PANES {
            p.resize(1, Axis::Height, None, false).unwrap();
            p.split(1, Axis::Height, 25.0).unwrap();
        }
        assert_eq!(panes(&p).len(), MAX_PANES);
        assert_eq!(p.split(1, Axis::Height, 25.0), Err(Error::TooManyPanes));
        let chrome: Vec<_> = panes(&p).iter().map(|f| (f.id, 25.0)).collect();
        p.update(50.0, 1.0, &chrome).unwrap();
        assert!(panes(&p)
            .iter()
            .all(|f| f.rect.width >= 100.0 && f.rect.height >= 25.0));
    }
}
