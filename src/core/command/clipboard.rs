//! Portable clipboard exchange used by Vim's `+` and `*` registers.
//!
//! Register policy stays in the command core while platform pasteboard I/O is
//! performed by an injected provider.  A command consumes an immutable read
//! snapshot prepared before its coordinator turn and emits owned write
//! requests after a successful turn; neither operation requires retaining a
//! mutable editor borrow while provider code runs.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use super::RegisterValue;

/// Vim clipboard-register identity.  Frontends may map both targets to one
/// native pasteboard, but core never aliases their state implicitly.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum ClipboardTarget {
    /// Vim's `+` register.
    Clipboard,
    /// Vim's `*` register (the platform primary selection where available).
    Primary,
}

impl ClipboardTarget {
    pub fn from_register(name: char) -> Option<Self> {
        match name {
            '+' => Some(Self::Clipboard),
            '*' => Some(Self::Primary),
            _ => None,
        }
    }

    pub fn register_name(self) -> char {
        match self {
            Self::Clipboard => '+',
            Self::Primary => '*',
        }
    }
}

/// Monotonic provider-local identity of clipboard contents.
///
/// The core treats this as an opaque comparison token.  It is not a document
/// revision and is never used as a persistent text position.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ClipboardGeneration(pub u64);

/// Cross-boundary clipboard contents.
///
/// Plain text is required.  `portable_register` is an optional richer Viem
/// representation preserving character/line/block shape and semantic hard
/// breaks.  Future rich styles can extend this record without weakening the
/// plain-text contract.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClipboardContent {
    plain_text: String,
    portable_register: Option<RegisterValue>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ClipboardContentError {
    PlainTextDoesNotMatchPortablePayload,
}

impl fmt::Display for ClipboardContentError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PlainTextDoesNotMatchPortablePayload => formatter
                .write_str("clipboard plain text does not match its portable register payload"),
        }
    }
}

impl std::error::Error for ClipboardContentError {}

impl ClipboardContent {
    /// Constructs plain external text.  Its U+000A values are interpreted as
    /// semantic line breaks when imported into an editor register.
    pub fn from_plain_text(text: impl Into<String>) -> Self {
        Self {
            plain_text: text.into(),
            portable_register: None,
        }
    }

    /// Constructs clipboard contents authored by the core, retaining the
    /// exact register shape and hard-break markers as the portable payload.
    pub fn from_register(value: RegisterValue) -> Self {
        if let Some(source) = value
            .clipboard_fragment()
            .and_then(|fragment| fragment.source_mode_text())
        {
            return Self::from_plain_text(source);
        }
        Self {
            plain_text: value.text.clone(),
            portable_register: Some(value),
        }
    }

    /// Validated constructor for a provider which supplies both forms.
    pub fn try_new(
        plain_text: impl Into<String>,
        portable_register: Option<RegisterValue>,
    ) -> Result<Self, ClipboardContentError> {
        let plain_text = plain_text.into();
        if portable_register
            .as_ref()
            .is_some_and(|value| value.text != plain_text)
        {
            return Err(ClipboardContentError::PlainTextDoesNotMatchPortablePayload);
        }
        Ok(Self {
            plain_text,
            portable_register,
        })
    }

    pub fn plain_text(&self) -> &str {
        &self.plain_text
    }

    pub fn portable_register(&self) -> Option<&RegisterValue> {
        self.portable_register.as_ref()
    }

    /// Resolves the content to an editor register.  Portable shape wins when
    /// available; mandatory plain-text fallback is characterwise and treats
    /// each U+000A as a semantic hard break.
    pub fn to_register(&self) -> RegisterValue {
        self.portable_register
            .clone()
            .unwrap_or_else(|| RegisterValue::characterwise(self.plain_text.clone()))
    }

