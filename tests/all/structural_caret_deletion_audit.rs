//! Native selected-text and caret deletion across structural source owners.
//! These enter through Core, then reopen independently and restore exact bytes.
use viem_core::command::visual_block::resolve_block_selection;
use viem_core::command::{CommandStatus, InputEvent, Key, Mode};
use viem_core::document::{BoundaryAffinity, Document, Encoding, Format};
use viem_core::layout::{MockTextMeasurementProvider};
use viem_core::{Core, CoreEvent, ViewId};

type Editor = Core<MockTextMeasurementProvider>;

const CASES: &[(Format, &str)] = &[
    (Format::Markdown, "> a\n>\n> b\n\nc"),
    (Format::Markdown, "- a\n\n  b\n- c"),
    (Format::Markdown, "- a\n  - b\n  - c\n- d"),
    (Format::Markdown, "- a\n- "),
    (Format::Markdown, "```\na\nb\nc\n```\n\nd"),
    (Format::Markdown, "> ```\n> a\n> b\n> ```"),
    (Format::Markdown, "a  \nb\n\nc"),
    (Format::Markdown, "a [b](x) c\n\nd"),
    (Format::Markdown, "a ![b](x) c\n\nd"),
    (Format::Markdown, "# a\n\n## b\n\nc"),

];

fn editor(format: Format, source: &str) -> (Editor, ViewId) {
    let document =
        Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap();
    let mut core = Core::new(document);
    let view = core.add_view(MockTextMeasurementProvider::new(), 300.0, 1000.0);
    (core, view)
}

fn key(core: &mut Editor, view: ViewId, key: Key) -> Result<(), String> {
    let outcome = core
        .handle(view, CoreEvent::Input(InputEvent::Key(key)))
        .map_err(|error| format!("{key:?}: {error:?}"))?;
    if let Some(command) = outcome.command {
        if !matches!(
            command.status,
            CommandStatus::Complete | CommandStatus::Pending
        ) {
            return Err(format!("{key:?}: {:?}", command.status));
        }
    }
    Ok(())
}

fn place(core: &mut Editor, view: ViewId, at: usize, extend: bool) -> Result<(), String> {
    core.handle(
        view,
        CoreEvent::PlaceCursor {
            document_revision: core.document().revision(),
            text_offset: at,
            affinity: BoundaryAffinity::Downstream,
            extend_selection: extend,
        },
    )
    .map(|_| ())
    .map_err(|error| format!("place {at}: {error:?}"))
}

fn verify_reopen_and_history(
    core: &mut Editor,
    view: ViewId,
    format: Format,
    original: &str,
) -> Result<(), String> {
    let saved = core.document().source_bytes();
    let reopened = Document::from_bytes(saved.clone(), Encoding::Utf8, format)
        .map_err(|error| format!("reopen: {error:?}"))?;
    if reopened.text() != core.document().text() {
        return Err(format!(
            "reopen changes {:?} to {:?}; saved={:?}",
            core.document().text(),
            reopened.text(),
            String::from_utf8_lossy(&saved)
        ));
    }
    let cursor = core.command_state(view).unwrap().cursor();
    core.document()
        .text_point(cursor)
        .map_err(|error| format!("cursor: {error:?}"))?;
    if saved != original.as_bytes() {
        if core.command_state(view).unwrap().mode() != Mode::Normal {
            key(core, view, Key::Escape)?;
        }
        key(core, view, Key::Char('u'))?;
        if core.document().source_bytes() != original.as_bytes() {
            return Err("undo did not restore exact source bytes".into());
        }
        key(core, view, Key::Ctrl('r'))?;
        if core.document().source_bytes() != saved {
            return Err("redo did not restore exact edited bytes".into());
        }
    }
    Ok(())
}

fn visible(text: &str) -> String {
    {
        text.to_owned()
    }
}

