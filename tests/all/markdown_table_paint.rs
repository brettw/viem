use serde_json::{json, Value};
use viem_core::document::{Color, Encoding, Format};
use viem_core::layout::{DocumentLayoutStyles, ResolvedTextPaint};
use viem_core::Document;

const SOURCE: &str = "| Head | Other |\n| :--- | ---: |\n| a\\|b | `c\\|d` |";
const RED: Color = Color {
    red: 1.,
    green: 0.,
    blue: 0.,
    alpha: 1.,
};
const BLUE: Color = Color {
    red: 0.,
    green: 0.,
    blue: 1.,
    alpha: 1.,
};
const GREEN: Color = Color {
    red: 0.,
    green: 1.,
    blue: 0.,
    alpha: 1.,
};

fn document(source: &str, body: Value, header: Value) -> Document {
    let mut document = Document::from_bytes(
        source.as_bytes().to_vec(),
        Encoding::Utf8,
        Format::MarkdownSource,
    )
    .unwrap();
    let diagnostics = document.replace_style_defaults(&serde_json::to_vec(&json!({"version":1,"block_styles":[
        {"id":"Table cell","name":"Table cell","role":"Paragraph","based_on":"Paragraph","character":{"foreground":GREEN},"block":body},
        {"id":"Table header","name":"Table header","role":"Paragraph","based_on":"Table cell","block":header}
    ]})).unwrap()).unwrap();
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    document
}

fn paint(styles: &DocumentLayoutStyles, at: usize) -> &ResolvedTextPaint {
    styles
        .paint_runs
        .iter()
        .find(|run| run.text_range.contains(&at))
        .map_or(&styles.default_paint, |run| &run.paint)
}

#[test]
fn source_furniture_uses_matching_collapsed_edges_and_keeps_content_paint() {
    let baseline = document(SOURCE, json!({}), json!({}));
    let baseline = DocumentLayoutStyles::resolve(baseline.projection()).unwrap();
    let document = document(
        SOURCE,
        json!({"border_left_width":1,"border_left_color":RED,"border_right_width":2,"border_right_color":BLUE,"border_top_width":4,"border_top_color":BLUE}),
        json!({"border_bottom_width":5,"border_bottom_color":RED}),
    );
    let styles = DocumentLayoutStyles::resolve(document.projection()).unwrap();
    let table = &document.projection().tables()[0];
    for row in &table.source_rows {
        for (index, &pipe) in row.pipes.iter().enumerate() {
            let actual = paint(&styles, pipe);
            assert_eq!(actual.foreground, if index == 0 { RED } else { BLUE });
            assert!(!actual.foreground_is_default);
        }
        for cell in &row.cells {
            for (offset, character) in document.text()[cell.clone()].char_indices() {
                let actual = paint(&styles, cell.start + offset);
                if row.delimiter && matches!(character, '-' | ':') {
                    assert_eq!(actual.foreground, RED);
                    assert!(!actual.foreground_is_default);
                } else if character == '|' || character.is_alphabetic() || character == ' ' {
                    assert_eq!(
                        actual,
                        paint(&baseline, cell.start + offset),
                        "content {character:?} at {}",
                        cell.start + offset
                    );
                }
            }
        }
    }
    assert_eq!(document.source_bytes(), SOURCE.as_bytes());
}

#[test]
fn delimiter_follows_thicker_body_top_but_header_wins_ties_and_header_only_tables() {
    for (source, header_width, expected) in [
        (SOURCE, 3, BLUE),
        (SOURCE, 4, RED),
        ("Head | Other\n:--- | ---:", 1, RED),
    ] {
        let document = document(
            source,
            json!({"border_top_width":4,"border_top_color":BLUE}),
            json!({"border_bottom_width":header_width,"border_bottom_color":RED}),
        );
        let styles = DocumentLayoutStyles::resolve(document.projection()).unwrap();
        let row = &document.projection().tables()[0].source_rows[1];
        for (offset, character) in document.text()[row.range.clone()].char_indices() {
            if matches!(character, '-' | ':') {
                assert_eq!(
                    paint(&styles, row.range.start + offset).foreground,
                    expected
                );
            }
        }
    }
}

#[test]
fn unspecified_edges_keep_text_color_and_explicit_transparent_black_is_retained() {
    let transparent = Color {
        red: 0.,
        green: 0.,
        blue: 0.,
        alpha: 0.,
    };
    let document = document(
        SOURCE,
        json!({"border_left_width":2,"border_right_width":1,"border_right_color":transparent}),
        json!({}),
    );
    let styles = DocumentLayoutStyles::resolve(document.projection()).unwrap();
    for row in &document.projection().tables()[0].source_rows {
        for (index, &pipe) in row.pipes.iter().enumerate() {
            let actual = paint(&styles, pipe);
            assert_eq!(
                actual.foreground,
                if index + 1 == row.pipes.len() {
                    transparent
                } else {
                    GREEN
                }
            );
            if index + 1 == row.pipes.len() {
                assert!(!actual.foreground_is_default);
            }
        }
        if row.delimiter {
            for (offset, character) in document.text()[row.range.clone()].char_indices() {
                if matches!(character, '-' | ':') {
                    assert_eq!(paint(&styles, row.range.start + offset).foreground, GREEN);
                }
            }
        }
    }
}

#[test]
fn omitted_outer_pipes_and_excess_body_cells_only_color_real_separators() {
    let source = "Head | Other\n--- | ---\na\\|b | c | extra";
    let document = document(
        source,
        json!({"border_left_width":2,"border_left_color":RED,"border_right_width":2,"border_right_color":BLUE}),
        json!({}),
    );
    let styles = DocumentLayoutStyles::resolve(document.projection()).unwrap();
    for row in &document.projection().tables()[0].source_rows {
        for &pipe in &row.pipes {
            assert_eq!(paint(&styles, pipe).foreground, BLUE);
        }
    }
    let escape = document.text().find("\\|").unwrap() + 1;
    assert_eq!(paint(&styles, escape).foreground, GREEN);
    assert_eq!(document.source_bytes(), source.as_bytes());
}
