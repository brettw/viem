use viem_core::document::{Document, Encoding, FileFormat, Format};

fn encode(encoding: Encoding, text: &str, with_bom: bool) -> Vec<u8> {
    match encoding {
        Encoding::Utf8 => {
            let mut bytes = if with_bom {
                vec![0xef, 0xbb, 0xbf]
            } else {
                Vec::new()
            };
            bytes.extend_from_slice(text.as_bytes());
            bytes
        }
        Encoding::Latin1 => text
            .chars()
            .map(|ch| u8::try_from(u32::from(ch)).expect("fixture is Latin-1 representable"))
            .collect(),
        Encoding::Utf16Le => {
            let mut bytes = if with_bom {
                vec![0xff, 0xfe]
            } else {
                Vec::new()
            };
            for unit in text.encode_utf16() {
                bytes.extend_from_slice(&unit.to_le_bytes());
            }
            bytes
        }
        Encoding::Utf16Be => {
            let mut bytes = if with_bom {
                vec![0xfe, 0xff]
            } else {
                Vec::new()
            };
            for unit in text.encode_utf16() {
                bytes.extend_from_slice(&unit.to_be_bytes());
            }
            bytes
        }
    }
}

fn spelling(file_format: FileFormat) -> &'static str {
    match file_format {
        FileFormat::Unix => "\n",
        FileFormat::Dos => "\r\n",
        FileFormat::Mac => "\r",
    }
}

fn has_bom(encoding: Encoding) -> bool {
    encoding != Encoding::Latin1
}

#[test]
fn inserted_breaks_use_every_forced_fileformat_in_both_adapters_and_all_encodings() {
    for encoding in [
        Encoding::Utf8,
        Encoding::Latin1,
        Encoding::Utf16Le,
        Encoding::Utf16Be,
    ] {
        for format in [Format::PlainText, Format::Markdown] {
            for file_format in [FileFormat::Unix, FileFormat::Dos, FileFormat::Mac] {
                let delimiter = spelling(file_format);
                let source_text = match format {
                    Format::PlainText => format!("one{delimiter}two"),
                    Format::Markdown => format!("# **one**{delimiter}_two_"),
                    _ => unreachable!("fixture enumerates plain text and Markdown WYSIWYG"),
                };
                let expected_text = match format {
                    Format::PlainText => format!("one{delimiter}new{delimiter}two"),
                    Format::Markdown => {
                        format!("# **one**{delimiter}{delimiter}new{delimiter}{delimiter}_two_")
                    }
                    _ => unreachable!("fixture enumerates plain text and Markdown WYSIWYG"),
                };
                let original = encode(encoding, &source_text, has_bom(encoding));
                let expected = encode(encoding, &expected_text, has_bom(encoding));
                let mut document = Document::from_bytes_with_file_format(
                    original.clone(),
                    encoding,
                    format,
                    file_format,
                )
                .unwrap();
                assert_eq!(document.source_bytes(), original);
                assert_eq!(document.text(), "one\ntwo");

                document.insert(3, "\nnew").unwrap_or_else(|error| {
                    panic!("insert failed for {encoding:?}/{format:?}/{file_format:?}: {error}")
                });
                assert_eq!(document.text(), "one\nnew\ntwo");
                assert_eq!(
                    document.source_bytes(),
                    expected,
                    "{encoding:?}/{format:?}/{file_format:?}"
                );
                assert!(document.undo());
                assert_eq!(document.source_bytes(), original);
                assert!(document.redo());
                assert_eq!(document.source_bytes(), expected);
            }
        }
    }
}

#[test]
fn fileformat_conversion_preserves_adapter_syntax_bom_and_history_in_all_encodings() {
    for encoding in [
        Encoding::Utf8,
        Encoding::Latin1,
        Encoding::Utf16Le,
        Encoding::Utf16Be,
    ] {
        for format in [Format::PlainText, Format::Markdown] {
            for target in [FileFormat::Unix, FileFormat::Mac] {
                let source_text = match format {
                    Format::PlainText => "Hé\r\ntwo\r\n",
                    Format::Markdown => "# **Hé**\r\n_two_\r\n",
                    _ => unreachable!("fixture enumerates plain text and Markdown WYSIWYG"),
                };
                let converted_text = source_text.replace("\r\n", spelling(target));
                let original = encode(encoding, source_text, has_bom(encoding));
                let converted = encode(encoding, &converted_text, has_bom(encoding));
                let mut document = Document::from_bytes_with_file_format(
                    original.clone(),
                    encoding,
                    format,
                    FileFormat::Dos,
                )
                .unwrap();
                let formatted = document.text().to_owned();

                document.set_file_format(target).unwrap_or_else(|error| {
                    panic!("conversion failed for {encoding:?}/{format:?}/{target:?}: {error}")
                });
                assert_eq!(document.file_format(), target);
                assert_eq!(document.text(), formatted);
                assert_eq!(document.source_bytes(), converted);
                assert_eq!(document.has_bom(), has_bom(encoding));
                assert!(document.undo());
                assert_eq!(document.file_format(), FileFormat::Dos);
                assert_eq!(document.source_bytes(), original);
                assert!(document.redo());
                assert_eq!(document.file_format(), target);
                assert_eq!(document.source_bytes(), converted);
            }
        }
    }
}

#[test]
fn bomless_utf16_stays_bomless_through_local_edits_and_history() {
    for encoding in [Encoding::Utf16Le, Encoding::Utf16Be] {
        for format in [Format::PlainText, Format::Markdown] {
            let source_text = match format {
                Format::PlainText => "Hé\r\nnext",
                Format::Markdown => "# **Hé**\r\nnext",
                _ => unreachable!("fixture enumerates plain text and Markdown WYSIWYG"),
            };
            let changed_text = source_text.replacen('é', "è", 1);
            let original = encode(encoding, source_text, false);
            let changed = encode(encoding, &changed_text, false);
            let mut document = Document::from_bytes_with_file_format(
                original.clone(),
                encoding,
                format,
                FileFormat::Dos,
            )
            .unwrap();
            assert!(!document.has_bom());
            assert_eq!(document.source_bytes(), original);

            let start = document.text().find('é').unwrap();
            document
                .replace(start..start + 'é'.len_utf8(), "è")
                .unwrap();
            assert!(!document.has_bom());
            assert_eq!(document.source_bytes(), changed);
            assert!(document.undo());
            assert!(!document.has_bom());
            assert_eq!(document.source_bytes(), original);
            assert!(document.redo());
            assert!(!document.has_bom());
            assert_eq!(document.source_bytes(), changed);
        }
    }
}