#[test]
fn native_character_selection_deletes_every_legal_range_and_reopens_exactly() {
    let mut failures = Vec::new();
    let mut checked = 0;
    for &(format, source) in CASES {
        let document =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap();
        let original = document.text().to_owned();
        let boundaries = (0..=original.len())
            .filter(|at| document.text_point(*at).is_ok())
            .collect::<Vec<_>>();
        for (index, &start) in boundaries.iter().enumerate() {
            for &end in &boundaries[index + 1..] {
                for reverse in [false, true] {
                    for deletion in [Key::Delete, Key::Backspace] {
                        let (mut core, view) = editor(format, source);
                        let result = (|| {
                            key(&mut core, view, Key::Char('i'))?;
                            place(&mut core, view, if reverse { end } else { start }, false)?;
                            place(&mut core, view, if reverse { start } else { end }, true)?;
                            key(&mut core, view, deletion)?;
                            let expected = format!("{}{}", &original[..start], &original[end..]);
                            if visible(core.document().text()) != visible(&expected)
                            {
                                return Err(format!(
                                    "expected {expected:?}, got {:?}",
                                    core.document().text()
                                ));
                            }
                            verify_reopen_and_history(&mut core, view, format, source)
                        })();
                        checked += 1;
                        if let Err(error) = result {
                            failures.push(format!("{format:?} {source:?} {start}..{end} reverse={reverse} {deletion:?}: {error}"));
                        }
                    }
                }
            }
        }
    }
    eprintln!("checked {checked} native character-selection deletions");
    assert!(checked >= 976);
    assert!(
        failures.is_empty(),
        "{} of {checked} deletions failed:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn insert_mode_caret_deletion_handles_structural_edges_without_edit_errors() {
    let mut failures = Vec::new();
    let mut checked = 0;
    for &(format, source) in CASES {
        let document =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap();
        let boundaries = (0..=document.text().len())
            .filter(|at| document.text_point(*at).is_ok())
            .collect::<Vec<_>>();
        for at in boundaries {
            for deletion in [Key::Delete, Key::Backspace] {
                let (mut core, view) = editor(format, source);
                let result = (|| {
                    key(&mut core, view, Key::Char('i'))?;
                    place(&mut core, view, at, false)?;
                    key(&mut core, view, deletion)?;
                    // At list/paragraph edges Backspace may change structure
                    // instead of removing a scalar. Its committed projection
                    // must still agree with reopening and have exact history.
                    verify_reopen_and_history(&mut core, view, format, source)
                })();
                checked += 1;
                if let Err(error) = result {
                    failures.push(format!(
                        "{format:?} {source:?} at={at} {deletion:?}: {error}"
                    ));
                }
            }
        }
    }
    eprintln!("checked {checked} Insert-mode caret deletions");
    assert!(checked >= 138);
    assert!(
        failures.is_empty(),
        "{} of {checked} deletions failed:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn disjoint_visual_block_deletions_keep_empty_rows_and_exact_history() {
    let mut checked = 0;
    for (format, source) in [
        (Format::Markdown, "- aa\n\n  bb\n- cc"),
        (Format::Markdown, "> aa\n>\n> bb\n\ncc"),
        (Format::Markdown, "```\naa\nbb\ncc\n```"),

    ] {
        for down in [1, 2] {
            for right in [0, 1] {
                let (mut core, view) = editor(format, source);
                key(&mut core, view, Key::Ctrl('v')).unwrap();
                for _ in 0..down {
                    key(&mut core, view, Key::Char('j')).unwrap();
                }
                for _ in 0..right {
                    key(&mut core, view, Key::Char('l')).unwrap();
                }
                let selection = core.command_state(view).unwrap().visual_block().unwrap();
                let selected = resolve_block_selection(
                    selection,
                    core.layout(view).unwrap().snapshot().unwrap(),
                    &core.document().hard_line_snapshot(),
                )
                .unwrap();
                let mut expected = core.document().text().to_owned();
                for segment in selected.range_set.segments.iter().rev() {
                    expected.replace_range(segment.range.clone(), "");
                }
                key(&mut core, view, Key::Delete)
                    .unwrap_or_else(|error| panic!("{source} down={down}, right={right}: {error}"));
                assert_eq!(
                    visible(core.document().text()),
                    visible(&expected),
                    "{source}"
                );
                verify_reopen_and_history(&mut core, view, format, source)
                    .unwrap_or_else(|error| panic!("{source}: {error}"));
                checked += 1;
            }
        }
    }
    eprintln!("checked {checked} disjoint Visual Block deletions");
}
