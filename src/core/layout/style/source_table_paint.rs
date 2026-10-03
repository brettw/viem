//! Paint Source table furniture from the same per-edge declarations as rich
//! borders. The characters, shaping, caret stops, and source remain unchanged.

use super::{push_paint_run, DocumentLayoutStyles, DocumentStyleError, PaintStyleRun};
use crate::document::{Color, Format, FormattedDocument, StyleSheet};
use crate::layout::BlockBoxStyle;
use std::ops::Range;

fn cell_box(document: &FormattedDocument, name: &str) -> Result<BlockBoxStyle, DocumentStyleError> {
    let resolve = |sheet: &StyleSheet| {
        sheet.resolve_assigned_paragraph_style(
            document.document_style(),
            &name.into(),
            &Default::default(),
            &Default::default(),
            None,
            &Default::default(),
        )
    };
    let resolved = resolve(document.style_sheet())
        .or_else(|_| resolve(&StyleSheet::for_format(Format::Markdown)))?;
    Ok(BlockBoxStyle::from_resolved(&resolved))
}

pub(super) fn apply(
    document: &FormattedDocument,
    region: &Range<usize>,
    styles: &mut DocumentLayoutStyles,
) -> Result<(), DocumentStyleError> {
    if region.is_empty() {
        return Ok(());
    }
    let tables = document.tables();
    let first = tables.partition_point(|table| table.range.end <= region.start);
    let relevant = tables[first..]
        .iter()
        .take_while(|table| table.range.start < region.end)
        .filter(|table| table.source_view);
    let mut boxes = None;
    let mut colors = Vec::new();
    for table in relevant {
        let (header, body) = match &boxes {
            Some(boxes) => boxes,
            None => boxes.insert((
                cell_box(document, "Table header")?,
                cell_box(document, "Table cell")?,
            )),
        };
        let mut index = table
            .source_rows
            .partition_point(|row| row.range.end <= region.start);
        while let Some(row) = table
            .source_rows
            .get(index)
            .filter(|row| row.range.start < region.end)
        {
            let cell = if index <= 1 { header } else { body };
            // Shared edges use the thicker side; equal widths favor the
            // preceding cell, matching the collapsed WYSIWYG grid.
            let inner = if cell.border.right >= cell.border.left {
                1
            } else {
                3
            };
            let pipe_start = row.pipes.partition_point(|pipe| *pipe < region.start);
            for &pipe in row.pipes[pipe_start..]
                .iter()
                .take_while(|pipe| **pipe < region.end)
            {
                let side = if row.cells.first().is_some_and(|cell| pipe < cell.start) {
                    3
                } else if row.cells.last().is_some_and(|cell| pipe >= cell.end) {
                    1
                } else {
                    inner
                };
                if let Some(color) = cell.border_colors[side] {
                    colors.push((pipe..pipe + 1, color));
                }
            }
            if row.delimiter {
                let color = if table.source_rows.len() > 2 && body.border.top > header.border.bottom
                {
                    body.border_colors[0]
                } else {
                    header.border_colors[2]
                };
                if let Some(color) = color {
                    let first_marker = row
                        .delimiter_markers
                        .partition_point(|marker| marker.end <= region.start);
                    for marker in row.delimiter_markers[first_marker..]
                        .iter()
                        .take_while(|marker| marker.start < region.end)
                    {
                        colors.push((
                            marker.start.max(region.start)..marker.end.min(region.end),
                            color,
                        ));
                    }
                }
            }
            index += 1;
        }
    }
    if !colors.is_empty() {
        overlay(styles, colors);
    }
    Ok(())
}

fn overlay(styles: &mut DocumentLayoutStyles, mut colors: Vec<(Range<usize>, Color)>) {
    colors.sort_unstable_by_key(|(range, _)| range.start);
    let mut boundaries = Vec::with_capacity(2 * (styles.paint_runs.len() + colors.len()));
    for range in styles
        .paint_runs
        .iter()
        .map(|run| &run.text_range)
        .chain(colors.iter().map(|(range, _)| range))
    {
        boundaries.extend([range.start, range.end]);
    }
    boundaries.sort_unstable();
    boundaries.dedup();
    let mut output: Vec<PaintStyleRun> = Vec::new();
    let mut paint_index = 0;
    let mut color_index = 0;
    for pair in boundaries.windows(2) {
        let range = pair[0]..pair[1];
        while paint_index < styles.paint_runs.len()
            && styles.paint_runs[paint_index].text_range.end <= range.start
        {
            paint_index += 1;
        }
        while color_index < colors.len() && colors[color_index].0.end <= range.start {
            color_index += 1;
        }
        let mut paint = styles
            .paint_runs
            .get(paint_index)
            .filter(|run| run.text_range.start <= range.start)
            .map_or(&styles.default_paint, |run| &run.paint)
            .clone();
        if let Some((_, color)) = colors
            .get(color_index)
            .filter(|(covered, _)| covered.start <= range.start)
        {
            paint.foreground = *color;
            paint.foreground_is_default = false;
        }
        if paint != styles.default_paint {
            push_paint_run(&mut output, range, paint);
        }
    }
    styles.paint_runs = output;
}
