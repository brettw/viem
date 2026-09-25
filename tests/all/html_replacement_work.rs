//! Deterministic end-to-end counters; run the ignored profile with --nocapture.
use std::time::Instant;
use viem_core::command::composition::{
    CompositionEvent, CompositionSession, CompositionTarget, CompositionUpdate,
};
use viem_core::command::{CommandStatus, InputEvent};
use viem_core::document::{
    measure_document_work, BoundaryAffinity, Document, DocumentWorkStatistics, Encoding, Format,
};
use viem_core::layout::MockTextMeasurementProvider;
use viem_core::{Core, CoreEvent, ViewId};

type Editor = Core<MockTextMeasurementProvider>;

fn input(core: &mut Editor, view: ViewId, event: CoreEvent) {
    let outcome = core.handle_with_layout(view, event).unwrap();
    assert!(outcome.command.is_none_or(|command| matches!(
        command.status,
        CommandStatus::Complete | CommandStatus::Pending
    )));
}

fn selection(core: &mut Editor, view: ViewId, start: usize, end: usize) {
    for (at, extend) in [(start, false), (end, true)] {
        core.handle(
            view,
            CoreEvent::PlaceCursor {
                document_revision: core.document().revision(),
                text_offset: at,
                affinity: BoundaryAffinity::Downstream,
                extend_selection: extend,
            },
        )
        .unwrap();
    }
}

fn fixture(
    paragraphs: usize,
    rich: bool,
    encoding: Encoding,
) -> (Editor, ViewId, DocumentWorkStatistics) {
    let body = if rich {
        "<p>ab<b>cd</b><a href='/x'>ef</a>&amp;</p>"
    } else {
        "<p>abcdefg</p>"
    };
    let source = body.repeat(paragraphs);
    let bytes = match encoding {
        Encoding::Utf16Le => source.encode_utf16().flat_map(u16::to_le_bytes).collect(),
        _ => source.into_bytes(),
    };
    let (document, open) =
        measure_document_work(|| Document::from_bytes(bytes, encoding, Format::Html).unwrap());
    let mut core = Core::new(document);
    let view = core.add_view(MockTextMeasurementProvider::new(), 400., 200.);
    (core, view, open)
}

fn replacement(
    core: &mut Editor,
    view: ViewId,
    at: usize,
    ime: bool,
) -> (DocumentWorkStatistics, DocumentWorkStatistics) {
    replacement_range(core, view, at, 1, ime)
}

fn replacement_range(
    core: &mut Editor,
    view: ViewId,
    at: usize,
    length: usize,
    ime: bool,
) -> (DocumentWorkStatistics, DocumentWorkStatistics) {
    let (_, replace) = measure_document_work(|| {
        selection(core, view, at, at + length);
        if ime {
            let target = CompositionTarget::at_offsets(core.document(), at..at + length).unwrap();
            input(
                core,
                view,
                CoreEvent::Composition(CompositionEvent::Begin(target)),
            );
            input(
                core,
                view,
                CoreEvent::Composition(CompositionEvent::Update(CompositionUpdate::new("Z", 1..1))),
            );
            input(core, view, CoreEvent::Composition(CompositionEvent::Commit));
        } else {
            input(core, view, CoreEvent::Input(InputEvent::text("X")));
        }
    });
    let (_, continued) =
        measure_document_work(|| input(core, view, CoreEvent::Input(InputEvent::text("Y"))));
    (replace, continued)
}

fn report(label: &str, elapsed: std::time::Duration, work: DocumentWorkStatistics) {
    println!("{label}: pair_elapsed_ms={} full_source_calls={} full_source_bytes={} range_source_bytes={} decoded={} tokenize={} tree_tokenize={} scope_visits={} index_visits={} index_copies={} index_rebuild={} scratch={} commits={} full_projection={} regional_projection={} projected_bytes={} scope_copies={} scope_syntax_bytes={} index_newly_retained_bytes={} full_parse_fallbacks={} index_nonconvergence={} recovery_context_fallbacks={} memory_visits={} memory_registered={} memory_release_visits={} memory_released={}",
        elapsed.as_millis(), work.source_full_materializations, work.source_full_materialized_bytes,
        work.source_range_materialized_bytes, work.source_decoded_bytes, work.html_tokenized_bytes,
        work.html_tree_tokenized_bytes, work.html_scope_entries_visited, work.html_index_nodes_visited,
        work.html_index_nodes_copied, work.html_index_entries_rebuilt, work.scratch_documents,
        work.transaction_commits, work.full_projection_candidates, work.regional_projection_candidates,
        work.projected_formatted_bytes, work.html_scope_entries_copied,
        work.html_scope_syntax_bytes_copied, work.html_index_newly_retained_bytes,
        work.fallback_counts[0], work.fallback_counts[1], work.fallback_counts[2], work.retained_memory_allocation_visits,
        work.retained_memory_allocations_registered, work.retained_memory_release_visits,
        work.retained_memory_allocations_released);
}

