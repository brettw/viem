//! Consistency fuzzer for user-visible editing behavior.
//!
//! Unlike `fuzz_backend`, every core error or non-success command status is a
//! finding (classified later), and the oracle also checks that edits at a caret
//! do something plausible, that undo restores exact bytes, that a fresh parse of
//! the saved source reproduces the incremental projection, that layout stays
//! available (so the view can scroll and type), and that arrow keys never get
//! stuck before the document edge.
//!
//! ```sh
//! cargo run --release --example fuzz_consistency -- probe --out target/consistency
//! cargo run --release --example fuzz_consistency -- walk --seed 1 --cases 200 --steps 300 --out target/consistency
//! cargo run --release --example fuzz_consistency -- replay FILE.json
//! ```
#[path = "fuzz_consistency/corpus.rs"]
mod corpus;

use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::BTreeMap;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use unicode_segmentation::UnicodeSegmentation;
use viem_core::command::clipboard::{
    ClipboardCommandContext, ClipboardContent, ClipboardGeneration, ClipboardSnapshot,
    ClipboardTarget,
};
use viem_core::command::{CommandStatus, InputEvent, Key, LineMode, Mode, NavigationKey};
use viem_core::document::{
    BoundaryAffinity, HistoryNavigationRequest, ListStyle, SemanticInlineStyle, TableEditIntent,
};
use viem_core::layout::MockTextMeasurementProvider;
use viem_core::{Core, CoreEvent, Document, Encoding, Format, ViewId};

type Ed = Core<MockTextMeasurementProvider>;

// ---------------------------------------------------------------------------
// Formats and seeds

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
enum Fmt {
    Wysiwyg,
    Source,
    Code,
    Plain,
}

impl Fmt {
    fn format(self) -> Format {
        match self {
            Fmt::Wysiwyg => Format::Markdown,
            Fmt::Source => Format::MarkdownSource,
            Fmt::Code => Format::Code,
            Fmt::Plain => Format::PlainText,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Seed {
    fmt: Fmt,
    source: String,
    filename: Option<String>,
}

fn all_seeds(include_files: bool) -> Vec<Seed> {
    let mut seeds = Vec::new();
    for source in corpus::MARKDOWN {
        for fmt in [Fmt::Wysiwyg, Fmt::Source] {
            seeds.push(Seed { fmt, source: (*source).into(), filename: None });
        }
    }
    for (name, source) in corpus::CODE {
        seeds.push(Seed { fmt: Fmt::Code, source: (*source).into(), filename: Some((*name).into()) });
    }
    for source in corpus::PLAIN {
        seeds.push(Seed { fmt: Fmt::Plain, source: (*source).into(), filename: None });
    }
    if std::env::var_os("FUZZ_CRLF").is_some() {
        for seed in &mut seeds {
            if seed.fmt != Fmt::Code && !seed.source.contains('\r') {
                seed.source = seed.source.replace('\n', "\r\n");
            }
        }
    }
    if include_files {
        for path in ["docs/markdown_demo.md", "MARKDOWN_GAPS.md", "README.md", "docs/markdown-tables.md"] {
            if let Ok(source) = std::fs::read_to_string(path) {
                for fmt in [Fmt::Wysiwyg, Fmt::Source] {
                    seeds.push(Seed { fmt, source: source.clone(), filename: None });
                }
            }
        }
    }
    seeds
}

// ---------------------------------------------------------------------------
// Replayable actions

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "op", rename_all = "snake_case")]
enum Act {
    Key { key: String },
    Text { text: String },
    /// Offsets are snapshot offsets in the state preceding this action.
    Place { offset: usize, downstream: bool, extend: bool },
    Word { offset: usize, extend: bool },
    Pointer { offset: usize },
    Paste { text: String },
    Semantic { style: String, enabled: bool },
    CharStyle { style: String },
    ParagraphStyle { style: String },
    List { style: Option<String> },
    Quote { enabled: bool },
    Indent { unindent: bool },
    TableInsert { columns: usize, rows: usize },
    TableRow { after: bool },
    TableColumn { after: bool },
    TableDeleteRow,
    TableDeleteColumn,
    ToggleSource,
    Resize { width: f32, height: f32 },
    Scroll { top: f32, left: f32 },
    Wrap { enabled: bool },
    LineMode { physical: bool },
    SmartQuotes { enabled: bool },
    Autoformat { enabled: bool },
    SelectAll,
    History { undo: bool },
    Compose { text: String, commit: bool },
    /// Begin and update marked text, leaving the composition active.
    ComposeOpen { text: String },
    ComposeEnd { commit: bool },
    /// Native link popup: insert over the current selection or caret.
    LinkInsert { text: String, destination: String },
    /// Native link popup: remove the link at the caret/selection.
    LinkRemove,
    /// macOS press-and-hold: replace the grapheme before the caret through a
    /// composition (beginComposition(replacing:) + commit).
    ComposeReplacePrev { text: String },
    /// Paste the content captured from the last native Copy.
    PasteStored,
    /// Native image popup: insert an image at the caret/selection.
    ImageInsert { destination: String },
    /// Poll asynchronous syntax highlighting until it settles (max ~2s).
    WaitSyntax,
    /// Select (creating if needed) another view of the same document.
    SwitchView { index: usize },
    Scale { scale: f32 },
}

fn parse_key(name: &str) -> Key {
    match name {
        "Esc" => Key::Escape,
        "Enter" => Key::Enter,
        "ShiftEnter" => Key::ShiftEnter,
        "Tab" => Key::Tab,
        "BackTab" => Key::BackTab,
        "BS" => Key::Backspace,
        "Del" => Key::Delete,
        "Left" => Key::Left,
        "Right" => Key::Right,
        "Up" => Key::Up,
        "Down" => Key::Down,
        "Home" => Key::Home,
        "End" => Key::End,
        "WordLeft" => Key::WordLeft,
        "WordRight" => Key::WordRight,
        "ParaStart" => Key::ParagraphStart,
        "ParaEnd" => Key::ParagraphEnd,
        "DocStart" => Key::DocumentStart,
        "DocEnd" => Key::DocumentEnd,
        "PageUp" => Key::PageUp,
        "PageDown" => Key::PageDown,
        "SelectAll" => Key::SelectAll,
        "Copy" => Key::CopySelection,
        "S-Left" => shifted(NavigationKey::Left),
        "S-Right" => shifted(NavigationKey::Right),
        "S-Up" => shifted(NavigationKey::Up),
        "S-Down" => shifted(NavigationKey::Down),
        "S-Home" => shifted(NavigationKey::Home),
        "S-End" => shifted(NavigationKey::End),
        "S-WordRight" => shifted(NavigationKey::WordRight),
        "S-WordLeft" => shifted(NavigationKey::WordLeft),
        other if other.starts_with("C-") => Key::Ctrl(other[2..].chars().next().unwrap()),
        other => {
            let mut chars = other.chars();
            let c = chars.next().expect("nonempty key");
            assert!(chars.next().is_none(), "unknown key {other:?}");
            Key::Char(c)
        }
    }
}

fn shifted(key: NavigationKey) -> Key {
    Key::ModifiedNavigation { key, modifiers: 1 }
}

/// Expand a compact Vim-ish key string: `<Esc>`, `<CR>`, `<BS>`, `<Del>`,
/// `<C-r>` and plain characters.
fn keys(spec: &str) -> Vec<Act> {
    let mut out = Vec::new();
    let mut rest = spec;
    while !rest.is_empty() {
        if rest.starts_with('<') {
            if let Some(end) = rest.find('>') {
                let name = &rest[1..end];
                let key = match name {
                    "CR" => "Enter".to_string(),
                    "Esc" => "Esc".into(),
                    "BS" => "BS".into(),
                    "Del" => "Del".into(),
                    "Tab" => "Tab".into(),
                    n if n.starts_with("C-") => n.to_string(),
                    n => n.to_string(),
                };
                out.push(Act::Key { key });
                rest = &rest[end + 1..];
                continue;
            }
        }
        let c = rest.chars().next().unwrap();
        out.push(Act::Key { key: c.to_string() });
        rest = &rest[c.len_utf8()..];
    }
    out
}

// ---------------------------------------------------------------------------
// Harness

struct H {
    core: Ed,
    view: ViewId,
    views: Vec<(ViewId, f32, f32)>,
    width: f32,
    height: f32,
    seed: Seed,
    trace: Vec<Act>,
    /// Actions slower than the threshold: (description, seconds).
    slow: Vec<(String, f64)>,
    /// Last clipboard content written by the core (native Copy/Cut).
    clipboard: Option<ClipboardContent>,
}

#[derive(Debug, Clone)]
enum Res {
    Ok,
    Status(String),
    Err(String),
}

impl Res {
    fn is_ok(&self) -> bool {
        matches!(self, Res::Ok)
    }
    fn describe(&self) -> String {
        match self {
            Res::Ok => "ok".into(),
            Res::Status(s) => format!("status {s}"),
            Res::Err(e) => format!("error {e}"),
        }
    }
}

impl H {
    fn open(seed: &Seed, width: f32, height: f32) -> Result<H, String> {
        let document = Document::from_bytes(seed.source.as_bytes().to_vec(), Encoding::Utf8, seed.fmt.format())
            .map_err(|e| format!("open: {e:?}"))?;
        let mut core = Core::new(document);
        if let Some(name) = &seed.filename {
            let _ = core.initialize_code_detection(name, true);
        }
        let view = core
            .try_add_view(MockTextMeasurementProvider::new(), width, height)
            .map_err(|e| format!("add view: {e:?}"))?;
        Ok(H { core, view, views: vec![(view, width, height)], width, height, seed: seed.clone(), trace: vec![Act::Resize { width, height }], slow: Vec::new(), clipboard: None })
    }

    fn text(&self) -> &str {
        self.core.document().text()
    }
    fn cursor(&self) -> usize {
        self.core.command_state(self.view).unwrap().cursor()
    }
    fn mode(&self) -> Mode {
        self.core.command_state(self.view).unwrap().mode()
    }
    fn bytes(&self) -> Vec<u8> {
        self.core.document().source_bytes()
    }

