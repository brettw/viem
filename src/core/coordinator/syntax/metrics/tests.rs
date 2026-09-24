use super::*;
use crate::coordinator::{syntax::code_metrics_changed, Core};
use crate::document::{
    code_style,
    syntax::{SyntaxRun, SyntaxStyleName},
    CharacterProperties, CharacterStyle, Color, Document, Encoding, FontSlant, Format,
    StyleDefinitionMetadata, StyleSheet,
};
use crate::layout::MockTextMeasurementProvider;

fn define(
    sheet: &mut StyleSheet,
    name: &str,
    parent: Option<&str>,
    properties: CharacterProperties,
) {
    let id = code_style::resolve_name(sheet, name)
        .cloned()
        .unwrap_or_else(|| StyleId(format!("test:{name}")));
    let based_on = parent
        .and_then(|name| code_style::resolve_name(sheet, name).cloned());
    sheet
        .insert_character_style(
            CharacterStyle {
                id,
                based_on,
                properties,
            },
            StyleDefinitionMetadata::generated(name),
        )
        .unwrap();
}

fn run(range: Range<usize>, name: &str) -> SyntaxRun {
    SyntaxRun {
        range,
        name: SyntaxStyleName(name.into()),
        origin: "test provider".into(),
        priority: 0,
    }
}

fn projected(sheet: &StyleSheet, runs: &[SyntaxRun]) -> FormattedDocument {
    let mut document =
        Document::from_bytes(b"fn main() { 42 }".to_vec(), Encoding::Utf8, Format::Code).unwrap();
    document.install_code_presentation(Arc::new(sheet.clone()), runs);
    document.projection().clone()
}

#[test]
fn effective_metric_coverage_ignores_paint_and_equivalent_capture_boundaries() {
    let mut sheet = code_style::default_sheet();
    define(
        &mut sheet,
        "Large",
        None,
        CharacterProperties {
            size: Some(32.),
            ..Default::default()
        },
    );
    define(
        &mut sheet,
        "InheritedLarge",
        Some("Large"),
        CharacterProperties {
            foreground: Some(Color {
                red: 1.,
                green: 0.,
                blue: 0.,
                alpha: 1.,
            }),
            ..Default::default()
        },
    );
    define(
        &mut sheet,
        "SameAsDefault",
        None,
        CharacterProperties {
            size: Some(14.),
            ..Default::default()
        },
    );
    let original = projected(&sheet, &[run(0..2, "Large")]);
    let paint = projected(
        &sheet,
        &[
            run(0..2, "Large"),
            run(3..7, "Function"),
            run(12..14, "SameAsDefault"),
        ],
    );
    assert!(
        !runs_change_metrics(&original, &paint),
        "unrelated font declarations must not reset heights for paint-only coverage"
    );
    let split = projected(&sheet, &[run(0..1, "Large"), run(1..2, "InheritedLarge")]);
    assert!(
        !runs_change_metrics(&original, &split),
        "equivalent resolved metrics coalesce across capture names"
    );
    assert!(
        runs_change_metrics(&original, &projected(&sheet, &[])),
        "eviction/removal restores default metrics"
    );
    assert!(
        runs_change_metrics(&projected(&sheet, &[]), &original),
        "new coverage changes metrics"
    );
    assert!(
        runs_change_metrics(&original, &projected(&sheet, &[run(3..7, "Large")])),
        "moving metric coverage invalidates heights"
    );
}

#[test]
fn inherited_font_style_edits_and_missing_definitions_have_correct_metric_effects() {
    let mut sheet = code_style::default_sheet();
    define(
        &mut sheet,
        "Parent",
        None,
        CharacterProperties {
            size: Some(32.),
            ..Default::default()
        },
    );
    define(
        &mut sheet,
        "Child",
        Some("Parent"),
        CharacterProperties::default(),
    );
    let old = sheet.clone();
    define(
        &mut sheet,
        "Child",
        Some("Parent"),
        CharacterProperties {
            foreground: Some(Color {
                red: 1.,
                green: 0.,
                blue: 0.,
                alpha: 1.,
            }),
            ..Default::default()
        },
    );
    assert!(!code_metrics_changed(&old, &sheet));
    assert!(!runs_change_metrics(
        &projected(&old, &[run(0..2, "Child")]),
        &projected(&sheet, &[run(0..2, "Child")])
    ));
    let old = sheet.clone();
    define(
        &mut sheet,
        "Parent",
        None,
        CharacterProperties {
            size: Some(40.),
            slant: Some(FontSlant::Italic),
            ..Default::default()
        },
    );
    assert!(code_metrics_changed(&old, &sheet));
    assert!(runs_change_metrics(
        &projected(&old, &[run(0..2, "Child")]),
        &projected(&sheet, &[run(0..2, "Child")])
    ));
    let child = code_style::resolve_name(&sheet, "Child").unwrap().clone();
    let old = sheet.clone();
    sheet.remove_character_style(&child, false).unwrap();
    assert!(code_metrics_changed(&old, &sheet));
    let missing = projected(&sheet, &[run(0..2, "Child")]);
    assert!(missing.style_spans().is_empty());
    assert!(runs_change_metrics(
        &projected(&old, &[run(0..2, "Child")]),
        &missing
    ));
}