    /// Interpret external plain text as prose input, without modifying the
    /// clipboard or exact Vim registers. Private payloads already describe
    /// their line breaks; only controls the destination cannot store change.
    pub(crate) fn to_paste_register(&self, format: crate::document::Format) -> RegisterValue {
        let value = self.to_register();
        let normalize_breaks = self.portable_register.is_none();
        let replace_null = format == crate::document::Format::Html;
        if !(normalize_breaks && value.text.contains('\r')
            || replace_null && value.text.contains('\0'))
        {
            return value;
        }
        let mut text = String::with_capacity(value.text.len());
        let mut breaks = Vec::with_capacity(value.hard_break_offsets().len());
        let mut characters = value.text.char_indices().peekable();
        while let Some((at, character)) = characters.next() {
            if normalize_breaks && character == '\r' {
                if characters.peek().is_some_and(|(_, next)| *next == '\n') {
                    characters.next();
                }
                breaks.push(text.len());
                text.push('\n');
            } else {
                if value.is_hard_break(at) {
                    breaks.push(text.len());
                }
                text.push(if replace_null && character == '\0' { '␀' } else { character });
            }
        }
        // A changed private fragment must not replay its original source over
        // the sanitized text. In particular, an RTF NUL pasted into HTML uses
        // the ordinary cross-format text path with the visible null marker.
        RegisterValue::try_new(text, value.kind, breaks)
            .expect("clipboard normalization preserves valid semantic break positions")
    }
}

/// Immutable provider read captured outside an editor coordinator turn.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClipboardSnapshot {
    target: ClipboardTarget,
    generation: ClipboardGeneration,
    content: ClipboardContent,
}

impl ClipboardSnapshot {
    pub fn new(
        target: ClipboardTarget,
        generation: ClipboardGeneration,
        content: ClipboardContent,
    ) -> Self {
        Self {
            target,
            generation,
            content,
        }
    }

    pub fn target(&self) -> ClipboardTarget {
        self.target
    }

    pub fn generation(&self) -> ClipboardGeneration {
        self.generation
    }

    pub fn content(&self) -> &ClipboardContent {
        &self.content
    }
}

/// Owned request emitted after a command transaction has committed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClipboardWriteRequest {
    target: ClipboardTarget,
    content: ClipboardContent,
}

impl ClipboardWriteRequest {
    pub fn new(target: ClipboardTarget, content: ClipboardContent) -> Self {
        Self { target, content }
    }

    pub fn from_register(target: ClipboardTarget, value: RegisterValue) -> Self {
        Self::new(target, ClipboardContent::from_register(value))
    }

    pub fn target(&self) -> ClipboardTarget {
        self.target
    }

    pub fn content(&self) -> &ClipboardContent {
        &self.content
    }
}

/// Clipboard capabilities and read snapshots supplied for one command turn.
///
/// Absence is explicit.  Merely naming `+` or `*` must not fall back to a
/// stale internal register slot.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ClipboardCommandContext {
    reads: BTreeMap<ClipboardTarget, ClipboardSnapshot>,
    writable: BTreeSet<ClipboardTarget>,
}

impl ClipboardCommandContext {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_read(mut self, snapshot: ClipboardSnapshot) -> Self {
        self.reads.insert(snapshot.target(), snapshot);
        self
    }

    pub fn with_write(mut self, target: ClipboardTarget) -> Self {
        self.writable.insert(target);
        self
    }

    pub fn read(&self, target: ClipboardTarget) -> Option<&ClipboardSnapshot> {
        self.reads.get(&target)
    }

    pub fn can_write(&self, target: ClipboardTarget) -> bool {
        self.writable.contains(&target)
    }
}

/// Narrow host interface.  Calls are made without an editor/core lock held.
pub trait ClipboardProvider {
    type Error;

    fn read(&mut self, target: ClipboardTarget) -> Result<ClipboardSnapshot, Self::Error>;

    fn write(&mut self, request: ClipboardWriteRequest)
        -> Result<ClipboardGeneration, Self::Error>;
}

/// Deterministic clipboard provider for command/coordinator tests.
#[derive(Clone, Debug)]
pub struct MemoryClipboardProvider {
    entries: BTreeMap<ClipboardTarget, (ClipboardGeneration, ClipboardContent)>,
    next_generation: Option<u64>,
    next_failure: Option<MemoryClipboardError>,
    reads: usize,
    writes: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MemoryClipboardError {
    Unavailable(ClipboardTarget),
    Injected(String),
    GenerationExhausted,
}

impl fmt::Display for MemoryClipboardError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable(target) => {
                write!(formatter, "clipboard target {target:?} is unavailable")
            }
            Self::Injected(message) => formatter.write_str(message),
            Self::GenerationExhausted => formatter.write_str("clipboard generation exhausted"),
        }
    }
}