    fn send(&mut self, event: CoreEvent) -> Res {
        let view = self.view;
        let core = &mut self.core;
        let label = format!("{event:?}").chars().take(120).collect::<String>();
        let started = std::time::Instant::now();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| core.handle_with_layout(view, event)));
        let elapsed = started.elapsed().as_secs_f64();
        if elapsed > 0.25 {
            self.slow.push((label, elapsed));
        }
        let result = match result {
            Ok(r) => r,
            Err(panic) => return Res::Err(format!("PANIC {}", panic_message(panic))),
        };
        if let Ok(outcome) = &result {
            if let Some(write) = outcome.command.as_ref().and_then(|c| c.clipboard_writes.last()) {
                self.clipboard = Some(write.content().clone());
            }
        }
        match result {
            Err(error) => Res::Err(format!("{error:?}")),
            Ok(outcome) => match outcome.command.map(|c| c.status) {
                None
                | Some(CommandStatus::Complete)
                | Some(CommandStatus::Pending)
                | Some(CommandStatus::Cancelled)
                | Some(CommandStatus::SearchNotFound) => Res::Ok,
                Some(status) => Res::Status(format!("{status:?}")),
            },
        }
    }

    fn input(&mut self, input: InputEvent) -> Res {
        self.send(CoreEvent::Input(input))
    }

    fn selection_identity(&mut self) -> Result<viem_core::LogicalSelectionIdentity, Res> {
        self.core.list_selection_identity(self.view).map_err(|e| Res::Err(format!("selection identity: {e:?}")))
    }

    fn table_at_cursor(&self) -> Option<(u64, usize, usize)> {
        let cursor = self.cursor();
        let projection = self.core.document().projection();
        let (table, row, cell) = projection.table_cell_at(cursor)?;
        let row_index = table.rows.iter().position(|r| std::ptr::eq(r, row))?;
        let column = row.cells.iter().position(|c| std::ptr::eq(c, cell))?;
        Some((table.id, row_index, column))
    }

    fn act(&mut self, act: &Act) -> Res {
        self.trace.push(act.clone());
        let doc_id = self.core.document().id();
        let revision = self.core.document().revision();
        match act {
            Act::Key { key } if key == "Copy" => {
                let clipboard = ClipboardCommandContext::new().with_write(ClipboardTarget::Clipboard);
                self.send(CoreEvent::InputWithClipboard { input: InputEvent::Key(Key::CopySelection), clipboard })
            }
            Act::Key { key } => self.input(InputEvent::Key(parse_key(key))),
            Act::Text { text } => self.input(InputEvent::Text(text.clone())),
            Act::Place { offset, downstream, extend } => self.send(CoreEvent::PlaceCursor {
                document_revision: revision,
                text_offset: *offset,
                affinity: if *downstream { BoundaryAffinity::Downstream } else { BoundaryAffinity::Upstream },
                extend_selection: *extend,
            }),
            Act::Word { offset, extend } => self.send(CoreEvent::SelectPointerWord {
                document_revision: revision,
                text_offset: *offset,
                affinity: BoundaryAffinity::Downstream,
                extend_selection: *extend,
            }),
            Act::Pointer { offset } => self.send(CoreEvent::BeginPointerGesture {
                document_revision: revision,
                text_offset: *offset,
                affinity: BoundaryAffinity::Downstream,
            }),
            Act::Paste { text } => {
                let clipboard = ClipboardCommandContext::new()
                    .with_read(ClipboardSnapshot::new(
                        ClipboardTarget::Clipboard,
                        ClipboardGeneration(1),
                        ClipboardContent::from_plain_text(text),
                    ))
                    .with_write(ClipboardTarget::Clipboard);
                self.send(CoreEvent::InputWithClipboard { input: InputEvent::Key(Key::PasteClipboard), clipboard })
            }
            Act::Semantic { style, enabled } => {
                let expected = match self.selection_identity() {
                    Ok(e) => e,
                    Err(r) => return r,
                };
                let style = match style.as_str() {
                    "strong" => SemanticInlineStyle::Strong,
                    "emphasis" => SemanticInlineStyle::Emphasis,
                    _ => SemanticInlineStyle::Code,
                };
                // The toolbar only sends what core presents as available.
                if let Ok(presentation) = self.core.selection_semantic_style_presentation(self.view, style) {
                    if (*enabled && !presentation.can_set()) || (!*enabled && !presentation.can_clear()) {
                        return Res::Ok;
                    }
                }
                self.send(CoreEvent::SetSelectionSemanticStyle { expected, style, enabled: *enabled })
            }
            Act::CharStyle { .. } if self.core.document().format() == Format::MarkdownSource
                && std::env::var_os("FUZZ_ALLOW_KNOWN_HANG").is_none() => {
                // Known hang (BUGS.md C30): skip so the run can finish.
                Res::Ok
            }
            Act::CharStyle { style } => {
                let expected = match self.selection_identity() {
                    Ok(e) => e,
                    Err(r) => return r,
                };
                let style_sheet_revision = self.core.document().projection().style_sheet().revision;
                self.send(CoreEvent::AssignNamedStyle {
                    expected,
                    style_sheet_revision,
                    namespace: viem_core::document::StyleNamespace::Character,
                    style: style.as_str().into(),
                })
            }
            Act::ParagraphStyle { style } => {
                let expected = match self.selection_identity() {
                    Ok(e) => e,
                    Err(r) => return r,
                };
                self.send(CoreEvent::SetParagraphStyle { expected, style: style.as_str().into() })
            }
            Act::List { style } => {
                let expected = match self.selection_identity() {
                    Ok(e) => e,
                    Err(r) => return r,
                };
                let style = style.as_ref().map(|s| if s == "bullet" { ListStyle::Bullet } else { ListStyle::Numbered });
                self.send(CoreEvent::SetListStyle { expected, style })
            }
            Act::Quote { enabled } => {
                let expected = match self.selection_identity() {
                    Ok(e) => e,
                    Err(r) => return r,
                };
                self.send(CoreEvent::SetBlockQuote { expected, enabled: *enabled })
            }
            Act::Indent { unindent } => {
                let expected = match self.selection_identity() {
                    Ok(e) => e,
                    Err(r) => return r,
                };
                self.send(CoreEvent::IndentList { expected, unindent: *unindent })
            }
            Act::TableInsert { columns, rows } => {
                let cursor = self.cursor();
                self.send(CoreEvent::TableEdit {
                    document: doc_id,
                    revision,
                    intent: TableEditIntent::Insert { range: cursor..cursor, columns: *columns, body_rows: *rows },
                })
            }
            Act::TableRow { after } => match self.table_at_cursor() {
                Some((table, row, _)) => self.send(CoreEvent::TableEdit {
                    document: doc_id,
                    revision,
                    intent: TableEditIntent::InsertRow { table, row, after: *after },
                }),
                None => Res::Ok,
            },
            Act::TableColumn { after } => match self.table_at_cursor() {
                Some((table, _, column)) => self.send(CoreEvent::TableEdit {
                    document: doc_id,
                    revision,
                    intent: TableEditIntent::InsertColumn { table, column, after: *after },
                }),
                None => Res::Ok,
            },
            Act::TableDeleteRow => match self.table_at_cursor() {
                Some((table, row, _)) => self.send(CoreEvent::TableEdit {
                    document: doc_id,
                    revision,
                    intent: TableEditIntent::DeleteRow { table, row },
                }),
                None => Res::Ok,
            },
            Act::TableDeleteColumn => match self.table_at_cursor() {
                Some((table, _, column)) => self.send(CoreEvent::TableEdit {
                    document: doc_id,
                    revision,
                    intent: TableEditIntent::DeleteColumn { table, column },
                }),
                None => Res::Ok,
            },
            Act::ToggleSource => {
                let format = self.core.document().format();
                if !format.is_markdown() {
                    return Res::Ok;
                }
                self.send(CoreEvent::SetMarkdownSource { document: doc_id, revision, source: !format.is_source_view() })
            }
            Act::Resize { width, height } => {
                self.width = *width;
                self.height = *height;
                self.send(CoreEvent::Resize { width: *width, height: *height })
            }
            Act::Scroll { top, left } => self.send(CoreEvent::SetViewportOrigin { left: *left, top: Some(*top) }),
            Act::Wrap { enabled } => self.send(CoreEvent::SetWrap(*enabled)),
            Act::LineMode { physical } => self.send(CoreEvent::SetLineMode(if *physical {
                LineMode::PhysicalSource
            } else {
                LineMode::Visual
            })),
            Act::SmartQuotes { enabled } => self.send(CoreEvent::SetSmartQuotes(*enabled)),
            Act::Autoformat { enabled } => self.send(CoreEvent::SetMarkdownAutodetect(*enabled)),
            Act::SelectAll => self.send(CoreEvent::SelectAll { document: doc_id, revision }),
            Act::History { undo } => self.send(CoreEvent::NavigateHistory(if *undo {
                HistoryNavigationRequest::Undo
            } else {
                HistoryNavigationRequest::Redo
            })),
            Act::Compose { text, commit } => {
                // Like macOS setMarkedText: replace the native selection, else the caret.
                let cursor = self.cursor();
                let range = match self.selection_identity() {
                    Ok(identity) if !identity.range().is_empty() && self.mode() != Mode::Normal => identity.range(),
                    _ => cursor..cursor,
                };
                let target = match viem_core::CompositionTarget::at_offsets(self.core.document(), range) {
                    Ok(t) => t,
                    Err(e) => return Res::Err(format!("composition target: {e:?}")),
                };
                let r = self.send(CoreEvent::Composition(viem_core::CompositionEvent::Begin(target)));
                if !r.is_ok() {
                    return r;
                }
                let update = viem_core::CompositionUpdate::new(text.clone(), text.len()..text.len());
                let r = self.send(CoreEvent::Composition(viem_core::CompositionEvent::Update(update)));
                if !r.is_ok() {
                    return r;
                }
                self.send(CoreEvent::Composition(if *commit {
                    viem_core::CompositionEvent::Commit
                } else {
                    viem_core::CompositionEvent::Cancel
                }))
            }
            Act::Scale { scale } => self.send(CoreEvent::SetScale(*scale)),
            Act::ComposeOpen { text } => {
                let cursor = self.cursor();
                let range = match self.selection_identity() {
                    Ok(identity) if !identity.range().is_empty() && self.mode() != Mode::Normal => identity.range(),
                    _ => cursor..cursor,
                };
                let target = match viem_core::CompositionTarget::at_offsets(self.core.document(), range) {
                    Ok(t) => t,
                    Err(e) => return Res::Err(format!("composition target: {e:?}")),
                };
                let r = self.send(CoreEvent::Composition(viem_core::CompositionEvent::Begin(target)));
                if !r.is_ok() {
                    return r;
                }
                let update = viem_core::CompositionUpdate::new(text.clone(), text.len()..text.len());
                self.send(CoreEvent::Composition(viem_core::CompositionEvent::Update(update)))
            }
            Act::LinkInsert { text, destination } => {
                let expected = match self.selection_identity() {
                    Ok(e) => e,
                    Err(r) => return r,
                };
                let range = expected.range();
                let text = if range.is_empty() { text.clone() } else { self.text()[range.clone()].to_string() };
                let (core, view) = (&mut self.core, self.view);
                let intent = viem_core::document::LinkEditIntent::Insert { range, text, destination: destination.clone() };
                match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| core.edit_link(view, expected, intent))) {
                    Err(panic) => Res::Err(format!("PANIC {}", panic_message(panic))),
                    Ok(Err(e)) => Res::Err(format!("{e:?}")),
                    Ok(Ok(_)) => Res::Ok,
                }
            }
            Act::LinkRemove => {
                let expected = match self.selection_identity() {
                    Ok(e) => e,
                    Err(r) => return r,
                };
                let range = expected.range();
                let point = match self.core.document().text_point(range.start) { Ok(p) => p, Err(_) => return Res::Ok };
                let Ok(Some(link)) = self.core.document().link_at(point) else { return Res::Ok };
                let _ = link;
                let (core, view) = (&mut self.core, self.view);
                let intent = viem_core::document::LinkEditIntent::Remove { range };
                match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| core.edit_link(view, expected, intent))) {
                    Err(panic) => Res::Err(format!("PANIC {}", panic_message(panic))),
                    Ok(Err(e)) => Res::Err(format!("{e:?}")),
                    Ok(Ok(_)) => Res::Ok,
                }
            }
            Act::ComposeReplacePrev { text } => {
                let cursor = self.cursor();
                let prev = self.text()[..cursor].graphemes(true).next_back().map_or(0, |g| g.len());
                if prev == 0 {
                    return Res::Ok;
                }
                let target = match viem_core::CompositionTarget::at_offsets(self.core.document(), cursor - prev..cursor) {
                    Ok(t) => t,
                    Err(e) => return Res::Err(format!("composition target: {e:?}")),
                };
                let r = self.send(CoreEvent::Composition(viem_core::CompositionEvent::Begin(target)));
                if !r.is_ok() {
                    return r;
                }
                let update = viem_core::CompositionUpdate::new(text.clone(), text.len()..text.len());
                let r = self.send(CoreEvent::Composition(viem_core::CompositionEvent::Update(update)));
                if !r.is_ok() {
                    return r;
                }
                self.send(CoreEvent::Composition(viem_core::CompositionEvent::Commit))
            }
            Act::PasteStored => {
                let Some(content) = self.clipboard.clone() else { return Res::Ok };
                let clipboard = ClipboardCommandContext::new()
                    .with_read(ClipboardSnapshot::new(ClipboardTarget::Clipboard, ClipboardGeneration(2), content))
                    .with_write(ClipboardTarget::Clipboard);
                self.send(CoreEvent::InputWithClipboard { input: InputEvent::Key(Key::PasteClipboard), clipboard })
            }
            Act::ImageInsert { destination } => {
                let expected = match self.selection_identity() {
                    Ok(e) => e,
                    Err(r) => return r,
                };
                let range = expected.range();
                let (core, view) = (&mut self.core, self.view);
                let intent = viem_core::document::ImageEditIntent::Insert { range, text: "alt".into(), destination: destination.clone() };
                match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| core.edit_image(view, expected, intent))) {
                    Err(panic) => Res::Err(format!("PANIC {}", panic_message(panic))),
                    Ok(Err(e)) => Res::Err(format!("{e:?}")),
                    Ok(Ok(_)) => Res::Ok,
                }
            }
            Act::WaitSyntax => {
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
                while std::time::Instant::now() < deadline {
                    self.core.poll_syntax();
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
                Res::Ok
            }
            Act::SwitchView { index } => {
                if let Some(slot) = self.views.iter().position(|(v, _, _)| *v == self.view) {
                    self.views[slot] = (self.view, self.width, self.height);
                }
                while self.views.len() <= *index {
                    match self.core.try_add_view(MockTextMeasurementProvider::new(), 450., 250.) {
                        Ok(v) => self.views.push((v, 450., 250.)),
                        Err(e) => return Res::Err(format!("add view: {e:?}")),
                    }
                }
                let (v, w, hh) = self.views[*index];
                self.view = v;
                self.width = w;
                self.height = hh;
                Res::Ok
            }
            Act::ComposeEnd { commit } => {
                if self.core.composition_overlay(self.view).ok().flatten().is_none() {
                    return Res::Ok;
                }
                self.send(CoreEvent::Composition(if *commit {
                    viem_core::CompositionEvent::Commit
                } else {
                    viem_core::CompositionEvent::Cancel
                }))
            }
        }
    }

    fn acts(&mut self, acts: &[Act]) -> Vec<Res> {
        acts.iter().map(|a| self.act(a)).collect()
    }

    /// The FFI's notion of a drawable layout: current revision/config/metrics.
    fn layout_current(&self) -> bool {
        let Some(layout) = self.core.presentation_layout(self.view) else { return false };
        let Ok(requirements) = self.core.layout_provider_requirements(self.view) else { return false };
        layout.snapshot().is_some_and(|snapshot| {
            snapshot.document_id == self.core.document().id()
                && snapshot.document_revision == self.core.document().revision()
                && snapshot.configuration_generation == layout.configuration_generation()
                && snapshot.measurement_environment_id == requirements.measurement_environment_id
                && snapshot.metrics_generation == requirements.metrics_generation
        })
    }

    /// Mirror the macOS refresh: if layout is unavailable, resize to the same
    /// size. Returns problems.
    fn refresh_layout(&mut self) -> Vec<(String, String)> {
        let mut problems = Vec::new();
        if !self.layout_current() {
            let (w, h) = (self.width, self.height);
            let (core, view) = (&mut self.core, self.view);
            match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| core.handle(view, CoreEvent::Resize { width: w, height: h }))) {
                Err(panic) => problems.push(("layout_refresh_panic".into(), panic_message(panic))),
                Ok(Err(e)) => problems.push(("layout_refresh_error".into(), format!("{e:?}"))),
                Ok(Ok(_)) => {}
            }
            if !self.layout_current() {
                problems.push(("layout_unavailable".into(), "no current layout after same-size resize".into()));
            }
        }
        problems
    }

    /// Check the other views too: cursor validity and drawable layout.
    fn check_other_views(&mut self) -> Vec<(String, String)> {
        let mut problems = Vec::new();
        let current = (self.view, self.width, self.height);
        for (view, w, hh) in self.views.clone() {
            if view == current.0 {
                continue;
            }
            self.view = view;
            self.width = w;
            self.height = hh;
            let cursor = self.cursor();
            if let Err(e) = self.core.document().text_point(cursor) {
                problems.push(("other_view_invalid_cursor".into(), format!("cursor {cursor}: {e:?}")));
            }
            for (kind, detail) in self.refresh_layout() {
                problems.push((format!("other_view_{kind}"), detail));
            }
        }
        self.view = current.0;
        self.width = current.1;
        self.height = current.2;
        problems
    }

    /// Invariants checked after every action.
    fn check(&mut self, after_input: bool, reproject: bool) -> Vec<(String, String)> {
        let mut problems = Vec::new();
        let cursor = self.cursor();
        if let Err(e) = self.core.document().text_point(cursor) {
            problems.push(("invalid_cursor".into(), format!("cursor {cursor}: {e:?}")));
        }
        problems.extend(self.refresh_layout());
        let snapshot_missing = self.core.presentation_layout(self.view).and_then(|l| l.snapshot()).is_none();
        if snapshot_missing && problems.is_empty() {
            problems.push(("layout_snapshot_missing".into(), "no layout snapshot".into()));
        }
        if after_input && problems.is_empty() && self.mode() != Mode::CommandLine {
            let layout = self.core.presentation_layout(self.view).unwrap();
            let snapshot = layout.snapshot().unwrap();
            let affinity = self.core.command_state(self.view).unwrap().boundary_affinity();
            let alternate = match affinity {
                BoundaryAffinity::Upstream => BoundaryAffinity::Downstream,
                _ => BoundaryAffinity::Upstream,
            };
            if !snapshot.coverage.contains_text_offset(cursor) {
                problems.push(("caret_not_materialized".into(), format!("cursor {cursor}")));
            } else if let Err(e) = snapshot.logical_endpoint_geometry(cursor, affinity) {
                if let Err(e2) = snapshot.logical_endpoint_geometry(cursor, alternate) {
                    problems.push(("caret_invisible".into(), format!("cursor {cursor} {affinity:?}: {e:?} / {e2:?}")));
                }
            }
        }
        // Mirror the FFI Visual-selection export that refreshPresentation
        // requires; errors other than outside-coverage break every refresh.
        if problems.is_empty() && !snapshot_missing && matches!(self.mode(), Mode::VisualCharacter | Mode::VisualLine) {
            let state = self.core.command_state(self.view).unwrap();
            if let Some(anchor) = state.visual_anchor() {
                let text = self.core.document().text();
                let (lo, hi) = (anchor.min(cursor), anchor.max(cursor));
                let hi = text[hi..].graphemes(true).next().map_or(hi, |g| hi + g.len());
                let layout = self.core.presentation_layout(self.view).unwrap();
                if let (Some(snapshot), Ok(a), Ok(b)) = (layout.snapshot(), self.core.document().text_point(lo), self.core.document().text_point(hi)) {
                    if let Ok(range) = viem_core::document::TextRange::new(a, b) {
                        match snapshot.materialized_selection_rectangles(range, state.boundary_affinity()) {
                            Ok(_) => {}
                            Err(viem_core::layout::LayoutError::OutsideMaterializedCoverage
                                | viem_core::layout::LayoutError::NoRows
                                | viem_core::layout::LayoutError::NoCaretStops
                                | viem_core::layout::LayoutError::NotACaretStop { .. }
                                | viem_core::layout::LayoutError::LongLineSliceNeedsMoreText { .. }) => {}
                            Err(e) => problems.push(("selection_export_error".into(), format!("{lo}..{hi}: {e:?}"))),
                        }
                    }
                }
            }
        }
        if reproject {
            let document = self.core.document();
            match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| reprojection_problem(document))) {
                Ok(Some(p)) => problems.push(p),
                Ok(None) => {}
                Err(panic) => problems.push(("reopen_panic".into(), panic_message(panic))),
            }
        }
        problems
    }
}

