use viem_core::command::{CommandStatus, InputEvent, Key};
use viem_core::document::{BoundaryAffinity, Encoding, Format};
use viem_core::layout::MockTextMeasurementProvider;
use viem_core::{Core, CoreEvent, Document, ViewId};

fn keys(core: &mut Core<MockTextMeasurementProvider>, view: ViewId, input: &str) {
    for key in input.chars() {
        let output = core
            .handle(view, CoreEvent::Input(InputEvent::Key(Key::Char(key))))
            .unwrap();
        assert!(
            matches!(
                output.command.unwrap().status,
                CommandStatus::Complete | CommandStatus::Pending
            ),
            "{key:?}"
        );
    }
}

fn fixtures() -> [(Format, &'static str); 4] {
    [
        (Format::Markdown, "- First words continue onto several wrapped rows with more prose.\n- Second item."),
        (Format::Markdown, "9) First words continue onto several wrapped rows with more prose.\n1) Second item."),
        (Format::Html, "<ul><li>First words continue onto several wrapped rows with more prose.</li><li>Second item.</li></ul>"),
        (Format::Html, "<ol start='9'><li>First words continue onto several wrapped rows with more prose.</li><li>Second item.</li></ol>"),
    ]
}

#[test]
fn start_end_and_counted_edits_address_list_body_only() {
    for (format, source) in fixtures() {
        let doc = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap();
        assert_eq!(
            doc.text(),
            "First words continue onto several wrapped rows with more prose.\nSecond item."
        );
        let mut core = Core::new(doc);
        let view = core.add_view(MockTextMeasurementProvider::new(), 900., 600.);
        keys(&mut core, view, "0");
        assert_eq!(core.command_state(view).unwrap().cursor(), 0);
        keys(&mut core, view, "2x");
        assert!(
            core.document().text().starts_with("rst words"),
            "{format:?}"
        );
        keys(&mut core, view, "u$");
        let expected = core.document().text().find('\n').unwrap();
        assert_eq!(
            core.command_state(view)
                .unwrap()
                .visual_position()
                .unwrap()
                .text_offset,
            expected
        );
        assert_eq!(core.command_state(view).unwrap().cursor(), expected - 1);
        keys(&mut core, view, "x");
        assert!(core
            .document()
            .text()
            .starts_with("First words continue onto several wrapped rows with more prose\n"));
        keys(&mut core, view, "u");
        assert_eq!(core.document().source_bytes(), source.as_bytes());
    }
}

#[test]
fn wrapped_list_end_targets_final_letter_and_append_stays_before_wrap_space() {
    for (format, source) in fixtures() {
        let doc = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap();
        let mut core = Core::new(doc);
        let view = core.add_view(MockTextMeasurementProvider::new(), 165., 800.);
        let before = core.document().text().to_owned();
        let row = &core.layout(view).unwrap().snapshot().unwrap().rows[0];
        assert!(row.wraps_to_next);
        let content_end = row.text_range.start
            + before[row.text_range.clone()]
                .trim_end_matches([' ', '\t'])
                .len();
        keys(&mut core, view, "$$");
        let state = core.command_state(view).unwrap();
        assert_eq!(
            state.visual_position().unwrap().text_offset,
            content_end,
            "{format:?}"
        );
        assert_eq!(state.boundary_affinity(), BoundaryAffinity::Upstream);
        assert!(!before[state.cursor()..]
            .chars()
            .next()
            .unwrap()
            .is_whitespace());
        keys(&mut core, view, "A");
        core.handle(view, CoreEvent::Input(InputEvent::text("XYZ")))
            .unwrap();
        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Escape)))
            .unwrap();
        let mut expected = before.clone();
        expected.insert_str(content_end, "XYZ");
        assert_eq!(core.document().text(), expected, "{format:?}");
        keys(&mut core, view, "u");
        assert_eq!(core.document().source_bytes(), source.as_bytes());
    }
}

#[test]
fn wrapped_list_operators_and_registers_contain_body_text_only() {
    for (format, source) in fixtures() {
        let doc = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap();
        let mut core = Core::new(doc);
        let view = core.add_view(MockTextMeasurementProvider::new(), 165., 800.);
        let before = core.document().text().to_owned();
        let row = &core.layout(view).unwrap().snapshot().unwrap().rows[0];
        let end = row.text_range.start
            + before[row.text_range.clone()]
                .trim_end_matches([' ', '\t'])
                .len();
        keys(&mut core, view, "\"ay$");
        assert_eq!(
            core.command_state(view)
                .unwrap()
                .register('a')
                .unwrap()
                .text,
            before[..end],
            "{format:?}"
        );
        keys(&mut core, view, "0d$");
        let remaining = &before[end..];
        let expected = if format == Format::Html && remaining.starts_with(' ') {
            format!("\u{a0}{}", &remaining[1..])
        } else {
            remaining.to_owned()
        };
        assert_eq!(core.document().text(), expected, "{format:?}");
        assert!(!core
            .document()
            .projection()
            .list_structure()
            .lists
            .is_empty());
        keys(&mut core, view, "u");
        assert_eq!(core.document().source_bytes(), source.as_bytes());
        let second = &core.layout(view).unwrap().snapshot().unwrap().rows[1];
        let end = second.text_range.start
            + before[second.text_range.clone()]
                .trim_end_matches([' ', '\t'])
                .len();
        keys(&mut core, view, "02$");
        assert_eq!(
            core.command_state(view)
                .unwrap()
                .visual_position()
                .unwrap()
                .text_offset,
            end,
            "{format:?}"
        );
    }
}

#[test]
fn unicode_list_body_end_deletes_one_whole_grapheme() {
    for (format, source) in [
        (Format::Markdown, "- café 👩🏽‍💻"),
        (Format::Html, "<ol><li>café 👩🏽‍💻</li></ol>"),
    ] {
        let doc = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap();
        let mut core = Core::new(doc);
        let view = core.add_view(MockTextMeasurementProvider::new(), 900., 600.);
        keys(&mut core, view, "$x");
        assert_eq!(core.document().text(), if format == Format::Html { "café\u{a0}" } else { "café " });
        if format == Format::Html {
            assert_eq!(core.document().source_bytes(), b"<ol><li>caf\xc3\xa9&nbsp;</li></ol>");
        }
        assert_eq!(
            core.command_state(view)
                .unwrap()
                .register('"')
                .unwrap()
                .text,
            "👩🏽‍💻"
        );
        keys(&mut core, view, "u0x");
        assert_eq!(core.document().text(), "afé 👩🏽‍💻");
        keys(&mut core, view, "u");
        assert_eq!(core.document().source_bytes(), source.as_bytes());
    }
}
