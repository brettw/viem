//! CSS normal-flow boxes around paragraph leaves. All distances are unscaled
//! layout units; callers apply the view scale exactly once.
use super::{EdgeInsets, ParagraphLayoutStyle};
use crate::document::{Color, ContainerIdentity, ResolvedParagraphStyle};

#[derive(Clone, Debug, PartialEq)]
pub struct BlockBoxStyle {
    pub margin: EdgeInsets,
    pub padding: EdgeInsets,
    pub border: EdgeInsets,
    /// Top, right, bottom, left. Missing declarations resolve to currentColor.
    pub border_colors: [Option<Color>; 4],
    pub background: Option<Color>,
    pub foreground: Color,
    pub foreground_is_default: bool,
    pub language_label: Option<String>,
    pub inline_start: f32,
    pub inline_end: f32,
}
impl Default for BlockBoxStyle {
    fn default() -> Self { Self { margin: EdgeInsets::default(), padding: EdgeInsets::default(), border: EdgeInsets::default(),
        border_colors: [None; 4], background: None, foreground: Color { red: 0., green: 0., blue: 0., alpha: 1. }, foreground_is_default: true, language_label: None, inline_start: 0., inline_end: 0. } }
}

impl BlockBoxStyle {
    pub fn from_resolved(style: &ResolvedParagraphStyle) -> Self {
        Self {
            margin: EdgeInsets { top: style.margin_top, right: style.margin_right, bottom: style.margin_bottom, left: style.margin_left },
            padding: EdgeInsets { top: style.padding_top, right: style.padding_right, bottom: style.padding_bottom, left: style.padding_left },
            border: EdgeInsets { top: style.border_top_width, right: style.border_right_width, bottom: style.border_bottom_width, left: style.border_left_width },
            border_colors: [style.border_top_color, style.border_right_color, style.border_bottom_color, style.border_left_color],
            background: style.background,
            foreground: style.character.foreground,
            foreground_is_default: style.character.foreground_is_default,
            language_label: None, inline_start: 0., inline_end: 0.,
        }
    }
    pub fn top(&self) -> f32 { self.padding.top + self.border.top }
    pub fn bottom(&self) -> f32 { self.padding.bottom + self.border.bottom }
    pub fn left(&self) -> f32 { self.left_in_direction(false) }
    pub fn right(&self) -> f32 { self.right_in_direction(false) }
    pub fn left_in_direction(&self, rtl: bool) -> f32 { self.margin.left + self.border.left + self.padding.left + if rtl { self.inline_end } else { self.inline_start } }
    pub fn right_in_direction(&self, rtl: bool) -> f32 { self.margin.right + self.border.right + self.padding.right + if rtl { self.inline_start } else { self.inline_end } }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ContainerLayoutStyle {
    pub id: ContainerIdentity,
    pub kind: crate::document::ContainerKind,
    pub style: BlockBoxStyle,
    pub starts_here: bool,
    pub ends_here: bool,
}

/// Adjoining margins collapse to the largest positive plus smallest negative.
/// Padding or a border interrupts the adjoining set, even at nested edges.
#[derive(Default)]
struct Boundary {
    advance: f32,
    positive: f32,
    negative: f32,
}
impl Boundary {
    fn margin(&mut self, value: f32) {
        self.positive = self.positive.max(value);
        self.negative = self.negative.min(value);
    }
    fn flush(&mut self) { self.advance += self.positive + self.negative; self.positive = 0.; self.negative = 0.; }
    fn edge(&mut self, value: f32) { if value > 0. { self.flush(); self.advance += value; } }
    fn finish(mut self) -> f32 { self.flush(); self.advance }
    fn close(&mut self, paragraph: &ParagraphLayoutStyle) {
        self.edge(paragraph.block_box.bottom());
        self.margin(paragraph.margin_bottom);
        for container in paragraph.containers.iter().rev().filter(|container| container.ends_here) {
            self.edge(container.style.bottom());
            self.margin(container.style.margin.bottom);
        }
    }
    fn open(&mut self, paragraph: &ParagraphLayoutStyle) {
        for container in paragraph.containers.iter().filter(|container| container.starts_here) {
            self.margin(container.style.margin.top);
            self.edge(container.style.top());
        }
        self.margin(paragraph.margin_top);
        self.edge(paragraph.block_box.top());
    }
}

pub(crate) fn before(paragraph: &ParagraphLayoutStyle) -> f32 {
    let mut boundary = Boundary::default(); boundary.open(paragraph); boundary.finish()
}
pub(crate) fn between(previous: &ParagraphLayoutStyle, next: &ParagraphLayoutStyle) -> f32 {
    let mut boundary = Boundary::default(); boundary.close(previous); boundary.open(next); boundary.finish()
}
pub(crate) fn after(paragraph: &ParagraphLayoutStyle) -> f32 {
    let mut boundary = Boundary::default(); boundary.close(paragraph); boundary.finish()
}

pub(crate) const REVERSE_FLOW_DIAGNOSTIC: &str = "Negative block margins would reverse document flow; Viem limits this boundary to keep the text editable.";

/// The vertical index and hit testing describe ordered editable visual rows. CSS reverse
/// flow needs a spatial overlap index, which this normal-flow subset does not
/// provide. Preserve every advancing boundary exactly; constrain only a band
/// that would otherwise disappear or reverse, and report that fallback.
pub(crate) fn editable_flow_position(previous_start: Option<f32>, requested: f32, scale: f32) -> (f32, bool) {
    match previous_start {
        Some(start) if requested <= start => ((start + scale).max(start.next_up()), true),
        None if requested < 0. => (0., true),
        _ => (requested, false),
    }
}

/// Distances from first/last text to each owner's border edges. External
/// collapsed margins are excluded from the painted box.
pub(crate) fn vertical_extents(paragraph: &ParagraphLayoutStyle) -> (Vec<f32>, Vec<f32>) {
    let count = paragraph.containers.len();
    let mut top = vec![0.; count + 1];
    let mut bottom = vec![0.; count + 1];
    let mut boundary = Boundary::default();
    let mut pending = Vec::new();
    for (index, container) in paragraph.containers.iter().enumerate().filter(|(_, c)| c.starts_here) {
        boundary.margin(container.style.margin.top);
        pending.push(index);
        if container.style.top() > 0. {
            boundary.flush();
            for index in pending.drain(..) { top[index] = boundary.advance; }
            boundary.edge(container.style.top());
        }
    }
    boundary.margin(paragraph.margin_top);
    pending.push(count);
    boundary.flush();
    for index in pending { top[index] = boundary.advance; }
    boundary.edge(paragraph.block_box.top());
    let content = boundary.finish();
    for (index, value) in top.iter_mut().enumerate() {
        if index == count || paragraph.containers[index].starts_here { *value = content - *value; }
    }
    let mut boundary = Boundary::default();
    boundary.edge(paragraph.block_box.bottom());
    bottom[count] = boundary.advance;
    boundary.margin(paragraph.margin_bottom);
    for (index, container) in paragraph.containers.iter().enumerate().rev().filter(|(_, c)| c.ends_here) {
        boundary.edge(container.style.bottom());
        bottom[index] = boundary.advance;
        boundary.margin(container.style.margin.bottom);
    }
    (top, bottom)
}

pub(crate) fn horizontal_insets(paragraph: &ParagraphLayoutStyle, rtl: bool) -> (f32, f32) {
    paragraph.containers.iter().fold((paragraph.block_box.left(), paragraph.block_box.right()), |(left, right), container|
        (left + container.style.left_in_direction(rtl), right + container.style.right_in_direction(rtl)))
}
