//! Native formatting availability and state: one core decision that both
//! frontends present for toolbars, menus and accelerators.
use viem_core::command::{InputEvent, Key};
use viem_core::document::BoundaryAffinity;
use viem_core::layout::MockTextMeasurementProvider;
use viem_core::FormattingAction::{self, *};
use viem_core::{Core, CoreEvent, Document, Encoding, Format, FormattingActionSet, FormattingActions, SemanticStyleState, ViewId};

fn open_as(source: &str, format: Format) -> (Core<MockTextMeasurementProvider>, ViewId) {
    let mut core = Core::new(Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap());
    let view = core.add_view(MockTextMeasurementProvider::new(), 600.0, 400.0);
    (core, view)
}

fn open(source: &str) -> (Core<MockTextMeasurementProvider>, ViewId) {
    open_as(source, Format::Markdown)
}

fn actions(core: &Core<MockTextMeasurementProvider>, view: ViewId) -> FormattingActions {
    core.formatting_actions(view, FormattingActionSet::ALL).unwrap()
}

fn key(core: &mut Core<MockTextMeasurementProvider>, view: ViewId, key: Key) {
    core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(key))).unwrap();
}

fn place(core: &mut Core<MockTextMeasurementProvider>, view: ViewId, at: usize, extend: bool) {
    core.handle_with_layout(view, CoreEvent::PlaceCursor {
        document_revision: core.document().revision(), text_offset: at,
        affinity: if extend { BoundaryAffinity::Upstream } else { BoundaryAffinity::Downstream },
        extend_selection: extend,
    }).unwrap();
}

fn select_all(core: &mut Core<MockTextMeasurementProvider>, view: ViewId) {
    let end = core.document().text().len();
    place(core, view, 0, false);
    place(core, view, end, true);
}

fn enabled(actions: &FormattingActions, expected: &[FormattingAction]) -> Vec<(FormattingAction, bool)> {
    expected.iter().map(|action| (*action, actions.is_enabled(*action))).collect()
}

const EDITS: &[FormattingAction] = &[Bold, Underline, Code, BlockQuote, AlertNote, BulletedList, Heading1, ParagraphStyles, CharacterStyles];

#[test]
fn formatting_needs_markdown_preview_and_input_that_no_command_owns() {
    let (mut core, view) = open("plain");
    assert!(EDITS.iter().all(|action| actions(&core, view).is_enabled(*action)));
    // Pending operator grammar and the command line take the next key.
    for (start, cancel) in [(Key::Char('d'), Key::Escape), (Key::Char(':'), Key::Escape)] {
        key(&mut core, view, start);
        assert_eq!(actions(&core, view).enabled().bits(), 0, "{start:?}");
        key(&mut core, view, cancel);
    }
    assert!(actions(&core, view).is_enabled(Bold));
    for format in [Format::PlainText, Format::Code] {
        let (core, view) = open_as("plain", format);
        assert_eq!(actions(&core, view).enabled().bits(), 0, "{format:?}");
    }
}

#[test]
fn nomodifiable_disables_formatting_and_readonly_disables_only_caret_typing_styles() {
    // Transactions ignore save-only `readonly`, but pending typing choices
    // at a caret are refused there, so those actions follow that refusal.
    const STRUCTURAL: &[FormattingAction] = &[BlockQuote, AlertNote, BulletedList, Heading1, ParagraphStyles];
    const CARET_TYPING: &[FormattingAction] = &[Bold, Underline, Code, CharacterStyles];
    let (mut core, view) = open("- item");
    for (command, structural, typing) in [("set readonly", true, false), ("set nomodifiable", false, false),
        ("set modifiable", true, false), ("set noreadonly", true, true)] {
        core.stage_command_line(view, command).unwrap();
        key(&mut core, view, Key::Enter);
        let actions = actions(&core, view);
        assert!(STRUCTURAL.iter().all(|action| actions.is_enabled(*action) == structural), "{command}");
        assert!(CARET_TYPING.iter().all(|action| actions.is_enabled(*action) == typing), "{command}");
        // State remains visible while the action is unavailable.
        assert_eq!(actions.state(BulletedList), SemanticStyleState::On, "{command}");
    }
}