#[test]
fn large_code_metric_publication_refreshes_two_views_with_bounded_layout_and_reuses_paint_shaping()
{
    let line = "fn item() { let answer = 42; }\n";
    let source = line.repeat(20_000).into_bytes();
    let document = Document::from_bytes(source.clone(), Encoding::Utf8, Format::Code).unwrap();
    let mut core = Core::<MockTextMeasurementProvider>::new(document);
    core.set_code_language(crate::document::syntax::detection::LanguageSelection::None);
    let views = [
        core.add_view(MockTextMeasurementProvider::new(), 400., 120.),
        core.add_view(MockTextMeasurementProvider::new(), 400., 120.),
    ];
    core.materialize_requested_viewport(views[1], 0., 200_000.)
        .unwrap();
    let before: Vec<_> = views
        .iter()
        .map(|id| {
            let view = &core.views[id];
            (
                view.viewport_anchor.unwrap(),
                view.layout.configuration_generation(),
                view.layout.snapshot().unwrap().rows[0].height(),
                view.engine.provider().request_calls(),
            )
        })
        .collect();
    let mut sheet = code_style::default_sheet();
    define(
        &mut sheet,
        "Large",
        None,
        CharacterProperties {
            size: Some(32.),
            slant: Some(FontSlant::Italic),
            ..Default::default()
        },
    );
    let mut runs = Vec::new();
    for (anchor, _, _, _) in &before {
        let first_line = core
            .document
            .projection()
            .text_tree()
            .hard_line_at_byte(anchor.anchor.offset())
            .unwrap();
        for hard_line in first_line.saturating_sub(20)..first_line + 40 {
            runs.push(run(
                hard_line * line.len()..hard_line * line.len() + 2,
                "Large",
            ));
        }
    }
    runs.sort_by_key(|run| run.range.start);
    let history = core.document.history_status().node_count;
    let revision = core.document.revision();
    let materialized = core
        .document
        .projection()
        .compatibility_text_is_materialized();
    core.publish_code_presentation(Arc::new(sheet.clone()), &runs);
    for (index, id) in views.iter().enumerate() {
        let view = &core.views[id];
        assert_eq!(
            view.viewport_anchor.unwrap().anchor.offset(),
            before[index].0.anchor.offset(),
            "publication retains visible text until replacement geometry"
        );
        assert_ne!(view.layout.configuration_generation(), before[index].1);
        assert!(!view.layout.content_height().is_exact());
        let anchor = before[index].0;
        let hard_line = anchor.anchor.offset() / line.len();
        let top = view
            .layout
            .hard_line_prefix_height(hard_line)
            .unwrap()
            .height() as f32
            + anchor.offset_from_reference;
        core.materialize_requested_viewport(*id, 0., top).unwrap();
        let view = &core.views[id];
        assert_eq!(
            view.viewport_anchor.unwrap().anchor.offset(),
            anchor.anchor.offset()
        );
        assert!(view
            .layout
            .snapshot()
            .unwrap()
            .rows
            .iter()
            .any(|row| row.height() > before[index].2));
        assert!(view.engine.provider().request_calls() > before[index].3);
        assert!(view.engine.provider().request_calls() - before[index].3 < 128);
        assert!(
            view.layout
                .regional_cached_ranges()
                .iter()
                .map(Range::len)
                .sum::<usize>()
                < 128
        );
        assert!(view.layout.height_index_statistics().run_count() < 128);
        assert!(!view
            .layout
            .hard_line_range_height(19_000..19_100)
            .unwrap()
            .is_exact());
    }
    let prior: Vec<_> = views
        .iter()
        .map(|id| {
            (
                core.views[id].engine.provider().request_calls(),
                core.views[id].layout.content_height(),
                core.views[id].layout.viewport_top(),
            )
        })
        .collect();
    // A later syntax result adds only color runs while existing custom fonts
    // stay unchanged. Preserve exact measured heights and reuse all shaping.
    runs.push(run(12..15, "Keyword"));
    runs.sort_by_key(|run| run.range.start);
    core.publish_code_presentation(Arc::new(sheet), &runs);
    for (index, id) in views.iter().enumerate() {
        assert_eq!(core.views[id].layout.content_height(), prior[index].1);
        core.materialize_requested_viewport(*id, 0., prior[index].2)
            .unwrap();
        assert_eq!(
            core.views[id].engine.provider().request_calls(),
            prior[index].0
        );
    }
    assert_eq!(core.document.source_bytes(), source);
    assert_eq!(core.document.revision(), revision);
    assert_eq!(core.document.history_status().node_count, history);
    assert!(!core.document.is_dirty());
    assert_eq!(
        core.document
            .projection()
            .compatibility_text_is_materialized(),
        materialized
    );
}
