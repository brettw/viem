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

impl super::CommandInterpreter {
    /// Platform Copy leaves every selection endpoint and presentation mode in
    /// place. Vim's yank operator retains its separate cursor/mode semantics.
    pub(super) fn copy_platform_selection(
        &mut self,
        document: &crate::document::Document,
        layout: Option<&super::LayoutCommandContext<'_>>,
    ) -> Result<super::CommandOutput, crate::document::DocumentError> {
        use super::*;
        if layout.is_some_and(|context| context.snapshot.document_id != document.id()
            || context.snapshot.document_revision != document.revision())
        {
            return Ok(CommandOutput {
                status: CommandStatus::Error("stale layout context".to_owned()),
                ..CommandOutput::complete()
            });
        }
        if !matches!(self.mode, Mode::VisualCharacter | Mode::VisualLine | Mode::VisualBlock) {
            return Ok(CommandOutput::complete());
        }
        if let Err(output) = self.require_register_write(Some('+')) { return Ok(output); }
        let value = if self.mode == Mode::VisualBlock {
            let Some(layout) = layout else { return Ok(layout_required("copy block selection")) };
            let resolved = match self.resolved_visual_block(document, layout) {
                Ok(resolved) => resolved,
                Err(error) => return Ok(visual_block_error(error)),
            };
            block_register_value(document, &resolved, Some('+'))
        } else {
            let range = if self.mode == Mode::VisualLine {
                self.line_selection_range(document, layout.map(|context| context.snapshot))
                    .unwrap_or_else(|| self.visual_extent(document).range)
            } else { self.visual_extent(document).range };
            if range.is_empty() { return Ok(CommandOutput::complete()); }
            RegisterValue::from_clipboard_fragment(document.clipboard_fragment(range)?.as_seen())
                .map_err(|_| crate::document::DocumentError::UnsupportedFormatting)?
        };
        // Copy is an out-of-band platform action. In particular, do not
        // consume a partially typed register prefix or pending key mapping.
        let effect = self.registers.yank(Some('+'), value);
        let mut output = CommandOutput::complete();
        if let Some(request) = effect.clipboard() {
            output.clipboard_writes.push(request);
        }
        Ok(output)
    }
}

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
        if let Some(mut source) = value
            .clipboard_fragment()
            .and_then(|fragment| fragment.source_mode_text())
        {
            // Source views publish their literal source as interoperable plain
            // text rather than a Viem-private payload. Preserve Vim's only
            // portable linewise marker when the final physical source line did
            // not already have one.
            if value.kind == super::RegisterKind::Linewise
                && !has_linewise_clipboard_terminator(&source)
            {
                source.push('\n');
            }
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
    /// available. Like Vim, mandatory plain-text fallback is linewise when it
    /// ends in CR or LF and characterwise otherwise; each U+000A is a semantic
    /// hard break.
    pub fn to_register(&self) -> RegisterValue {
        self.portable_register.clone().unwrap_or_else(|| {
            if has_linewise_clipboard_terminator(&self.plain_text) {
                RegisterValue::linewise(self.plain_text.clone())
            } else {
                RegisterValue::characterwise(self.plain_text.clone())
            }
        })
    }

    /// Interpret external plain text as prose input, without modifying the
    /// clipboard or exact Vim registers. Private payloads already describe
    /// their line breaks; only controls the destination cannot store change.
    pub(crate) fn to_paste_register(&self) -> RegisterValue {
        let value = self.to_register();
        let normalize_breaks = self.portable_register.is_none();
        if !normalize_breaks || !value.text.contains('\r') {
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
                text.push(character);
            }
        }
        RegisterValue::try_new(text, value.kind, breaks)
            .expect("clipboard normalization preserves valid semantic break positions")
    }
}

