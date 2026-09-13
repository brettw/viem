//! Whitespace presentation policy. Markers never introduce text or caret stops.
use super::{LayoutRect, LayoutSnapshot};
use crate::document::{CharacterProperties, Color, Format, FormattedTextTree};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use unicode_width::UnicodeWidthChar;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WhitespaceBasis {
    Spaces,
    ParagraphEn,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct VisibleWhitespaceOptions {
    pub enabled: bool,
    pub listchars: String,
    pub style: CharacterProperties,
}
impl Default for VisibleWhitespaceOptions {
    fn default() -> Self {
        Self {
            enabled: true,
            listchars: "tab:>-,trail:*,extends:>,precedes:<".into(),
            style: CharacterProperties {
                foreground: Some(Color {
                    red: 0.0,
                    green: 0.0,
                    blue: 139.0 / 255.0,
                    alpha: 1.0,
                }),
                ..Default::default()
            },
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct WhitespacePresentationOptions {
    pub code_whitespace: WhitespaceBasis,
    pub other_whitespace: WhitespaceBasis,
    /// Extra leading-width units reserved as a margin on wrapped Code rows.
    pub code_wrapped_line_indent: u32,
    pub visible_whitespace: VisibleWhitespaceOptions,
}
impl Default for WhitespacePresentationOptions {
    fn default() -> Self {
        Self {
            code_whitespace: WhitespaceBasis::ParagraphEn,
            other_whitespace: WhitespaceBasis::Spaces,
            code_wrapped_line_indent: 4,
            visible_whitespace: Default::default(),
        }
    }
}
impl WhitespacePresentationOptions {
    pub fn validate(&self) -> Result<(), ListCharsError> {
        if self.code_wrapped_line_indent > 1024 {
            return Err(ListCharsError(
                "Code wrapped line indent must be between 0 and 1024".into(),
            ));
        }
        ListChars::parse(&self.visible_whitespace.listchars)?;
        let sheet = crate::document::StyleSheet::default();
        sheet
            .resolve_document_style(
                &sheet.base_paragraph,
                &Default::default(),
                &self.visible_whitespace.style,
            )
            .map_err(|error| {
                ListCharsError(format!("invalid whitespace character style: {error:?}"))
            })?;
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ListCharsError(pub String);
impl std::fmt::Display for ListCharsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for ListCharsError {}

/// Vim's twelve listchars categories. A later duplicate replaces an earlier one.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ListChars {
    values: BTreeMap<String, Vec<char>>,
}
impl ListChars {
    pub fn parse(input: &str) -> Result<Self, ListCharsError> {
        let mut result = Self::default();
        if input.is_empty() {
            return Ok(result);
        }
        for item in input.split_terminator(',') {
            let (name, spelling) = item.split_once(':').ok_or_else(|| {
                ListCharsError("listchars entries require name:characters".into())
            })?;
            let mut chars = Vec::new();
            let mut input = spelling.chars().peekable();
            while let Some(c) = input.next() {
                let c = if c == '\\' && matches!(input.peek(), Some('x' | 'u' | 'U')) {
                    let escape = input.next().expect("looked ahead at an encoded character");
                    let digits = match escape {
                        'x' => 2,
                        'u' => 4,
                        'U' => 8,
                        _ => {
                            return Err(ListCharsError(
                                "listchars escapes use \\x, \\u, or \\U".into(),
                            ))
                        }
                    };
                    let hex: String = input.by_ref().take(digits).collect();
                    if hex.len() != digits {
                        return Err(ListCharsError("incomplete listchars Unicode escape".into()));
                    }
                    u32::from_str_radix(&hex, 16)
                        .ok()
                        .and_then(char::from_u32)
                        .ok_or_else(|| ListCharsError("invalid listchars Unicode escape".into()))?
                } else {
                    c
                };
                if c.width() != Some(1) || c.is_control() {
                    return Err(ListCharsError(
                        "listchars markers must be printable single-cell characters".into(),
                    ));
                }
                chars.push(c);
            }
            let valid = match name {
                "tab" | "leadtab" => (2..=3).contains(&chars.len()),
                "multispace" | "leadmultispace" => !chars.is_empty(),
                "eol" | "space" | "lead" | "trail" | "extends" | "precedes" | "conceal"
                | "nbsp" => chars.len() == 1,
                _ => {
                    return Err(ListCharsError(format!(
                        "unknown listchars category: {name}"
                    )))
                }
            };
            if !valid {
                return Err(ListCharsError(format!(
                    "invalid number of characters for listchars {name}"
                )));
            }
            result.values.insert(name.into(), chars);
        }
        if result.get("leadtab").is_some() && result.get("tab").is_none() {
            return Err(ListCharsError("listchars leadtab requires tab".into()));
        }
        Ok(result)
    }
    pub fn get(&self, name: &str) -> Option<&[char]> {
        self.values.get(name).map(Vec::as_slice)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WhitespaceMarkerKind {
    Tab,
    Space,
    NonbreakingSpace,
    EndOfLine,
    Extends,
    Precedes,
}

/// Each glyph is fitted and clipped to this existing whitespace cell. Markers
/// at viewport edges are overlays; their rectangles never extend row geometry.
#[derive(Clone, Debug, PartialEq)]
pub struct WhitespaceMarker {
    pub kind: WhitespaceMarkerKind,
    pub text: String,
    pub rect: LayoutRect,
    pub row_index: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct WhitespaceConfiguration {
    pub options: WhitespacePresentationOptions,
    pub format: Format,
    pub tabstop: u32,
}
impl Default for WhitespaceConfiguration {
    fn default() -> Self {
        Self {
            options: Default::default(),
            format: Format::PlainText,
            tabstop: 2,
        }
    }
}
impl WhitespaceConfiguration {
    pub fn basis(&self) -> WhitespaceBasis {
        if self.format.is_code() {
            self.options.code_whitespace
        } else {
            self.options.other_whitespace
        }
    }
}

/// A small view-owned cache retains only weak snapshot identity and line
/// boundary summaries, never source buffers. Scrolling an unchanged giant
/// whitespace line does not repeatedly scan its prefix or suffix.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct WhitespaceBoundsCache {
    identity: Option<crate::document::FormattedTextSnapshotIdentity>,
    lines: BTreeMap<(usize, usize), (usize, usize)>,
    space_runs: BTreeMap<usize, usize>,
    #[cfg(test)]
    scans: usize,
    #[cfg(test)]
    run_scans: usize,
}
impl WhitespaceBoundsCache {
    fn bounds(
        &mut self,
        tree: &FormattedTextTree,
        _line: usize,
        range: std::ops::Range<usize>,
    ) -> (usize, usize) {
        let identity = tree.snapshot_identity();
        if self.identity.as_ref() != Some(&identity) {
            self.lines.clear();
            self.space_runs.clear();
            self.identity = Some(identity);
        }
        let key = (range.start, range.end);
        if let Some(bounds) = self.lines.get(&key) {
            return *bounds;
        }
        let bounds = whitespace_bounds(tree, range);
        if self.lines.len() >= 256 {
            self.lines.clear();
        }
        self.lines.insert(key, bounds);
        #[cfg(test)]
        {
            self.scans += 1;
        }
        bounds
    }

    fn space_run_start(
        &mut self,
        tree: &FormattedTextTree,
        at: usize,
        line: std::ops::Range<usize>,
    ) -> usize {
        if let Some((&start, &end)) = self.space_runs.range(..=at).next_back() {
            if at < end {
                return start;
            }
        }
        let mut start = at;
        while start > line.start {
            let mut lower = start.saturating_sub(4096).max(line.start);
            while lower < start && !tree.is_char_boundary(lower).unwrap_or(false) {
                lower += 1;
            }
            let Ok(chunk) = tree.slice(lower..start) else {
                break;
            };
            let count = chunk.bytes().rev().take_while(|&b| b == b' ').count();
            start -= count;
            if count < chunk.len() {
                break;
            }
        }
        let mut end = at;
        while end < line.end {
            let chunk = tree.byte_chunk_at(end);
            let count = chunk
                .iter()
                .take(line.end - end)
                .take_while(|&&b| b == b' ')
                .count();
            end += count;
            if count == 0 || end < line.end && count < chunk.len() {
                break;
            }
        }
        if self.space_runs.len() >= 256 {
            self.space_runs.clear();
        }
        self.space_runs.insert(start, end);
        #[cfg(test)]
        {
            self.run_scans += 1;
        }
        start
    }
}

/// Bounded byte lookup used by marker planning, including sparse long rows.
fn byte(tree: &FormattedTextTree, at: usize) -> Option<u8> {
    if at >= tree.byte_len() {
        None
    } else {
        tree.byte_chunk_at(at).first().copied()
    }
}
fn whitespace_bounds(tree: &FormattedTextTree, range: std::ops::Range<usize>) -> (usize, usize) {
    let mut first = range.start;
    while first < range.end {
        let chunk = tree.byte_chunk_at(first);
        let count = chunk
            .iter()
            .take(range.end - first)
            .take_while(|&&b| b == b' ' || b == b'\t')
            .count();
        first += count;
        if count == 0 || first < range.end && count < chunk.len() {
            break;
        }
    }
    if first == range.end {
        return (first, range.start);
    }
    let mut last = range.end;
    while last > first {
        let mut start = last.saturating_sub(4096).max(first);
        while start < last && !tree.is_char_boundary(start).unwrap_or(false) {
            start += 1;
        }
        let Ok(chunk) = tree.slice(start..last) else {
            break;
        };
        let count = chunk
            .bytes()
            .rev()
            .take_while(|&b| b == b' ' || b == b'\t')
            .count();
        last -= count;
        if count < chunk.len() {
            break;
        }
    }
    (first, last)
}

pub(super) fn marker_plan(
    config: &WhitespaceConfiguration,
    cache: &mut WhitespaceBoundsCache,
    tree: &FormattedTextTree,
    snapshot: &LayoutSnapshot,
    viewport: LayoutRect,
    wrap: bool,
) -> Vec<WhitespaceMarker> {
    if config.format.is_wysiwyg() || !config.options.visible_whitespace.enabled
        || viewport.width <= 0.0 || viewport.height <= 0.0 {
        return Vec::new();
    }
    let Ok(chars) = ListChars::parse(&config.options.visible_whitespace.listchars) else {
        return Vec::new();
    };
    let mut markers = Vec::new();
    for (row_index, row) in snapshot.rows.iter().enumerate() {
        if row.y + row.height() <= viewport.y || row.y >= viewport.y + viewport.height {
            continue;
        }
        let (leading_end, trailing_start) =
            cache.bounds(tree, row.hard_line_index, row.hard_line_range.clone());
        let mut previous_space: Option<(usize, usize)> = None;
        for cluster in &row.clusters {
            if cluster.x + cluster.advance <= viewport.x || cluster.x >= viewport.x + viewport.width
            {
                continue;
            }
            let Ok(text) = tree.slice(cluster.text_range.clone()) else {
                continue;
            };
            let mut source = text.char_indices().peekable();
            while let Some((local, c)) = source.next() {
                let at = cluster.text_range.start + local;
                let leading = at < leading_end;
                let trailing = at >= trailing_start;
                let multiple = c == ' '
                    && (at > row.hard_line_range.start && byte(tree, at - 1) == Some(b' ')
                        || byte(tree, at + 1) == Some(b' '));
                let fallback_tab = ['^', 'I'];
                let pattern = match c {
                    '\t' => Some(
                        if leading {
                            chars.get("leadtab").or_else(|| chars.get("tab"))
                        } else {
                            chars.get("tab")
                        }
                        .unwrap_or(&fallback_tab),
                    ),
                    ' ' if trailing => chars.get("trail")
                        .or_else(|| if multiple { chars.get("multispace") } else { None })
                        .or_else(|| chars.get("space")),
                    ' ' if leading => (if multiple {
                        chars.get("leadmultispace")
                    } else {
                        None
                    })
                    .or_else(|| chars.get("lead"))
                    .or_else(|| {
                        if multiple {
                            chars.get("multispace")
                        } else {
                            None
                        }
                    })
                    .or_else(|| chars.get("space")),
                    ' ' => {
                        if byte(tree, at.saturating_sub(1)) == Some(b' ')
                            || byte(tree, at + 1) == Some(b' ')
                        {
                            chars.get("multispace").or_else(|| chars.get("space"))
                        } else {
                            chars.get("space")
                        }
                    }
                    '\u{a0}' | '\u{202f}' => chars.get("nbsp"),
                    _ => None,
                };
                let Some(pattern) = pattern else {
                    continue;
                };
                let scalar_count = text.chars().count().max(1);
                let cell_width = cluster.advance / scalar_count as f32;
                let before = text[..local].chars().count();
                let rect = LayoutRect {
                    x: cluster.x + before as f32 * cell_width,
                    y: row.y,
                    width: cell_width,
                    height: row.height(),
                };
                if c == '\t' {
                    // A tab pattern fills its established width. Drawing fits
                    // each marker inside a subdivision without adding stops.
                    let unit = cluster.whitespace_unit.unwrap_or(snapshot.whitespace_unit);
                    let cells = (rect.width / unit).ceil().max(1.0).min(1024.0) as usize;
                    let fallback =
                        chars.get("tab").is_none() && (!leading || chars.get("leadtab").is_none());
                    if fallback {
                        markers.push(WhitespaceMarker {
                            kind: WhitespaceMarkerKind::Tab,
                            text: "^I".into(),
                            rect,
                            row_index,
                        });
                        continue;
                    }
                    for cell in 0..cells {
                        let glyph = if cell + 1 == cells && pattern.len() == 3 {
                            pattern[2]
                        } else if cell == 0 {
                            pattern[0]
                        } else {
                            pattern[1]
                        };
                        markers.push(WhitespaceMarker {
                            kind: WhitespaceMarkerKind::Tab,
                            text: glyph.to_string(),
                            rect: LayoutRect {
                                x: rect.x + cell as f32 * rect.width / cells as f32,
                                width: rect.width / cells as f32,
                                ..rect
                            },
                            row_index,
                        });
                    }
                } else {
                    let mut run_start = previous_space
                        .filter(|(previous, _)| *previous + 1 == at)
                        .map_or(at, |(_, start)| start);
                    if run_start == at && pattern.len() > 1 {
                        run_start = cache.space_run_start(tree, at, row.hard_line_range.clone());
                    }
                    previous_space = Some((at, run_start));
                    let glyph = pattern[(at - run_start) % pattern.len()];
                    markers.push(WhitespaceMarker {
                        kind: if c == ' ' {
                            WhitespaceMarkerKind::Space
                        } else {
                            WhitespaceMarkerKind::NonbreakingSpace
                        },
                        text: glyph.to_string(),
                        rect,
                        row_index,
                    });
                }
            }
        }
        let cell_width = snapshot.whitespace_unit;
        let mut edge = |name, kind, x: f32| {
            if let Some(pattern) = chars.get(name) {
                markers.push(WhitespaceMarker {
                    kind,
                    text: pattern[0].to_string(),
                    rect: LayoutRect {
                        x,
                        y: row.y,
                        width: cell_width.min(viewport.width),
                        height: row.height(),
                    },
                    row_index,
                });
            }
        };
        if row.text_range.end == row.hard_line_range.end {
            let x = row
                .carets
                .iter()
                .filter(|caret| caret.point.text_offset == row.hard_line_range.end)
                .map(|caret| caret.x)
                .next()
                .unwrap_or(row.paragraph_content_x + row.width);
            if x >= viewport.x && x < viewport.x + viewport.width {
                edge("eol", WhitespaceMarkerKind::EndOfLine, x);
            }
        }
        if !wrap {
            if row.paragraph_content_x < viewport.x {
                edge("precedes", WhitespaceMarkerKind::Precedes, viewport.x);
            }
            if row.paragraph_content_x + row.width > viewport.x + viewport.width {
                edge(
                    "extends",
                    WhitespaceMarkerKind::Extends,
                    viewport.x + viewport.width - cell_width.min(viewport.width),
                );
            }
        }
    }
    markers
}

#[cfg(test)]
#[path = "whitespace_tests.rs"]
mod tests;