#[test]
fn instrumentation_covers_preparation_and_commit_separately_from_open() {
    let (mut core, view, open) = fixture(8, true, Encoding::Utf8);
    assert!(open.source_decoded_bytes > 0);
    assert!(open.html_tokenized_bytes > 0);
    let (replace, continued) = replacement(&mut core, view, 2, false);
    assert!(replace.transaction_commits > 0);
    assert!(replace.model_requests > 0);
    assert!(replace.source_range_materialized_bytes > 0);
    assert!(continued.transaction_commits > 0);
    assert_eq!(
        core.document()
            .projection()
            .text_tree()
            .slice(0..9)
            .unwrap(),
        "abXYdef&\n"
    );
}

#[test]
#[ignore = "explicit performance profile: HTML_REPLACEMENT_PROFILE_COUNTS=1000,10000,100000 cargo test --test html_replacement_work -- --ignored --nocapture"]
fn html_replacement_work_profile() {
    let counts = std::env::var("HTML_REPLACEMENT_PROFILE_COUNTS")
        .unwrap_or_else(|_| "1000,10000,100000".into());
    for count in counts
        .split(',')
        .map(|count| count.parse::<usize>().unwrap())
    {
        for (rich, encoding) in [
            (false, Encoding::Utf8),
            (true, Encoding::Utf8),
            (true, Encoding::Utf16Le),
        ] {
            let start = Instant::now();
            let (mut core, view, open) = fixture(count, rich, encoding);
            report(
                &format!("open p={count} rich={rich} enc={encoding:?}"),
                start.elapsed(),
                open,
            );
            for (name, paragraph) in [("start", 0), ("middle", count / 2), ("end", count - 1)] {
                for ime in [false, true] {
                    let at = core
                        .document()
                        .projection()
                        .hard_line_range(paragraph)
                        .unwrap()
                        .start
                        + 2;
                    let start = Instant::now();
                    let (replace, continued) = replacement(&mut core, view, at, ime);
                    report(
                        &format!(
                            "replace p={count} rich={rich} enc={encoding:?} at={name} ime={ime}"
                        ),
                        start.elapsed(),
                        replace,
                    );
                    report(
                        &format!(
                            "continued p={count} rich={rich} enc={encoding:?} at={name} ime={ime}"
                        ),
                        start.elapsed(),
                        continued,
                    );
                    assert_local(replace, "profile replacement");
                    assert_local(continued, "profile continued typing");
                }
            }
        }
    }
}

fn assert_local(work: DocumentWorkStatistics, label: &str) {
    assert_eq!(work.source_full_materializations, 0, "{label}: {work:?}");
    assert!(
        work.source_range_materialized_bytes < 32_768,
        "{label}: {work:?}"
    );
    assert!(work.source_decoded_bytes < 32_768, "{label}: {work:?}");
    assert!(work.html_tokenized_bytes < 32_768, "{label}: {work:?}");
    assert!(work.html_tree_tokenized_bytes < 32_768, "{label}: {work:?}");
    assert!(work.html_scope_entries_visited < 512, "{label}: {work:?}");
    assert!(work.html_index_entries_rebuilt < 512, "{label}: {work:?}");
    assert!(work.html_index_nodes_visited < 4_096, "{label}: {work:?}");
    assert!(
        work.retained_memory_allocation_visits < 16_384,
        "{label}: {work:?}"
    );
    assert!(
        work.retained_memory_release_visits < 16_384,
        "{label}: {work:?}"
    );
    assert_eq!(work.full_projection_candidates, 0, "{label}: {work:?}");
}

#[test]
fn unrelated_paragraphs_do_not_grow_native_or_ime_replacement_work() {
    for paragraphs in [1_000, 10_000] {
        for encoding in [Encoding::Utf8, Encoding::Utf16Le] {
            for paragraph in [0, paragraphs / 2, paragraphs - 1] {
                for (offset, length, ime) in [
                    (0, 1, false),
                    (2, 1, false),
                    (4, 1, false),
                    (6, 1, false),
                    (2, 1, true),
                    (2, 2, false),
                    (2, 2, true),
                    (4, 2, false),
                    (4, 2, true),
                ] {
                    let (mut core, view, _) = fixture(paragraphs, true, encoding);
                    let second_view = core.add_view(MockTextMeasurementProvider::new(), 240., 140.);
                    let at = core
                        .document()
                        .projection()
                        .hard_line_range(paragraph)
                        .unwrap()
                        .start
                        + offset;
                    let (replace, continued) = replacement_range(&mut core, view, at, length, ime);
                    let label = format!("p={paragraphs} enc={encoding:?} paragraph={paragraph} offset={offset} length={length} ime={ime}");
                    assert_local(replace, &format!("replacement {label}"));
                    assert_local(continued, &format!("continued typing {label}"));
                    assert!(core.command_state(second_view).is_some());
                }
            }
        }
    }
}

