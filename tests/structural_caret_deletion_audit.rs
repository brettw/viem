//! Native selected-text and caret deletion across structural source owners.
//! These enter through Core, then reopen independently and restore exact bytes.
use viem_core::command::visual_block::resolve_block_selection;
use viem_core::command::{CommandStatus, InputEvent, Key, Mode, SelectionOrigin};
use viem_core::document::{BoundaryAffinity, Document, Encoding, Format, ModelRequest, TextEdit};
use viem_core::layout::{DocumentLayoutStyles, MockTextMeasurementProvider};
use viem_core::{Core, CoreEvent, ViewId};

type Editor = Core<MockTextMeasurementProvider>;

const CASES: &[(Format, &str)] = &[
    (Format::Html, "<div>a</div><div>b</div>"),
    (
        Format::Html,
        "<blockquote><p>a</p><p>b</p></blockquote><p>c</p>",
    ),
    (Format::Html, "<ul><li>a<p>b</p></li><li>c</li></ul>"),
    (
        Format::Html,
        "<ol start='4'><li>a</li><li><p>b</p></li></ol>",
    ),
    (
        Format::Html,
        "<ul><li>a<ul><li>b</li><li>c</li></ul></li><li>d</li></ul>",
    ),
    (Format::Html, "<ul><li></li><li>b</li></ul>"),
    (Format::Html, "<ul><li>a</li><li></li></ul>"),
    (
        Format::Html,
        "<ol start='4'><li>a<ul><li>b</li></ul></li></ol>",
    ),
    (Format::Html, "<pre>a\nb\nc</pre>"),
    (Format::Html, "<pre><code>a\nb</code></pre><p>c</p>"),
    (Format::Html, "<p>a<br></p><p><br>b</p>"),
    (Format::Html, "<p>a</p><p></p><p>b</p>"),
    (
        Format::Html,
        "<p>a</p><table><tr><td>b</td></tr></table><p>c</p>",
    ),
    (Format::Html, "<p>a<img src='x'>b</p>"),
    (Format::Html, "<p>a<svg><text>x</text></svg>b</p>"),
    (
        Format::Html,
        "<p><b>a</b></p><blockquote>b</blockquote><p>c</p>",
    ),
    (
        Format::Html,
        "<dl><dt>a</dt><dd>b</dd><dt>c</dt><dd>d</dd></dl>",
    ),
    (Format::Html, "<ul><li>a<br>b<br>c</li><li>d</li></ul>"),
    (Format::Html, "<p>a &fjlig; b</p><p>c</p>"),
    (Format::Html, "<ul><li>a </li><li>b</li></ul>"),
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
    (Format::Rtf, r"{\rtf1 a\par b\par c}"),
    (Format::Rtf, r"{\rtf1 {\b a\par b}\par c}"),
    (Format::Rtf, r"{\rtf1 a\line b\line c\par d}"),
    (Format::Rtf, r"{\rtf1 a{\pict\pngblip 00}b\par c}"),
    (Format::Rtf, r"{\rtf1 a\par \par b}"),
    (
        Format::Rtf,
        r"{\rtf1 {\*\unknown keep}a\par b{\*\unknown keep}}",
    ),
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

fn visible(format: Format, text: &str) -> String {
    if format == Format::Html {
        text.replace('\u{a0}', " ")
    } else {
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
                            if visible(format, core.document().text()) != visible(format, &expected)
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
    assert!(checked > 2_000);
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
    assert!(checked > 350);
    assert!(
        failures.is_empty(),
        "{} of {checked} deletions failed:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn clearing_selected_atomic_objects_removes_their_opaque_bodies() {
    let prefix = "<!doctype html><html><head><meta charset='utf-8'><script>keep()</script></head><body><!--outside--><p>a</p>";
    let suffix = "<p>b</p></body></html>";
    for object in [
        "<p><iframe src='keep'>x</iframe>b</p>",
        "<textarea>fallback &lt;b&gt;</textarea>",
        "<iframe src='keep'><!--object comment--><script>object script</script></iframe>",
        "<object><object><meta name='object metadata'><!--object comment--></object></object>",
        "<svg><title>object title</title><!--object comment--><text>x</text></svg>",
        "<math><annotation><!--object comment-->x</annotation></math>",
        "<video><!--object comment-->fallback</video>",
        "<audio><!--object comment-->fallback</audio>",
        "<canvas><!--object comment-->fallback</canvas>",
        "<table><tr><td><!--object comment-->x</td></tr></table>",
        "<xmp><!--literal text-->&lt;x&gt;</xmp>",
        "<svg/>tail",
        "<math/>tail",
        "<img src='keep'>tail",
    ] {
        let source = format!("{prefix}{object}{suffix}");
        for line_selection in [false, true] {
            let (mut core, view) = editor(Format::Html, &source);
            if line_selection {
                key(&mut core, view, Key::Char('V')).unwrap();
                key(&mut core, view, Key::Char('G')).unwrap();
                core.set_selection_origin(view, SelectionOrigin::Mouse, Mode::Insert)
                    .unwrap();
            } else {
                key(&mut core, view, Key::SelectAll).unwrap();
            }
            key(&mut core, view, Key::Delete)
                .unwrap_or_else(|error| panic!("{object}, line={line_selection}: {error}"));
            assert_eq!(core.document().text(), "", "{object}");
            let saved = String::from_utf8(core.document().source_bytes()).unwrap();
            assert_eq!(saved, "<!doctype html><html><head><meta charset='utf-8'><script>keep()</script></head><body><!--outside--></body></html>", "{object}");
            verify_reopen_and_history(&mut core, view, Format::Html, &source).unwrap();
        }
    }
}

#[test]
fn clear_document_keeps_complete_nested_hidden_templates() {
    let hidden = "<template data-keep='outer'>before<template data-keep='inner'><p>hidden</p></template>after</template>";
    let source = format!("<p>a</p>{hidden}<p>b</p><!--keep-->");
    let (mut core, view) = editor(Format::Html, &source);
    assert_eq!(core.document().text(), "a\nb");
    key(&mut core, view, Key::SelectAll).unwrap();
    key(&mut core, view, Key::Delete).unwrap();
    assert_eq!(core.document().text(), "");
    assert_eq!(
        core.document().source_bytes(),
        format!("{hidden}<!--keep-->").as_bytes()
    );
    verify_reopen_and_history(&mut core, view, Format::Html, &source).unwrap();
}

#[test]
fn disjoint_visual_block_deletions_keep_empty_rows_and_exact_history() {
    let mut checked = 0;
    for (format, source) in [
        (Format::Html, "<div>a</div><div>b</div>"),
        (Format::Html, "<p>a</p><p></p><p>b</p>"),
        (Format::Html, "<ul><li>aa<p>bb</p></li><li>cc</li></ul>"),
        (
            Format::Html,
            "<blockquote><p>aa</p><p>bb</p></blockquote><p>cc</p>",
        ),
        (Format::Html, "<pre>aa\nbb\ncc</pre>"),
        (
            Format::Html,
            "<p>aa</p><table><tr><td>x</td></tr></table><p>bb</p>",
        ),
        (Format::Markdown, "- aa\n\n  bb\n- cc"),
        (Format::Markdown, "> aa\n>\n> bb\n\ncc"),
        (Format::Markdown, "```\naa\nbb\ncc\n```"),
        (Format::Rtf, r"{\rtf1 aa\par bb\par cc}"),
        (Format::Rtf, r"{\rtf1 aa\line bb\line cc}"),
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
                    core.document().text(),
                )
                .unwrap();
                let mut expected = core.document().text().to_owned();
                for segment in selected.range_set.segments.iter().rev() {
                    expected.replace_range(segment.range.clone(), "");
                }
                key(&mut core, view, Key::Delete)
                    .unwrap_or_else(|error| panic!("{source} down={down}, right={right}: {error}"));
                assert_eq!(
                    visible(format, core.document().text()),
                    visible(format, &expected),
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

#[test]
fn implicit_paragraph_repairs_keep_owner_declarations_and_untouched_bytes() {
    for (opening, body, closing) in [
        (
            "<div data-keep='owner' style='font-size:20pt;color:#123456;text-align:right'>",
            "a",
            "</div><div>b</div>",
        ),
        (
            "<dl><dt data-keep='owner' style='font-size:20pt;color:#123456;text-align:right'>",
            "a",
            "</dt><dd>b</dd></dl>",
        ),
        (
            "<ul><li data-keep='owner' style='font-size:20pt;color:#123456;text-align:right'>",
            "a",
            "<p>b</p></li><li>c</li></ul>",
        ),
    ] {
        let source = format!("<!--before-->{opening}{body}{closing}<!--after-->");
        let mut document =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Html).unwrap();
        let owner = document.projection().blocks()[0].clone();
        let character =
            DocumentLayoutStyles::semantic_character_at(document.projection(), 0, false).unwrap();
        let body_at = "<!--before-->".len() + opening.len();
        let prepared = document
            .prepare_model_request(ModelRequest::ApplyTextEdits {
                document: document.id(),
                revision: document.revision(),
                edits: vec![TextEdit::new(0..1, "")],
            })
            .unwrap();
        assert!(
            prepared.summary().source_patches().iter().all(|patch| {
                patch.range().start >= body_at && patch.range().end <= body_at + body.len()
            }),
            "{source}: {:?}",
            prepared.summary().source_patches()
        );
        document.commit_model_transaction(prepared).unwrap();
        let saved = String::from_utf8(document.source_bytes()).unwrap();
        assert!(
            saved.starts_with(&format!("<!--before-->{opening}")),
            "{saved}"
        );
        assert!(
            saved.ends_with(&format!("{closing}<!--after-->")),
            "{saved}"
        );
        let reopened =
            Document::from_bytes(saved.as_bytes().to_vec(), Encoding::Utf8, Format::Html).unwrap();
        assert_eq!(reopened.text(), document.text());
        for projection in [document.projection(), reopened.projection()] {
            let retained = &projection.blocks()[0];
            assert_eq!(retained.style, owner.style);
            assert_eq!(retained.direct_paragraph, owner.direct_paragraph);
            // Introducing the required explicit paragraph makes inherited
            // container traits paragraph defaults instead of an implicit
            // body's character spans. The effective caret style is unchanged.
            assert_eq!(
                DocumentLayoutStyles::semantic_character_at(projection, 0, false).unwrap(),
                character,
                "{source}"
            );
        }
        assert!(document.undo());
        assert_eq!(document.source_bytes(), source.as_bytes());
    }
}

#[test]
fn structural_deletion_keeps_shared_container_bytes_and_surviving_style() {
    for (prefix, selected, remaining) in [
        (
            "<blockquote data-keep='outer' style='font-size:20pt;color:#123456'>",
            "<p>a</p>",
            "<!--keep--><p>b</p></blockquote>",
        ),
        (
            "<blockquote data-keep='outer'><blockquote data-keep='inner' style='color:#123456'>",
            "<p>a</p>",
            "<!--keep--><p>b</p></blockquote><p>c</p></blockquote>",
        ),
        (
            "<pre data-keep='pre' style='font-size:20pt;color:#123456'>",
            "a\n",
            "b</pre>",
        ),
        (
            "<blockquote data-keep='outer'><pre data-keep='pre' style='color:#123456'>",
            "a\n",
            "b</pre><p>c</p></blockquote>",
        ),
        (
            "<ul data-keep='list'><li data-keep='item' style='font-size:20pt;color:#123456'>",
            "<p>a</p>",
            "<!--keep--><p>b</p></li><li>c</li></ul>",
        ),
    ] {
        let source = format!("{prefix}{selected}{remaining}");
        let mut document =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Html).unwrap();
        let survivor =
            DocumentLayoutStyles::semantic_character_at(document.projection(), 2, false).unwrap();
        let expected_text = document.text()[2..].to_owned();
        document.delete_lines(0..2).unwrap();
        assert_eq!(document.text(), expected_text, "{source}");
        assert_eq!(
            document.source_bytes(),
            format!("{prefix}{remaining}").as_bytes(),
            "{source}"
        );
        let reopened =
            Document::from_bytes(document.source_bytes(), Encoding::Utf8, Format::Html).unwrap();
        assert_eq!(reopened.text(), document.text());
        for projection in [document.projection(), reopened.projection()] {
            assert_eq!(
                DocumentLayoutStyles::semantic_character_at(projection, 0, false).unwrap(),
                survivor,
                "{source}"
            );
        }
        assert!(document.undo());
        assert_eq!(document.source_bytes(), source.as_bytes());
    }
}
