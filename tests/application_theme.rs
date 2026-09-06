use evim_core::document::{Encoding, Format};
use evim_core::layout::{EdgeInsets, MockTextMeasurementProvider};
use evim_core::{Core, Document};

#[test]
fn unspecified_colors_remain_theme_defaults_without_rewriting_any_format() {
    for (format, source) in [
        (Format::PlainText, "words"),
        (Format::Markdown, "**words**"),
        (Format::Html, "<p>words</p>"),
        (Format::Rtf, r"{\rtf1 words}"),
    ] {
        let document =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap();
        let mut core = Core::new(document);
        let view = core.add_view(MockTextMeasurementProvider::new(), 200.0, 100.0);
        let snapshot = core.layout(view).unwrap().snapshot().unwrap();
        assert!(snapshot.default_paint.foreground_is_default, "{format:?}");
        assert!(snapshot.canvas_background_is_default, "{format:?}");
        assert!(snapshot
            .paint_runs
            .iter()
            .all(|run| run.paint.foreground_is_default));
        assert_eq!(core.document().source_bytes(), source.as_bytes());
    }
}

#[test]
fn explicit_black_is_distinct_from_theme_default() {
    for (format, source) in [
        (Format::Html, "<p style='color:#000000'>words</p>"),
        (
            Format::Rtf,
            r"{\rtf1{\colortbl;\red0\green0\blue0;}\cf1 words}",
        ),
    ] {
        let document =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap();
        let mut core = Core::new(document);
        let view = core.add_view(MockTextMeasurementProvider::new(), 200.0, 100.0);
        let snapshot = core.layout(view).unwrap().snapshot().unwrap();
        assert!(
            snapshot
                .paint_runs
                .iter()
                .any(|run| !run.paint.foreground_is_default),
            "{format:?}"
        );
        assert_eq!(core.document().source_bytes(), source.as_bytes());
    }
}

#[test]
fn application_padding_changes_geometry_and_invalidates_only_the_view() {
    let mut core = Core::new(Document::new(
        "one two three four five six seven eight nine ten",
    ));
    let first = core.add_view(MockTextMeasurementProvider::new(), 160.0, 100.0);
    let second = core.add_view(MockTextMeasurementProvider::new(), 160.0, 100.0);
    let original = core.document().source_bytes();
    let source_revision = core.document().revision();
    let before = core.layout(first).unwrap().snapshot().unwrap().clone();
    let other = core.layout(second).unwrap().snapshot().unwrap().clone();
    let padding = EdgeInsets {
        top: 23.0,
        left: 31.0,
        bottom: 17.0,
        right: 19.0,
    };
    core.set_view_insets(first, padding).unwrap();
    let after = core.layout(first).unwrap().snapshot().unwrap();
    assert_ne!(
        before.configuration_generation,
        after.configuration_generation
    );
    assert_eq!(after.content_insets, padding);
    assert_eq!(after.usable_width, 110.0);
    assert_eq!(after.rows[0].clusters[0].x, 31.0);
    assert!(after.rows[0].y >= 23.0);
    assert_ne!(after.rows[0].text_range, before.rows[0].text_range);
    assert_eq!(core.layout(second).unwrap().snapshot().unwrap(), &other);
    assert_eq!(core.document().revision(), source_revision);
    assert_eq!(core.document().source_bytes(), original);
    assert!(core
        .set_view_insets(
            first,
            EdgeInsets {
                left: f32::NAN,
                ..padding
            }
        )
        .is_err());
    let generation = core.layout(first).unwrap().configuration_generation();
    core.set_view_insets(first, padding).unwrap();
    assert_eq!(
        core.layout(first).unwrap().configuration_generation(),
        generation
    );
}

#[test]
fn changing_padding_in_a_large_document_keeps_layout_regional() {
    let source = "a paragraph with enough words to wrap more than once\n".repeat(20_000);
    let mut core = Core::new(Document::new(source.clone()));
    let view = core.add_view(MockTextMeasurementProvider::new(), 180.0, 100.0);
    for left in [30.0, 60.0, 15.0] {
        core.set_view_insets(
            view,
            EdgeInsets {
                top: 28.0,
                left,
                bottom: 28.0,
                right: 30.0,
            },
        )
        .unwrap();
        assert!(
            core.layout(view)
                .unwrap()
                .regional_cache_statistics()
                .hard_line_count()
                < 500
        );
        assert!(core.layout(view).unwrap().snapshot().unwrap().rows.len() < 500);
    }
    assert_eq!(core.document().source_bytes(), source.as_bytes());
}
