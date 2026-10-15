//! Shared helpers for editing tests: drive a real `Core` with a laid-out view,
//! and check the invariants every edit must keep — the saved bytes reopen as
//! displayed, undo/redo restore exact bytes, and local edits stay regional.
//! Prefer these over per-file copies so new boundary cases are cheap to add.
#![allow(dead_code)]

use viem_core::command::clipboard::{
    ClipboardCommandContext, ClipboardContent, ClipboardGeneration, ClipboardSnapshot,
    ClipboardTarget,
};
use viem_core::command::{InputEvent, Key};
use viem_core::document::{
    measure_document_work, trace_candidate_strategies, BlockKind, BoundaryAffinity,
    CandidateEvent, CandidateOutcome, Document, DocumentWorkStatistics, Encoding, Format,
    HistoryNavigationRequest,
};
use viem_core::layout::MockTextMeasurementProvider;
use viem_core::{Core, CoreEvent, ViewId};

/// A core with one laid-out view over `source`.
pub struct Editor {
    pub core: Core<MockTextMeasurementProvider>,
    pub view: ViewId,
}

impl Editor {
    pub fn open(source: &str, format: Format) -> Self {
        let document = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format)
            .expect("fixture opens");
        let mut core = Core::new(document);
        let view = core.add_view(MockTextMeasurementProvider::new(), 800., 2000.);
        Self { core, view }
    }

    pub fn document(&self) -> &Document {
        self.core.document()
    }

    pub fn source(&self) -> String {
        String::from_utf8(self.document().source_bytes()).expect("UTF-8 fixture")
    }

    /// Place the caret at a formatted offset (downstream).
    pub fn place(&mut self, offset: usize) {
        let revision = self.document().revision();
        self.core
            .handle(self.view, CoreEvent::PlaceCursor {
                document_revision: revision,
                text_offset: offset,
                affinity: BoundaryAffinity::Downstream,
                extend_selection: false,
            })
            .expect("caret placement");
    }

    /// Place the caret at the first occurrence of `needle` in the formatted text.
    pub fn place_at(&mut self, needle: &str) {
        let offset = self.document().text().find(needle).unwrap_or_else(|| panic!("{needle:?} in text"));
        self.place(offset);
    }

    pub fn key(&mut self, key: Key) {
        self.core.handle(self.view, CoreEvent::Input(InputEvent::Key(key))).expect("key input");
    }

    /// Type text one key at a time; `\n` is Enter.
    pub fn type_keys(&mut self, text: &str) {
        for character in text.chars() {
            let input = if character == '\n' { InputEvent::Key(Key::Enter) } else { InputEvent::key(character) };
            self.core.handle(self.view, CoreEvent::Input(input)).expect("typed input");
        }
    }

    /// Paste external plain text through the native clipboard path. Markdown
    /// Preview interprets it as Markdown.
    pub fn paste_plain(&mut self, text: &str) {
        self.paste_with(Key::PasteClipboard, text);
    }

    /// Native Paste and Match Style: external text stays literal.
    pub fn paste_matching_style(&mut self, text: &str) {
        self.paste_with(Key::PasteClipboardMatchingStyle, text);
    }

    fn paste_with(&mut self, key: Key, text: &str) {
        let clipboard = ClipboardCommandContext::new().with_read(ClipboardSnapshot::new(
            ClipboardTarget::Clipboard,
            ClipboardGeneration(1),
            ClipboardContent::from_plain_text(text),
        ));
        self.core
            .handle(self.view, CoreEvent::InputWithClipboard { input: InputEvent::Key(key), clipboard })
            .unwrap_or_else(|error| panic!("paste {text:?}: {error:?}"));
    }

    pub fn set_autoformat(&mut self, enabled: bool) {
        self.core.handle(self.view, CoreEvent::SetMarkdownAutodetect(enabled)).expect("autoformat option");
    }

    pub fn undo(&mut self) {
        self.core.handle(self.view, CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo)).expect("undo");
    }

    pub fn redo(&mut self) {
        self.core.handle(self.view, CoreEvent::NavigateHistory(HistoryNavigationRequest::Redo)).expect("redo");
    }
}

/// The formatted text and every block's kind, style, quote depth and list
/// membership: what a reader sees and what structural commands act on.
pub fn shape(document: &Document) -> (String, Vec<String>) {
    let blocks = document
        .projection()
        .blocks()
        .iter()
        .map(|block| {
            let list = match &block.kind {
                BlockKind::ListItem { ordered, level, container_start, item_start, .. } => format!(
                    " list(ordered={ordered} level={level} new_list={container_start} item_start={item_start} loose={})",
                    block.list_loose
                ),
                _ => String::new(),
            };
            format!("{:?} {:?} {:?} q{}{list}", block.range, block.kind, block.style.0, block.quote_depth)
        })
        .collect();
    (document.text().to_owned(), blocks)
}

/// Saving and reopening must reproduce exactly what the editor displays.
pub fn assert_reopens_as_displayed(document: &Document, context: &str) {
    let saved = document.source_bytes();
    let fresh = Document::from_bytes(saved.clone(), document.encoding(), document.format())
        .expect("saved bytes reopen");
    assert_eq!(shape(document), shape(&fresh), "{context}: saved {:?}", String::from_utf8_lossy(&saved));
}

/// Undo restores `before` exactly and redo restores `after` exactly.
pub fn assert_exact_undo_redo(editor: &mut Editor, before: &str, after: &str, context: &str) {
    editor.key(Key::Escape);
    editor.undo();
    assert_eq!(editor.source(), before, "{context}: undo");
    editor.redo();
    assert_eq!(editor.source(), after, "{context}: redo");
}

/// Run `operation` and require that it neither reprojected nor flattened the
/// whole document. On failure, the message lists every candidate strategy's
/// outcome, including decline reasons, to show why the edit was not regional.
pub fn assert_regional<T>(context: &str, operation: impl FnOnce() -> T) -> (T, DocumentWorkStatistics) {
    let ((result, events), work) = measure_document_work(|| trace_candidate_strategies(operation));
    let explain = || explain_candidates(&events);
    assert_eq!(work.full_projection_candidates, 0, "{context}: {}\n{work:?}", explain());
    assert_eq!(work.formatted_full_materialized_bytes, 0, "{context}: {}\n{work:?}", explain());
    assert_eq!(work.compatibility_record_materializations, 0, "{context}: {}\n{work:?}", explain());
    (result, work)
}

/// One line per strategy attempt, for assertion messages.
pub fn explain_candidates(events: &[CandidateEvent]) -> String {
    events
        .iter()
        .filter(|event| event.outcome != CandidateOutcome::Declined("not applicable"))
        .map(|event| format!("{} {:?}", event.strategy.name(), event.outcome))
        .collect::<Vec<_>>()
        .join("; ")
}
