use viem_core::document::*;
use viem_core::{Document, Encoding, Format};

fn open() -> Document {
    Document::from_bytes(br"{\rtf1\ansi{\fonttbl{\f0\fnil Arial;}}{\stylesheet{\s0\fs24 Normal;}{\s5\sbasedon0\snext0\b\fs32\unknown44 Heading 1;}{\*\cs2\i Accent;}}\s5 Title\par \s0 Body {\cs2 accent}}".to_vec(), Encoding::Utf8, Format::Rtf).unwrap()
}
fn apply(document: &mut Document, intent: PersistedStyleIntent) {
    document
        .apply_style_request(StyleModelRequest::new(
            document.id(),
            document.revision(),
            StyleModelIntent::Persisted(intent),
        ))
        .unwrap();
}
fn edit(document: &mut Document, edit: StyleDefinitionEdit) {
    apply(
        document,
        PersistedStyleIntent::EditStyleDefinition {
            origin: StyleDefinitionOrigin::SourceBacked,
            edit,
        },
    );
}
fn range(document: &Document, start: usize, end: usize) -> TextRange {
    TextRange::new(
        document.text_point(start).unwrap(),
        document.text_point(end).unwrap(),
    )
    .unwrap()
}

#[test]
fn empty_paragraph_assignment_survives_scope_closures_and_reopen() {
    let header = r"{\rtf1{\stylesheet{\s0 Normal;}{\s5\sbasedon0\snext0\b Heading 1;}}";
    let mut failures = Vec::new();
    for (requested, expected) in [("Paragraph", "Paragraph"), ("Heading1", "RtfP5")] {
        for (body, at) in [
            (r"\s5\par \s0 Tail}", 0),
            (r"\s0 A\par \s5\par \s0 Tail}", 2),
            (r"\s5 A\par}", 2),
            (r"\s5{\i }\par \s0 Tail}", 0),
            (r"\s5{\*\unknown keep}\par \s0 Tail}", 0),
            (r"\s5{\i }{\*\unknown \par keep}\par \s0 Tail}", 0),
            (r"\s5{\i }\par \s0 Tail\par \s5 Last}", 0),
            (r"\s0 A\par {\s5 }\par \s0 Tail}", 2),
            (r"\s0 A\par {\s5\i }\par \s0 Tail}", 2),
        ] {
            let source = format!("{header}{body}");
            let mut document =
                Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Rtf)
                    .unwrap();
            let previous = document.projection().blocks().to_vec();
            let selected = range(&document, at, at);
            let request = StyleModelRequest::new(
                document.id(),
                document.revision(),
                StyleModelIntent::Persisted(PersistedStyleIntent::AssignBlockStyle {
                    target: StyleBlockTarget::Paragraphs(selected),
                    style: requested.into(),
                }),
            );
            if let Err(error) = document.apply_style_request(request) {
                failures.push(format!("{body:?} at {at}, {requested}: {error:?}"));
                continue;
            }
            let saved = document.source_bytes();
            assert!(saved.starts_with(header.as_bytes()));
            if body.contains(r"{\*\unknown \par keep}") {
                assert!(String::from_utf8_lossy(&saved).contains(r"{\*\unknown \par keep}"));
            }
            let mut reopened =
                Document::from_bytes(saved.clone(), Encoding::Utf8, Format::Rtf).unwrap();
            for snapshot in [&document, &reopened] {
                for (old, block) in previous.iter().zip(snapshot.projection().blocks()) {
                    assert_eq!(
                        block.style,
                        if old.range == (at..at) {
                            expected.into()
                        } else {
                            old.style.clone()
                        },
                        "{body}"
                    );
                }
            }
            reopened.replace(at..at, "X").unwrap();
            assert_eq!(
                reopened
                    .projection()
                    .blocks()
                    .iter()
                    .find(|block| block.range.contains(&at))
                    .unwrap()
                    .style
                    .0,
                expected,
                "{body}: subsequent typing"
            );
            let typed = viem_core::layout::DocumentLayoutStyles::character_at(
                reopened.projection(),
                at,
                false,
            )
            .unwrap();
            assert_eq!(
                typed.slant,
                if body.contains(r"\i") {
                    FontSlant::Italic
                } else {
                    FontSlant::Upright
                },
                "{body}: retained inline context"
            );
            assert!(document.undo());
            assert_eq!(document.source_bytes(), source.as_bytes());
            assert!(document.redo());
            assert_eq!(document.source_bytes(), saved);
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn imports_native_style_handles_links_and_body_assignments() {
    let document = open();
    assert_eq!(document.text(), "Title\nBody accent");
    let sheet = document.projection().style_sheet();
    let heading = sheet.block_style(&"RtfP5".into()).unwrap();
    assert_eq!(heading.based_on, Some("Paragraph".into()));
    assert_eq!(heading.next_paragraph_style, Some("Paragraph".into()));
    assert_eq!(heading.character.size, Some((16.0).into()));
    assert_eq!(
        document.projection().blocks()[0].style,
        StyleId::from("RtfP5")
    );
    assert_eq!(
        document.projection().blocks()[0].kind,
        BlockKind::Heading(1)
    );
    assert_eq!(
        sheet
            .character_style(&"RtfC2".into())
            .unwrap()
            .properties
            .slant,
        Some(FontSlant::Italic)
    );
    assert!(document
        .projection()
        .style_spans()
        .iter()
        .any(|span| span.range == (11..17)
            && span.application == StyleApplication::Named("RtfC2".into())));
}

#[test]
fn editing_and_renaming_one_rtf_style_preserves_siblings_and_handle_identity() {
    let mut document = open();
    let original = document.source_bytes();
    let mut heading = document
        .projection()
        .style_sheet()
        .block_style(&"RtfP5".into())
        .unwrap()
        .clone();
    heading.character.size = Some((18.0).into());
    edit(&mut document, StyleDefinitionEdit::UpdateBlock(heading));
    let changed = String::from_utf8(document.source_bytes()).unwrap();
    assert!(changed.contains(r"{\*\cs2\i Accent;}"));
    assert!(changed.contains(r"\s5 Title\par \s0 Body {\cs2 accent}"));
    assert!(!changed.contains(r"\unknown44"));
    edit(
        &mut document,
        StyleDefinitionEdit::UpdateMetadata {
            namespace: StyleNamespace::Block,
            id: "RtfP5".into(),
            metadata: StyleDefinitionMetadata {
                display_name: "Chapter".into(),
                origin: StyleDefinitionOrigin::SourceBacked,
            },
        },
    );
    assert_eq!(
        document
            .projection()
            .style_sheet()
            .block_style_metadata(&"RtfP5".into())
            .unwrap()
            .display_name,
        "Chapter"
    );
    assert!(document.undo());
    assert!(document.undo());
    assert_eq!(document.source_bytes(), original);
}

#[test]
fn creates_assigns_and_deletes_native_style_without_renumbering() {
    let mut document = open();
    let original = document.source_bytes();
    edit(
        &mut document,
        StyleDefinitionEdit::InsertBlock {
            style: BlockStyle {
                id: "RtfP9".into(),
                based_on: Some("RtfP5".into()),
                next_paragraph_style: Some("Paragraph".into()),
                role: BlockRole::Paragraph,
                character: CharacterProperties::default(),
                block: BlockProperties {
                    spacing_after: Some(8.0),
                    ..Default::default()
                },
            },
            metadata: StyleDefinitionMetadata {
                display_name: "Chapter Child".into(),
                origin: StyleDefinitionOrigin::SourceBacked,
            },
        },
    );
    let selected = range(&document, 6, 17);
    apply(
        &mut document,
        PersistedStyleIntent::AssignBlockStyle {
            target: StyleBlockTarget::Paragraphs(selected),
            style: "RtfP9".into(),
        },
    );
    assert_eq!(
        document.projection().blocks()[1].style,
        StyleId::from("RtfP9")
    );
    edit(
        &mut document,
        StyleDefinitionEdit::DeleteBlock("RtfP9".into()),
    );
    assert!(document
        .projection()
        .style_sheet()
        .block_style(&"RtfP9".into())
        .is_none());
    assert_eq!(
        document.projection().blocks()[1].style,
        StyleId::from("Paragraph")
    );
    for _ in 0..3 {
        assert!(document.undo());
    }
    assert_eq!(document.source_bytes(), original);
}

#[test]
fn deleting_in_use_character_style_uses_default_not_parent_and_retains_direct_properties() {
    let source = r"{\rtf1{\stylesheet{\s0 Normal;}{\*\cs2\i Parent;}{\*\cs3\sbasedon2\b Child;}}\cs3\ul Text{\*\unknown Opaque}}";
    let mut document =
        Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Rtf).unwrap();
    edit(
        &mut document,
        StyleDefinitionEdit::DeleteCharacter("RtfC3".into()),
    );
    let resolved =
        viem_core::layout::DocumentLayoutStyles::character_at(document.projection(), 0, false)
            .unwrap();
    assert_eq!(resolved.slant, FontSlant::Upright);
    assert!(!resolved.bold);
    assert!(resolved.underline);
    let saved = String::from_utf8(document.source_bytes()).unwrap();
    assert!(saved.contains(r"{\*\cs2\i Parent;}"));
    assert!(saved.ends_with(r"{\*\unknown Opaque}}"));
    let reopened =
        Document::from_bytes(saved.as_bytes().to_vec(), Encoding::Utf8, Format::Rtf).unwrap();
    assert_eq!(
        viem_core::layout::DocumentLayoutStyles::character_at(reopened.projection(), 0, false)
            .unwrap(),
        resolved
    );
    assert!(document.undo());
    assert_eq!(document.source_bytes(), source.as_bytes());
}

#[test]
fn named_rtf_optional_font_features_round_trip_without_rewriting_opaque_style_controls() {
    let mut document = open();
    let original = document.source_bytes();
    let mut style = document
        .projection()
        .style_sheet()
        .character_style(&"RtfC2".into())
        .unwrap()
        .clone();
    style.properties.open_type_features = Some(std::collections::BTreeMap::from([
        ("liga".into(), 0),
        ("ss01".into(), 1),
    ]));
    edit(&mut document, StyleDefinitionEdit::UpdateCharacter(style));
    let selected =
        viem_core::layout::DocumentLayoutStyles::character_at(document.projection(), 12, false)
            .unwrap();
    assert_eq!(selected.open_type_features.get("liga"), Some(&0));
    assert_eq!(selected.open_type_features.get("ss01"), Some(&1));
    let saved = document.source_bytes();
    assert!(String::from_utf8(saved.clone())
        .unwrap()
        .contains("\\unknown44"));
    let reopened = Document::from_bytes(saved, Encoding::Utf8, Format::Rtf).unwrap();
    assert_eq!(
        viem_core::layout::DocumentLayoutStyles::character_at(reopened.projection(), 12, false)
            .unwrap(),
        selected
    );
    assert!(document.undo());
    assert_eq!(document.source_bytes(), original);
}

#[test]
fn native_style_unicode_names_and_resource_patches_round_trip_exactly() {
    let mut document = open();
    let original = document.source_bytes();
    let mut style = document
        .projection()
        .style_sheet()
        .block_style(&"RtfP5".into())
        .unwrap()
        .clone();
    style.character.font_families = Some(vec!["Georgia".into()]);
    style.character.foreground = Some(Color {
        red: 1.0,
        green: 0.0,
        blue: 0.0,
        alpha: 1.0,
    });
    edit(&mut document, StyleDefinitionEdit::UpdateBlock(style));
    edit(
        &mut document,
        StyleDefinitionEdit::UpdateMetadata {
            namespace: StyleNamespace::Block,
            id: "RtfP5".into(),
            metadata: StyleDefinitionMetadata {
                display_name: "Chapter 😀; α *".into(),
                origin: StyleDefinitionOrigin::SourceBacked,
            },
        },
    );
    let native = String::from_utf8(document.source_bytes()).unwrap();
    assert!(native.contains(r"{\f0\fnil Arial;}"));
    assert!(native.contains(r"{\f1\fnil Georgia;}"));
    let reopened =
        Document::from_bytes(document.source_bytes(), Encoding::Utf8, Format::Rtf).unwrap();
    assert_eq!(
        reopened
            .projection()
            .style_sheet()
            .block_style_metadata(&"RtfP5".into())
            .unwrap()
            .display_name,
        "Chapter 😀; α *"
    );
    assert_eq!(
        reopened
            .projection()
            .style_sheet()
            .block_style(&"RtfP5".into())
            .unwrap()
            .character
            .font_families,
        Some(vec!["Georgia".into()])
    );
    assert!(document.undo());
    assert!(document.undo());
    assert_eq!(document.source_bytes(), original);
}

#[test]
fn plain_resets_named_character_overrides_default_font_and_script_position() {
    let source=br"{\rtf1\ansi\deff3{\fonttbl{\f0 Arial;}{\f3 Georgia;}}{\colortbl;\red255\green0\blue0;}{\stylesheet{\s0\b\fs32 Normal;}{\*\cs7\i Accent;}}\s0\cs7\f0\fs36\cf1\super old\plain new}";
    let document = Document::from_bytes(source.to_vec(), Encoding::Utf8, Format::Rtf).unwrap();
    let span = document
        .projection()
        .style_spans()
        .iter()
        .find_map(|span| {
            if span.range.contains(&4) {
                if let StyleApplication::Direct(properties) = &span.application {
                    Some(properties)
                } else {
                    None
                }
            } else {
                None
            }
        })
        .unwrap();
    assert_eq!(span.font_families, Some(vec!["Georgia".into()]));
    assert_eq!(span.weight, Some(400));
    assert_eq!(span.size, Some((12.0).into()));
    assert_eq!(span.slant, Some(FontSlant::Upright));
    assert_eq!(span.script_position, Some(ScriptPosition::Normal));
    assert_eq!(span.foreground, None);
    assert!(!document
        .projection()
        .style_spans()
        .iter()
        .any(|span| span.range.contains(&4)
            && matches!(span.application, StyleApplication::Named(_))));
}

#[test]
fn enter_uses_following_paragraph_style_and_repeats_as_an_undoable_intention() {
    use viem_core::command::{CommandInterpreter, InputEvent, Key};
    let source = br"{\rtf1{\stylesheet{\s0 Normal;}{\s5\sbasedon0\snext0\b Heading 1;}}\s5 Title}";
    let mut document = Document::from_bytes(source.to_vec(), Encoding::Utf8, Format::Rtf).unwrap();
    let mut commands = CommandInterpreter::new();
    for event in [InputEvent::key('A'), InputEvent::Key(Key::Enter)] {
        commands.handle(&mut document, event).unwrap();
    }
    assert_eq!(document.text(), "Title\n");
    assert_eq!(
        document.projection().blocks()[1].style,
        StyleId::from("Paragraph")
    );
    commands
        .handle(&mut document, InputEvent::text("Body"))
        .unwrap();
    assert_eq!(document.text(), "Title\nBody");
    assert_eq!(
        document.projection().blocks()[1].style,
        StyleId::from("Paragraph")
    );
    commands
        .handle(&mut document, InputEvent::Key(Key::Escape))
        .unwrap();
    commands
        .handle(&mut document, InputEvent::key('u'))
        .unwrap();
    assert_eq!(document.source_bytes(), source);
}

#[test]
fn heading_alias_reuses_native_handle_and_new_heading_allocates_once() {
    let mut document = open();
    let original = document.source_bytes();
    document
        .set_paragraph_style(6..17, "Heading1".into())
        .unwrap();
    assert_eq!(
        document.projection().blocks()[1].style,
        StyleId::from("RtfP5")
    );
    assert_eq!(
        document
            .projection()
            .style_sheet()
            .block_styles()
            .filter(|style| document
                .projection()
                .style_sheet()
                .block_style_metadata(&style.id)
                .is_some_and(|metadata| metadata.origin == StyleDefinitionOrigin::SourceBacked))
            .count(),
        2
    );
    assert!(document.undo());
    assert_eq!(document.source_bytes(), original);
    let mut document =
        Document::from_bytes(br"{\rtf1 Title}".to_vec(), Encoding::Utf8, Format::Rtf).unwrap();
    document
        .set_paragraph_style(0..0, "Heading2".into())
        .unwrap();
    assert_eq!(
        document.projection().blocks()[0].kind,
        BlockKind::Heading(2)
    );
    assert_eq!(
        document.projection().blocks()[0].style,
        StyleId::from("RtfP1")
    );
}

#[test]
fn mid_paragraph_enter_retains_current_style_and_copies_sparse_direct_defaults() {
    use viem_core::command::{CommandInterpreter, InputEvent, Key};
    let source=br"{\rtf1{\stylesheet{\s0 Normal;}{\s5\sbasedon0\snext0\b Heading 1;}}\s5\i\sa120 Title rest}";
    let mut document = Document::from_bytes(source.to_vec(), Encoding::Utf8, Format::Rtf).unwrap();
    let mut commands = CommandInterpreter::new();
    for event in [
        InputEvent::key('3'),
        InputEvent::key('l'),
        InputEvent::key('i'),
        InputEvent::Key(Key::Enter),
        InputEvent::Key(Key::Escape),
    ] {
        commands.handle(&mut document, event).unwrap();
    }
    assert_eq!(document.text(), "Tit\nle rest");
    assert_eq!(
        document.projection().blocks()[1].style,
        StyleId::from("RtfP5")
    );
    assert_eq!(
        document.projection().blocks()[1]
            .direct_paragraph
            .spacing_after,
        Some(6.0)
    );
    assert!(document.projection().style_spans().iter().any(|span|span.range.contains(&4)&&matches!(&span.application,StyleApplication::Direct(properties) if properties.slant==Some(FontSlant::Italic))));
    commands
        .handle(&mut document, InputEvent::key('u'))
        .unwrap();
    assert_eq!(document.source_bytes(), source);
}

#[test]
fn freshly_authored_heading_enters_implicit_normal_style_without_s0_definition() {
    use viem_core::command::{CommandInterpreter, InputEvent, Key};
    let mut document =
        Document::from_bytes(br"{\rtf1 Title}".to_vec(), Encoding::Utf8, Format::Rtf).unwrap();
    document
        .set_paragraph_style(0..0, "Heading2".into())
        .unwrap();
    let heading = document.source_bytes();
    let mut commands = CommandInterpreter::new();
    for event in [
        InputEvent::key('A'),
        InputEvent::Key(Key::Enter),
        InputEvent::text("Body"),
        InputEvent::Key(Key::Escape),
    ] {
        commands.handle(&mut document, event).unwrap();
    }
    assert_eq!(document.text(), "Title\nBody");
    assert_eq!(
        document.projection().blocks()[1].style,
        StyleId::from("Paragraph")
    );
    assert!(document.undo());
    assert_eq!(document.source_bytes(), heading);
}

#[test]
fn character_assignment_across_source_groups_preserves_unselected_text_and_direct_overrides() {
    let source =
        br"{\rtf1{\stylesheet{\*\cs2\i Accent;}}{\b Before pick} more{\*\unknown Opaque} tail}";
    let mut document = Document::from_bytes(source.to_vec(), Encoding::Utf8, Format::Rtf).unwrap();
    let selected = range(&document, 7, 16);
    apply(
        &mut document,
        PersistedStyleIntent::AssignCharacterStyle {
            range: selected,
            style: "RtfC2".into(),
        },
    );
    assert_eq!(document.text(), "Before pick more tail");
    for at in 0..document.text().len() {
        let spans = document
            .projection()
            .style_spans()
            .iter()
            .filter(|span| span.range.contains(&at))
            .collect::<Vec<_>>();
        assert_eq!(
            spans
                .iter()
                .any(|span| span.application == StyleApplication::Named("RtfC2".into())),
            (7..16).contains(&at)
        );
        assert_eq!(spans.iter().any(|span|matches!(&span.application,StyleApplication::Direct(value) if value.bold==Some(true))),at<11);
    }
    assert!(String::from_utf8_lossy(&document.source_bytes()).contains(r"{\*\unknown Opaque}"));
    assert!(document.undo());
    assert_eq!(document.source_bytes(), source);
}


#[test]
fn default_paragraph_character_assignment_handles_a_real_cs0_and_keeps_source_scopes() {
    use viem_core::layout::DocumentLayoutStyles;
    let header = r"{\rtf1\deff2{\fonttbl{\f0 Arial;}{\f1 Courier;}{\f2 Times New Roman;}}{\stylesheet{\s0\fs24 Normal;}{\s5\sbasedon0\fs40 Heading;}{\*\cs0\b Zero;}{\*\cs2\f1\i Accent;}}";
    for assigned in [0, 2] {
        let source = format!("{header}\\s5\\li60\\cs{assigned}\\ul Before pick{{\\*\\unknown Opaque}} after}}");
        let mut document = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Rtf).unwrap();
        let before = DocumentLayoutStyles::character_at(document.projection(), 0, false).unwrap();
        let selected = range(&document, 7, 11);
        apply(&mut document, PersistedStyleIntent::AssignCharacterStyle {
            range: selected,
            style: "".into(),
        });
        assert_eq!(document.text(), "Before pick after");
        let cleared = DocumentLayoutStyles::character_at(document.projection(), 7, false).unwrap();
        assert_eq!(cleared.font_families, vec!["Times New Roman"]);
        assert_eq!(cleared.size, 20.0);
        assert_eq!(cleared.slant, FontSlant::Upright);
        assert!(!cleared.bold);
        assert!(cleared.underline, "Sparse direct formatting survives named-style clearing");
        for at in [0, 12] {
            assert_eq!(DocumentLayoutStyles::character_at(document.projection(), at, false).unwrap(), before);
        }
        assert!(!document.projection().style_spans().iter().any(|span| {
            span.range.contains(&7) && matches!(span.application, StyleApplication::Named(_))
        }));
        assert_eq!(document.projection().blocks()[0].direct_paragraph.leading_indent, Some(3.0));
        let saved = document.source_bytes();
        assert!(saved.starts_with(header.as_bytes()), "Styles and font tables remain byte exact");
        assert!(String::from_utf8_lossy(&saved).contains(r"{\*\unknown Opaque}"));
        let reopened = Document::from_bytes(saved.clone(), Encoding::Utf8, Format::Rtf).unwrap();
        assert_eq!(DocumentLayoutStyles::character_at(reopened.projection(), 7, false).unwrap(), cleared);
        let mut heading = document.projection().style_sheet().block_style(&"RtfP5".into()).unwrap().clone();
        heading.character.size = Some((22.0).into());
        edit(&mut document, StyleDefinitionEdit::UpdateBlock(heading));
        assert_eq!(DocumentLayoutStyles::character_at(document.projection(), 7, false).unwrap().size, 22.0,
            "Clearing must retain paragraph inheritance rather than freeze its appearance as direct formatting");
        assert!(document.undo());
        assert_eq!(document.source_bytes(), saved);
        assert!(document.undo());
        assert_eq!(document.source_bytes(), source.as_bytes());
    }
}
