//! Exercise structural deletion through the same commands used by native line selection.
use viem_core::command::{CommandStatus, InputEvent, Key, Mode, SelectionOrigin};
use viem_core::document::{BoundaryAffinity, Document, Encoding, Format};
use viem_core::layout::MockTextMeasurementProvider;
use viem_core::{Core, CoreEvent, ViewId};

type Editor = Core<MockTextMeasurementProvider>;

const FIXTURES: &[(Format, &str)] = &[
    (Format::Markdown, "- a\n- b"),
    (Format::Markdown, "- a\n  - b\n  - c\n- d"),
    (Format::Markdown, "- a\n\n  b\n- c"),
    (Format::Markdown, "1. a\n2. b\n   - c"),
    (Format::Markdown, "- a  \n  b\n- c"),
    (Format::Markdown, "> a\n>\n> b"),
    (Format::Markdown, "# a\n\nb\n\n```\nc\nd\n```"),
    (Format::Markdown, "a\n\n---\n\nb"),
    (Format::Rtf, "{\\rtf1 a\\par b}"),
    (Format::Rtf, "{\\rtf1 a\\line b\\par c\\par}"),
    (Format::Rtf, "{\\rtf1 {\\b a\\par b}\\par c}"),
    (Format::Rtf, "{\\rtf1 a\\par {\\pict\\pngblip 00}\\par b}"),
    (Format::Rtf, "{\\rtf1{\\pntext 1.}{\\*\\pn\\pnlvlbody\\pndec\\pnstart1}a\\par {\\pntext 2.}{\\*\\pn\\pnlvlbody\\pndec\\pnstart2}b}"),
];

fn editor(source: &[u8], format: Format) -> (Editor, ViewId) {
    let mut core =
        Core::new(Document::from_bytes(source.to_vec(), Encoding::Utf8, format).unwrap());
    let view = core.add_view(MockTextMeasurementProvider::new(), 10_000., 3000.);
    (core, view)
}

fn key(core: &mut Editor, view: ViewId, key: Key) -> Result<(), String> {
    let output = core
        .handle(view, CoreEvent::Input(InputEvent::Key(key)))
        .map_err(|error| format!("{key:?}: {error:?}"))?;
    let status = output.command.unwrap().status;
    if !matches!(
        status,
        CommandStatus::Complete | CommandStatus::Pending | CommandStatus::Cancelled
    ) {
        return Err(format!("{key:?}: {status:?}"));
    }
    Ok(())
}

fn delete_lines(
    core: &mut Editor,
    view: ViewId,
    start: usize,
    extra_lines: usize,
    reverse: bool,
    native: Option<Mode>,
    deletion: Key,
) -> Result<(), String> {
    core.handle(
        view,
        CoreEvent::PlaceCursor {
            document_revision: core.document().revision(),
            text_offset: start,
            affinity: BoundaryAffinity::Downstream,
            extend_selection: false,
        },
    )
    .map_err(|error| format!("place: {error:?}"))?;
    key(core, view, Key::Char('V'))?;
    for _ in 0..extra_lines {
        key(core, view, Key::Char(if reverse { 'k' } else { 'j' }))?;
    }
    if let Some(return_mode) = native {
        core.set_selection_origin(view, SelectionOrigin::Mouse, return_mode)
            .map_err(|error| format!("origin: {error:?}"))?;
    }
    key(core, view, deletion)
}

#[test]
fn all_contiguous_line_selections_delete_like_plain_text_and_round_trip() {
    let mut failures = Vec::new();
    let mut checked = 0;
    for &(format, source) in FIXTURES {
        let document =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap();
        let text = document.text().to_owned();
        let starts = std::iter::once(0)
            .chain(text.match_indices('\n').map(|(at, _)| at + 1))
            .collect::<Vec<_>>();
        for (line, &start) in starts.iter().enumerate() {
            for extra_lines in 0..starts.len() - line {
                for reverse in [false, true] {
                    let start = if reverse {
                        starts[line + extra_lines]
                    } else {
                        start
                    };
                    for (native, deletion) in [
                        (None, Key::Char('d')),
                        (None, Key::Delete),
                        (None, Key::Backspace),
                        (Some(Mode::Normal), Key::Delete),
                        (Some(Mode::Normal), Key::Backspace),
                        (Some(Mode::Insert), Key::Delete),
                        (Some(Mode::Insert), Key::Backspace),
                    ] {
                        checked += 1;
                        let case = format!("{format:?} {source:?}, lines {line}..={}, reverse={reverse}, native={native:?}, {deletion:?}", line + extra_lines);
                        let (mut plain, plain_view) = editor(text.as_bytes(), Format::PlainText);
                        delete_lines(
                            &mut plain,
                            plain_view,
                            start,
                            extra_lines,
                            reverse,
                            native,
                            deletion,
                        )
                        .unwrap();
                        let (mut core, view) = editor(source.as_bytes(), format);
                        if let Err(error) = delete_lines(
                            &mut core,
                            view,
                            start,
                            extra_lines,
                            reverse,
                            native,
                            deletion,
                        ) {
                            failures.push(format!("{case}: {error}"));
                            continue;
                        }
                        let after = core.document().source_bytes();
                        let visible = core.document().text().replace('\u{a0}', " ");
                        let expected = plain.document().text().replace('\u{a0}', " ");
                        if visible != expected {
                            failures.push(format!(
                                "{case}: expected {expected:?}, got {visible:?}; source={:?}",
                                String::from_utf8_lossy(&after)
                            ));
                        }
                        assert_eq!(
                            core.command_state(view).unwrap().mode(),
                            plain.command_state(plain_view).unwrap().mode(),
                            "{case}"
                        );
                        assert_eq!(
                            core.command_state(view)
                                .unwrap()
                                .register('"')
                                .map(|r| (&r.text, r.kind)),
                            plain
                                .command_state(plain_view)
                                .unwrap()
                                .register('"')
                                .map(|r| (&r.text, r.kind)),
                            "{case}"
                        );
                        let reopened =
                            Document::from_bytes(after.clone(), Encoding::Utf8, format).unwrap();
                        assert_eq!(reopened.text(), core.document().text(), "{case}");
                        if after != source.as_bytes() {
                            key(&mut core, view, Key::Escape).unwrap();
                            key(&mut core, view, Key::Char('u')).unwrap();
                            assert_eq!(core.document().source_bytes(), source.as_bytes(), "{case}");
                            key(&mut core, view, Key::Ctrl('r')).unwrap();
                            assert_eq!(core.document().source_bytes(), after, "{case}");
                        }
                    }
                }
            }
        }
    }
    eprintln!(
        "checked {checked} structural command deletions; {} failures",
        failures.len()
    );
    assert!(
        failures.is_empty(),
        "{} failures:\n{}",
        failures.len(),
        failures
            .iter()
            .take(180)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}
