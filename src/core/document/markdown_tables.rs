//! Lossless GFM table structure. Source offsets remain physical byte offsets;
//! formatted ranges are bound to their containing projection revision.
use super::line_endings::NormalizedText;
use pulldown_cmark::{Alignment, Event, Tag, TagEnd};
use std::ops::Range;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum TableAlignment {
    #[default]
    Unspecified,
    Left,
    Center,
    Right,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MarkdownTableCell {
    pub id: u64,
    pub range: Range<usize>,
    pub source_range: Range<usize>,
    pub missing: bool,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MarkdownTableRow {
    pub id: u64,
    pub range: Range<usize>,
    pub source_range: Range<usize>,
    pub cells: Vec<MarkdownTableCell>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MarkdownTableSourceRow {
    pub range: Range<usize>,
    pub source_range: Range<usize>,
    pub source_body: Range<usize>,
    /// Every original cell spelling, including whitespace and ignored extras.
    pub source_cells: Vec<Range<usize>>,
    pub delimiter: bool,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MarkdownTable {
    pub id: u64,
    pub range: Range<usize>,
    pub source_range: Range<usize>,
    pub columns: Vec<TableAlignment>,
    pub rows: super::TableRows<MarkdownTableRow>,
    pub source_rows: super::TableRows<MarkdownTableSourceRow>,
}
fn shifted(range: &Range<usize>, delta: i128) -> Option<Range<usize>> {
    Some(
        usize::try_from(range.start as i128 + delta).ok()?
            ..usize::try_from(range.end as i128 + delta).ok()?,
    )
}
impl super::range_index::RangedItem for MarkdownTable {
    fn range(&self) -> &Range<usize> { &self.range }
    fn owned_heap_bytes(&self) -> usize { self.columns.capacity() * std::mem::size_of::<TableAlignment>() }
    fn visit_shared_memory(&self, visitor: &mut super::history_memory::MemoryVisitor<'_>) {
        self.rows.visit_retained_memory(visitor);
        self.source_rows.visit_retained_memory(visitor);
    }
    fn with_range(&self, range: Range<usize>) -> Self {
        self.with_transform(range, 0, None).expect("valid table shift")
    }
    fn with_transform(&self, range: Range<usize>, source_delta: i128, _: Option<u64>) -> Option<Self> {
        let delta = range.start as i128 - self.range.start as i128;
        Some(Self {
            id: self.id, range, source_range: shifted(&self.source_range, source_delta)?,
            columns: self.columns.clone(),
            rows: self.rows.shifted(delta, source_delta)?,
            source_rows: self.source_rows.shifted(delta, source_delta)?,
        })
    }
}
impl super::range_index::RangedItem for MarkdownTableRow {
    fn range(&self) -> &Range<usize> {
        &self.range
    }
    fn owned_heap_bytes(&self) -> usize {
        self.cells.capacity() * std::mem::size_of::<MarkdownTableCell>()
    }
    fn with_range(&self, range: Range<usize>) -> Self {
        self.with_transform(range, 0, None)
            .expect("valid table row shift")
    }
    fn with_transform(
        &self,
        range: Range<usize>,
        source_delta: i128,
        _revision: Option<u64>,
    ) -> Option<Self> {
        let delta = range.start as i128 - self.range.start as i128;
        let mut row = self.clone();
        row.range = range;
        row.source_range = shifted(&row.source_range, source_delta)?;
        for cell in &mut row.cells {
            cell.range = shifted(&cell.range, delta)?;
            cell.source_range = shifted(&cell.source_range, source_delta)?;
        }
        Some(row)
    }
}
impl super::range_index::RangedItem for MarkdownTableSourceRow {
    fn range(&self) -> &Range<usize> {
        &self.range
    }
    fn owned_heap_bytes(&self) -> usize {
        self.source_cells.capacity() * std::mem::size_of::<Range<usize>>()
    }
    fn with_range(&self, range: Range<usize>) -> Self {
        self.with_transform(range, 0, None)
            .expect("valid table source row shift")
    }
    fn with_transform(
        &self,
        range: Range<usize>,
        source_delta: i128,
        _revision: Option<u64>,
    ) -> Option<Self> {
        let mut row = self.clone();
        row.range = range;
        row.source_range = shifted(&row.source_range, source_delta)?;
        row.source_body = shifted(&row.source_body, source_delta)?;
        for cell in &mut row.source_cells {
            *cell = shifted(cell, source_delta)?;
        }
        Some(row)
    }
}
#[derive(Clone, Debug, PartialEq)]
pub enum TableEditIntent {
    PasteCells {
        table: u64,
        row: usize,
        column: usize,
        cells: Vec<Vec<super::TableClipboardCell>>,
    },
    Insert {
        range: Range<usize>,
        columns: usize,
        body_rows: usize,
    },
    InsertRow {
        table: u64,
        row: usize,
        after: bool,
    },
    DeleteRow {
        table: u64,
        row: usize,
    },
    InsertColumn {
        table: u64,
        column: usize,
        after: bool,
    },
    DeleteColumn {
        table: u64,
        column: usize,
    },
    SetAlignment {
        table: u64,
        column: usize,
        alignment: TableAlignment,
    },
    ClearCells {
        table: u64,
        rows: Range<usize>,
        columns: Range<usize>,
    },
    ReplaceCells {
        table: u64,
        rows: Range<usize>,
        columns: Range<usize>,
        text: String,
        anchor_row: usize,
        anchor_column: usize,
    },
    SetCellsInlineProperty { table: u64, rows: Range<usize>, columns: Range<usize>, property: super::StyleProperty, enabled: bool },
    SetCellsStrikethrough {
        table: u64,
        rows: Range<usize>,
        columns: Range<usize>,
        enabled: bool,
    },
    AssignCellsNamedStyle {
        table: u64,
        rows: Range<usize>,
        columns: Range<usize>,
        style: super::StyleId,
    },
    SetCellsSemanticStyle {
        table: u64,
        rows: Range<usize>,
        columns: Range<usize>,
        style: super::SemanticInlineStyle,
        enabled: bool,
    },
}
#[derive(Clone, Debug)]
pub(super) struct TableSyntax {
    pub range: Range<usize>,
    pub columns: Vec<TableAlignment>,
    pub rows: Vec<RowSyntax>,
}
#[derive(Clone, Debug)]
pub(super) struct RowSyntax {
    pub range: Range<usize>,
    pub body: Range<usize>,
    pub cells: Vec<Range<usize>>,
    pub delimiter: bool,
}

/// Split using GFM's pipe rule, independent of inline backtick scopes.
pub(super) fn split_row(text: &str, range: Range<usize>) -> Vec<Range<usize>> {
    let bytes = text.as_bytes();
    let mut pipes = Vec::new();
    let mut slashes = 0;
    for at in range.clone() {
        match bytes[at] {
            b'\\' => slashes += 1,
            b'|' => {
                // GFM tables remove one backslash immediately before a
                // pipe before inline parsing, including after another slash.
                if slashes == 0 {
                    pipes.push(at);
                }
                slashes = 0;
            }
            _ => slashes = 0,
        }
    }
    let first = range.start + text[range.clone()].len()
        - text[range.clone()].trim_start_matches([' ', '\t']).len();
    let last = range.start + text[range.clone()].trim_end_matches([' ', '\t']).len();
    let leading = pipes.first() == Some(&first);
    let trailing = pipes.last().is_some_and(|at| at + 1 == last);
    let start = if leading { first + 1 } else { range.start };
    let end = if trailing { last - 1 } else { range.end };
    let mut at = start;
    let mut cells = Vec::new();
    for pipe in pipes
        .iter()
        .copied()
        .filter(|pipe| start <= *pipe && *pipe < end)
    {
        cells.push(at..pipe);
        at = pipe + 1;
    }
    cells.push(at..end.max(at));
    cells
}

/// Table recognition shares the projection's block parser event stream.
#[derive(Default)]
pub(super) struct TableParser {
    pub tables: Vec<TableSyntax>,
    current: Option<TableSyntax>,
}
impl TableParser {
    pub fn event(&mut self, text: &str, event: &Event<'_>, range: &Range<usize>) {
        match event {
            Event::Start(Tag::Table(alignments)) => {
                self.current = Some(TableSyntax {
                    range: range.clone(),
                    columns: alignments
                        .into_iter()
                        .map(|a| match a {
                            Alignment::None => TableAlignment::Unspecified,
                            Alignment::Left => TableAlignment::Left,
                            Alignment::Center => TableAlignment::Center,
                            Alignment::Right => TableAlignment::Right,
                        })
                        .collect(),
                    rows: Vec::new(),
                })
            }
            Event::Start(Tag::TableHead | Tag::TableRow) => {
                if let Some(table) = &mut self.current {
                    let end =
                        range.start + text[range.clone()].trim_end_matches(['\r', '\n']).len();
                    let physical_start = text[..range.start].rfind('\n').map_or(0, |at| at + 1);
                    let cells = split_row(text, range.start..end);
                    table.rows.push(RowSyntax {
                        range: physical_start..range.end,
                        body: range.start..end,
                        cells,
                        delimiter: false,
                    });
                }
            }
            Event::End(TagEnd::Table) => {
                if let Some(mut table) = self.current.take() {
                    if let Some(header) = table.rows.first() {
                        let start = header.range.end;
                        let finish = text[start..]
                            .find('\n')
                            .map_or(text.len(), |at| start + at + 1);
                        let end = start + text[start..finish].trim_end_matches(['\r', '\n']).len();
                        let prefix = super::markdown_quotes::prefix(&text[start..end]);
                        let body_start = start + prefix + text[start + prefix..end].len()
                            - text[start + prefix..end]
                                .trim_start_matches([' ', '\t'])
                                .len();
                        let cells = split_row(text, body_start..end);
                        table.rows.insert(
                            1,
                            RowSyntax {
                                range: start..finish,
                                body: body_start..end,
                                cells,
                                delimiter: true,
                            },
                        );
                    }
                    if let Some(first) = table.rows.first() {
                        table.range.start = first.range.start;
                    }
                    self.tables.push(table);
                }
            }
            _ => {}
        }
    }
}
impl TableSyntax {
    pub(super) fn to_source(mut self, input: &NormalizedText) -> Self {
        let at = |offset: usize| input.source_at(offset);
        self.range = at(self.range.start)..at(self.range.end);
        for row in &mut self.rows {
            row.range = at(row.range.start)..at(row.range.end);
            row.body = at(row.body.start)..at(row.body.end);
            for cell in &mut row.cells {
                *cell = at(cell.start)..at(cell.end);
            }
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn gfm_rows_project_cells_and_keep_source() {
        let source = b"| A | B |\n| :- | -: |\n| x | **wide**<br>line |\n| short |\n";
        let doc = super::super::Document::from_bytes(
            source.to_vec(),
            super::super::Encoding::Utf8,
            super::super::Format::Markdown,
        )
        .unwrap();
        assert_eq!(doc.text(), "A\nB\nx\nwide\nline\nshort\n");
        let table = &doc.projection().tables()[0];
        assert_eq!(
            table.columns,
            vec![TableAlignment::Left, TableAlignment::Right]
        );
        assert_eq!(table.rows.len(), 3);
        assert_eq!(table.rows[2].cells.len(), 2);
        assert_eq!(doc.projection().presentation_line_count(), 3);
    }
}