impl std::error::Error for MemoryClipboardError {}

impl Default for MemoryClipboardProvider {
    fn default() -> Self {
        Self {
            entries: BTreeMap::new(),
            next_generation: Some(1),
            next_failure: None,
            reads: 0,
            writes: 0,
        }
    }
}

impl MemoryClipboardProvider {
    pub fn new() -> Self {
        Self::default()
    }

    /// Simulates an external clipboard update and returns its generation.
    pub fn set_external(
        &mut self,
        target: ClipboardTarget,
        content: ClipboardContent,
    ) -> Result<ClipboardGeneration, MemoryClipboardError> {
        let generation = self.allocate_generation()?;
        self.entries.insert(target, (generation, content));
        Ok(generation)
    }

    pub fn fail_next(&mut self, message: impl Into<String>) {
        self.next_failure = Some(MemoryClipboardError::Injected(message.into()));
    }

    pub fn read_count(&self) -> usize {
        self.reads
    }

    pub fn write_count(&self) -> usize {
        self.writes
    }

    pub fn content(&self, target: ClipboardTarget) -> Option<&ClipboardContent> {
        self.entries.get(&target).map(|(_, content)| content)
    }

    fn allocate_generation(&mut self) -> Result<ClipboardGeneration, MemoryClipboardError> {
        let value = self
            .next_generation
            .ok_or(MemoryClipboardError::GenerationExhausted)?;
        self.next_generation = value.checked_add(1);
        Ok(ClipboardGeneration(value))
    }