fn panic_message(panic: Box<dyn std::any::Any + Send>) -> String {
    panic
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| panic.downcast_ref::<&str>().map(|s| s.to_string()))
        .unwrap_or_default()
}

fn block_summary(document: &Document) -> Vec<String> {
    document
        .projection()
        .blocks()
        .iter()
        .map(|b| {
            format!(
                "{:?} {:?} q{} loose{} hr{} html{} lang{:?} style{}",
                b.range, b.kind, b.quote_depth, b.list_loose, b.thematic_break, b.markdown_html, b.code_language, b.style.0
            )
        })
        .collect()
}

/// Block structure without ranges: kinds, list/quote nesting and styles.
fn block_shape(document: &Document) -> Vec<String> {
    document
        .projection()
        .blocks()
        .iter()
        .map(|b| format!("{:?} q{} hr{} html{} style{}", b.kind, b.quote_depth, b.thematic_break, b.markdown_html, b.style.0))
        .collect()
}

fn reprojection_problem(document: &Document) -> Option<(String, String)> {
    let fresh = match Document::from_bytes_with_file_format(
        document.source_bytes(),
        document.encoding(),
        document.format(),
        document.file_format(),
    ) {
        Ok(f) => f,
        Err(e) => return Some(("reopen_error".into(), format!("{e:?}"))),
    };
    if fresh.text() != document.text() {
        return Some((
            "reproject_text".into(),
            format!(
                "incremental={:?} fresh={:?} source={:?}",
                document.text(),
                fresh.text(),
                String::from_utf8_lossy(&document.source_bytes())
            ),
        ));
    }
    if document.format().is_markdown() {
        let (a, b) = (block_summary(document), block_summary(&fresh));
        if a != b {
            let first = a.iter().zip(&b).position(|(x, y)| x != y).unwrap_or(a.len().min(b.len()));
            return Some((
                "reproject_blocks".into(),
                format!(
                    "incremental={:?} fresh={:?} source={:?}",
                    a.get(first),
                    b.get(first),
                    String::from_utf8_lossy(&document.source_bytes())
                ),
            ));
        }
        if document.projection().style_spans() != fresh.projection().style_spans() {
            return Some((
                "reproject_styles".into(),
                format!("text={:?} source={:?}", document.text(), String::from_utf8_lossy(&document.source_bytes())),
            ));
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Findings

#[derive(Clone, Debug, Serialize)]
struct Finding {
    kind: String,
    signature: String,
    fmt: Fmt,
    source: String,
    context: String,
    detail: String,
    trace: Vec<Act>,
    filename: Option<String>,
}

fn normalize(text: &str) -> String {
    let mut out = String::new();
    let mut digits = false;
    for c in text.chars() {
        if c.is_ascii_digit() {
            if !digits {
                out.push('#');
            }
            digits = true;
        } else {
            digits = false;
            out.push(c);
        }
    }
    out.chars().take(240).collect()
}

struct Sink {
    findings: Mutex<BTreeMap<String, (usize, Vec<Finding>)>>,
    probes: AtomicUsize,
}

impl Sink {
    fn new() -> Self {
        Self { findings: Mutex::new(BTreeMap::new()), probes: AtomicUsize::new(0) }
    }
    fn add(&self, finding: Finding) {
        let key = format!("{:?}|{}|{}", finding.fmt, finding.kind, finding.signature);
        let mut map = self.findings.lock().unwrap();
        let entry = map.entry(key).or_insert((0, Vec::new()));
        entry.0 += 1;
        if entry.1.len() < 12 {
            // Prefer the shortest sources as examples.
            entry.1.push(finding);
            entry.1.sort_by_key(|f| (f.source.len(), f.trace.len()));
        } else if finding.source.len() < entry.1.last().unwrap().source.len() {
            entry.1.pop();
            entry.1.push(finding);
            entry.1.sort_by_key(|f| (f.source.len(), f.trace.len()));
        }
    }
    fn write(&self, out: &PathBuf, label: &str) {
        std::fs::create_dir_all(out).unwrap();
        let map = self.findings.lock().unwrap();
        let mut file = std::fs::File::create(out.join(format!("{label}-findings.jsonl"))).unwrap();
        let mut summary = std::fs::File::create(out.join(format!("{label}-summary.txt"))).unwrap();
        let mut rows: Vec<_> = map.iter().collect();
        rows.sort_by(|a, b| b.1 .0.cmp(&a.1 .0));
        writeln!(summary, "probes: {}", self.probes.load(Ordering::Relaxed)).unwrap();
        for (key, (count, examples)) in rows {
            writeln!(summary, "{count:7}  {key}").unwrap();
            if let Some(example) = examples.first() {
                writeln!(summary, "         e.g. source={:?} ctx={} detail={}", example.source, example.context,
                    example.detail.chars().take(300).collect::<String>()).unwrap();
            }
            for example in examples {
                writeln!(file, "{}", json!({"key": key, "count": count, "finding": example})).unwrap();
            }
        }
    }
}

fn finding(h: &H, kind: &str, signature: &str, context: String, detail: String) -> Finding {
    Finding {
        kind: kind.into(),
        signature: normalize(signature),
        fmt: h.seed.fmt,
        source: h.seed.source.clone(),
        context,
        detail,
        trace: h.trace.clone(),
        filename: h.seed.filename.clone(),
    }
}

/// Signature for an error/status: strip payloads that vary between examples.
fn error_signature(res: &Res) -> String {
    let text = res.describe();
    // Keep the variant path, drop numbers.
    normalize(&text)
}

// ---------------------------------------------------------------------------
// Probe helpers

fn boundaries(text: &str) -> Vec<usize> {
    let mut b: Vec<usize> = text.grapheme_indices(true).map(|(i, _)| i).collect();
    b.push(text.len());
    b
}

fn sample<T: Clone>(items: &[T], max: usize) -> Vec<T> {
    if items.len() <= max {
        return items.to_vec();
    }
    let step = items.len() as f64 / max as f64;
    (0..max).map(|i| items[(i as f64 * step) as usize].clone()).collect()
}

fn op_label(acts: &[Act]) -> String {
    acts.iter()
        .map(|a| match a {
            Act::Key { key } => format!("<{key}>"),
            Act::Text { text } => format!("{text:?}"),
            other => format!("{other:?}"),
        })
        .collect::<Vec<_>>()
        .join("")
}

fn text_acts(t: &str) -> Vec<Act> {
    vec![Act::Text { text: t.into() }]
}

fn insert_ops(fmt: Fmt) -> Vec<Vec<Act>> {
    let mut ops: Vec<Vec<Act>> = Vec::new();
    for t in ["x", " ", "é"] {
        ops.push(text_acts(t));
    }
    if matches!(fmt, Fmt::Wysiwyg | Fmt::Source) {
        for t in ["*", "`", "|", "#", ">", "-", "[", "\\", "<", "1", "_", "~", "!", "&"] {
            ops.push(text_acts(t));
        }
        ops.push(vec![Act::Text { text: "-".into() }, Act::Text { text: " ".into() }]);
        ops.push(vec![Act::Text { text: "#".into() }, Act::Text { text: " ".into() }]);
        ops.push(vec![Act::Text { text: ">".into() }, Act::Text { text: " ".into() }]);
        ops.push(vec![Act::Text { text: "`".into() }, Act::Text { text: "`".into() }, Act::Text { text: "`".into() }]);
        ops.push(vec![Act::Text { text: "*".into() }, Act::Text { text: "x".into() }, Act::Text { text: "*".into() }]);
    } else {
        for t in ["{", "\"", "\t", "/"] {
            ops.push(text_acts(t));
        }
    }
    for k in ["Enter", "ShiftEnter", "BS", "Del", "Tab", "BackTab"] {
        ops.push(vec![Act::Key { key: k.into() }]);
    }
    ops.push(vec![Act::Key { key: "Enter".into() }, Act::Text { text: "x".into() }]);
    ops.push(vec![Act::Key { key: "Enter".into() }, Act::Key { key: "Enter".into() }]);
    ops.push(vec![Act::Key { key: "Enter".into() }, Act::Key { key: "BS".into() }]);
    ops.push(vec![Act::Key { key: "BS".into() }, Act::Text { text: "x".into() }]);
    ops.push(vec![Act::Key { key: "BS".into() }, Act::Key { key: "BS".into() }]);
    ops.push(vec![Act::Key { key: "Del".into() }, Act::Text { text: "x".into() }]);
    ops.push(vec![Act::Key { key: "Del".into() }, Act::Key { key: "Del".into() }]);
    ops.push(vec![Act::Key { key: "Tab".into() }, Act::Text { text: "x".into() }]);
    ops.push(vec![Act::Key { key: "ShiftEnter".into() }, Act::Text { text: "x".into() }]);
    ops.push(vec![Act::Paste { text: "pasted".into() }]);
    ops.push(vec![Act::Paste { text: "line one\nline two\n".into() }]);
    ops.push(vec![Act::Paste { text: "**md** | x\n\n- y".into() }]);
    ops.push(vec![Act::Compose { text: "かな".into(), commit: true }]);
    ops.push(vec![Act::Compose { text: "かな".into(), commit: false }]);
    ops.push(vec![Act::ComposeReplacePrev { text: "é".into() }]);
    if matches!(fmt, Fmt::Wysiwyg | Fmt::Source) {
        ops.push(vec![Act::Semantic { style: "strong".into(), enabled: true }, Act::Text { text: "x".into() }]);
        ops.push(vec![Act::CharStyle { style: "Code".into() }, Act::Text { text: "x".into() }]);
        ops.push(vec![Act::Semantic { style: "emphasis".into(), enabled: true }, Act::Text { text: "x".into() }]);
        ops.push(vec![Act::ParagraphStyle { style: "Heading1".into() }]);
        ops.push(vec![Act::ParagraphStyle { style: "Paragraph".into() }]);
        ops.push(vec![Act::ParagraphStyle { style: "Code Block".into() }]);
        ops.push(vec![Act::List { style: Some("bullet".into()) }]);
        ops.push(vec![Act::List { style: Some("numbered".into()) }]);
        ops.push(vec![Act::List { style: None }]);
        ops.push(vec![Act::Quote { enabled: true }]);
        ops.push(vec![Act::Quote { enabled: false }]);
        ops.push(vec![Act::Indent { unindent: false }]);
        ops.push(vec![Act::Indent { unindent: true }]);
        ops.push(vec![Act::TableInsert { columns: 2, rows: 1 }]);
        ops.push(vec![Act::TableRow { after: true }]);
        ops.push(vec![Act::TableColumn { after: false }]);
        ops.push(vec![Act::TableDeleteRow]);
        ops.push(vec![Act::TableDeleteColumn]);
        ops.push(vec![Act::LinkInsert { text: "t".into(), destination: "https://e.com".into() }]);
        ops.push(vec![Act::LinkInsert { text: "t".into(), destination: "https://e.com".into() }, Act::Text { text: "x".into() }]);
        ops.push(vec![Act::LinkRemove]);
        ops.push(vec![Act::ImageInsert { destination: "pic.png".into() }]);
        ops.push(vec![Act::ImageInsert { destination: "pic.png".into() }, Act::Text { text: "x".into() }]);
    }
    ops
}

fn normal_ops() -> Vec<Vec<Act>> {
    normal_op_specs().iter().map(|spec| keys(spec)).collect()
}

fn normal_op_specs() -> Vec<&'static str> {
    vec![
        "x", "X", "dd", "D", "J", "gJ", "ox<Esc>", "Ox<Esc>", "rz", "r<CR>", "~", ">>", "<<", "dw", "db", "de",
        "cwx<Esc>", "ccx<Esc>", "Sx<Esc>", "Cx<Esc>", "sx<Esc>", "i<CR><Esc>", "a<BS><Esc>", "i<BS><Esc>",
        "a<Del><Esc>", "yyp", "yyP", "ylp", "xp", "ddp", "ddP", "dap", "dip", "ciwx<Esc>", "das", "vd", "Vd",
        "vjd", "Vjd", "<C-v>jd", "vlcx<Esc>", "Vjcx<Esc>", "Ax<Esc>", "Ix<Esc>", "A<CR><Esc>", "I<BS><Esc>",
        "o<Esc>", "O<Esc>", "3x", "2dd", "d}", "d{", "dG", "dgg", "guu", "gUU", "g~~", "J.", "x.", "dd.", "==",
        "gqq", ":s/a/b/<CR>", ":s/$/!/<CR>", ":s/^/#/<CR>", ":d<CR>", ":m+1<CR>", ":t.<CR>", ":sort<CR>",
        ":g/a/d<CR>", "yiwP", "vipd", "v$d", "v0d", "R<BS>xy<Esc>", "Rxyz<Esc>", "<C-a>", "<C-x>", "xu<C-r>",
        // Replace-mode restoration: each of these must leave the source byte-identical.
        "Rxy<BS><BS><Esc>", "R<CR><BS><Esc>", "Rxyz<C-w><Esc>", "Rxyz<C-u><Esc>", "R<Tab><BS><Esc>",
        // Insert-mode editing keys.
        "i<C-w><Esc>", "i<C-u><Esc>", "A<C-w><Esc>", "i<Tab><Esc>", "I<C-t><Esc>", "I<C-d><Esc>", "A<CR>x<Esc>",
        "I<BS><BS><Esc>", "a<Del><Del><Esc>", "i<C-e><Esc>", "i<C-y><Esc>", "ix<BS><BS><Esc>", "a<CR><BS><Esc>",
        // Dot repeat after a motion.
        "cwx<Esc>w.", "A!<Esc>j.", "ox<Esc>.", "dwj.", "2dw.", "Ix<Esc>j.", "ciwy<Esc>w.", "rzl.", "3~.",
        // Ex line commands.
        ":j<CR>", ":.,+1j<CR>", ":><CR>", ":$d<CR>", ":1,2d<CR>", ":m0<CR>", ":m$<CR>", ":t$<CR>",
        ":t0<CR>", ":.,$d<CR>", ":%j<CR>", ":v/a/d<CR>", ":g/^$/d<CR>", ":norm Ax<CR>", ":s/a/X/g<CR>", ":%s/ /_/g<CR>",
        ":sort u<CR>", ":sort i<CR>", ":sort!<CR>", ":retab<CR>", ":%norm Ax<CR>", ":2,3><CR>", ":.,+1y|pu<CR>",
        // Characterwise Visual operations.
        "vlld", "vllr-", "vllU", "vll~", "vllJ", "vll>", "vllcx<Esc>", "vllyP", "vllohd", "veed", "vbd", "veey$p",
        "vllx", "vllX", "vllD", "vllY", "vllC!<Esc>", "vllS!<Esc>", "vllgJ",
    ]
}

/// Differential cases for reference Vim: run every Normal-mode op at every
/// character of each literal seed and print one JSON line per case with the
/// resulting source. `tools/vim_diff.py` replays the same cases in Vim.
fn vim_cases(sources: &[Seed], out: &str, max_positions: usize) {
    use std::io::Write;
    let file = std::fs::File::create(out).unwrap();
    let file = Mutex::new(std::io::BufWriter::new(file));
    // VIM_OPS="spec|spec|..." limits the run to exact op specs.
    let only: Option<Vec<String>> = std::env::var("VIM_OPS").ok().map(|v| v.split('|').map(String::from).collect());
    let specs: Vec<&str> = normal_op_specs()
        .into_iter()
        .filter(|s| !s.contains("<C-a>") && !s.contains("<C-x>"))
        .filter(|s| only.as_ref().is_none_or(|only| only.iter().any(|o| o == s)))
        .collect();
    parallel(sources, 8, |seed| {
        let Ok(base) = H::open(seed, probe_width(), H_) else { return };
        let text = base.text().to_string();
        let mut positions: Vec<usize> = text.grapheme_indices(true).map(|(i, _)| i).collect();
        if positions.is_empty() {
            positions.push(0);
        }
        let positions = if max_positions == 0 { positions } else { sample(&positions, max_positions) };
        for &pos in &positions {
            for spec in &specs {
                let Ok(mut h) = H::open(seed, probe_width(), H_) else { return };
                let _ = h.act(&Act::Place { offset: pos, downstream: true, extend: false });
                if h.mode() != Mode::Normal {
                    let _ = h.act(&Act::Key { key: "Esc".into() });
                }
                let cursor = h.cursor();
                let results = h.acts(&keys(spec));
                let _ = h.act(&Act::Key { key: "Esc".into() });
                let error = results.iter().find(|r| !r.is_ok()).map(|r| r.describe()).unwrap_or_default();
                let line = json!({
                    "fmt": seed.fmt, "filename": seed.filename, "source": seed.source, "cursor": cursor,
                    "op": spec, "error": error, "result": String::from_utf8_lossy(&h.bytes()),
                });
                writeln!(file.lock().unwrap(), "{line}").unwrap();
            }
        }
    });
}

/// Normal-mode operations that must leave the source byte-identical.
const RESTORING_OPS: &[&str] = &[
    "<R><x><y><BS><BS><Esc>",
    "<R><Enter><BS><Esc>",
    "<R><x><y><z><C-w><Esc>",
    "<R><x><y><z><C-u><Esc>",
    "<R><Tab><BS><Esc>",
];

/// Line-local Normal-mode operations: in literal views they never add or
/// remove a source line.
const LINE_PRESERVING_OPS: &[&str] = &[
    "x", "X", "3x", "rz", "~", "D", "ccx<Esc>", "Sx<Esc>", "Cx<Esc>", "sx<Esc>", "cwx<Esc>", "ciwx<Esc>",
    "Rxyz<Esc>", "Ax<Esc>", "Ix<Esc>", "guu", "gUU", "g~~", ">>", "<<", "==", "x.", "v0d", "yiwP",
];

/// WYSIWYG operations that edit text inside one paragraph and must keep the
/// block structure: kinds, list and quote nesting, and paragraph styles.
const SHAPE_PRESERVING_OPS: &[&str] = &[
    "x", "rz", "~", "ciwx<Esc>", "cwx<Esc>", "sx<Esc>", "Rxyz<Esc>", "Ax<Esc>", "Ix<Esc>", "guu", "gUU", "g~~",
    "yiwP", "x.", "ccx<Esc>", "Sx<Esc>", "Cx<Esc>",
];

fn line_count(bytes: &[u8]) -> usize {
    bytes.iter().enumerate().filter(|&(i, &b)| b == b'\n' || (b == b'\r' && bytes.get(i + 1) != Some(&b'\n'))).count()
}

/// Expected plausibility for Insert-mode operations at a caret.
fn plausibility(op: &[Act], before_text: &str, before_bytes: &[u8], cursor: usize, h: &H) -> Option<(String, String)> {
    let after_text = h.text();
    let after_bytes = h.bytes();
    let changed = after_bytes != before_bytes;
    match op {
        [Act::Text { text }] if text == "x" => {
            let mut expected = before_text.to_string();
            expected.insert_str(cursor, "x");
            if after_text != expected {
                if !changed {
                    return Some(("insert_noop".into(), "typing x changed nothing".into()));
                }
                return Some((
                    "insert_misplaced".into(),
                    format!("expected {:?} got {:?}", expected, after_text),
                ));
            }
        }
        [Act::Text { text }] if !changed && !text.is_empty() => {
            return Some(("insert_noop".into(), format!("typing {text:?} changed nothing")));
        }
        [Act::Key { key }] if key == "BS" && !changed && cursor > 0 && h.cursor() == cursor => {
            return Some(("backspace_noop".into(), format!("backspace at {cursor} did nothing")));
        }
        [Act::Key { key }] if (key == "BS" || key == "Del") && changed => {
            // Expected: exactly the adjacent grapheme removed. Anything else
            // (structural joins excepted when the removed item is a break)
            // is reported for review.
            let (lo, hi) = if key == "BS" {
                let g = before_text[..cursor].graphemes(true).next_back().map_or(0, |g| g.len());
                (cursor - g, cursor)
            } else {
                let g = before_text[cursor..].graphemes(true).next().map_or(0, |g| g.len());
                (cursor, cursor + g)
            };
            let mut expected = before_text.to_string();
            expected.replace_range(lo..hi, "");
            if after_text != expected && after_text != before_text {
                let removed = &before_text[lo..hi];
                let kind = if removed == "\n" { "delete_break_unexpected" } else { "delete_unexpected" };
                return Some((kind.into(), format!("{key} at {cursor}: expected {:?} got {:?}", expected, after_text)));
            }
        }
        [Act::Key { key }] if key == "Del" && !changed && cursor < before_text.len() && h.cursor() == cursor => {
            return Some(("delete_noop".into(), format!("delete at {cursor} did nothing")));
        }
        [Act::Key { key }] if (key == "Enter" || key == "ShiftEnter") && !changed => {
            return Some(("enter_noop".into(), format!("{key} at {cursor} did nothing")));
        }
        _ => {}
    }
    None
}

// ---------------------------------------------------------------------------
// Probe campaign

const W_DEFAULT: f32 = 600.;

/// Probe view width; `FUZZ_WIDTH` overrides it to exercise soft-wrapped rows.
fn probe_width() -> f32 {
    std::env::var("FUZZ_WIDTH").ok().and_then(|w| w.parse().ok()).unwrap_or(W_DEFAULT)
}
const H_: f32 = 2000.;

fn filter_ops(ops: &mut Vec<Vec<Act>>) {
    if let Ok(filter) = std::env::var("FUZZ_OPS") {
        ops.retain(|op| filter.split(',').any(|f| op_label(op).contains(f)));
    }
}

fn probe_insert(seed: &Seed, sink: &Sink, max_positions: usize) {
    let Ok(base) = H::open(seed, probe_width(), H_) else { return };
    let text = base.text().to_string();
    let positions = sample(&boundaries(&text), max_positions);
    let mut ops = insert_ops(seed.fmt);
    filter_ops(&mut ops);
    for &pos in &positions {
        for downstream in [true, false] {
            if !downstream && (pos == 0 || pos == text.len()) {
                continue;
            }
            for op in &ops {
                sink.probes.fetch_add(1, Ordering::Relaxed);
                let Ok(mut h) = H::open(seed, probe_width(), H_) else { return };
                let r = h.act(&Act::Key { key: "i".into() });
                if !r.is_ok() {
                    sink.add(finding(&h, "enter_insert", &error_signature(&r), format!("pos {pos}"), r.describe()));
                    continue;
                }
                let r = h.act(&Act::Place { offset: pos, downstream, extend: false });
                if !r.is_ok() {
                    sink.add(finding(&h, "place_insert", &error_signature(&r), format!("pos {pos} ds {downstream}"), r.describe()));
                    continue;
                }
                if h.mode() != Mode::Insert {
                    sink.add(finding(&h, "place_left_insert", &format!("{:?}", h.mode()), format!("pos {pos}"), String::new()));
                    continue;
                }
                let cursor = h.cursor();
                if cursor != pos {
                    sink.add(finding(&h, "place_snapped", "insert caret snapped", format!("pos {pos} -> {cursor} ds {downstream}"), String::new()));
                }
                let before_text = h.text().to_string();
                let before_bytes = h.bytes();
                let ctx = format!("insert pos {pos} ds {downstream} op {}", op_label(op));
                let results = h.acts(op);
                let mut failed = false;
                for r in &results {
                    if !r.is_ok() {
                        failed = true;
                        sink.add(finding(&h, "op_error", &format!("{} {}", op_label(op), error_signature(r)), ctx.clone(), r.describe()));
                        break;
                    }
                }
                for (kind, detail) in h.check(true, true) {
                    sink.add(finding(&h, &kind, &format!("{} {}", op_label(op), normalize(&detail).chars().take(60).collect::<String>()), ctx.clone(), detail));
                }
                for (label, secs) in std::mem::take(&mut h.slow) {
                    sink.add(finding(&h, "slow_action", &op_label(op), ctx.clone(), format!("{secs:.2}s {label}")));
                }
                if failed {
                    // Can the user still type here?
                    let source_now = h.bytes();
                    let _ = h.act(&Act::Key { key: "Esc".into() });
                    let r1 = h.act(&Act::Key { key: "i".into() });
                    let r2 = h.act(&Act::Text { text: "y".into() });
                    if !r1.is_ok() || !r2.is_ok() || h.bytes() == source_now {
                        sink.add(finding(&h, "stuck_after_error", &format!("{} then type", op_label(op)), ctx.clone(),
                            format!("{} / {}", r1.describe(), r2.describe())));
                    }
                    continue;
                }
                if let Some((kind, detail)) = plausibility(op, &before_text, &before_bytes, cursor, &h) {
                    sink.add(finding(&h, &kind, &op_label(op), ctx.clone(), detail));
                }
                // Undo/redo exactness.
                let r = h.act(&Act::Key { key: "Esc".into() });
                if !r.is_ok() {
                    sink.add(finding(&h, "escape_error", &error_signature(&r), ctx.clone(), r.describe()));
                    continue;
                }
                let after = h.bytes();
                if after != before_bytes {
                    let r = h.act(&Act::Key { key: "u".into() });
                    if !r.is_ok() || h.bytes() != before_bytes {
                        sink.add(finding(&h, "undo_mismatch", &op_label(op), ctx.clone(), format!(
                            "{}: before={:?} after_undo={:?}", r.describe(), String::from_utf8_lossy(&before_bytes),
                            String::from_utf8_lossy(&h.bytes()))));
                        continue;
                    }
                    let r = h.act(&Act::Key { key: "C-r".into() });
                    if !r.is_ok() || h.bytes() != after {
                        sink.add(finding(&h, "redo_mismatch", &op_label(op), ctx.clone(), format!(
                            "{}: expected={:?} got={:?}", r.describe(), String::from_utf8_lossy(&after),
                            String::from_utf8_lossy(&h.bytes()))));
                    }
                    for (kind, detail) in h.check(true, false) {
                        sink.add(finding(&h, &format!("{kind}_after_redo"), &op_label(op), ctx.clone(), detail));
                    }
                }
            }
        }
    }
}

fn probe_normal(seed: &Seed, sink: &Sink, max_positions: usize) {
    let Ok(base) = H::open(seed, probe_width(), H_) else { return };
    let text = base.text().to_string();
    let mut positions: Vec<usize> = text.grapheme_indices(true).map(|(i, _)| i).collect();
    if positions.is_empty() {
        positions.push(0);
    }
    let positions = sample(&positions, max_positions);
    let mut ops = normal_ops();
    filter_ops(&mut ops);
    for &pos in &positions {
        for op in &ops {
            sink.probes.fetch_add(1, Ordering::Relaxed);
            let Ok(mut h) = H::open(seed, probe_width(), H_) else { return };
            // Reset any yank state, then place.
            let r = h.act(&Act::Place { offset: pos, downstream: true, extend: false });
            if !r.is_ok() {
                sink.add(finding(&h, "place_normal", &error_signature(&r), format!("pos {pos}"), r.describe()));
                break;
            }
            if h.mode() != Mode::Normal {
                let _ = h.act(&Act::Key { key: "Esc".into() });
            }
            let cursor = h.cursor();
            let before_bytes = h.bytes();
            let before_shape = block_shape(h.core.document());
            let ctx = format!("normal pos {pos} (cursor {cursor}) op {}", op_label(op));
            let results = h.acts(op);
            for r in &results {
                if !r.is_ok() && !benign_status(r) {
                    sink.add(finding(&h, "normal_op_error", &format!("{} {}", op_label(op), error_signature(r)), ctx.clone(), r.describe()));
                    break;
                }
            }
            for (kind, detail) in h.check(true, true) {
                sink.add(finding(&h, &kind, &format!("{} {}", op_label(op), normalize(&detail).chars().take(60).collect::<String>()), ctx.clone(), detail));
            }
            for (label, secs) in std::mem::take(&mut h.slow) {
                sink.add(finding(&h, "slow_action", &op_label(op), ctx.clone(), format!("{secs:.2}s {label}")));
            }
            let _ = h.act(&Act::Key { key: "Esc".into() });
            let after = h.bytes();
            if matches!(seed.fmt, Fmt::Source | Fmt::Plain | Fmt::Code)
                && LINE_PRESERVING_OPS.iter().any(|spec| op_label(&keys(spec)) == op_label(op))
                && results.iter().all(|r| r.is_ok())
                && line_count(&after) != line_count(&before_bytes)
            {
                sink.add(finding(&h, "line_count_changed", &op_label(op), ctx.clone(), format!(
                    "before={:?} after={:?}", String::from_utf8_lossy(&before_bytes), String::from_utf8_lossy(&after))));
            }
            if seed.fmt == Fmt::Wysiwyg
                && SHAPE_PRESERVING_OPS.iter().any(|spec| op_label(&keys(spec)) == op_label(op))
                && results.iter().all(|r| r.is_ok())
            {
                let after_shape = block_shape(h.core.document());
                if after_shape != before_shape {
                    sink.add(finding(&h, "block_shape_changed", &op_label(op), ctx.clone(), format!(
                        "before={:?} after={:?} source_before={:?} source_after={:?}",
                        before_shape, after_shape, String::from_utf8_lossy(&before_bytes), String::from_utf8_lossy(&after))));
                }
            }
            if RESTORING_OPS.contains(&op_label(op).as_str()) && after != before_bytes && results.iter().all(|r| r.is_ok()) {
                sink.add(finding(&h, "replace_restore_mismatch", &op_label(op), ctx.clone(), format!(
                    "before={:?} after={:?}", String::from_utf8_lossy(&before_bytes), String::from_utf8_lossy(&after))));
            }
            if after != before_bytes {
                // Undo everything (dot ops may make two units).
                for _ in 0..4 {
                    if h.bytes() == before_bytes {
                        break;
                    }
                    let r = h.act(&Act::Key { key: "u".into() });
                    if !r.is_ok() {
                        break;
                    }
                }
                if h.bytes() != before_bytes {
                    sink.add(finding(&h, "undo_mismatch", &op_label(op), ctx.clone(), format!(
                        "before={:?} after_undo={:?}", String::from_utf8_lossy(&before_bytes), String::from_utf8_lossy(&h.bytes()))));
                }
            }
        }
    }
}

/// Statuses which are legitimate Vim refusals, not consistency findings.
fn benign_status(r: &Res) -> bool {
    match r {
        Res::Status(s) => {
            s.contains("empty register")
                || s.contains("no previous")
                || s.contains("text object is empty")
                || s.contains("not enough characters")
                || s.contains("already at the")
                || s.contains("SearchNotFound")
                || s.contains("pattern not found")
                || s.contains("E486")
                || s.contains("PatternNotFound")
                || s.contains("jump list is empty")
                || s.contains("no preferred redo state")
                || s.contains("AddressOutOfBounds")
                || s.contains("reflow is not supported for Markdown")
                || s.contains("text object could not be resolved")
                || s.starts_with("status Unsupported(\"normal key")
                || s.starts_with("status Unsupported(\"visual key")
                || s.starts_with("status Unsupported(\"normal command")
                || s.contains("CountError")
        }
        _ => false,
    }
}

fn probe_selection(seed: &Seed, sink: &Sink, max_pairs: usize) {
    let Ok(base) = H::open(seed, probe_width(), H_) else { return };
    let text = base.text().to_string();
    let b = boundaries(&text);
    let mut pairs = Vec::new();
    for (i, &start) in b.iter().enumerate() {
        for &end in &b[i + 1..] {
            pairs.push((start, end));
        }
    }
    let pairs = sample(&pairs, max_pairs);
    let mut ops: Vec<Vec<Act>> = vec![
        vec![Act::Key { key: "BS".into() }],
        vec![Act::Key { key: "Del".into() }],
        vec![Act::Text { text: "x".into() }],
        vec![Act::Key { key: "Enter".into() }],
        vec![Act::Paste { text: "p".into() }],
        vec![Act::Paste { text: "l1\nl2\n".into() }],
        vec![Act::Key { key: "BS".into() }, Act::Key { key: "BS".into() }],
        vec![Act::Key { key: "BS".into() }, Act::Text { text: "x".into() }],
        vec![Act::Key { key: "Copy".into() }],
        // Composed input replaces the selection like typing (macOS dead keys, press-and-hold, IME).
        vec![Act::Compose { text: "é".into(), commit: true }],
        vec![
            Act::Compose { text: "é".into(), commit: true },
            Act::Compose { text: "ñ".into(), commit: true },
            Act::Text { text: "x".into() },
            Act::History { undo: true },
            Act::Key { key: "l".into() },
        ],
    ];
    if matches!(seed.fmt, Fmt::Wysiwyg | Fmt::Source) {
        ops.push(vec![Act::Semantic { style: "strong".into(), enabled: true }]);
        ops.push(vec![Act::Semantic { style: "emphasis".into(), enabled: true }]);
        ops.push(vec![Act::CharStyle { style: "Code".into() }]);
        ops.push(vec![Act::CharStyle { style: "".into() }]);
        ops.push(vec![Act::Semantic { style: "strong".into(), enabled: false }]);
        ops.push(vec![Act::ParagraphStyle { style: "Heading2".into() }]);
        ops.push(vec![Act::ParagraphStyle { style: "Code Block".into() }]);
        ops.push(vec![Act::List { style: Some("bullet".into()) }]);
        ops.push(vec![Act::Quote { enabled: true }]);
        ops.push(vec![Act::Indent { unindent: false }]);
        ops.push(vec![Act::LinkInsert { text: String::new(), destination: "https://e.com".into() }]);
        ops.push(vec![Act::LinkRemove]);
        ops.push(vec![Act::ImageInsert { destination: "pic.png".into() }]);
    }
    filter_ops(&mut ops);
    for &(start, end) in &pairs {
        for reverse in [false, true] {
            for op in &ops {
                sink.probes.fetch_add(1, Ordering::Relaxed);
                let Ok(mut h) = H::open(seed, probe_width(), H_) else { return };
                let (a, z) = if reverse { (end, start) } else { (start, end) };
                let r1 = h.act(&Act::Pointer { offset: a });
                let r2 = h.act(&Act::Place { offset: z, downstream: true, extend: true });
                if !r1.is_ok() || !r2.is_ok() {
                    sink.add(finding(&h, "select_error", &format!("{} {}", error_signature(&r1), error_signature(&r2)), format!("{a}..{z}"), String::new()));
                    continue;
                }
                let before_bytes = h.bytes();
                let before_text = h.text().to_string();
                // Oracle: the core's own logical selection.
                let selected = match h.core.list_selection_identity(h.view) {
                    Ok(identity) => identity.range(),
                    Err(_) => continue,
                };
                let (start, end) = (selected.start, selected.end);
                if start == end {
                    continue;
                }
                let ctx = format!("select {a}..{z} (core {start}..{end}) mode {:?} op {}", h.mode(), op_label(op));
                let results = h.acts(op);
                let mut failed = false;
                for r in &results {
                    if !r.is_ok() {
                        failed = true;
                        sink.add(finding(&h, "selection_op_error", &format!("{} {}", op_label(op), error_signature(r)), ctx.clone(), r.describe()));
                        break;
                    }
                }
                for (kind, detail) in h.check(true, true) {
                    sink.add(finding(&h, &kind, &format!("{} {}", op_label(op), normalize(&detail).chars().take(60).collect::<String>()), ctx.clone(), detail));
                }
                for (label, secs) in std::mem::take(&mut h.slow) {
                    sink.add(finding(&h, "slow_action", &op_label(op), ctx.clone(), format!("{secs:.2}s {label}")));
                }
                if failed {
                    continue;
                }
                // Deleting a nonempty selection must remove its text.
                if matches!(op.as_slice(), [Act::Key { key }] if key == "BS" || key == "Del") {
                    let (s, e) = (start.min(end), start.max(end));
                    let mut expected = before_text.clone();
                    expected.replace_range(s..e, "");
                    if h.text() != expected && h.text() == before_text {
                        sink.add(finding(&h, "selection_delete_noop", &op_label(op), ctx.clone(), String::new()));
                    } else if h.text() != expected {
                        sink.add(finding(&h, "selection_delete_mismatch", &op_label(op), ctx.clone(),
                            format!("expected {expected:?} got {:?}", h.text())));
                    }
                }
                let replacement = match op.as_slice() {
                    [Act::Text { text }] if text == "x" => Some("x"),
                    [Act::Compose { text, commit: true }] if text == "é" => Some("é"),
                    _ => None,
                };
                if let Some(replacement) = replacement {
                    let (s, e) = (start.min(end), start.max(end));
                    let mut expected = before_text.clone();
                    expected.replace_range(s..e, replacement);
                    if h.text() != expected {
                        sink.add(finding(&h, "selection_replace_mismatch", &op_label(op), ctx.clone(),
                            format!("expected {expected:?} got {:?}", h.text())));
                    }
                }
                let _ = h.act(&Act::Key { key: "Esc".into() });
                let after = h.bytes();
                if after != before_bytes {
                    let r = h.act(&Act::Key { key: "u".into() });
                    if !r.is_ok() || h.bytes() != before_bytes {
                        sink.add(finding(&h, "undo_mismatch", &op_label(op), ctx.clone(), format!(
                            "{} before={:?} after_undo={:?}", r.describe(), String::from_utf8_lossy(&before_bytes),
                            String::from_utf8_lossy(&h.bytes()))));
                    }
                }
            }
        }
    }
}

fn caret_xy(h: &H, offset: usize) -> Option<(f32, f32)> {
    let layout = h.core.presentation_layout(h.view)?;
    let snapshot = layout.snapshot()?;
    let affinity = h.core.command_state(h.view)?.boundary_affinity();
    snapshot
        .logical_endpoint_geometry(offset, affinity)
        .or_else(|_| snapshot.logical_endpoint_geometry(offset, match affinity {
            BoundaryAffinity::Upstream => BoundaryAffinity::Downstream,
            _ => BoundaryAffinity::Upstream,
        }))
        .ok()
        .map(|g| (g.rect.x, g.rect.y))
}

fn caret_y(h: &H, offset: usize) -> Option<f32> {
    let layout = h.core.presentation_layout(h.view)?;
    let snapshot = layout.snapshot()?;
    let affinity = h.core.command_state(h.view)?.boundary_affinity();
    snapshot.logical_endpoint_geometry(offset, affinity).ok().map(|g| g.rect.y)
}

fn has_rtl(text: &str) -> bool {
    text.chars().any(|c| matches!(c as u32, 0x0590..=0x08FF | 0xFB1D..=0xFEFC))
}

/// Arrow keys must make progress until the document edge.
fn probe_navigation(seed: &Seed, sink: &Sink) {
    for insert in [false, true] {
        for (key, forward, vertical) in [
            ("Right", true, false),
            ("Left", false, false),
            ("Down", true, true),
            ("Up", false, true),
            ("WordRight", true, false),
            ("WordLeft", false, false),
            ("ParaEnd", true, true),
            ("ParaStart", false, true),
            ("PageDown", true, true),
            ("j", true, true),
            ("k", false, true),
            ("w", true, false),
            ("b", false, false),
            ("}", true, true),
            ("{", false, true),
        ] {
            if insert && key.len() == 1 {
                continue;
            }
            for wrap in [true, false] {
                sink.probes.fetch_add(1, Ordering::Relaxed);
                let Ok(mut h) = H::open(seed, 300., 4000.) else { return };
                if !wrap {
                    let _ = h.act(&Act::Wrap { enabled: false });
                }
                if insert {
                    let _ = h.act(&Act::Key { key: "i".into() });
                }
                let edge_key = if forward { "DocEnd" } else { "DocStart" };
                let r = h.act(&Act::Key { key: edge_key.into() });
                let edge = h.cursor();
                for (kind, detail) in h.check(true, false) {
                    sink.add(finding(&h, &kind, &format!("nav {edge_key} {}", normalize(&detail).chars().take(50).collect::<String>()), format!("insert {insert} wrap {wrap}"), detail));
                }
                let edge_y = caret_y(&h, edge);
                if !r.is_ok() {
                    sink.add(finding(&h, "nav_error", &format!("{edge_key} {}", error_signature(&r)), format!("insert {insert}"), r.describe()));
                }
                let start_key = if forward { "DocStart" } else { "DocEnd" };
                let _ = h.act(&Act::Key { key: start_key.into() });
                let mut visited = vec![h.cursor()];
                let limit = h.text().len() + 64;
                let ctx_base = format!("insert {insert} wrap {wrap} key {key}");
                for _ in 0..limit {
                    let before = h.cursor();
                    let before_mode = h.mode();
                    let r = h.act(&Act::Key { key: key.into() });
                    if !r.is_ok() {
                        sink.add(finding(&h, "nav_error", &format!("{key} {}", error_signature(&r)), format!("{ctx_base} at {before}"), r.describe()));
                        break;
                    }
                    if h.mode() != before_mode {
                        sink.add(finding(&h, "nav_mode_change", &format!("{key} {:?}->{:?}", before_mode, h.mode()), format!("{ctx_base} at {before}"), String::new()));
                        break;
                    }
                    let after = h.cursor();
                    let problems = h.check(true, false);
                    let had_problems = !problems.is_empty();
                    for (kind, detail) in problems {
                        sink.add(finding(&h, &kind, &format!("nav {key} {}", normalize(&detail).chars().take(50).collect::<String>()), format!("{ctx_base} at {before}"), detail));
                    }
                    if had_problems {
                        break;
                    }
                    if h.core.document().source_bytes() != seed.source.as_bytes() {
                        sink.add(finding(&h, "nav_changed_source", key, format!("{ctx_base} at {before}"), String::new()));
                        break;
                    }
                    if after == before {
                        // Arrived?
                        let stuck = if vertical {
                            match (caret_y(&h, after), edge_y) {
                                (Some(y), Some(ey)) => if forward { y + 0.5 < ey } else { y > ey + 0.5 },
                                _ => false,
                            }
                        } else {
                            after != edge
                        };
                        // Normal-mode horizontal motions stop on the last
                        // character; only report when another motion goes further.
                        if stuck {
                            let tail = if forward { &h.text()[after..] } else { &h.text()[..after] };
                            sink.add(finding(&h, "nav_stuck", &format!("{} {key}", if insert { "insert" } else { "normal" }),
                                format!("{ctx_base} stuck at {after}, edge {edge}"), format!("remaining {:?}", tail.chars().take(80).collect::<String>())));
                        }
                        break;
                    }
                    let backwards = if forward { after < before } else { after > before };
                    if backwards && !has_rtl(h.text()) && !vertical {
                        sink.add(finding(&h, "nav_reversed", &format!("{} {key}", if insert { "insert" } else { "normal" }),
                            format!("{ctx_base} {before} -> {after}"), String::new()));
                        break;
                    }
                    if visited.contains(&after) && !vertical {
                        sink.add(finding(&h, "nav_cycle", &format!("{} {key}", if insert { "insert" } else { "normal" }),
                            format!("{ctx_base} revisits {after}"), String::new()));
                        break;
                    }
                    visited.push(after);
                }
                // Native Left/Right in Insert should reach every caret stop.
                if insert && (key == "Right" || key == "Left") && !has_rtl(h.text()) {
                    let text = h.text().to_string();
                    let mut missing = Vec::new();
                    for b in boundaries(&text) {
                        if !visited.contains(&b) && h.core.document().text_point(b).is_ok() {
                            missing.push(b);
                        }
                    }
                    if !missing.is_empty() {
                        sink.add(finding(&h, "nav_unreachable", &format!("insert {key}"), format!("{ctx_base}"),
                            format!("unvisited boundaries {:?} in {:?}", missing.iter().take(12).collect::<Vec<_>>(), text.chars().take(120).collect::<String>())));
                    }
                }
            }
        }
    }
}

/// From every caret position, one arrow press must move unless at the edge.
fn probe_navigation_positions(seed: &Seed, sink: &Sink, max_positions: usize) {
    for (width, wrap) in [(300., true), (70., true), (300., false)] {
        let Ok(base) = H::open(seed, width, 4000.) else { return };
        let text = base.text().to_string();
        if has_rtl(&text) {
            continue;
        }
        let positions = sample(&boundaries(&text), max_positions);
        for insert in [true, false] {
            // Edges in this mode.
            let edge = |key: &str| -> Option<(usize, Option<f32>)> {
                let mut h = H::open(seed, width, 4000.).ok()?;
                if !wrap { let _ = h.act(&Act::Wrap { enabled: false }); }
                if insert { let _ = h.act(&Act::Key { key: "i".into() }); }
                let _ = h.act(&Act::Key { key: key.into() });
                let c = h.cursor();
                Some((c, caret_y(&h, c)))
            };
            let Some((end, end_y)) = edge("DocEnd") else { return };
            let Some((start, start_y)) = edge("DocStart") else { return };
            for &pos in &positions {
                if !insert && pos == text.len() && pos > 0 {
                    continue;
                }
                for downstream in [true, false] {
                    if !downstream && (pos == 0 || !insert) {
                        continue;
                    }
                    for key in ["Right", "Left", "Down", "Up", "WordRight", "WordLeft", "End", "Home"] {
                        sink.probes.fetch_add(1, Ordering::Relaxed);
                        let Ok(mut h) = H::open(seed, width, 4000.) else { return };
                        if !wrap { let _ = h.act(&Act::Wrap { enabled: false }); }
                        if insert { let _ = h.act(&Act::Key { key: "i".into() }); }
                        let r = h.act(&Act::Place { offset: pos, downstream, extend: false });
                        if !r.is_ok() { continue; }
                        if insert && h.mode() != Mode::Insert { continue; }
                        let before = h.cursor();
                        let before_y = caret_y(&h, before);
                        let ctx = format!("{} pos {pos} (cursor {before}) ds {downstream} width {width} wrap {wrap} key {key}",
                            if insert { "insert" } else { "normal" });
                        let r = h.act(&Act::Key { key: key.into() });
                        if !r.is_ok() {
                            sink.add(finding(&h, "navpos_error", &format!("{key} {}", error_signature(&r)), ctx, r.describe()));
                            continue;
                        }
                        for (kind, detail) in h.check(true, false) {
                            sink.add(finding(&h, &kind, &format!("navpos {key} {}", normalize(&detail).chars().take(50).collect::<String>()), ctx.clone(), detail));
                        }
                        if h.mode() == Mode::VisualCharacter { continue; } // image selection
                        let after = h.cursor();
                        let after_y = caret_y(&h, after);
                        let stuck = after == before && match key {
                            "Right" | "WordRight" => before < end,
                            "Left" | "WordLeft" => before > start,
                            "Down" => matches!((before_y, end_y), (Some(y), Some(e)) if y + 0.5 < e),
                            "Up" => matches!((before_y, start_y), (Some(y), Some(s)) if y > s + 0.5),
                            _ => false,
                        };
                        if stuck {
                            sink.add(finding(&h, "navpos_stuck", &format!("{} {key}", if insert { "insert" } else { "normal" }), ctx.clone(),
                                format!("text {:?}", text.chars().take(120).collect::<String>())));
                        }
                        let wrong_way = match key {
                            "Right" | "WordRight" => after < before,
                            "Left" | "WordLeft" => after > before,
                            "Down" => matches!((before_y, after_y), (Some(a), Some(b)) if b + 0.5 < a),
                            "Up" => matches!((before_y, after_y), (Some(a), Some(b)) if b > a + 0.5),
                            _ => false,
                        };
                        if wrong_way {
                            sink.add(finding(&h, "navpos_reversed", &format!("{} {key}", if insert { "insert" } else { "normal" }), ctx.clone(),
                                format!("{before} -> {after}, y {before_y:?} -> {after_y:?}")));
                        }
                    }
                }
            }
        }
    }
}

/// Copy a selection with native Copy, then paste the captured payload at
/// another caret (Insert mode) and over another selection.
fn probe_copy_paste(seed: &Seed, sink: &Sink, max: usize) {
    let Ok(base) = H::open(seed, probe_width(), H_) else { return };
    let text = base.text().to_string();
    let b = boundaries(&text);
    let mut pairs = Vec::new();
    for (i, &start) in b.iter().enumerate() {
        for &end in &b[i + 1..] {
            pairs.push((start, end));
        }
    }
    let pairs = sample(&pairs, max);
    let targets = sample(&b, 12);
    for &(start, end) in &pairs {
        for &target in &targets {
            sink.probes.fetch_add(1, Ordering::Relaxed);
            let Ok(mut h) = H::open(seed, probe_width(), H_) else { return };
            let _ = h.act(&Act::Pointer { offset: start });
            let _ = h.act(&Act::Place { offset: end, downstream: true, extend: true });
            let r = h.act(&Act::Key { key: "Copy".into() });
            if !r.is_ok() || h.clipboard.is_none() {
                continue;
            }
            let _ = h.act(&Act::Key { key: "Esc".into() });
            let _ = h.act(&Act::Key { key: "i".into() });
            let _ = h.act(&Act::Place { offset: target, downstream: true, extend: false });
            let before = h.bytes();
            let ctx = format!("copy {start}..{end} paste at {target}");
            let r = h.act(&Act::PasteStored);
            if !r.is_ok() {
                sink.add(finding(&h, "copy_paste_error", &error_signature(&r), ctx.clone(), r.describe()));
            }
            for (kind, detail) in h.check(true, true) {
                sink.add(finding(&h, &kind, &format!("paste {}", normalize(&detail).chars().take(60).collect::<String>()), ctx.clone(), detail));
            }
            if r.is_ok() && h.bytes() == before && start != end {
                sink.add(finding(&h, "copy_paste_noop", "paste changed nothing", ctx.clone(), String::new()));
            }
            let _ = h.act(&Act::Key { key: "Esc".into() });
            if h.bytes() != before {
                let r = h.act(&Act::Key { key: "u".into() });
                if !r.is_ok() || h.bytes() != before {
                    sink.add(finding(&h, "undo_mismatch", "paste", ctx.clone(), r.describe()));
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Random walk

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
    fn chance(&mut self, a: u64, b: u64) -> bool {
        self.next() % b < a
    }
    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len())]
    }
}

fn random_act(rng: &mut Rng, h: &H) -> Vec<Act> {
    let text = h.text();
    let b = boundaries(text);
    let markdown = matches!(h.seed.fmt, Fmt::Wysiwyg | Fmt::Source);
    let pick_offset = |rng: &mut Rng| b[rng.below(b.len())];
    match rng.below(110) {
        0..=24 => {
            // Insert-mode typing burst with native navigation.
            let mut acts = Vec::new();
            if !matches!(h.mode(), Mode::Insert | Mode::Replace) {
                acts.push(Act::Key { key: rng.pick(&["i", "a", "A", "I", "o", "O"]).to_string() });
            }
            let tokens = [
                "x", " ", "a", "*", "**", "_", "`", "```", "|", "#", "# ", "> ", "- ", "1. ", "[", "]", "(", ")", "!",
                "<", ">", "&", "\\", "~~", "é", "👩‍💻", "العربية", "\t", "\"", "'", "<br>", "---", "| a |", "[l](u)",
            ];
            let special = [
                "Enter", "ShiftEnter", "BS", "Del", "Tab", "BackTab", "Left", "Right", "Up", "Down", "Home", "End",
                "WordLeft", "WordRight", "S-Right", "S-Left", "S-Down", "S-Up", "ParaStart", "ParaEnd",
            ];
            for _ in 0..1 + rng.below(8) {
                if rng.chance(1, 2) {
                    acts.push(Act::Text { text: rng.pick(&tokens).to_string() });
                } else {
                    acts.push(Act::Key { key: rng.pick(&special).to_string() });
                }
            }
            acts
        }
        25..=39 => {
            // Normal-mode commands.
            let mut acts = vec![Act::Key { key: "Esc".into() }];
            let ops = normal_ops();
            acts.extend(rng.pick(&ops).clone());
            acts
        }
        40..=51 => vec![Act::Place { offset: pick_offset(rng), downstream: rng.chance(1, 2), extend: rng.chance(1, 3) }],
        52..=54 => vec![Act::Word { offset: pick_offset(rng), extend: rng.chance(1, 3) }],
        55..=59 => vec![Act::Key {
            key: rng.pick(&["BS", "Del", "Enter", "Tab", "BackTab", "Right", "Left", "Down", "Up"]).to_string(),
        }],
        60..=63 => vec![Act::Paste {
            text: rng.pick(&["p", "two\nlines", "line\n", "| a | b |\n| - | - |\n| 1 | 2 |", "**b**", "- x\n- y\n", "```\nc\n```", "\n", "é👩‍💻"]).to_string(),
        }],
        64..=73 if markdown => {
            let choices: Vec<Act> = vec![
                Act::Semantic { style: "strong".into(), enabled: rng.chance(1, 2) },
                Act::Semantic { style: "emphasis".into(), enabled: rng.chance(1, 2) },
                Act::CharStyle { style: rng.pick(&["Code", ""]).to_string() },
                Act::ParagraphStyle { style: rng.pick(&["Heading1", "Heading2", "Heading6", "Paragraph", "Code Block"]).to_string() },
                Act::List { style: rng.pick(&[Some("bullet".to_string()), Some("numbered".to_string()), None]).clone() },
                Act::Quote { enabled: rng.chance(1, 2) },
                Act::Indent { unindent: rng.chance(1, 2) },
                Act::TableInsert { columns: 1 + rng.below(3), rows: rng.below(3) },
                Act::TableRow { after: rng.chance(1, 2) },
                Act::TableColumn { after: rng.chance(1, 2) },
                Act::TableDeleteRow,
                Act::TableDeleteColumn,
                Act::ToggleSource,
                Act::LinkInsert { text: "t".into(), destination: "https://e.com".into() },
                Act::LinkRemove,
                Act::ImageInsert { destination: "pic.png".into() },
            ];
            vec![rng.pick(&choices).clone()]
        }
        74..=77 => vec![Act::History { undo: rng.chance(2, 3) }],
        78..=80 => vec![Act::SelectAll],
        81..=83 => vec![Act::Scroll { top: *rng.pick(&[0., 50., 400., 5000., 1e9]), left: *rng.pick(&[0., 30., 4000.]) }],
        84..=85 => vec![Act::Resize { width: *rng.pick(&[40., 120., 300., 900.]), height: *rng.pick(&[30., 200., 900.]) }],
        86 => vec![Act::Wrap { enabled: rng.chance(1, 2) }],
        87 => vec![Act::LineMode { physical: rng.chance(1, 2) }],
        88 => vec![Act::SmartQuotes { enabled: rng.chance(1, 2) }],
        89 => vec![Act::Autoformat { enabled: rng.chance(2, 3) }],
        90..=92 => vec![Act::Compose { text: rng.pick(&["か", "é", "ñ", "中文"]).to_string(), commit: rng.chance(3, 4) }],
        93 => vec![Act::Scale { scale: *rng.pick(&[0.5, 1., 2.]) }],
        94 => vec![Act::ComposeOpen { text: rng.pick(&["に", "ほん", "´", "中"]).to_string() }],
        95 => vec![Act::ComposeEnd { commit: rng.chance(1, 2) }],
        96 => vec![Act::SwitchView { index: rng.below(2) }],
        97 => {
            let mut acts = vec![Act::Key { key: "Esc".into() }];
            acts.extend(keys(rng.pick(&[
                "/a<CR>", "/b<CR>n", "?x<CR>", "*", "#", "nN", "qaxq@a", "qbA!<Esc>q2@b", "Rxy<BS><BS><Esc>",
                "i<C-n><C-n><Esc>", "A <C-p><Esc>", "i<C-v>u00e9<Esc>", "i<C-v><Tab><Esc>", "ma`a", "u<C-r>u",
                "gv", "vipy", "yiwviwp", "ggVGd", "ggVGyP", ":%s/a/A/g<CR>", ":g/^$/d<CR>", "gg=G",
            ])));
            acts
        }
        100 | 101 => vec![Act::Key { key: "Copy".into() }, Act::Place { offset: pick_offset(rng), downstream: true, extend: false }, Act::PasteStored],
        98 => vec![Act::Pointer { offset: pick_offset(rng) }, Act::Place { offset: pick_offset(rng), downstream: true, extend: true }],
        99 => {
            let mut acts = vec![Act::SmartQuotes { enabled: true }];
            if !matches!(h.mode(), Mode::Insert | Mode::Replace) {
                acts.push(Act::Key { key: "a".into() });
            }
            for t in ["\"", "x", "'", " ", "\"", "it's"] {
                acts.push(Act::Text { text: t.into() });
            }
            acts
        }
        _ => {
            // Visual mode operations.
            let mut acts = vec![Act::Key { key: "Esc".into() }];
            acts.extend(keys(rng.pick(&["vjd", "vjjc", "Vjd", "Vd", "<C-v>jjd", "<C-v>jIx<Esc>", "vwd", "v$y", "Vjy", "vipd", "vjx", "Vj>", "Vj<", "vjJ", "vj~", "vjU", "<C-v>j$Ax<Esc>", "vlS", "vjo<Esc>"])));
            acts
        }
    }
}

fn walk(seed_index: u64, steps: usize, seeds: &[Seed], sink: &Sink) {
    let mut rng = Rng(seed_index.wrapping_mul(0x5851F42D4C957F2D) ^ 0xDEADBEEF);
    let seed = seeds[rng.below(seeds.len())].clone();
    let (w, hgt) = (*rng.pick(&[120., 300., 700.]), *rng.pick(&[60., 300., 1200.]));
    let Ok(mut h) = H::open(&seed, w, hgt) else { return };
    let mut step = 0;
    while step < steps {
        let acts = random_act(&mut rng, &h);
        for act in acts {
            step += 1;
            sink.probes.fetch_add(1, Ordering::Relaxed);
            let before = h.bytes();
            let r = h.act(&act);
            let pure = matches!(act, Act::Place { .. } | Act::Word { .. } | Act::Scroll { .. } | Act::Resize { .. }
                | Act::Wrap { .. } | Act::LineMode { .. } | Act::SelectAll | Act::Scale { .. } | Act::SmartQuotes { .. }
                | Act::Autoformat { .. });
            let ctx = format!("walk {seed_index} step {step} act {}", op_label(std::slice::from_ref(&act)));
            if pure && before != h.bytes() {
                sink.add(finding(&h, "pure_action_changed_source", act_kind(&act), ctx.clone(), String::new()));
            }
            if !r.is_ok() && !benign_status(&r) {
                sink.add(finding(&h, "walk_error", &format!("{} {}", act_kind(&act), error_signature(&r)), ctx.clone(), r.describe()));
                if before != h.bytes() {
                    sink.add(finding(&h, "failed_action_changed_source", act_kind(&act), ctx.clone(), r.describe()));
                }
                // Stuck check: can we still type and scroll?
                let snapshot_bytes = h.bytes();
                let _ = h.act(&Act::Key { key: "Esc".into() });
                let r1 = h.act(&Act::Key { key: "i".into() });
                let r2 = h.act(&Act::Text { text: "q".into() });
                let r3 = h.act(&Act::Scroll { top: 0., left: 0. });
                if !r1.is_ok() || !r2.is_ok() || !r3.is_ok() || h.bytes() == snapshot_bytes {
                    sink.add(finding(&h, "stuck_after_error", &format!("{} {}", act_kind(&act), error_signature(&r)), ctx.clone(),
                        format!("{} / {} / {}", r1.describe(), r2.describe(), r3.describe())));
                    return;
                }
                let _ = h.act(&Act::Key { key: "Esc".into() });
            }
            let after_input = matches!(act, Act::Key { .. } | Act::Text { .. } | Act::Paste { .. });
            let reproject = h.core.document().source_byte_len() < 20_000 || step % 25 == 0;
            let mut problems = h.check(after_input, reproject);
            problems.extend(h.check_other_views());
            for (label, secs) in std::mem::take(&mut h.slow) {
                sink.add(finding(&h, "slow_action", act_kind(&act), ctx.clone(), format!("{secs:.2}s {label}")));
            }
            let fatal = !problems.is_empty();
            for (kind, detail) in problems {
                sink.add(finding(&h, &kind, &format!("{} {}", act_kind(&act), normalize(&detail).chars().take(50).collect::<String>()), ctx.clone(), detail));
            }
            if fatal {
                return;
            }
        }
    }
}

fn act_kind(act: &Act) -> &'static str {
    match act {
        Act::Key { .. } => "key",
        Act::Text { .. } => "text",
        Act::Place { .. } => "place",
        Act::Word { .. } => "word",
        Act::Pointer { .. } => "pointer",
        Act::Paste { .. } => "paste",
        Act::Semantic { .. } => "semantic",
        Act::CharStyle { .. } => "char_style",
        Act::ParagraphStyle { .. } => "paragraph_style",
        Act::List { .. } => "list",
        Act::Quote { .. } => "quote",
        Act::Indent { .. } => "indent",
        Act::TableInsert { .. } => "table_insert",
        Act::TableRow { .. } => "table_row",
        Act::TableColumn { .. } => "table_column",
        Act::TableDeleteRow => "table_delete_row",
        Act::TableDeleteColumn => "table_delete_column",
        Act::ToggleSource => "toggle_source",
        Act::Resize { .. } => "resize",
        Act::Scroll { .. } => "scroll",
        Act::Wrap { .. } => "wrap",
        Act::LineMode { .. } => "line_mode",
        Act::SmartQuotes { .. } => "smart_quotes",
        Act::Autoformat { .. } => "autoformat",
        Act::SelectAll => "select_all",
        Act::History { .. } => "history",
        Act::Compose { .. } => "compose",
        Act::Scale { .. } => "scale",
        Act::ComposeOpen { .. } => "compose_open",
        Act::ComposeEnd { .. } => "compose_end",
        Act::SwitchView { .. } => "switch_view",
        Act::WaitSyntax => "wait_syntax",
        Act::LinkInsert { .. } => "link_insert",
        Act::LinkRemove => "link_remove",
        Act::ImageInsert { .. } => "image_insert",
        Act::PasteStored => "paste_stored",
        Act::ComposeReplacePrev { .. } => "compose_replace_prev",
    }
}

// ---------------------------------------------------------------------------
// Replay: {"seed": Seed, "trace": [Act]}

fn replay(path: &str) {
    let value: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let finding = value.get("finding").unwrap_or(&value);
    let seed = Seed {
        fmt: serde_json::from_value(finding["fmt"].clone()).unwrap(),
        source: finding["source"].as_str().unwrap().to_string(),
        filename: finding["filename"].as_str().map(str::to_string),
    };
    let trace: Vec<Act> = serde_json::from_value(finding["trace"].clone()).unwrap();
    run_trace(&seed, &trace, true);
}

fn run_trace(seed: &Seed, trace: &[Act], verbose: bool) -> Vec<String> {
    let (mut w, mut hgt) = (W_DEFAULT, H_);
    if let Some(Act::Resize { width, height }) = trace.first() {
        w = *width;
        hgt = *height;
    }
    let mut h = H::open(seed, w, hgt).unwrap();
    let mut log = Vec::new();
    for act in trace {
        let r = h.act(act);
        let problems = h.check(matches!(act, Act::Key { .. } | Act::Text { .. } | Act::Paste { .. }), true);
        let viewport = h.core.layout(h.view).map(|l| (l.viewport_left(), l.viewport_top())).unwrap_or_default();
        let line = format!(
            "{:?} -> {} | mode {:?} cursor {} xy {:?} vp {:?} text {:?} problems {:?}",
            act,
            r.describe(),
            h.mode(),
            h.cursor(),
            caret_xy(&h, h.cursor()),
            viewport,
            h.text().chars().take(200).collect::<String>(),
            problems
        );
        if verbose {
            println!("{line}");
            println!("    source {:?}", String::from_utf8_lossy(&h.bytes()).chars().take(300).collect::<String>());
        }
        log.push(line);
    }
    log
}

/// Re-run a trace and collect walk-style finding signatures.
fn evaluate_trace(seed: &Seed, trace: &[Act]) -> Vec<String> {
    let (mut w, mut hgt) = (W_DEFAULT, H_);
    if let Some(Act::Resize { width, height }) = trace.first() {
        w = *width;
        hgt = *height;
    }
    let Ok(mut h) = H::open(seed, w, hgt) else { return vec!["open failed".into()] };
    let mut out = Vec::new();
    for act in trace {
        let before = h.bytes();
        let r = h.act(act);
        if !r.is_ok() && !benign_status(&r) {
            out.push(format!("walk_error|{} {}", act_kind(act), error_signature(&r)));
            if before != h.bytes() {
                out.push(format!("failed_action_changed_source|{}", act_kind(act)));
            }
        }
        let pure = matches!(act, Act::Place { .. } | Act::Word { .. } | Act::Scroll { .. } | Act::Resize { .. }
            | Act::Wrap { .. } | Act::LineMode { .. } | Act::SelectAll | Act::Scale { .. } | Act::SmartQuotes { .. }
            | Act::Autoformat { .. });
        if pure && before != h.bytes() {
            out.push(format!("pure_action_changed_source|{}", act_kind(act)));
        }
        let after_input = matches!(act, Act::Key { .. } | Act::Text { .. } | Act::Paste { .. });
        let mut problems = h.check(after_input, true);
        problems.extend(h.check_other_views());
        for (kind, detail) in problems {
            out.push(format!("{kind}|{} {}", act_kind(act), normalize(&detail).chars().take(50).collect::<String>()));
        }
    }
    out
}

fn minimize(path: &str, want: Option<String>) {
    let value: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let finding = value.get("finding").unwrap_or(&value);
    let seed = Seed {
        fmt: serde_json::from_value(finding["fmt"].clone()).unwrap(),
        source: finding["source"].as_str().unwrap().to_string(),
        filename: finding["filename"].as_str().map(str::to_string),
    };
    let mut trace: Vec<Act> = serde_json::from_value(finding["trace"].clone()).unwrap();
    let target = want.unwrap_or_else(|| format!("{}|", finding["kind"].as_str().unwrap()));
    let reproduces = |t: &[Act]| {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| evaluate_trace(&seed, t)))
            .unwrap_or_else(|_| vec!["harness panic".into()])
            .iter()
            .any(|s| s.starts_with(&target) || s.contains(&target))
    };
    if !reproduces(&trace) {
        eprintln!("does not reproduce {target:?}; signatures: {:?}", evaluate_trace(&seed, &trace));
        return;
    }
    // Keep the first Resize; delta-debug the rest, then truncate after the hit.
    let mut chunk = (trace.len() / 2).max(1);
    while chunk >= 1 {
        let mut i = 1;
        let mut changed = false;
        while i < trace.len() {
            let end = (i + chunk).min(trace.len());
            let mut candidate = trace[..i].to_vec();
            candidate.extend_from_slice(&trace[end..]);
            if reproduces(&candidate) {
                trace = candidate;
                changed = true;
            } else {
                i += chunk;
            }
        }
        if !changed {
            if chunk == 1 { break; }
            chunk /= 2;
        }
    }
    // Truncate to the first action producing the signature.
    for n in 1..=trace.len() {
        if reproduces(&trace[..n]) {
            trace.truncate(n);
            break;
        }
    }
    println!("{}", serde_json::to_string(&json!({"finding": {"fmt": seed.fmt, "source": seed.source, "filename": seed.filename, "trace": trace, "kind": finding["kind"]}})).unwrap());
    eprintln!("minimized to {} actions", trace.len());
    run_trace(&seed, &trace, true);
}