#[test]
fn tables_exclude_paragraph_actions_and_code_blocks_exclude_character_actions() {
    let (core, view) = open("| a |\n| - |\n| b |");
    let table = actions(&core, view);
    assert_eq!(enabled(&table, &[ParagraphStyles, BlockQuote, BulletedList, Heading1, CodeBlock, Table, CharacterStyles]),
        [(ParagraphStyles, false), (BlockQuote, false), (BulletedList, false), (Heading1, false), (CodeBlock, false),
            (Table, false), (CharacterStyles, true)]);
    let (core, view) = open("```\ncode\n```");
    let code = actions(&core, view);
    assert_eq!(enabled(&code, &[CharacterStyles, Code, Underline, Strikethrough, BlockQuote, AlertNote, CodeBlock]),
        [(CharacterStyles, false), (Code, false), (Underline, false), (Strikethrough, false), (BlockQuote, false),
            (AlertNote, false), (CodeBlock, true)]);
    assert_eq!(code.state(CodeBlock), SemanticStyleState::On);
}

#[test]
fn alert_kinds_follow_alert_eligibility_and_the_current_kind_can_be_chosen_to_clear_it() {
    // Block quote would nest this item's quote inside its parent's marker.
    let (mut core, view) = open("- a\n  - b");
    place(&mut core, view, 2, false);
    let nested = actions(&core, view);
    assert_eq!(enabled(&nested, &[BlockQuote, AlertNote]), [(BlockQuote, true), (AlertNote, false)]);
    let (core, view) = open("> [!NOTE]\n> body");
    let alert = actions(&core, view);
    assert_eq!(enabled(&alert, &[AlertNote, AlertTip, BlockQuote]), [(AlertNote, true), (AlertTip, true), (BlockQuote, true)]);
    assert_eq!((alert.state(AlertNote), alert.state(AlertTip), alert.state(BlockQuote)),
        (SemanticStyleState::On, SemanticStyleState::Off, SemanticStyleState::On));
    assert_eq!(FormattingAction::alert(viem_core::document::AlertKind::Caution), AlertCaution);
}

#[test]
fn states_distinguish_uniform_and_mixed_membership() {
    for (source, action, state) in [
        ("- one\n- two", BulletedList, SemanticStyleState::On),
        ("- one\n\ntwo", BulletedList, SemanticStyleState::Mixed),
        ("1. one\n2. two", NumberedList, SemanticStyleState::On),
        ("1. one\n2. two", BulletedList, SemanticStyleState::Off),
        ("- [ ] one\n- [x] two", TaskList, SemanticStyleState::On),
        ("> one\n\ntwo", BlockQuote, SemanticStyleState::Mixed),
        ("> [!TIP]\n> one\n\n> two", AlertTip, SemanticStyleState::Mixed),
        ("# one", Heading1, SemanticStyleState::On),
        ("# one", Heading0, SemanticStyleState::Off),
        ("one", Heading0, SemanticStyleState::On),
        ("**one**", Bold, SemanticStyleState::On),
        ("**one** two", Bold, SemanticStyleState::Mixed),
        ("`one`", Code, SemanticStyleState::On),
    ] {
        let (mut core, view) = open(source);
        select_all(&mut core, view);
        assert_eq!(actions(&core, view).state(action), state, "{source:?} {action:?}");
    }
}

#[test]
fn bold_needs_a_settable_target_and_clearing_needs_a_clearable_one() {
    let (mut core, view) = open("**one** two");
    let text = core.document().text().to_owned();
    // Clearing on a fully bold selection and setting on plain text.
    for (word, state) in [("one", SemanticStyleState::On), ("two", SemanticStyleState::Off)] {
        let start = text.find(word).unwrap();
        place(&mut core, view, start, false);
        place(&mut core, view, start + word.len(), true);
        let actions = actions(&core, view);
        assert_eq!((actions.state(Bold), actions.is_enabled(Bold)), (state, true), "{word}");
    }
}

