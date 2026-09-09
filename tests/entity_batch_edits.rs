use viem_core::document::TextEdit;
use viem_core::layout::DocumentLayoutStyles;
use viem_core::{Document, Encoding, Format};

fn html(source: &str) -> Document {
    Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Html).unwrap()
}

#[test]
fn disjoint_edits_rewrite_one_shared_entity_once() {
    let source = "<p><b>&fjlig;</b><!--keep--> tail</p>";
    let mut document = html(source);
    document
        .apply_edits(vec![TextEdit::new(0..1, "F"), TextEdit::new(1..2, "J")])
        .unwrap();
    assert_eq!(document.text(), "FJ tail");
    assert_eq!(
        String::from_utf8(document.source_bytes()).unwrap(),
        "<p><b>FJ</b><!--keep--> tail</p>"
    );
    assert!(document.undo());
    assert_eq!(document.source_bytes(), source.as_bytes());
}

#[test]
fn batch_entity_rewrites_preserve_gaps_in_their_original_style() {
    let source = "<p><i>A</i><b>&fjlig;</b><u>Z</u><!--keep--></p>";
    let mut document = html(source);
    let original_j = DocumentLayoutStyles::character_at(document.projection(), 2, false).unwrap();
    document
        .apply_edits(vec![
            TextEdit::new(0..1, "X"),
            TextEdit::new(1..2, "Y"),
            TextEdit::new(3..4, "Q"),
        ])
        .unwrap();
    assert_eq!(document.text(), "XYjQ");
    assert_eq!(
        DocumentLayoutStyles::character_at(document.projection(), 2, false).unwrap(),
        original_j
    );
    assert!(document.undo());
    assert_eq!(document.source_bytes(), source.as_bytes());
}

#[test]
fn cross_run_batches_keep_the_unselected_contributor_suffix_style() {
    let source = "<p><i>A</i><b>&fjlig;</b><u>Z</u></p>";
    let mut document = html(source);
    let original_j = DocumentLayoutStyles::character_at(document.projection(), 2, false).unwrap();
    document
        .apply_edits(vec![TextEdit::new(0..2, "X"), TextEdit::new(3..4, "Q")])
        .unwrap();
    assert_eq!(document.text(), "XjQ");
    assert_eq!(
        DocumentLayoutStyles::character_at(document.projection(), 1, false).unwrap(),
        original_j
    );
}

#[test]
fn shared_source_patch_retains_separate_logical_change_ranges() {
    let document = html("<p>&fjlig;</p>");
    let prepared = document
        .prepare_model_request(viem_core::document::ModelRequest::ApplyTextEdits {
            document: document.id(),
            revision: document.revision(),
            edits: vec![TextEdit::new(0..1, "F"), TextEdit::new(1..2, "J")],
        })
        .unwrap();
    assert_eq!(prepared.summary().source_patches().len(), 1);
    assert_eq!(
        prepared
            .summary()
            .formatted_splices()
            .iter()
            .map(|splice| splice.old_range())
            .collect::<Vec<_>>(),
        vec![0..1, 1..2]
    );
}