// ---------------------------------------------------------------------------

fn parallel<F: Fn(&Seed) + Sync>(seeds: &[Seed], threads: usize, f: F) {
    let next = AtomicUsize::new(0);
    std::thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                if i >= seeds.len() {
                    break;
                }
                let seed = &seeds[i];
                let started = std::time::Instant::now();
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(seed)));
                let secs = started.elapsed().as_secs_f64();
                if secs > 20.0 {
                    eprintln!("SLOW SEED {secs:.1}s #{i} {:?} {:?}", seed.fmt, seed.source.chars().take(80).collect::<String>());
                }
                if let Err(panic) = result {
                    let message = panic
                        .downcast_ref::<String>()
                        .cloned()
                        .or_else(|| panic.downcast_ref::<&str>().map(|s| s.to_string()))
                        .unwrap_or_default();
                    eprintln!("PANIC on seed {:?} {:?}: {message}", seed.fmt, seed.source);
                }
            });
        }
    });
}

fn main() {
    if std::env::var_os("FUZZ_PANIC_BACKTRACE").is_some() {
        std::panic::set_hook(Box::new(|info| {
            eprintln!("panic: {info}\n{}", std::backtrace::Backtrace::force_capture());
        }));
    } else {
        std::panic::set_hook(Box::new(|_| {}));
    }
    let args: Vec<String> = std::env::args().collect();
    let arg = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    let out = PathBuf::from(arg("--out").unwrap_or_else(|| "target/consistency".into()));
    let threads = arg("--threads").and_then(|s| s.parse().ok()).unwrap_or(8);
    let filter = arg("--filter");
    let fmt_filter = arg("--fmt");
    let mode = args.get(1).cloned().unwrap_or_else(|| "probe".into());
    let mut seeds = all_seeds(mode == "walk" || args.iter().any(|a| a == "--files"));
    if let Some(f) = &filter {
        seeds.retain(|s| s.source.contains(f.as_str()));
    }
    if let Some(f) = &fmt_filter {
        seeds.retain(|s| format!("{:?}", s.fmt).to_lowercase() == f.to_lowercase());
    }
    if let Some(index) = arg("--seed-index").and_then(|s| s.parse::<usize>().ok()) {
        seeds = vec![seeds[index].clone()];
    }
    if let Some(range) = arg("--seed-range") {
        let (a, b) = range.split_once("..").unwrap();
        seeds = seeds[a.parse::<usize>().unwrap()..b.parse::<usize>().unwrap()].to_vec();
    }
    let started = std::time::Instant::now();
    match mode.as_str() {
        "count" => println!("{}", seeds.len()),
        "text" => {
            // text FMT FILE: print the projected text as JSON.
            let fmt: Fmt = serde_json::from_value(json!(args[2])).unwrap();
            let source = std::fs::read_to_string(&args[3]).unwrap();
            let h = H::open(&Seed { fmt, source, filename: None }, W_DEFAULT, H_).unwrap();
            println!("{}", serde_json::to_string(h.text()).unwrap());
        }
        "seed" => {
            for i in args[2].split(',') {
                let seed = &seeds[i.parse::<usize>().unwrap()];
                println!("{i}: {:?} {:?}", seed.fmt, seed.source);
            }
        }
        "replay" => replay(&args[2]),
        "vimcases" => {
            // vimcases OUT_JSONL [--positions N] [--sources JSON_FILE]: literal seeds only.
            let positions: usize = arg("--positions").and_then(|s| s.parse().ok()).unwrap_or(0);
            let sources: Vec<Seed> = match arg("--sources") {
                Some(path) => {
                    let list: Vec<serde_json::Value> = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
                    list.into_iter()
                        .map(|v| Seed {
                            fmt: serde_json::from_value(v["fmt"].clone()).unwrap_or(Fmt::Plain),
                            source: v["source"].as_str().unwrap().into(),
                            filename: v["filename"].as_str().map(Into::into),
                        })
                        .collect()
                }
                None => seeds.iter().filter(|s| matches!(s.fmt, Fmt::Plain | Fmt::Code)).cloned().collect(),
            };
            vim_cases(&sources, &args[2], positions);
        }
        "minimize" => minimize(&args[2], arg("--want")),
        "script" => {
            // script FMT WIDTH SOURCE ACTS_JSON
            let fmt: Fmt = serde_json::from_value(json!(args[2])).unwrap();
            let width: f32 = args[3].parse().unwrap();
            let source = args[4].replace("\\n", "\n");
            let acts: Vec<Act> = serde_json::from_str(&args[5]).unwrap();
            let seed = Seed { fmt, source, filename: arg("--filename") };
            let mut trace = vec![Act::Resize { width, height: 4000. }];
            trace.extend(acts);
            run_trace(&seed, &trace, true);
        }
        "probe" => {
            let which = arg("--probes").unwrap_or_else(|| "nav,insert,normal,selection".into());
            let positions: usize = arg("--positions").and_then(|s| s.parse().ok()).unwrap_or(40);
            for probe in which.split(',') {
                let sink = Sink::new();
                eprintln!("probe {probe} over {} seeds", seeds.len());
                match probe {
                    "nav" => parallel(&seeds, threads, |s| probe_navigation(s, &sink)),
                    "navpos" => parallel(&seeds, threads, |s| probe_navigation_positions(s, &sink, positions)),
                    "insert" => parallel(&seeds, threads, |s| probe_insert(s, &sink, positions)),
                    "normal" => parallel(&seeds, threads, |s| probe_normal(s, &sink, positions / 2)),
                    "selection" => parallel(&seeds, threads, |s| probe_selection(s, &sink, positions * 2)),
                    "copypaste" => parallel(&seeds, threads, |s| probe_copy_paste(s, &sink, positions)),
                    other => panic!("unknown probe {other}"),
                }
                sink.write(&out, probe);
                eprintln!("probe {probe} done in {:?}", started.elapsed());
            }
        }
        "walk" => {
            let first: u64 = arg("--seed").and_then(|s| s.parse().ok()).unwrap_or(1);
            let cases: u64 = arg("--cases").and_then(|s| s.parse().ok()).unwrap_or(100);
            let steps: usize = arg("--steps").and_then(|s| s.parse().ok()).unwrap_or(300);
            let sink = Sink::new();
            let ids: Vec<Seed> = (first..first + cases)
                .map(|i| Seed { fmt: Fmt::Plain, source: i.to_string(), filename: None })
                .collect();
            let all = seeds.clone();
            parallel(&ids, threads, |s| walk(s.source.parse().unwrap(), steps, &all, &sink));
            sink.write(&out, &format!("walk-{first}"));
            eprintln!("walk done in {:?}", started.elapsed());
        }
        other => panic!("unknown mode {other}"),
    }
}