#[test]
fn huge_paragraph_replacement_does_not_scan_its_untouched_prefix() {
    for bytes in [8_192, 2_000_000] {
        let source = format!("<p>{}<b>bold</b> tail</p>", "a".repeat(bytes));
        let document =
            Document::from_bytes(source.into_bytes(), Encoding::Utf8, Format::Html).unwrap();
        let mut core = Core::new(document);
        let view = core.add_view(MockTextMeasurementProvider::new(), 400., 200.);
        let (replace, continued) = replacement(&mut core, view, bytes + 1, false);
        assert_local(replace, "huge paragraph replacement");
        assert_local(continued, "huge paragraph continued typing");
    }
}

#[test]
#[ignore = "explicit profile of actual affected nesting and huge attribute costs"]
fn html_replacement_local_structure_profile() {
    for (name, source, at) in [
        (
            "huge paragraph",
            format!("<p>{}<b>bold</b> tail</p>", "a".repeat(2_000_000)),
            2_000_001,
        ),
        (
            "deep nesting",
            format!(
                "<p>{}bold{}</p>",
                "<span>".repeat(256),
                "</span>".repeat(256)
            ),
            1,
        ),
        (
            "huge selected ancestor attribute",
            format!(
                "<p><a href='/x' title='{}'>bold</a> tail</p>",
                "a".repeat(1_000_000)
            ),
            1,
        ),
    ] {
        let start = Instant::now();
        let (document, open) = measure_document_work(|| {
            Document::from_bytes(source.into_bytes(), Encoding::Utf8, Format::Html).unwrap()
        });
        report(&format!("open {name}"), start.elapsed(), open);
        let mut core = Core::new(document);
        let view = core.add_view(MockTextMeasurementProvider::new(), 400., 200.);
        for ime in [false, true] {
            let start = Instant::now();
            let (replace, continued) = replacement(&mut core, view, at, ime);
            report(
                &format!("replace {name} ime={ime}"),
                start.elapsed(),
                replace,
            );
            report(
                &format!("continued {name} ime={ime}"),
                start.elapsed(),
                continued,
            );
        }
    }
}

#[test]
fn huge_single_word_ime_model_preparation_and_commit_are_local() {
    for bytes in [8_192, 2_000_000] {
        let source = format!("<p>{}<b>bold</b> tail</p>", "a".repeat(bytes));
        let mut document =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Html).unwrap();
        let (_, work) = measure_document_work(|| {
            let mut composition =
                CompositionSession::begin_at_offsets(&document, bytes + 1..bytes + 2).unwrap();
            composition
                .update(&document, CompositionUpdate::new("Z", 1..1))
                .unwrap();
            composition
                .prepare_commit(&document)
                .unwrap()
                .apply(&mut document)
                .unwrap();
        });
        assert_local(work, "huge single-word IME model commit");
        assert_eq!(
            document
                .projection()
                .text_tree()
                .slice(bytes..bytes + 9)
                .unwrap(),
            "bZld tail"
        );
        let reopened =
            Document::from_bytes(document.source_bytes(), Encoding::Utf8, Format::Html).unwrap();
        assert_eq!(document.text(), reopened.text());
        assert!(document.undo());
        assert_eq!(document.source_bytes(), source.as_bytes());
    }
}

#[test]
fn huge_word_and_wrapped_prose_ime_replacement_and_continued_typing_are_local() {
    let bytes = 2_000_000;
    for (name, filler) in [
        ("unbroken word", "a".repeat(bytes)),
        ("wrapped prose", "a ".repeat(bytes / 2)),
    ] {
        let source = format!("<p>{filler}<b>bold</b> tail</p>");
        let document =
            Document::from_bytes(source.into_bytes(), Encoding::Utf8, Format::Html).unwrap();
        let mut core = Core::new(document);
        let view = core.add_view(MockTextMeasurementProvider::new(), 400., 200.);
        let (replace, continued) = replacement(&mut core, view, bytes + 1, true);
        assert_local(replace, &format!("huge {name} IME replacement"));
        assert_local(continued, &format!("huge {name} IME continued typing"));
    }
}

#[test]
fn whole_bold_replacement_between_spaces_stays_local() {
    for paragraphs in [1_000, 10_000] {
        for paragraph in [0, paragraphs / 2, paragraphs - 1] {
            for ime in [false, true] {
                let source = "<p>alpha <b>bold</b> omega</p>\n".repeat(paragraphs);
                let document =
                    Document::from_bytes(source.into_bytes(), Encoding::Utf8, Format::Html)
                        .unwrap();
                let mut core = Core::new(document);
                let view = core.add_view(MockTextMeasurementProvider::new(), 400., 200.);
                let at = core
                    .document()
                    .projection()
                    .hard_line_range(paragraph)
                    .unwrap()
                    .start
                    + 6;
                let started = Instant::now();
                let (replace, continued) = replacement_range(&mut core, view, at, 4, ime);
                let label = format!(
                    "whole bold with spaces p={paragraphs} paragraph={paragraph} ime={ime}"
                );
                report(&format!("replace {label}"), started.elapsed(), replace);
                report(&format!("continued {label}"), started.elapsed(), continued);
                assert_local(replace, &label);
                assert_local(continued, &format!("continued {label}"));
            }
        }
    }
}
