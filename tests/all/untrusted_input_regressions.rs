//! Hostile document and clipboard shapes found by fuzzing (see fuzz/). Work
//! must stay proportional to the input, and edits of malformed input must
//! fail as ordinary errors rather than panics.
use viem_core::command::{InputEvent, Key};
use viem_core::document::{measure_document_work, ClipboardFragment, Document, Format};
use viem_core::layout::MockTextMeasurementProvider;
use viem_core::{Core, CoreError, CoreEvent, ViewId};

#[test]
fn nested_container_markers_on_one_row_measure_prefixes_linearly() {
    for marker in ["- ", "* ", "1. ", "> ", ">", "> - ", "- > "] {
        let source = format!("{}a\n", marker.repeat(4_000));
        for format in [Format::Markdown, Format::MarkdownSource] {
            let (document, work) = measure_document_work(|| {
                Document::from_bytes_detect_encoding(source.clone().into_bytes(), format)
            });
            assert_eq!(document.unwrap().source_bytes(), source.as_bytes());
            // Each projection pass scans a row a constant number of times;
            // rescanning it per nested container was quadratic.
            assert!(
                work.markdown_container_prefix_bytes <= 64 * source.len(),
                "{marker:?} {format:?}: {} bytes for a {}-byte row",
                work.markdown_container_prefix_bytes,
                source.len()
            );
        }
    }
}

#[test]
fn formatting_reconstructed_into_many_paragraphs_imports_from_the_clipboard() {
    // Every paragraph reopens the active formatting elements, and each clone
    // looks up its original opening tag.
    let html = format!("<b><i><u><s>{}", "<p>x".repeat(20_000));
    let (fragment, plain) = ClipboardFragment::from_html_utf8(html.as_bytes()).unwrap();
    assert_eq!(plain.matches('x').count(), 20_000);
    ClipboardFragment::from_json(fragment.json(), &plain).unwrap();
}

fn replay(source: &[u8], width: f32, resize: bool, events: Vec<CoreEvent>) {
    let document = Document::from_bytes_detect_encoding(source.to_vec(), Format::Markdown).unwrap();
    let mut core = Core::new(document);
    let view: ViewId = core.add_view(MockTextMeasurementProvider::new(), width, 200.);
    if resize {
        core.handle(view, CoreEvent::Resize { width: width / 3. + 1., height: 90. }).unwrap();
    }
    for event in events {
        let event = match event {
            CoreEvent::SetMarkdownSource { source, .. } => CoreEvent::SetMarkdownSource {
                document: core.document().id(),
                revision: core.document().revision(),
                source,
            },
            event => event,
        };
        let description = format!("{event:?}");
        if let Err(error @ CoreError::InputPanic) = core.handle(view, event) {
            panic!("{description}: {error:?}");
        }
    }
}

fn keys(text: &str) -> Vec<CoreEvent> {
    text.chars().map(|key| CoreEvent::Input(InputEvent::key(key))).collect()
}

fn key(key: Key) -> Vec<CoreEvent> {
    vec![CoreEvent::Input(InputEvent::Key(key))]
}

fn text(text: &str) -> Vec<CoreEvent> {
    vec![CoreEvent::Input(InputEvent::text(text))]
}

#[test]
fn editing_a_bare_autolink_whose_label_differs_from_its_range_fails_cleanly() {
    let events = [keys("A"), keys(":%s/\\w/x/g\n"), keys("n"), key(Key::Enter), keys("dap"), key(Key::Escape)];
    replay(b"https://\x01(\xfa[![img](iing)](hx.y)", 300., false, events.concat());
}

#[test]
fn overlapping_supporting_patches_fail_before_local_simulation() {
    let events = [
        key(Key::Up),
        keys("n"),
        key(Key::Enter),
        keys("dap"),
        text("a"),
        text("a"),
        key(Key::Left),
        text("a"),
        text("a"),
        text("\u{202e}"),
        keys("$"),
        text("> "),
        keys("A"),
        keys("gUU"),
    ];
    replay(b"-a\n>a\n>  \x01", 300., true, events.concat());
}

#[test]
fn spelling_context_bands_tolerate_an_owner_after_the_edited_line() {
    let events = [
        keys("i"),
        text("a"),
        key(Key::Enter),
        text(" "),
        key(Key::Escape),
        vec![CoreEvent::SetMarkdownSource {
            document: viem_core::document::DocumentId(0),
            revision: viem_core::Revision(0),
            source: true,
        }],
    ];
    replay(b"<pre\n\n\n\n>", 300., false, events.concat());
}

#[test]
fn deeply_nested_source_rows_lay_out_and_reach_the_end() {
    // Indentation exhausts the width, so each nested row wraps per grapheme;
    // decoration and inline style work must not multiply by the depth.
    for source in [
        "> - ".repeat(3_000) + "a\n",
        "- ".repeat(3_000) + "a\n",
        format!("x{}y{}\n", "<b>".repeat(3_000), "</b>".repeat(3_000)),
    ] {
        for format in [Format::MarkdownSource, Format::Markdown] {
            let document = Document::from_bytes_detect_encoding(source.clone().into_bytes(), format).unwrap();
            let mut core = Core::new(document);
            let view = core.add_view(MockTextMeasurementProvider::new(), 600., 400.);
            core.handle(view, CoreEvent::Input(InputEvent::Key(Key::DocumentEnd))).unwrap();
            assert!(core.layout(view).and_then(|layout| layout.snapshot()).is_some());
            assert_eq!(core.document().source_bytes(), source.as_bytes());
        }
    }
}
