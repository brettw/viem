use viem_core::document::{
    BoundaryAffinity, FormattedPayloadEdit, FormattedPayloadEditRequest, FormattedTextPayload,
    ModelRequest, ProjectionWorkScope, TextEdit,
};
use viem_core::layout::{LayoutEngine, MockTextMeasurementProvider, ViewLayout};
use viem_core::{Core, CoreEvent, Document, Encoding, Format};

fn html(source: &str) -> Document {
    Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Html).unwrap()
}

#[test]
fn recovered_atomic_owners_compose_with_every_legal_paragraph_selection() {
    for body in [
        "<p>A</p><table><b>B</b><tr><td>keep</td></tr></table><p>C</p>",
        "<p>A</p><table>B<tr><td>keep</td></tr></table><p>C</p>",
        "<p>A</p><b>D</b><table><i>B</i><tr><td>keep</td></tr></table>E<p>C</p>",
        "<p>A</p><div style='color:red'><table><b>B</b><tr><td>keep</td></tr></table></div><p>C</p>",
        "<p>A</p><table><b>B<tr><td>keep</td></tr></table><p>C</p>",
        "<p>A</p><table><b>B</b><tr><td>keep</td></tr></table><table><i>D</i><tr><td>keep2</td></tr></table><p>C</p>",
    ] {
        for doctype in ["", "<!doctype html>"] {
            let source = format!("{doctype}{body}<!--unchanged-->");
            let original = html(&source);
            let text = original.text().to_owned();
            for start in 0..text.len() {
                for end in start + 1..=text.len() {
                    if original.text_point(start).is_err() || original.text_point(end).is_err() {
                        continue;
                    }
                    for replacement in ["", "X"] {
                        let mut document = html(&source);
                        document.replace(start..end, replacement)
                            .unwrap_or_else(|error| panic!("{source} {start}..{end} -> {replacement:?}: {error:?}"));
                        assert_eq!(document.text(), format!("{}{replacement}{}", &text[..start], &text[end..]));
                        assert!(document.source_bytes().ends_with(b"<!--unchanged-->"));
                        let reopened = Document::from_bytes(document.source_bytes(), Encoding::Utf8, Format::Html).unwrap();
                        assert_eq!(document.text(), reopened.text());
                        assert!(document.undo());
                        assert_eq!(document.source_bytes(), source.as_bytes());
                    }
                }
            }
        }
    }
}

#[test]
fn recovered_owner_payloads_rebind_to_scratch_without_losing_breaks_or_affinity() {
    let source = "<p>A</p><table><b>B</b><tr><td>keep</td></tr></table><p>C</p>";
    for range in [2..6, 1..7] {
        for text in ["X", "X\nY"] {
            for affinity in [BoundaryAffinity::Upstream, BoundaryAffinity::Downstream] {
                let mut document = html(source);
                let original = document.text().to_owned();
                let breaks = text.match_indices('\n').map(|(at, _)| at).collect();
                let payload =
                    FormattedTextPayload::new(&document.hard_line_snapshot(), text, breaks)
                        .unwrap();
                let request = FormattedPayloadEditRequest::new(
                    document.id(),
                    document.revision(),
                    vec![FormattedPayloadEdit::new(range.clone(), payload)
                        .with_boundary_affinity(affinity)],
                );
                let prepared = document.prepare_formatted_payload_request(request).unwrap();
                document.commit_model_transaction(prepared).unwrap();
                assert_eq!(
                    document.text(),
                    format!(
                        "{}{text}{}",
                        &original[..range.start],
                        &original[range.end..]
                    )
                );
                assert_eq!(
                    document.hard_line_snapshot().line_count(),
                    document.text().matches('\n').count() + 1
                );
                assert!(document.undo());
                assert_eq!(document.source_bytes(), source.as_bytes());
            }
        }
    }
}

#[test]
fn styled_flow_owner_preserves_empty_paragraph_and_source_flow() {
    let source = "<h1>A</h1><div class=\"viem-p-506172616772617068\"></div><p>C</p>";
    let document = html(source);
    assert_eq!(document.text(), "A\n\nC");
    assert_eq!(document.projection().blocks()[1].range, 2..2);
    assert_eq!(document.projection().blocks()[1].style.0, "Paragraph");
    let mut core = Core::new(
        Document::from_bytes(
            source.as_bytes().to_vec(),
            Encoding::Utf8,
            Format::HtmlSource,
        )
        .unwrap(),
    );
    let view = core.add_view(MockTextMeasurementProvider::new(), 2000., 300.);
    core.handle(view, CoreEvent::SetParagraphFlow(true))
        .unwrap();
    let rows = &core.layout(view).unwrap().snapshot().unwrap().rows;
    assert_eq!(rows.len(), 3);
    assert_eq!(
        &source[rows[1].text_range.clone()],
        "<div class=\"viem-p-506172616772617068\"></div>"
    );
}

#[test]
fn styled_flow_owner_typing_in_a_large_document_keeps_projection_and_layout_local() {
    let source =
        "<div class=\"viem-p-506172616772617068\">Unchanged paragraph.</div>\n".repeat(10_000);
    let mut document = html(&source);
    let at = document.projection().blocks()[5000].range.start + 3;
    let unaffected = document.projection().blocks()[9000].id;
    let mut engine = LayoutEngine::new(MockTextMeasurementProvider::new());
    engine.set_cache_capacity(10_010);
    let mut view = ViewLayout::new(600., 300.);
    engine.relayout(&document, &mut view).unwrap();
    let shaped = engine.provider().request_calls();
    let prepared = document
        .prepare_model_request(ModelRequest::ApplyTextEdits {
            document: document.id(),
            revision: document.revision(),
            edits: vec![TextEdit::new(at..at, "X")],
        })
        .unwrap();
    let work = prepared.summary().projection_work();
    assert_eq!(work.scope(), ProjectionWorkScope::RegionalHardLines);
    assert!(work.decoded_source_bytes() < 256, "{work:?}");
    assert_eq!(work.full_text_bytes_materialized(), 0);
    document.commit_model_transaction(prepared).unwrap();
    assert_eq!(document.projection().blocks()[9000].id, unaffected);
    engine.relayout(&document, &mut view).unwrap();
    assert!(engine.provider().request_calls() - shaped <= 1);
    assert_eq!(view.snapshot().unwrap().rows.len(), 10_000);
    assert!(document.undo());
    assert_eq!(document.source_bytes(), source.as_bytes());
    engine.relayout(&document, &mut view).unwrap();
    assert!(engine.provider().request_calls() - shaped <= 1);
}
