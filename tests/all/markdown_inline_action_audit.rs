//! User-action regressions for source-owned inline replacement and insertion.
use viem_core::command::{InputEvent, Key};
use viem_core::document::{BoundaryAffinity, LinkEditIntent};
use viem_core::layout::MockTextMeasurementProvider;
use viem_core::{Core, CoreEvent, Document, Encoding, Format, ViewId};

fn open(source: &str) -> (Core<MockTextMeasurementProvider>, ViewId) {
    let mut core = Core::new(
        Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown).unwrap(),
    );
    let view = core.add_view(MockTextMeasurementProvider::new(), 600., 400.);
    (core, view)
}

fn place(core: &mut Core<MockTextMeasurementProvider>, view: ViewId, at: usize, extend: bool) {
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

fn input(core: &mut Core<MockTextMeasurementProvider>, view: ViewId, event: InputEvent) {
    let (result, trace) = viem_core::document::trace_candidate_strategies(|| {
        core.handle_with_layout(view, CoreEvent::Input(event.clone()))
    });
    result.unwrap_or_else(|error| {
        panic!(
            "{event:?}: {error:?}; source {:?}; {trace:?}",
            String::from_utf8_lossy(&core.document().source_bytes())
        )
    });
}

fn history(
    core: &mut Core<MockTextMeasurementProvider>,
    view: ViewId,
    original: &str,
    expected: &str,
) {
    let saved = core.document().source_bytes();
    let reopened = Document::from_bytes(saved.clone(), Encoding::Utf8, Format::Markdown).unwrap();
    assert_eq!(reopened.text(), expected);
    input(core, view, InputEvent::key('u'));
    assert_eq!(core.document().source_bytes(), original.as_bytes());
    input(core, view, InputEvent::Key(Key::Ctrl('r')));
    assert_eq!(core.document().source_bytes(), saved);
}

#[test]
fn changing_a_link_suffix_at_eof_retains_its_owner_and_title() {
    for source in [
        "[link](https://example.com)",
        "[link](https://example.com \"title\")",
        "before [link](<https://example.com> 'authored title')",
    ] {
        for command in ["C", "cw", "c2w", "cW", "ce", "vlc", "vllc"] {
            let (mut core, view) = open(source);
            let start = core.document().text().find("link").unwrap();
            place(&mut core, view, start + 2, false);
            for key in command.chars() {
                input(&mut core, view, InputEvent::key(key));
            }
            input(&mut core, view, InputEvent::text("x"));
            input(&mut core, view, InputEvent::Key(Key::Escape));
            assert_eq!(
                core.document().source_bytes(),
                source.replace("link", "lix").as_bytes(),
                "{source}: {command}"
            );
            assert_eq!(
                core.document()
                    .link_at(core.document().text_point(start + 2).unwrap())
                    .unwrap()
                    .as_deref(),
                Some("https://example.com")
            );
            let expected = core.document().text().to_owned();
            history(&mut core, view, source, &expected);
        }
    }
}

#[test]
fn native_link_suffix_replacement_retains_destination_and_source_spelling() {
    let source = "[link](<https://example.com> 'title')";
    for start in [1, 2] {
        for reversed in [false, true] {
            let (mut core, view) = open(source);
            input(&mut core, view, InputEvent::key('i'));
            let (anchor, active) = if reversed { (4, start) } else { (start, 4) };
            place(&mut core, view, anchor, false);
            place(&mut core, view, active, true);
            input(&mut core, view, InputEvent::text("x"));
            input(&mut core, view, InputEvent::Key(Key::Escape));
            let label = format!("{}x", &"link"[..start]);
            assert_eq!(core.document().text(), label);
            assert_eq!(
                core.document().source_bytes(),
                source.replace("link", &label).as_bytes()
            );
            history(&mut core, view, source, &label);
        }
    }
}

#[test]
fn inserting_an_adjacent_link_at_the_first_label_boundary_keeps_both_links() {
    let source = "[link](https://example.com \"title\")";
    let (mut core, view) = open(source);
    input(&mut core, view, InputEvent::key('i'));
    place(&mut core, view, 0, false);
    let selection = core.list_selection_identity(view).unwrap();
    core.edit_link(
        view,
        selection.clone(),
        LinkEditIntent::Insert {
            range: selection.range(),
            text: "new".into(),
            destination: "https://new.example/".into(),
        },
    )
    .unwrap();
    assert_eq!(core.document().text(), "newlink");
    assert_eq!(
        core.document()
            .link_at(core.document().text_point(0).unwrap())
            .unwrap()
            .as_deref(),
        Some("https://new.example/")
    );
    assert_eq!(
        core.document()
            .link_at(core.document().text_point(3).unwrap())
            .unwrap()
            .as_deref(),
        Some("https://example.com")
    );
    assert!(String::from_utf8(core.document().source_bytes())
        .unwrap()
        .ends_with(source));
    input(&mut core, view, InputEvent::Key(Key::Escape));
    history(&mut core, view, source, "newlink");
}

#[test]
fn replacing_across_passive_inline_scopes_retains_the_first_selected_style() {
    use viem_core::layout::DocumentLayoutStyles;
    let source = "<ins>under</ins> <sup>sup</sup> <sub>sub</sub>";
    for range in [1..11, 1..12, 2..12, 5..7, 5..8, 8..12, 0..12] {
        for reversed in [false, true] {
            for delete_first in [true, false] {
                let (mut core, view) = open(source);
                let old_text = core.document().text().to_owned();
                let old_style = DocumentLayoutStyles::semantic_character_at(
                    core.document().projection(),
                    range.start,
                    false,
                )
                .unwrap();
                let (anchor, active) = if reversed {
                    (range.end, range.start)
                } else {
                    (range.start, range.end)
                };
                place(&mut core, view, anchor, false);
                place(&mut core, view, active, true);
                if delete_first {
                    input(&mut core, view, InputEvent::Key(Key::Backspace));
                }
                input(&mut core, view, InputEvent::text("x"));
                input(&mut core, view, InputEvent::Key(Key::Escape));
                let expected = format!("{}x{}", &old_text[..range.start], &old_text[range.end..]);
                assert_eq!(
                    core.document().text(),
                    expected,
                    "{range:?}, reversed={reversed}, delete_first={delete_first}"
                );
                let actual = DocumentLayoutStyles::semantic_character_at(
                    core.document().projection(),
                    range.start,
                    false,
                )
                .unwrap();
                assert_eq!(
                    (actual.underline, actual.superscript, actual.subscript),
                    (
                        old_style.underline,
                        old_style.superscript,
                        old_style.subscript
                    )
                );
                history(&mut core, view, source, &expected);
            }
        }
    }
}

#[test]
fn putting_multiple_passive_inline_scopes_preserves_each_copied_style() {
    use viem_core::layout::DocumentLayoutStyles;
    let source = "<ins>under</ins> <sup>sup</sup> <sub>sub</sub>";
    for (start, copied) in [(0, 0..9), (6, 6..13)] {
        let (mut core, view) = open(source);
        let old_text = core.document().text().to_owned();
        let expected_styles = copied
            .clone()
            .map(|at| {
                let style = DocumentLayoutStyles::semantic_character_at(
                    core.document().projection(),
                    at,
                    false,
                )
                .unwrap();
                (style.underline, style.superscript, style.subscript)
            })
            .collect::<Vec<_>>();
        place(&mut core, view, start, false);
        for key in "veey$p".chars() {
            input(&mut core, view, InputEvent::key(key));
        }
        let expected = format!("{old_text}{}", &old_text[copied]);
        assert_eq!(core.document().text(), expected);
        for (index, expected_style) in expected_styles.into_iter().enumerate() {
            let style = DocumentLayoutStyles::semantic_character_at(
                core.document().projection(),
                old_text.len() + index,
                false,
            )
            .unwrap();
            assert_eq!(
                (style.underline, style.superscript, style.subscript),
                expected_style
            );
        }
        history(&mut core, view, source, &expected);
    }
}

#[test]
fn copying_passive_inline_scopes_preserves_authored_wrapper_attributes() {
    let source = "<ins data-note='keep'>under</ins> <sup title='script'>sup</sup> tail";
    for (at, command, expected, copied_source) in [
        (
            0,
            "veey$p",
            "under sup tailunder sup",
            "<ins data-note='keep'>under</ins> <sup title='script'>sup</sup>",
        ),
        (
            1,
            "vly$p",
            "under sup tailnd",
            "<ins data-note='keep'>nd</ins>",
        ),
    ] {
        let (mut core, view) = open(source);
        place(&mut core, view, at, false);
        for key in command.chars() {
            input(&mut core, view, InputEvent::key(key));
        }
        assert_eq!(core.document().text(), expected);
        assert_eq!(
            core.document().source_bytes(),
            format!("{source}{copied_source}").as_bytes()
        );
        history(&mut core, view, source, expected);
    }
}

#[test]
fn copying_nested_equal_effect_scopes_keeps_outer_attributes() {
    for tag in ["ins", "sup", "sub"] {
        let source = format!("<{tag} data-note='keep'><{tag}>under</{tag}></{tag}> tail");
        for (at, command, copied) in [(0, "vey$p", "under"), (1, "vly$p", "nd")] {
            let (mut core, view) = open(&source);
            place(&mut core, view, at, false);
            for key in command.chars() {
                input(&mut core, view, InputEvent::key(key));
            }
            let expected = format!("under tail{copied}");
            assert_eq!(core.document().text(), expected);
            assert_eq!(
                core.document().source_bytes(),
                format!("{source}<{tag} data-note='keep'><{tag}>{copied}</{tag}></{tag}>")
                    .as_bytes()
            );
            history(&mut core, view, &source, &expected);
        }
    }
}

#[test]
fn bounded_passive_copy_never_claims_omitted_attributes_are_normalizable() {
    let source = format!(
        "<ins data-note='keep'>{}under{}</ins> tail",
        "<ins>".repeat(64),
        "</ins>".repeat(64)
    );
    let (core, _) = open(&source);
    let fragment = core.document().clipboard_fragment(1..3).unwrap();
    let exported: serde_json::Value = serde_json::from_str(fragment.json()).unwrap();
    assert_eq!(exported["requires_source_semantics"], true);
    assert_eq!(core.document().source_bytes(), source.as_bytes());
}
