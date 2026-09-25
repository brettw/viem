use viem_core::command::composition::{CompositionEvent, CompositionTarget, CompositionUpdate};
use viem_core::command::{InputEvent, Key};
use viem_core::document::{BoundaryAffinity, Document, Encoding, Format};
use viem_core::layout::{DocumentLayoutStyles, MockTextMeasurementProvider};
use viem_core::{Core, CoreEvent, ViewId};

type Editor = Core<MockTextMeasurementProvider>;

fn input(core: &mut Editor, view: ViewId, event: CoreEvent) {
    let label = format!("{event:?}");
    core.handle_with_layout(view, event)
        .unwrap_or_else(|error| {
            panic!(
                "{label}: {error:?}, before text {:?}",
                core.document().text()
            )
        });
}

fn key(core: &mut Editor, view: ViewId, value: Key) {
    input(core, view, CoreEvent::Input(InputEvent::Key(value)));
}

fn text(core: &mut Editor, view: ViewId, value: &str) {
    input(core, view, CoreEvent::Input(InputEvent::text(value)));
}

fn visible(value: &str) -> String {
    value.replace('\u{a0}', " ")
}

#[test]
fn replacement_space_keeps_insert_session_boundaries_after_supporting_whitespace_changes() {
    for source in [
        "<p>A <b>B</b> C</p>\r\n<p>D</p>",
        "<p>A <a href='/b'><b>B</b></a> C</p>\r\n<p>D</p>",
    ] {
        for encoding in [Encoding::Utf8, Encoding::Utf16Le, Encoding::Utf16Be] {
            let bytes: Vec<u8> = match encoding {
                Encoding::Utf8 => source.as_bytes().to_vec(),
                Encoding::Utf16Le => source.encode_utf16().flat_map(u16::to_le_bytes).collect(),
                _ => source.encode_utf16().flat_map(u16::to_be_bytes).collect(),
            };
            for reverse in [false, true] {
                for route in ["native", "vim", "ime"] {
                    let mut core = Core::new(
                        Document::from_bytes(bytes.clone(), encoding, Format::Html).unwrap(),
                    );
                    let view = core.add_view(MockTextMeasurementProvider::new(), 400., 160.);
                    if route == "vim" {
                        for ch in (if reverse { "llllvhhc" } else { "llvllc" }).chars() {
                            key(&mut core, view, Key::Char(ch));
                        }
                        text(&mut core, view, " ");
                    } else {
                        // Insert mode permits the end-of-line caret used as
                        // the anchor of a backwards native selection.
                        key(&mut core, view, Key::Char('i'));
                        for (index, at) in (if reverse { [5, 2] } else { [2, 5] })
                            .into_iter()
                            .enumerate()
                        {
                            core.handle(
                                view,
                                CoreEvent::PlaceCursor {
                                    document_revision: core.document().revision(),
                                    text_offset: at,
                                    affinity: BoundaryAffinity::Downstream,
                                    extend_selection: index == 1,
                                },
                            )
                            .unwrap();
                        }
                        if route == "ime" {
                            let target =
                                CompositionTarget::at_offsets(core.document(), 2..5).unwrap();
                            input(
                                &mut core,
                                view,
                                CoreEvent::Composition(CompositionEvent::Begin(target)),
                            );
                            input(
                                &mut core,
                                view,
                                CoreEvent::Composition(CompositionEvent::Update(
                                    CompositionUpdate::new(" ", 1..1),
                                )),
                            );
                            input(
                                &mut core,
                                view,
                                CoreEvent::Composition(CompositionEvent::Commit),
                            );
                        } else {
                            text(&mut core, view, " ");
                        }
                    }
                    assert_eq!(
                        visible(core.document().text()),
                        "A  \nD",
                        "{encoding:?} {route} reverse={reverse}"
                    );
                    let inserted = core.document().text().char_indices().nth(2).unwrap().0;
                    assert!(
                        !DocumentLayoutStyles::semantic_character_at(
                            core.document().projection(),
                            1,
                            false
                        )
                        .unwrap()
                        .bold
                    );
                    assert!(
                        DocumentLayoutStyles::semantic_character_at(
                            core.document().projection(),
                            inserted,
                            false
                        )
                        .unwrap()
                        .bold
                    );
                    assert_eq!(
                        core.document()
                            .link_at(core.document().text_point(1).unwrap())
                            .unwrap(),
                        None
                    );
                    assert_eq!(
                        core.document()
                            .link_at(core.document().text_point(inserted).unwrap())
                            .unwrap()
                            .as_deref(),
                        source.contains("href=").then_some("/b")
                    );
                    core.document()
                        .text_point(core.command_state(view).unwrap().cursor())
                        .unwrap();
                    text(&mut core, view, "Y");
                    assert_eq!(visible(core.document().text()), "A  Y\nD");
                    let fresh = Document::from_bytes(
                        core.document().source_bytes(),
                        encoding,
                        Format::Html,
                    )
                    .unwrap();
                    assert_eq!(core.document().text(), fresh.text());
                    for (at, _) in fresh.text().char_indices() {
                        assert_eq!(
                            DocumentLayoutStyles::semantic_character_at(
                                core.document().projection(),
                                at,
                                false
                            )
                            .unwrap(),
                            DocumentLayoutStyles::semantic_character_at(
                                fresh.projection(),
                                at,
                                false
                            )
                            .unwrap()
                        );
                    }
                    if route != "ime" {
                        key(&mut core, view, Key::Ctrl('u'));
                        assert_eq!(
                            visible(core.document().text()),
                            "A \nD",
                            "Insert Ctrl-U must retain the preceding space"
                        );
                    }
                    key(&mut core, view, Key::Escape);
                    key(&mut core, view, Key::Char('u'));
                    if route == "ime" {
                        key(&mut core, view, Key::Char('u'));
                    }
                    assert_eq!(core.document().source_bytes(), bytes);
                }
            }
        }
    }
}
