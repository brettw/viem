//! Shared diagnostics and semantic-boundary helpers for Markdown feature contracts.
use viem_core::command::{CommandInterpreter, InputEvent, Mode};
use viem_core::document::{Document, Encoding, FileFormat, Format};

#[derive(Clone, Copy, Debug)]
pub struct EncodedFixture {
    pub encoding: Encoding,
    pub file_format: FileFormat,
    pub bom: bool,
}

impl EncodedFixture {
    pub fn bytes(self, source: &str) -> Vec<u8> {
        let ending = match self.file_format { FileFormat::Unix => "\n", FileFormat::Dos => "\r\n", FileFormat::Mac => "\r" };
        let source = format!("{}{}", if self.bom { "\u{feff}" } else { "" }, source.replace('\n', ending));
        match self.encoding {
            Encoding::Utf8 => source.into_bytes(),
            Encoding::Utf16Le => source.encode_utf16().flat_map(u16::to_le_bytes).collect(),
            Encoding::Utf16Be => source.encode_utf16().flat_map(u16::to_be_bytes).collect(),
            Encoding::Latin1 => source.chars().map(|ch| u8::try_from(ch as u32).expect("fixture must be Latin-1")).collect(),
        }
    }

    #[track_caller]
    pub fn open(self, source: &str) -> Document {
        Document::from_bytes_with_file_format(self.bytes(source), self.encoding, Format::Markdown, self.file_format)
            .unwrap_or_else(|error| panic!("{self:?}, source={source:?}: {error:?}"))
    }

    #[track_caller]
    pub fn assert_history(self, document: &mut Document, before: &[u8], after: &[u8]) {
        assert_eq!(document.source_bytes(), after, "{self:?}, text={:?}", document.text());
        let reopened = Document::from_bytes_with_file_format(after.to_vec(), self.encoding, Format::Markdown, self.file_format)
            .unwrap_or_else(|error| panic!("{self:?}: reopening {after:?}: {error:?}"));
        assert_eq!(reopened.text(), document.text(), "{self:?}: reopened text");
        assert_eq!(reopened.projection().style_spans(), document.projection().style_spans(), "{self:?}: reopened styles");
        assert!(document.undo(), "{self:?}: missing undo");
        assert_eq!(document.source_bytes(), before, "{self:?}: undo bytes");
        assert!(document.redo(), "{self:?}: missing redo");
        assert_eq!(document.source_bytes(), after, "{self:?}: redo bytes");
    }
}

#[track_caller]
pub fn place_insert_boundary(commands: &mut CommandInterpreter, document: &mut Document, at: usize) {
    commands.handle(document, InputEvent::key('i')).unwrap();
    assert_eq!(commands.mode(), Mode::Insert);
    assert!(commands.set_cursor(document, at), "invalid insertion boundary {at}, text={:?}", document.text());
    assert_eq!(commands.cursor(), at, "insertion boundary was clamped; text={:?}", document.text());
}