fn has_linewise_clipboard_terminator(text: &str) -> bool {
    text.ends_with('\n') || text.ends_with('\r')
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
    fn plain_clipboard_trailing_line_break_is_linewise_like_vim() {
        for text in ["one\n", "one\r", "one\r\n"] {
            assert_eq!(
                ClipboardContent::from_plain_text(text).to_register().kind,
                RegisterKind::Linewise,
                "{text:?}"
            );
        }
        for text in ["one", "one\ntwo"] {
            assert_eq!(
                ClipboardContent::from_plain_text(text).to_register().kind,
                RegisterKind::Characterwise,
                "{text:?}"
            );
        }
    }

    #[test]
    fn mismatched_plain_and_portable_forms_are_rejected() {
        assert_eq!(
            ClipboardContent::try_new("plain", Some(RegisterValue::characterwise("different"))),
            Err(ClipboardContentError::PlainTextDoesNotMatchPortablePayload)
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
    use crate::command::{CommandInterpreter, CommandStatus, InputEvent, Key, Mode, RegisterKind};
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
    fn star_line_yank_keeps_the_unnamed_register_linewise_for_bare_put() {
        let context = ClipboardCommandContext::new().with_write(ClipboardTarget::Primary);
        let mut document = open(b"one\ntwo", Format::PlainText);
        let mut commands = CommandInterpreter::new();
        for character in "\"*yy".chars() {
            key(&mut commands, &mut document, &context, character);
        }
        assert_eq!(commands.register('"').unwrap().kind, RegisterKind::Linewise);

        key(&mut commands, &mut document, &context, 'p');
        assert_eq!(document.text(), "one\none\ntwo");
    }

    #[test]
    fn source_mode_line_yank_round_trips_through_plain_system_clipboard() {
        let write_context = ClipboardCommandContext::new().with_write(ClipboardTarget::Primary);
        let mut source = open(b"one", Format::Code);
        let mut commands = CommandInterpreter::new();
        let mut writes = Vec::new();
        for character in "\"*yy".chars() {
            writes.extend(
                key(&mut commands, &mut source, &write_context, character).clipboard_writes,
            );
        }
        assert_eq!(writes.len(), 1);
        let content = writes.pop().unwrap().content().clone();
        assert_eq!(content.plain_text(), "one\n");
        assert!(content.portable_register().is_none());

        let read_context = ClipboardCommandContext::new().with_read(ClipboardSnapshot::new(
            ClipboardTarget::Primary,
            ClipboardGeneration(1),
            content,
        ));
        let mut target = open(b"tail", Format::Code);
        let mut commands = CommandInterpreter::new();
        for character in "\"*p".chars() {
            key(&mut commands, &mut target, &read_context, character);
        }
        assert_eq!(target.text(), "tail\none");
    }

    #[test]
    fn wrapped_visual_row_clipboard_yanks_preserve_source_and_resolved_styles() {
        use crate::layout::MockTextMeasurementProvider;
        use crate::{Core, CoreEvent};

        let source = b"__abcdefgh ijklmnop qrstuvwxyz__";
        for input in ["\"*yy", "\"*cc", "V\"*y", "V\"*c"] {
            let mut core = Core::new(open(source, Format::Markdown));
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
                .contains("__"));
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
            (Format::Markdown, "__word__", "word"),
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
        let source = b"**ab** outside-one\\\n**ab** outside-two";
        for operation in ['y', 'c'] {
            let mut core = Core::new(open(source, Format::Markdown));
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
    #[test]
    fn platform_copy_preserves_directed_selection_and_vim_yank_still_exits() {
        use crate::command::{Mode, SelectionOrigin};
        use crate::layout::MockTextMeasurementProvider;
        use crate::{Core, CoreEvent};
        for source in ["**one two** three\n\nfour five", "abcdef ghijkl mnopqr stuvwx yz"] {
            for sequence in ["vll", "vllo", "V", "Vjo", "\u{16}lj", "\u{16}ljo"] {
                for native in [false, true] {
                    let mut core = Core::new(open(source.as_bytes(), Format::Markdown));
                    let view = core.add_view(MockTextMeasurementProvider::new(), 85., 200.);
                    for ch in sequence.chars() {
                        let key = if ch == '\u{16}' { Key::Ctrl('v') } else { Key::Char(ch) };
                        core.handle(view, CoreEvent::Input(InputEvent::Key(key))).unwrap();
                    }
                    if native { core.set_selection_origin(view, SelectionOrigin::Mouse, Mode::Insert).unwrap(); }
                    let before = core.command_state(view).unwrap().clone();
                    let revision = core.document().revision();
                    let history = core.document().history_status();
                    let context = ClipboardCommandContext::new().with_write(ClipboardTarget::Clipboard);
                    let mut previous = None;
                    for _ in 0..2 {
                        let output = core.handle(view, CoreEvent::InputWithClipboard {
                            input: InputEvent::Key(Key::CopySelection), clipboard: context.clone(),
                        }).unwrap().command.unwrap();
                        assert_eq!(output.status, CommandStatus::Complete, "{sequence}");
                        assert!(!output.document_changed && !output.cursor_moved && !output.mode_changed);
                        assert_eq!(output.clipboard_writes.len(), 1);
                        let copy = output.clipboard_writes[0].content();
                        assert_eq!(output.clipboard_writes[0].target(), ClipboardTarget::Clipboard);
                        assert!(!copy.plain_text().is_empty());
                        assert!(copy.portable_register().unwrap().clipboard_fragment().is_some());
                        if let Some(previous) = &previous { assert_eq!(copy, previous); }
                        previous = Some(copy.clone());
                        let after = core.command_state(view).unwrap();
                        assert_eq!(after.mode, before.mode);
                        assert_eq!(after.cursor, before.cursor);
                        assert_eq!(after.visual_anchor, before.visual_anchor);
                        assert_eq!(after.boundary_affinity, before.boundary_affinity);
                        assert_eq!(after.selection_behavior, before.selection_behavior);
                        assert_eq!(after.selection_exclusive, before.selection_exclusive);
                        assert_eq!(after.selection_return_mode, before.selection_return_mode);
                        assert_eq!(after.visual_to_line_end, before.visual_to_line_end);
                        assert_eq!(after.visual_block, before.visual_block);
                        assert_eq!(after.visual_position, before.visual_position);
                        assert_eq!(after.desired_x, before.desired_x);
                        assert_eq!(core.document().revision(), revision);
                        assert_eq!(core.document().history_status(), history);
                        assert_eq!(core.document().source_bytes(), source.as_bytes());
                    }
                    if !native {
                        core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Char('y')))).unwrap();
                        assert_eq!(core.command_state(view).unwrap().mode(), Mode::Normal);
                    }
                }
            }
        }
    }

    #[test]
    fn platform_copy_keeps_exact_final_newline_and_rich_or_source_content() {
        use crate::layout::MockTextMeasurementProvider;
        use crate::{Core, CoreEvent};
        for (source, format, plain) in [
            ("__one__ two", Format::Markdown, "one two"),
            ("one two", Format::PlainText, "one two"),
            ("one two\n", Format::PlainText, "one two\n"),
            ("__one__ two", Format::MarkdownSource, "__one__ two"),
        ] {
            let mut core = Core::new(open(source.as_bytes(), format));
            let view = core.add_view(MockTextMeasurementProvider::new(), 400., 200.);
            core.handle(view, CoreEvent::Input(InputEvent::Key(Key::SelectAll))).unwrap();
            let before = core.list_selection_identity(view).unwrap();
            let output = core.handle(view, CoreEvent::InputWithClipboard {
                input: InputEvent::Key(Key::CopySelection),
                clipboard: ClipboardCommandContext::new().with_write(ClipboardTarget::Clipboard),
            }).unwrap().command.unwrap();
            assert_eq!(output.status, CommandStatus::Complete);
            assert_eq!(core.list_selection_identity(view).unwrap(), before);
            let content = output.clipboard_writes[0].content();
            assert_eq!(content.plain_text(), plain);
            if format.is_source_view() { assert!(content.portable_register().is_none()); }
            else if format.is_wysiwyg() {
                let fragment = content.portable_register().unwrap().clipboard_fragment().unwrap();
                let json: serde_json::Value = serde_json::from_str(fragment.json()).unwrap();
                assert_eq!(json["character_runs"][0]["bold"], true);
            }
        }
    }

    fn assert_pending_copy_state(before: &CommandInterpreter, after: &CommandInterpreter) {
        assert_eq!(after.mode, before.mode);
        assert_eq!(after.cursor, before.cursor);
        assert_eq!(after.visual_anchor, before.visual_anchor);
        assert_eq!(after.boundary_affinity, before.boundary_affinity);
        assert_eq!(after.visual_block, before.visual_block);
        assert_eq!(after.visual_position, before.visual_position);
        assert_eq!(after.selection_behavior, before.selection_behavior);
        assert_eq!(after.selection_exclusive, before.selection_exclusive);
        assert_eq!(after.pending, before.pending);
        assert_eq!(after.count, before.count);
        assert_eq!(after.mapping_pending, before.mapping_pending);
        assert_eq!(after.requested_register, before.requested_register);
        assert_eq!(after.register_pending, before.register_pending);
        assert_eq!(after.clipboard_copy_as_seen, before.clipboard_copy_as_seen);
        assert_eq!(after.select_visual_once, before.select_visual_once);
        assert_eq!(after.select_visual_just_started, before.select_visual_just_started);
        assert_eq!(after.select_visual_return, before.select_visual_return);
        assert_eq!(after.insert_normal_once, before.insert_normal_once);
        assert_eq!(after.ctrl_o_just_started, before.ctrl_o_just_started);
        assert_eq!(after.recording, before.recording);
        assert_eq!(after.insert_controls.join_next_horizontal, before.insert_controls.join_next_horizontal);
    }

    #[test]
    fn platform_copy_preserves_pending_headless_commands_and_macro_recording() {
        let context = ClipboardCommandContext::new().with_write(ClipboardTarget::Clipboard);
        for (prefix, continuation) in [("vld", 'd'), ("vl\"a", 'y'), ("vlr", 'X'), ("vl3", 'l'), ("qavl", 'y')] {
            let mut document = Document::new("abcdef");
            let mut commands = CommandInterpreter::new();
            commands.mappings.execute("vmap dd y").unwrap().unwrap();
            for character in prefix.chars() { key(&mut commands, &mut document, &context, character); }
            let before = commands.clone();
            let revision = document.revision();
            let output = commands.handle_with_clipboard_context(&mut document,
                InputEvent::Key(Key::CopySelection), &context).unwrap();
            assert_eq!(output.clipboard_writes.len(), 1, "{prefix}");
            assert_eq!(output.clipboard_writes[0].content().plain_text(), "ab");
            assert_eq!(document.revision(), revision);
            assert_eq!(document.text(), "abcdef");
            assert_pending_copy_state(&before, &commands);
            key(&mut commands, &mut document, &context, continuation);
            match prefix {
                "vld" => { assert_eq!(commands.mode(), Mode::Normal); assert_eq!(document.text(), "abcdef"); }
                "vl\"a" => assert_eq!(commands.register('a').unwrap().text, "ab"),
                "vlr" => assert_eq!(document.text(), "XXcdef"),
                "vl3" => assert_eq!(commands.cursor(), 4),
                _ => assert_eq!(document.text(), "abcdef"),
            }
        }
    }

    #[test]
    fn platform_copy_bypasses_core_mapping_replay_and_preserves_temporary_visual_mode() {
        use crate::layout::MockTextMeasurementProvider;
        use crate::{Core, CoreEvent};
        for prefix in ["vld", "Vd", "\u{16}ld"] {
            let mut core = Core::new(Document::new("abcdef\nghijkl"));
            assert!(core.initialize_startup("vmap dd y").is_empty());
            let view = core.add_view(MockTextMeasurementProvider::new(), 400., 200.);
            for character in prefix.chars() {
                let key = if character == '\u{16}' { Key::Ctrl('v') } else { Key::Char(character) };
                core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(key))).unwrap();
            }
            let before = core.command_state(view).unwrap().clone();
            let revision = core.document().revision();
            let output = core.handle_with_layout(view, CoreEvent::InputWithClipboard {
                input: InputEvent::Key(Key::CopySelection),
                clipboard: ClipboardCommandContext::new().with_write(ClipboardTarget::Clipboard),
            }).unwrap().command.unwrap();
            assert_eq!(output.clipboard_writes.len(), 1, "{prefix}");
            assert_eq!(core.document().revision(), revision);
            assert_pending_copy_state(&before, core.command_state(view).unwrap());
            core.handle_with_layout(view, CoreEvent::Input(InputEvent::key('d'))).unwrap();
            assert_eq!(core.document().text(), "abcdef\nghijkl");
            assert_eq!(core.command_state(view).unwrap().mode(), Mode::Normal);
        }
        let mut core = Core::new(Document::new("abcdef"));
        let view = core.add_view(MockTextMeasurementProvider::new(), 400., 200.);
        for key in [Key::SelectAll, Key::Ctrl('o')] {
            core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(key))).unwrap();
        }
        let before = core.command_state(view).unwrap().clone();
        let output = core.handle_with_layout(view, CoreEvent::InputWithClipboard {
            input: InputEvent::Key(Key::CopySelection),
            clipboard: ClipboardCommandContext::new().with_write(ClipboardTarget::Clipboard),
        }).unwrap().command.unwrap();
        assert_eq!(output.clipboard_writes[0].content().plain_text(), "abcdef");
        assert_pending_copy_state(&before, core.command_state(view).unwrap());
        core.handle_with_layout(view, CoreEvent::Input(InputEvent::key('y'))).unwrap();
        assert_eq!(core.document().text(), "abcdef");
        assert_eq!(core.command_state(view).unwrap().register('0').unwrap().text, "abcdef");
    }

}
