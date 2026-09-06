use evim_core::document::{BlockKind, StyleId};
use evim_core::{Document, Encoding, Format};

#[test]
fn paragraph_menu_assignments_update_source_markers_and_undo() {
    for format in [Format::Markdown, Format::MarkdownSource] {
        let source = b"__First__\r\nSecond";
        let mut document = Document::from_bytes(source.to_vec(), Encoding::Utf8, format).unwrap();
        document
            .set_paragraph_style(0..document.text().len(), StyleId::from("Heading2"))
            .unwrap();
        assert_eq!(document.source_bytes(), b"## __First__\r\n## Second");
        assert!(document
            .projection()
            .blocks()
            .iter()
            .all(|block| block.kind == BlockKind::Heading(2)));
        assert_eq!(
            document.text(),
            if format == Format::Markdown {
                "First\nSecond"
            } else {
                "## __First__\n## Second"
            }
        );
        document
            .set_paragraph_style(0..0, StyleId::from("Heading1"))
            .unwrap();
        assert_eq!(document.source_bytes(), b"# __First__\r\n## Second");
        document
            .set_paragraph_style(0..document.text().len(), StyleId::from("Paragraph"))
            .unwrap();
        assert_eq!(document.source_bytes(), source);
        for _ in 0..3 {
            assert!(document.undo());
        }
        assert_eq!(document.source_bytes(), source);
        assert!(document.redo());
        assert_eq!(document.source_bytes(), b"## __First__\r\n## Second");
    }
}