#[test]
fn link_reference_and_image_follow_their_contexts() {
    let (core, view) = open("[label](https://example.com)");
    let link = actions(&core, view);
    assert_eq!((link.state(Link), link.state(Reference)), (SemanticStyleState::On, SemanticStyleState::Off));
    let (core, view) = open("[label][r]\n\n[r]: https://example.com");
    let reference = actions(&core, view);
    assert_eq!((reference.state(Link), reference.state(Reference), reference.is_enabled(Reference)),
        (SemanticStyleState::On, SemanticStyleState::On, true));
    // The caret on an image in Normal mode is that image, not a place to insert one.
    let (core, view) = open("![alt](image.png)");
    let image = actions(&core, view);
    assert_eq!((image.state(Image), image.is_enabled(Image)), (SemanticStyleState::On, false));
    let (core, view) = open("plain");
    let plain = actions(&core, view);
    assert_eq!((plain.state(Image), plain.is_enabled(Image)), (SemanticStyleState::Off, true));
}

#[test]
fn table_insertion_needs_a_place_outside_tables() {
    let (core, view) = open("plain");
    assert!(actions(&core, view).is_enabled(Table));
    let (core, view) = open("| a |\n| - |\n| b |");
    assert!(!actions(&core, view).is_enabled(Table));
}

#[test]
fn only_requested_actions_are_reported() {
    let (core, view) = open("# heading");
    let requested = FormattingActionSet::of(&[Bold]);
    let actions = core.formatting_actions(view, requested).unwrap();
    assert!(actions.is_enabled(Bold));
    assert_eq!((actions.is_enabled(Heading1), actions.state(Heading1)), (false, SemanticStyleState::Off));
    assert_eq!(actions.enabled().bits() & !requested.bits(), 0);
    assert_eq!(FormattingActionSet::from_bits(u64::MAX), FormattingActionSet::ALL);
}

#[test]
fn formatting_actions_ffi_reports_the_core_decision() {
    use viem_core::ffi::*;
    for (source, task, available) in [
        ("ordinary", false, true), ("3. [X] task", true, true),
        ("- [ ] task", true, true), ("# heading", false, false),
        ("```\ncode\n```", false, false),
    ] {
        let fixture = super::ffi_core_surface::table_ffi_fixture(source.as_bytes(), VIEM_FORMAT_MARKDOWN);
        let mut info = ViemFormattingActions::default();
        let bit = 1u64 << VIEM_FORMATTING_TASK_LIST;
        let other_lists = (1u64 << VIEM_FORMATTING_BULLETED_LIST) | (1u64 << VIEM_FORMATTING_NUMBERED_LIST);
        unsafe {
            assert_eq!(viem_core_view_formatting_actions(fixture.handle, fixture.view, bit | other_lists, &mut info), ViemStatus::Ok);
        }
        assert_eq!(info.on & bit != 0, task, "{source}");
        assert_eq!(info.enabled & bit != 0, available, "{source}");
        // A task item is exclusively a task, never also bulleted or numbered.
        if task { assert_eq!((info.on | info.mixed) & other_lists, 0, "{source}"); }
        assert_eq!((info.enabled | info.on | info.mixed) & !(bit | other_lists), 0, "{source}");
    }
    for action in FormattingAction::ALL {
        assert_eq!(FormattingAction::from_id(action.id()), Some(action));
    }
    assert_eq!((VIEM_FORMATTING_ALERT_CAUTION, VIEM_FORMATTING_HEADING_6, VIEM_FORMATTING_CHARACTER_STYLES),
        (AlertCaution.id(), Heading6.id(), CharacterStyles.id()));
}


#[test]
fn availability_reads_local_context_without_parsing_or_preparing_edits() {
    use viem_core::document::measure_document_work;
    for count in [32, 4_000] {
        let source = format!("> [!NOTE]\n> - item [link](https://example.com)\n\n{}", "Unrelated **bold** paragraph.\n\n".repeat(count));
        let (mut core, view) = open(&source);
        place(&mut core, view, 2, false);
        let (actions, work) = measure_document_work(|| core.formatting_actions(view, FormattingActionSet::ALL).unwrap());
        assert_eq!(actions.state(AlertNote), SemanticStyleState::On, "{count}");
        assert_eq!(work.source_full_materializations, 0, "{count}: {work:?}");
        assert_eq!(work.full_projection_candidates, 0, "{count}: {work:?}");
        assert_eq!(work.scratch_documents, 0, "{count}: {work:?}");
        assert_eq!(work.markdown_block_parse_calls, 0, "{count}: {work:?}");
        assert!(work.source_decoded_bytes < 2048, "{count}: {work:?}");
    }
}
