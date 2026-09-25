use viem_core::document::{Document, Encoding, Format, ModelRequest, TextEdit};

#[test]
fn legal_edits_in_empty_and_adjacent_html_structures() {
    let mut failures = Vec::new();
    let mut checked = 0;
    for source in [
        "",
        "<p></p>",
        "<div></div>",
        "<div></div><p></p>",
        "<p></p><div></div>",
        "<p></p><p></p>",
        "<p>a</p><p></p>",
        "<p></p><p>a</p>",
        "<div><p></p></div><p>a</p>",
        "<div>a</div><p></p>",
        "<p></p><div>a</div>",
        "<div>a<div></div></div>",
        "<div>a<p></p></div>",
        "<div></div><p>a</p>",
        "<div><p></p><p></p></div>",
        "<p><b></b></p><p></p>",
        "<p>a<br></p><p></p>",
        "<ul><li></li></ul><p></p>",
        "<p></p><ul><li></li></ul>",
        "<ul><li></li><li></li></ul>",
        "<ul><li>a</li><li></li></ul>",
        "<ul><li></li><li>a</li></ul>",
        "<ul><li><p></p></li><li>a</li></ul>",
        "<ul><li><ul><li></li></ul></li></ul>",
        "<ul><li>a<ul><li></li></ul></li></ul>",
        "<ul><li><p></p><ul><li>a</li></ul></li></ul>",
        "<blockquote></blockquote><p></p>",
        "<p></p><blockquote></blockquote>",
        "<blockquote><p></p></blockquote><p>a</p>",
        "<pre></pre><p></p>",
        "<p></p><pre></pre>",
        "<pre>\n</pre><p>a</p>",
        "<pre>\n\n</pre><p>a</p>",
        "<pre>\n\nx</pre>",
        "<pre>&#10;&#10;</pre><p>a</p>",
        "<div><!--keep--></div><p></p>",
        "<div><span></span></div><p>a</p>",
        "<p></p><!--keep--><p></p>",
        "<p><br></p><p></p>",
        "<p>a</p><div></div><p>b</p>",
        "<div><p>a</p></div><div><p></p></div>",
        "<p>a</p><ul><li><p></p><p></p></li></ul>",
    ] {
        let mut document =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Html).unwrap();
        let original_text = document.text().to_owned();
        let boundaries = (0..=original_text.len())
            .filter(|&at| document.text_point(at).is_ok())
            .collect::<Vec<_>>();
        for (index, &start) in boundaries.iter().enumerate() {
            for &end in &boundaries[index..] {
                for replacement in ["", "x", " ", "\n", "x\ny"] {
                    checked += 1;
                    let mut expected = original_text.clone();
                    expected.replace_range(start..end, replacement);
                    match document.prepare_model_request(ModelRequest::ApplyTextEdits {
                        document: document.id(),
                        revision: document.revision(),
                        edits: vec![TextEdit::new(start..end, replacement)],
                    }) {
                        Err(error) => failures.push(format!("source={source:?} text={original_text:?} {start}..{end} => {replacement:?}: {error:?}")),
                        Ok(prepared) => {
                            let no_op = prepared.is_no_op();
                            let patches = prepared.summary().source_patches().to_vec();
                            document.commit_model_transaction(prepared).unwrap();
                            let actual = document.text().replace('\u{a0}', " ");
                            if actual != expected {
                                failures.push(format!("source={source:?} {start}..{end} => {replacement:?}: expected {expected:?}, actual {actual:?}"));
                            }
                            let after = document.source_bytes();
                            let (mut before_at, mut after_at) = (0, 0);
                            for patch in patches {
                                let untouched = patch.range().start - before_at;
                                assert_eq!(
                                    &source.as_bytes()[before_at..before_at + untouched],
                                    &after[after_at..after_at + untouched],
                                );
                                before_at = patch.range().end;
                                after_at += untouched + patch.replacement().len();
                            }
                            assert_eq!(&source.as_bytes()[before_at..], &after[after_at..]);
                            let reopened =
                                Document::from_bytes(after.clone(), Encoding::Utf8, Format::Html)
                                    .unwrap();
                            assert_eq!(reopened.text(), document.text(), "reopen {source:?}");
                            if !no_op {
                                assert!(document.undo());
                                assert_eq!(document.source_bytes(), source.as_bytes());
                                assert!(document.redo());
                                assert_eq!(document.source_bytes(), after);
                                assert!(document.undo());
                            }
                        }
                    }
                }
            }
        }
    }
    eprintln!("checked {checked} empty/adjacent HTML edits");
    assert!(
        failures.is_empty(),
        "{} failure(s):\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn typing_in_empty_pre_retains_ignored_initial_line_ending() {
    use viem_core::command::{InputEvent, Key};
    use viem_core::document::BoundaryAffinity;
    use viem_core::layout::MockTextMeasurementProvider;
    use viem_core::{Core, CoreEvent};

    for initial in ["\n", "\r\n", "\r", "&#10;", "&#xA;"] {
        for encoding in [Encoding::Utf8, Encoding::Utf16Le, Encoding::Utf16Be] {
            for css in ["", " style='white-space:normal'"] {
                for typed in ["x", " ", "\n", "x\ny"] {
                    let source = format!("<pre{css}>{initial}</pre><p>after</p>");
                    let original = match encoding {
                        Encoding::Utf16Le => source.encode_utf16().flat_map(u16::to_le_bytes).collect(),
                        Encoding::Utf16Be => source.encode_utf16().flat_map(u16::to_be_bytes).collect(),
                        _ => source.as_bytes().to_vec(),
                    };
                    let document =
                        Document::from_bytes(original.clone(), encoding, Format::Html).unwrap();
                    assert_eq!(document.text(), "\nafter", "{source:?}");
                    let mut core = Core::new(document);
                    let view = core.add_view(MockTextMeasurementProvider::new(), 400., 200.);
                    core.handle(view, CoreEvent::Input(InputEvent::key('i')))
                        .unwrap();
                    core.handle(
                        view,
                        CoreEvent::PlaceCursor {
                            document_revision: core.document().revision(),
                            text_offset: 0,
                            affinity: BoundaryAffinity::Downstream,
                            extend_selection: false,
                        },
                    )
                    .unwrap();
                    core.handle(view, CoreEvent::Input(InputEvent::text(typed)))
                        .unwrap_or_else(|error| panic!("{source:?} {encoding:?} => {typed:?}: {error:?}"));
                    assert_eq!(
                        core.document().text().replace('\u{a0}', " "),
                        format!("{typed}\nafter")
                    );
                    let after = core.document().source_bytes();
                    let after_source = match encoding {
                        Encoding::Utf16Le | Encoding::Utf16Be => String::from_utf16(
                            &after
                                .chunks_exact(2)
                                .map(|unit| {
                                    let bytes = [unit[0], unit[1]];
                                    if encoding == Encoding::Utf16Le {
                                        u16::from_le_bytes(bytes)
                                    } else {
                                        u16::from_be_bytes(bytes)
                                    }
                                })
                                .collect::<Vec<_>>(),
                        )
                        .unwrap(),
                        _ => String::from_utf8(after.clone()).unwrap(),
                    };
                    assert!(
                        after_source.starts_with(&format!("<pre{css}>{initial}")),
                        "{after_source}"
                    );
                    let reopened =
                        Document::from_bytes(after.clone(), encoding, Format::Html).unwrap();
                    assert_eq!(reopened.text(), core.document().text());
                    assert_eq!(
                        reopened.projection().style_spans(),
                        core.document().projection().style_spans()
                    );
                    core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Escape)))
                        .unwrap();
                    core.handle(view, CoreEvent::Input(InputEvent::key('u')))
                        .unwrap();
                    assert_eq!(core.document().source_bytes(), original);
                    core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Ctrl('r'))))
                        .unwrap();
                    assert_eq!(core.document().source_bytes(), after);
                }
            }
        }
    }
}
