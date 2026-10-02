use std::collections::BTreeMap;
use viem_core::document::{Encoding, Format};
use viem_core::layout::{
    compute_layout_job, LayoutEngine, LayoutExecutionContext, MockTextMeasurementProvider,
    ViewLayout,
};
use viem_core::{Core, CoreEvent, Document};

#[test]
fn progressive_cells_match_complete_shaping_across_fragment_boundaries() {
    let cases = [
        (
            "ltr",
            format!(
                "{}fi AV e\u{301} 👩‍🚀 {}",
                "a".repeat(4095),
                "fi AV e\u{301} 👩‍🚀 ".repeat(600)
            ),
        ),
        ("rtl", "אבג שלום مرحبا ".repeat(900)),
        (
            "mixed",
            format!(
                "abc {} ENGLISH 123 {} done",
                "אבג ".repeat(1800),
                "مرحبا ".repeat(1200)
            ),
        ),
    ];
    for (name, text) in cases {
        for format in [Format::Markdown, Format::MarkdownSource] {
            let source = format!("| H | N |\n| - | - |\n| {text} | next |");
            let document =
                Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap();
            let table = &document.projection().tables()[0];
            let range = if format == Format::Markdown {
                table.rows[1].cells[0].range.clone()
            } else {
                table.source_rows[2].cells[0].clone()
            };
            let spelling = document.text()[range.clone()].to_owned();
            // The ordinary unwrapped text path produces a complete reference;
            // it does not use table measurement or table fragment placement.
            let literal = Document::from_bytes(
                spelling.as_bytes().to_vec(),
                Encoding::Utf8,
                Format::PlainText,
            )
            .unwrap();
            let mut reference_view = ViewLayout::new(600., 300.);
            reference_view.set_wrap(false);
            LayoutEngine::new(MockTextMeasurementProvider::new())
                .relayout(&literal, &mut reference_view)
                .unwrap();
            let reference = reference_view.snapshot().unwrap();
            let reference_row = &reference.rows[0];
            // Ordinary RTL prose uses natural paragraph placement; table
            // placement instead follows its explicitly left-aligned column.
            let reference_left = reference_row
                .clusters
                .iter()
                .map(|cluster| cluster.x)
                .reduce(f32::min)
                .unwrap_or(reference_row.paragraph_content_x);
            let expected = reference_row
                .clusters
                .iter()
                .map(|cluster| (cluster.text_range.start, cluster))
                .collect::<BTreeMap<_, _>>();
            let mut core = Core::new(document);
            let view = core.add_view(MockTextMeasurementProvider::new(), 600., 300.);
            let mut iterations = 0;
            while let Some(request) = core.prepare_view_table_refinement(view).unwrap() {
                iterations += 1;
                assert!(
                    iterations < 30,
                    "{name} {format:?}: measurement did not converge"
                );
                let candidate = compute_layout_job(
                    &mut LayoutEngine::new(MockTextMeasurementProvider::new()),
                    &request,
                    LayoutExecutionContext::WorkerPool,
                )
                .unwrap();
                assert!(core.install_view_table_refinement(view, candidate).unwrap());
            }
            let snapshot = core.layout(view).unwrap().snapshot().unwrap();
            let body_row = snapshot
                .rows
                .iter()
                .find(|row| {
                    row.clusters
                        .iter()
                        .any(|cluster| range.contains(&cluster.text_range.start))
                })
                .unwrap();
            let origin = if format == Format::Markdown {
                body_row.paragraph_content_x
            } else {
                let first_pipe = body_row
                    .clusters
                    .iter()
                    .find(|cluster| cluster.text_range.end == range.start)
                    .unwrap();
                first_pipe.x + first_pipe.advance
            };
            for left in [
                0.,
                reference_row.width / 2.,
                (reference_row.width - 500.).max(0.),
            ] {
                core.handle(view, CoreEvent::SetViewportOrigin { left, top: None })
                    .unwrap();
                let snapshot = core.layout(view).unwrap().snapshot().unwrap();
                let mut compared = 0;
                for cluster in snapshot
                    .rows
                    .iter()
                    .flat_map(|row| &row.clusters)
                    .filter(|cluster| range.contains(&cluster.text_range.start))
                {
                    let offset = cluster.text_range.start - range.start;
                    let expected = expected.get(&offset).unwrap_or_else(|| {
                        panic!(
                            "{name} {format:?}: fragment created a different cluster at {offset}"
                        )
                    });
                    assert_eq!(
                        cluster.text_range.end - range.start,
                        expected.text_range.end,
                        "{name} {format:?}"
                    );
                    assert_eq!(
                        cluster.bidi_level, expected.bidi_level,
                        "{name} {format:?} at {offset}"
                    );
                    assert!(
                        (cluster.advance - expected.advance).abs() < 0.02,
                        "{name} {format:?} at {offset}"
                    );
                    assert!(
                        (cluster.x - origin - (expected.x - reference_left)).abs() < 0.03,
                        "{name} {format:?} at {offset}: actual x={} expected x={}",
                        cluster.x - origin,
                        expected.x - reference_left
                    );
                    compared += 1;
                }
                assert!(
                    compared > 0,
                    "{name} {format:?}: viewport {left} omitted the cell"
                );
            }
        }
    }
}

#[test]
fn source_pipes_share_column_boundaries_across_header_and_body_font_sizes() {
    for long in [false, true] {
        let text = if long {
            "body ".repeat(1000)
        } else {
            "body".into()
        };
        let source = format!("| Head | Second |\n| --- | --- |\n| {text} | End |");
        let mut document = Document::from_bytes(
            source.as_bytes().to_vec(),
            Encoding::Utf8,
            Format::MarkdownSource,
        )
        .unwrap();
        document.replace_style_defaults(br#"{"version":1,"block_styles":[{"id":"Table header","name":"Table header","role":"Paragraph","character":{"size":28},"block":{}}]}"#).unwrap();
        let mut view = ViewLayout::new(100_000., 300.);
        LayoutEngine::new(MockTextMeasurementProvider::new())
            .relayout(&document, &mut view)
            .unwrap();
        let snapshot = view.snapshot().unwrap();
        let pipes = snapshot
            .rows
            .iter()
            .map(|row| {
                row.clusters
                    .iter()
                    .filter(|cluster| document.text().get(cluster.text_range.clone()) == Some("|"))
                    .map(|cluster| cluster.x)
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        for row in &pipes[1..] {
            assert_eq!(row.len(), pipes[0].len());
            for (actual, expected) in row.iter().zip(&pipes[0]) {
                assert!((actual - expected).abs() < 0.01, "{actual} != {expected}");
            }
        }
        assert_eq!(document.source_bytes(), source.as_bytes());
    }
}
