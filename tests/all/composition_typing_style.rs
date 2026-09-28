use viem_core::command::composition::{CompositionEvent, CompositionTarget, CompositionUpdate};
use viem_core::command::{InputEvent, Key};
use viem_core::document::{
    BoundaryAffinity, Document, Encoding, FontSlant, Format, SemanticInlineStyle,
};
use viem_core::layout::{DocumentLayoutStyles, MockTextMeasurementProvider};
use viem_core::{Core, CoreEvent};

#[test]
fn marked_text_commits_pending_style_atomically_and_retains_it_for_subsequent_typing() {
    for (format, source, at) in [
        (Format::Markdown, "word", 0),
        (Format::MarkdownSource, "word", 0),

    ] {
        let mut core = Core::new(
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap(),
        );
        let view = core.add_view(MockTextMeasurementProvider::new(), 300.0, 160.0);
        core.handle(view, CoreEvent::Input(InputEvent::key('i')))
            .unwrap();
        core.handle(
            view,
            CoreEvent::PlaceCursor {
                document_revision: core.document().revision(),
                text_offset: at,
                affinity: BoundaryAffinity::Downstream,
                extend_selection: false,
            },
        )
        .unwrap();
        let expected = core.list_selection_identity(view).unwrap();
        core.handle(
            view,
            CoreEvent::SetSelectionSemanticStyle {
                expected,
                style: SemanticInlineStyle::Emphasis,
                enabled: true,
            },
        )
        .unwrap();
        let target = CompositionTarget::at_offsets(core.document(), at..at).unwrap();
        core.handle(
            view,
            CoreEvent::Composition(CompositionEvent::Begin(target)),
        )
        .unwrap();
        core.handle(
            view,
            CoreEvent::Composition(CompositionEvent::Update(CompositionUpdate::new("猫", 3..3))),
        )
        .unwrap();
        assert_eq!(
            core.document().source_bytes(),
            source.as_bytes(),
            "{format:?}: marked text must remain an overlay"
        );
        let outcome = core
            .handle(view, CoreEvent::Composition(CompositionEvent::Commit))
            .unwrap();
        assert!(outcome.document_changed);
        let caret = core.command_state(view).unwrap().cursor();
        core.document().text_point(caret).unwrap();
        assert_ne!(
            DocumentLayoutStyles::character_at(core.document().projection(), caret, true)
                .unwrap()
                .slant,
            FontSlant::Upright,
            "{format:?}"
        );
        assert_ne!(
            core.selected_character_style(view).unwrap().slant,
            FontSlant::Upright
        );
        let after_composition = core.document().source_bytes();
        core.handle(view, CoreEvent::Input(InputEvent::text("é")))
            .unwrap();
        assert_ne!(
            core.selected_character_style(view).unwrap().slant,
            FontSlant::Upright
        );
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Escape)))
            .unwrap();
        core.handle(view, CoreEvent::Input(InputEvent::key('u')))
            .unwrap();
        assert_eq!(
            core.document().source_bytes(),
            after_composition,
            "{format:?}: next typing has its own undo unit"
        );
        core.handle(view, CoreEvent::Input(InputEvent::key('u')))
            .unwrap();
        assert_eq!(
            core.document().source_bytes(),
            source.as_bytes(),
            "{format:?}: composition text and formatting undo together"
        );
    }
}