    fn take_failure(&mut self) -> Result<(), MemoryClipboardError> {
        match self.next_failure.take() {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}

impl ClipboardProvider for MemoryClipboardProvider {
    type Error = MemoryClipboardError;

    fn read(&mut self, target: ClipboardTarget) -> Result<ClipboardSnapshot, Self::Error> {
        self.reads = self.reads.saturating_add(1);
        self.take_failure()?;
        let (generation, content) = self
            .entries
            .get(&target)
            .cloned()
            .ok_or(MemoryClipboardError::Unavailable(target))?;
        Ok(ClipboardSnapshot::new(target, generation, content))
    }

    fn write(
        &mut self,
        request: ClipboardWriteRequest,
    ) -> Result<ClipboardGeneration, Self::Error> {
        self.writes = self.writes.saturating_add(1);
        self.take_failure()?;
        let generation = self.allocate_generation()?;
        self.entries
            .insert(request.target(), (generation, request.content));
        Ok(generation)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::{RegisterKind, RegisterValue};

    #[test]
    fn plain_text_fallback_and_portable_payload_have_explicit_break_semantics() {
        let plain = ClipboardContent::from_plain_text("a\nb");
        assert_eq!(plain.to_register().kind, RegisterKind::Characterwise);
        assert_eq!(plain.to_register().hard_break_offsets(), &[1]);

        let portable =
            RegisterValue::try_new("literal\nline\n", RegisterKind::Characterwise, vec![12])
                .unwrap();
        let content = ClipboardContent::from_register(portable.clone());
        assert_eq!(content.plain_text(), portable.text);
        assert_eq!(content.to_register(), portable);
    }

    #[test]
    fn mismatched_plain_and_portable_forms_are_rejected() {
        assert_eq!(
            ClipboardContent::try_new("plain", Some(RegisterValue::characterwise("different"))),
            Err(ClipboardContentError::PlainTextDoesNotMatchPortablePayload)
        );
    }

    #[test]
    fn memory_provider_keeps_plus_and_star_distinct() {
        let mut provider = MemoryClipboardProvider::new();
        let plus_generation = provider
            .set_external(
                ClipboardTarget::Clipboard,
                ClipboardContent::from_plain_text("plus"),
            )
            .unwrap();
        let star_generation = provider
            .set_external(
                ClipboardTarget::Primary,
                ClipboardContent::from_plain_text("star"),
            )
            .unwrap();
        assert_ne!(plus_generation, star_generation);

        assert_eq!(
            provider
                .read(ClipboardTarget::Clipboard)
                .unwrap()
                .content()
                .plain_text(),
            "plus"
        );
        assert_eq!(
            provider
                .read(ClipboardTarget::Primary)
                .unwrap()
                .content()
                .plain_text(),
            "star"
        );
    }

    #[test]
    fn failed_fake_write_is_atomic_and_retryable() {
        let mut provider = MemoryClipboardProvider::new();
        provider
            .set_external(
                ClipboardTarget::Clipboard,
                ClipboardContent::from_plain_text("old"),
            )
            .unwrap();
        let request = ClipboardWriteRequest::from_register(
            ClipboardTarget::Clipboard,
            RegisterValue::linewise("new\n"),
        );
        provider.fail_next("pasteboard unavailable");
        assert_eq!(
            provider.write(request.clone()),
            Err(MemoryClipboardError::Injected(
                "pasteboard unavailable".into()
            ))
        );
        assert_eq!(
            provider
                .content(ClipboardTarget::Clipboard)
                .unwrap()
                .plain_text(),
            "old"
        );

        let generation = provider.write(request).unwrap();
        assert!(generation.0 > 0);
        assert_eq!(
            provider
                .content(ClipboardTarget::Clipboard)
                .unwrap()
                .portable_register()
                .unwrap()
                .kind,
            RegisterKind::Linewise
        );
    }

    #[test]
    fn command_context_distinguishes_read_and_write_availability() {
        let target = ClipboardTarget::Clipboard;
        let snapshot = ClipboardSnapshot::new(
            target,
            ClipboardGeneration(7),
            ClipboardContent::from_plain_text("contents"),
        );
        let reads_only = ClipboardCommandContext::new().with_read(snapshot.clone());
        assert_eq!(reads_only.read(target), Some(&snapshot));
        assert!(!reads_only.can_write(target));

        let writes_only = ClipboardCommandContext::new().with_write(target);
        assert!(writes_only.read(target).is_none());
        assert!(writes_only.can_write(target));
    }
}

#[cfg(test)]
mod rich_tests {
    use super::*;
    use crate::command::{CommandInterpreter, CommandStatus, InputEvent, Key, Mode};
    use crate::document::{Document, Encoding, FileFormat, Format};

    fn open(source: &[u8], format: Format) -> Document {
        Document::from_bytes_with_file_format(
            source.to_vec(),
            Encoding::Utf8,
            format,
            FileFormat::Unix,
        )
        .unwrap()
    }
    fn key(
        commands: &mut CommandInterpreter,
        document: &mut Document,
        context: &ClipboardCommandContext,
        character: char,
    ) -> crate::command::CommandOutput {
        let output = commands
            .handle_with_clipboard_context(document, InputEvent::Key(Key::Char(character)), context)
            .unwrap();
        assert!(
            matches!(
                output.status,
                CommandStatus::Complete | CommandStatus::Pending
            ),
            "{:?}",
            output.status
        );
        output
    }
    #[test]
    fn star_copy_alias_has_yank_motion_and_visual_semantics_without_changing_other_c() {
        let context = ClipboardCommandContext::new().with_write(ClipboardTarget::Primary);
        for command in ["\"*cw", "\"*cc", "vll\"*c", "\"*2cw"] {
            let mut document = open(b"one two three", Format::PlainText);
            let mut commands = CommandInterpreter::new();
            let mut writes = Vec::new();
            for c in command.chars() {
                writes.extend(key(&mut commands, &mut document, &context, c).clipboard_writes);
            }
            assert_eq!(document.source_bytes(), b"one two three", "{command}");
            assert_eq!(commands.mode(), Mode::Normal, "{command}");
            assert_eq!(writes.len(), 1, "{command}");
            assert!(!writes[0].content().plain_text().is_empty());
            if command == "\"*cc" {
                assert_eq!(writes[0].content().plain_text(), "one two three");
            }
        }
        let mut document = open(b"one two", Format::PlainText);
        let mut commands = CommandInterpreter::new();
        for c in "cw".chars() {
            key(&mut commands, &mut document, &context, c);
        }
        assert_eq!(commands.mode(), Mode::Insert);
        assert_eq!(document.text(), " two");
    }

    #[test]
    fn wrapped_visual_row_clipboard_yanks_preserve_source_and_resolved_styles() {
        use crate::layout::MockTextMeasurementProvider;
        use crate::{Core, CoreEvent};

        let source = b"<p><B title='keep'>abcdefgh ijklmnop qrstuvwxyz</B></p>";
        for input in ["\"*yy", "\"*cc", "V\"*y", "V\"*c"] {
            let mut core = Core::new(open(source, Format::Html));
            let view = core.add_view(MockTextMeasurementProvider::new(), 55.0, 200.0);
            let row = core.layout(view).unwrap().snapshot().unwrap().rows[0]
                .text_range
                .clone();
            assert!(row.end < core.document().text().len());
            let selected = core.document().text()[row].to_owned();
            let context = ClipboardCommandContext::new().with_write(ClipboardTarget::Primary);
            let mut writes = Vec::new();
            for character in input.chars() {
                let result = core
                    .handle(
                        view,
                        CoreEvent::InputWithClipboard {
                            input: InputEvent::Key(Key::Char(character)),
                            clipboard: context.clone(),
                        },
                    )
                    .unwrap();
                let output = result.command.unwrap();
                assert!(
                    matches!(
                        output.status,
                        CommandStatus::Complete | CommandStatus::Pending
                    ),
                    "{input}: {:?}",
                    output.status
                );
                writes.extend(output.clipboard_writes);
            }
            assert_eq!(writes.len(), 1, "{input}");
            let content = writes[0].content();
            assert_eq!(content.plain_text(), selected, "{input}");
            let fragment = content
                .portable_register()
                .unwrap()
                .clipboard_fragment()
                .unwrap();
            let value: serde_json::Value = serde_json::from_str(fragment.json()).unwrap();
            assert_eq!(value["character_runs"][0]["bold"], true);
            assert!(value["source_text"]
                .as_str()
                .unwrap()
                .contains("<B title='keep'>"));
            assert_eq!(core.document().source_bytes(), source);
            assert_eq!(core.command_state(view).unwrap().mode(), Mode::Normal);
        }
    }

    #[test]
    fn copy_alias_uses_visible_projection_in_each_line_policy_without_synthetic_eof_break() {
        use crate::command::LineMode;
        use crate::layout::MockTextMeasurementProvider;
        use crate::{Core, CoreEvent};
        for (format, source, expected) in [
            (Format::PlainText, "word", "word"),
            (Format::MarkdownSource, "__word__", "__word__"),
            (
                Format::HtmlSource,
                "<p><b>word</b></p>",
                "<p><b>word</b></p>",
            ),
            (Format::Markdown, "__word__", "word"),
            (Format::Html, "<p><b>word</b></p>", "word"),
        ] {
            for policy in [LineMode::Visual, LineMode::PhysicalSource] {
                for input in ["V\"*c", "\"*cc"] {
                    let mut core = Core::new(open(source.as_bytes(), format));
                    let view = core.add_view(MockTextMeasurementProvider::new(), 1000.0, 200.0);
                    core.handle(view, CoreEvent::SetLineMode(policy)).unwrap();
                    let context =
                        ClipboardCommandContext::new().with_write(ClipboardTarget::Primary);
                    let mut writes = Vec::new();
                    for character in input.chars() {
                        let output = core
                            .handle(
                                view,
                                CoreEvent::InputWithClipboard {
                                    input: InputEvent::Key(Key::Char(character)),
                                    clipboard: context.clone(),
                                },
                            )
                            .unwrap()
                            .command
                            .unwrap();
                        assert!(
                            matches!(
                                output.status,
                                CommandStatus::Complete | CommandStatus::Pending
                            ),
                            "{input} {format:?} {policy:?}: {:?}",
                            output.status
                        );
                        writes.extend(output.clipboard_writes);
                    }
                    assert_eq!(writes.len(), 1);
                    assert_eq!(
                        writes[0].content().plain_text(),
                        expected,
                        "{input} {format:?} {policy:?}"
                    );
                    assert_eq!(core.document().source_bytes(), source.as_bytes());
                    assert_eq!(core.command_state(view).unwrap().mode(), Mode::Normal);
                }
            }
        }
    }

    #[test]
    fn rectangular_clipboard_yank_and_copy_alias_preserve_segment_payloads() {
        use crate::layout::MockTextMeasurementProvider;
        use crate::{Core, CoreEvent};
        let source = b"<p><b>ab</b> outside-one<br><b>ab</b> outside-two</p>";
        for operation in ['y', 'c'] {
            let mut core = Core::new(open(source, Format::Html));
            let view = core.add_view(MockTextMeasurementProvider::new(), 1000.0, 200.0);
            let context = ClipboardCommandContext::new().with_write(ClipboardTarget::Primary);
            let mut writes = Vec::new();
            for key in [
                Key::Ctrl('v'),
                Key::Char('l'),
                Key::Char('j'),
                Key::Char('"'),
                Key::Char('*'),
                Key::Char(operation),
            ] {
                let output = core
                    .handle(
                        view,
                        CoreEvent::InputWithClipboard {
                            input: InputEvent::Key(key),
                            clipboard: context.clone(),
                        },
                    )
                    .unwrap()
                    .command
                    .unwrap();
                assert!(
                    matches!(
                        output.status,
                        CommandStatus::Complete | CommandStatus::Pending
                    ),
                    "{key:?}: {:?}",
                    output.status
                );
                writes.extend(output.clipboard_writes);
            }
            assert_eq!(writes.len(), 1);
            let content = writes[0].content();
            assert_eq!(content.plain_text(), "ab\nab");
            let fragment = content
                .portable_register()
                .unwrap()
                .clipboard_fragment()
                .unwrap();
            let value: serde_json::Value = serde_json::from_str(fragment.json()).unwrap();
            assert_eq!(value["source_segments"].as_array().unwrap().len(), 2);
            assert!(!fragment.json().contains("outside"));
            assert!(crate::document::ClipboardFragment::from_json(
                fragment.json(),
                content.plain_text()
            )
            .is_ok());
            assert_eq!(core.document().source_bytes(), source);
        }
    }

    #[test]
    fn clipboard_yank_and_insert_paste_roundtrip_authored_source_without_touching_ordinary_yanks() {
        for (format, source) in [
            (Format::Markdown, b"__bold__".as_slice()),
            (Format::Html, b"<P><B title='x'>A&#38;B</B></P>".as_slice()),
        ] {
            let mut source_document = open(source, format);
            let mut commands = CommandInterpreter::new();
            let context = ClipboardCommandContext::new().with_write(ClipboardTarget::Clipboard);
            let mut output = None;
            for c in "v$\"+y".chars() {
                output = Some(key(&mut commands, &mut source_document, &context, c));
            }
            let content = output.unwrap().clipboard_writes[0].content().clone();
            let fragment = content
                .portable_register()
                .unwrap()
                .clipboard_fragment()
                .unwrap();
            assert!(fragment.json().contains("source_bytes"));
            let context = ClipboardCommandContext::new().with_read(ClipboardSnapshot::new(
                ClipboardTarget::Clipboard,
                ClipboardGeneration(1),
                content,
            ));
            let mut target = open(b"", format);
            let mut commands = CommandInterpreter::new();
            key(&mut commands, &mut target, &context, 'i');
            commands
                .handle_with_clipboard_context(
                    &mut target,
                    InputEvent::Key(Key::Ctrl('r')),
                    &context,
                )
                .unwrap();
            key(&mut commands, &mut target, &context, '+');
            commands
                .handle_with_clipboard_context(&mut target, InputEvent::Key(Key::Escape), &context)
                .unwrap();
            assert_eq!(target.source_bytes(), source);
            assert!(target.undo());
            assert!(target.source_bytes().is_empty());
            assert!(target.redo());
            assert_eq!(target.source_bytes(), source);
            assert_eq!(source_document.source_bytes(), source);

            let mut ordinary = CommandInterpreter::new();
            for c in "v$y".chars() {
                key(
                    &mut ordinary,
                    &mut source_document,
                    &ClipboardCommandContext::new(),
                    c,
                );
            }
            assert!(ordinary
                .register('0')
                .unwrap()
                .clipboard_fragment()
                .is_none());
        }
    }
}
