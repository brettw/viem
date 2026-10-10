use std::collections::BTreeMap;
use std::fmt;

use super::clipboard::{ClipboardCommandContext, ClipboardTarget, ClipboardWriteRequest};
use super::{InputEvent, Key};
use crate::document::ArtifactPath;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RegisterKind {
    Characterwise,
    Linewise,
    /// Newline-separated display rows captured by a Visual Block operation.
    /// The final row has no implicit trailing newline.
    Blockwise,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RegisterValue {
    pub text: String,
    pub kind: RegisterKind,
    /// Sorted byte offsets of U+000A values which represent semantic hard
    /// breaks. An unlisted U+000A is ordinary formatted content. In a
    /// blockwise register, row-separator U+000A values are deliberately
    /// unmarked because they describe display rows, not document structure.
    hard_break_offsets: Vec<usize>,
    /// Source/style image captured before a yank or deletion. Text-only
    /// register transformations clear it rather than keep stale provenance.
    pub(crate) clipboard_fragment: Option<crate::document::ClipboardFragment>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RegisterValueError {
    BreakOffsetOutOfBounds { offset: usize, text_length: usize },
    BreakOffsetIsNotLineFeed { offset: usize },
    BreakOffsetsNotStrictlyIncreasing { previous: usize, offset: usize },
    BlockwiseSemanticBreak { offset: usize },
}

impl fmt::Display for RegisterValueError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BreakOffsetOutOfBounds {
                offset,
                text_length,
            } => write!(
                formatter,
                "register break offset {offset} is outside text length {text_length}"
            ),
            Self::BreakOffsetIsNotLineFeed { offset } => write!(
                formatter,
                "register break offset {offset} does not name U+000A"
            ),
            Self::BreakOffsetsNotStrictlyIncreasing { previous, offset } => write!(
                formatter,
                "register break offsets are not strictly increasing at {previous}, {offset}"
            ),
            Self::BlockwiseSemanticBreak { offset } => write!(
                formatter,
                "blockwise row separator at {offset} cannot be a semantic hard break"
            ),
        }
    }
}

impl std::error::Error for RegisterValueError {}

/// Semantic classification of text removed by one delete/change command.
///
/// Vim's numbered-register exception depends on the resolved motion, not just
/// on whether the captured bytes happen to contain a newline. Keeping this
/// typed prevents register policy from reverse-engineering command intent from
/// the payload after the fact.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DeletionClass {
    /// An ordinary deletion of less than one hard line.
    Small,
    /// A linewise, block-multiline, or otherwise at-least-one-line deletion.
    Large,
    /// One of Vim's exceptional delete motions (`%`, sentence, paragraph,
    /// mark, or search family), with its resolved line containment retained.
    Exceptional { within_line: bool },
}

/// External state needed to resolve non-stored registers for one command
/// turn. It is deliberately borrowed and never retained by [`Registers`].
#[derive(Clone, Copy, Debug)]
pub struct RegisterReadContext<'a> {
    clipboard: &'a ClipboardCommandContext,
    artifact_path: Option<&'a ArtifactPath>,
}

impl<'a> RegisterReadContext<'a> {
    pub fn new(
        clipboard: &'a ClipboardCommandContext,
        artifact_path: Option<&'a ArtifactPath>,
    ) -> Self {
        Self {
            clipboard,
            artifact_path,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RegisterReadError {
    ClipboardUnavailable(ClipboardTarget),
    NoCurrentArtifact,
    ArtifactPathIsNotUtf8,
    /// A recorded macro has a normalized key event which cannot be
    /// losslessly materialized as formatted text. Its inspection notation is
    /// deliberately not treated as literal register contents.
    MacroContainsNonTextKeys {
        register: char,
    },
}

impl fmt::Display for RegisterReadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ClipboardUnavailable(target) => {
                write!(
                    formatter,
                    "clipboard register {} is unavailable",
                    target.register_name()
                )
            }
            Self::NoCurrentArtifact => formatter.write_str("current filename register is empty"),
            Self::ArtifactPathIsNotUtf8 => formatter
                .write_str("current filename cannot be represented as a Unicode text register"),
            Self::MacroContainsNonTextKeys { register } => write!(
                formatter,
                "macro register {register} contains non-text keys and cannot be put"
            ),
        }
    }
}

impl std::error::Error for RegisterReadError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RegisterWriteError {
    ReadOnly(char),
    ClipboardUnavailable(ClipboardTarget),
}

impl fmt::Display for RegisterWriteError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ReadOnly(name) => write!(formatter, "register {name} is read-only"),
            Self::ClipboardUnavailable(target) => {
                write!(
                    formatter,
                    "clipboard register {} is unavailable",
                    target.register_name()
                )
            }
        }
    }
}

impl std::error::Error for RegisterWriteError {}

/// Success-dependent external effect of a register write. This plain value is
/// intentionally not `must_use` while legacy command call sites are migrated;
/// the command interpreter collects it before publishing a completed turn.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct RegisterWriteEffect {
    clipboard: Option<ClipboardWriteRequest>,
}

impl RegisterWriteEffect {
    pub(crate) fn clipboard(self) -> Option<ClipboardWriteRequest> {
        self.clipboard
    }
}

impl RegisterValue {
    pub fn characterwise(text: impl Into<String>) -> Self {
        let text = text.into();
        let hard_break_offsets = line_feed_offsets(&text);
        Self::try_new(text, RegisterKind::Characterwise, hard_break_offsets)
            .expect("line-feed offsets derived from register text are valid")
    }

    pub fn linewise(text: impl Into<String>) -> Self {
        let text = text.into();
        let hard_break_offsets = line_feed_offsets(&text);
        Self::try_new(text, RegisterKind::Linewise, hard_break_offsets)
            .expect("line-feed offsets derived from register text are valid")
    }

    pub fn blockwise(text: impl Into<String>) -> Self {
        Self::try_new(text, RegisterKind::Blockwise, Vec::new())
            .expect("an unmarked blockwise register is valid")
    }

    /// Creates a register whose semantic hard breaks are explicit. This is
    /// the lossless constructor used when capturing formatted content; unlike
    /// the convenience constructors, it never infers meaning from U+000A.
    pub fn try_new(
        text: impl Into<String>,
        kind: RegisterKind,
        hard_break_offsets: Vec<usize>,
    ) -> Result<Self, RegisterValueError> {
        let text = text.into();
        validate_break_offsets(&text, kind, &hard_break_offsets)?;
        Ok(Self {
            text,
            kind,
            hard_break_offsets,
            clipboard_fragment: None,
        })
    }

    pub fn hard_break_offsets(&self) -> &[usize] {
        &self.hard_break_offsets
    }

    pub fn clipboard_fragment(&self) -> Option<&crate::document::ClipboardFragment> {
        self.clipboard_fragment.as_ref()
    }

    /// Creates a register from a validated portable clipboard fragment,
    /// retaining its styles, register shape, and semantic hard breaks.
    pub fn from_clipboard_fragment(fragment: crate::document::ClipboardFragment) -> Result<Self, RegisterValueError> {
        let (text, kind, breaks) = fragment.register_parts();
        let kind = match kind { 2 => RegisterKind::Linewise, 3 => RegisterKind::Blockwise, _ => RegisterKind::Characterwise };
        let mut result = Self::try_new(text, kind, breaks)?;
        result.clipboard_fragment = Some(fragment);
        Ok(result)
    }

    pub(crate) fn is_hard_break(&self, offset: usize) -> bool {
        self.hard_break_offsets.binary_search(&offset).is_ok()
    }

    /// Concatenates an actually inserted payload without applying named-
    /// register shape transitions. Used by Vim's characterwise `.` register.
    pub(crate) fn append_inserted_payload(&mut self, fragment: &Self) {
        self.clipboard_fragment = None;
        let appended_at = self.text.len();
        self.text.push_str(&fragment.text);
        self.hard_break_offsets.extend(
            fragment
                .hard_break_offsets
                .iter()
                .map(|offset| appended_at + *offset),
        );
        debug_assert!(
            validate_break_offsets(&self.text, self.kind, &self.hard_break_offsets).is_ok()
        );
    }

    pub(crate) fn truncate_inserted_payload(&mut self, new_length: usize) {
        self.clipboard_fragment = None;
        debug_assert!(self.text.is_char_boundary(new_length));
        self.text.truncate(new_length);
        self.hard_break_offsets
            .retain(|offset| *offset < new_length);
    }
}

fn line_feed_offsets(text: &str) -> Vec<usize> {
    text.bytes()
        .enumerate()
        .filter_map(|(offset, byte)| (byte == b'\n').then_some(offset))
        .collect()
}

fn validate_break_offsets(
    text: &str,
    kind: RegisterKind,
    hard_break_offsets: &[usize],
) -> Result<(), RegisterValueError> {
    let mut previous = None;
    for &offset in hard_break_offsets {
        if offset >= text.len() {
            return Err(RegisterValueError::BreakOffsetOutOfBounds {
                offset,
                text_length: text.len(),
            });
        }
        if let Some(previous) = previous {
            if previous >= offset {
                return Err(RegisterValueError::BreakOffsetsNotStrictlyIncreasing {
                    previous,
                    offset,
                });
            }
        }
        if text.as_bytes()[offset] != b'\n' {
            return Err(RegisterValueError::BreakOffsetIsNotLineFeed { offset });
        }
        if kind == RegisterKind::Blockwise {
            return Err(RegisterValueError::BlockwiseSemanticBreak { offset });
        }
        previous = Some(offset);
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct StoredRegister {
    value: RegisterValue,
    /// Present only when this slot was authored by macro recording. The text
    /// value remains the UTF-8 inspection projection; this event program is
    /// the sole authority for macro replay.
    replay: Option<Vec<InputEvent>>,
}

impl StoredRegister {
    fn text(value: RegisterValue) -> Self {
        Self {
            value,
            replay: None,
        }
    }

    fn macro_program(events: Vec<InputEvent>) -> Self {
        Self {
            value: macro_events_as_register_value(&events),
            replay: Some(events),
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Registers {
    values: BTreeMap<char, StoredRegister>,
    /// Vim's last-insert register. Kept separate from writable slots so a
    /// register-targeted yank/delete can never overwrite it accidentally.
    last_insert: Option<RegisterValue>,
}

impl Default for Registers {
    fn default() -> Self {
        let mut values = BTreeMap::new();
        values.insert('"', StoredRegister::text(RegisterValue::characterwise("")));
        Self {
            values,
            last_insert: None,
        }
    }
}

impl Registers {
    pub(crate) fn names(&self) -> Vec<char> {
        let mut names = self.values.keys().copied().collect::<Vec<_>>();
        if self.last_insert.is_some() {
            names.push('.');
        }
        names.sort_unstable();
        names.dedup();
        names
    }

    pub(crate) fn get(&self, name: char) -> Option<&RegisterValue> {
        if name == '.' {
            return self.last_insert.as_ref();
        }
        let normalized = normalize_read_name(name)?;
        self.values.get(&normalized).map(|stored| &stored.value)
    }

    pub(crate) fn read(
        &self,
        name: char,
        context: RegisterReadContext<'_>,
    ) -> Result<Option<RegisterValue>, RegisterReadError> {
        if let Some(target) = ClipboardTarget::from_register(name) {
            return context
                .clipboard
                .read(target)
                .map(|snapshot| Some(snapshot.content().to_register()))
                .ok_or(RegisterReadError::ClipboardUnavailable(target));
        }
        if name == '%' {
            let path = context
                .artifact_path
                .ok_or(RegisterReadError::NoCurrentArtifact)?;
            let path = std::str::from_utf8(path.as_bytes())
                .map_err(|_| RegisterReadError::ArtifactPathIsNotUtf8)?;
            return Ok(Some(RegisterValue::characterwise(path)));
        }
        if name == '_' {
            return Ok(Some(RegisterValue::characterwise("")));
        }
        Ok(self.get(name).cloned())
    }

    /// Resolves a register for insertion into formatted text. Recorded macro
    /// programs with non-text keys are rejected instead of inserting their
    /// human-readable inspection notation.
    pub(crate) fn read_for_put(
        &self,
        name: char,
        context: RegisterReadContext<'_>,
    ) -> Result<Option<RegisterValue>, RegisterReadError> {
        if let Some(normalized) = normalize_named(name) {
            if let Some(events) = self
                .values
                .get(&normalized)
                .and_then(|stored| stored.replay.as_deref())
            {
                return macro_events_as_put_value(events).map(Some).ok_or(
                    RegisterReadError::MacroContainsNonTextKeys {
                        register: normalized,
                    },
                );
            }
        }
        self.read(name, context)
    }

    pub(crate) fn validate_write(
        &self,
        requested: Option<char>,
        clipboard: &ClipboardCommandContext,
    ) -> Result<(), RegisterWriteError> {
        let Some(name) = requested else {
            return Ok(());
        };
        if matches!(name, '.' | '%') {
            return Err(RegisterWriteError::ReadOnly(name));
        }
        if let Some(target) = ClipboardTarget::from_register(name) {
            if !clipboard.can_write(target) {
                return Err(RegisterWriteError::ClipboardUnavailable(target));
            }
        }
        Ok(())
    }

    pub(crate) fn set_last_insert(&mut self, value: RegisterValue) {
        self.last_insert = Some(value);
    }

    pub(crate) fn yank(
        &mut self,
        requested: Option<char>,
        value: RegisterValue,
    ) -> RegisterWriteEffect {
        if requested == Some('_') {
            return RegisterWriteEffect::default();
        }
        let (unnamed, effect) = self.write_explicit(requested, &value);
        if requested.is_none() || requested == Some('"') {
            self.values.insert('0', StoredRegister::text(value));
        }
        self.values.insert('"', StoredRegister::text(unnamed));
        effect
    }

    pub(crate) fn delete(
        &mut self,
        requested: Option<char>,
        value: RegisterValue,
        class: DeletionClass,
    ) -> RegisterWriteEffect {
        if requested == Some('_') {
            return RegisterWriteEffect::default();
        }
        let (unnamed, effect) = self.write_explicit(requested, &value);
        // Vim's small-delete exception applies only when no register was
        // specified. Any explicit writable destination (including `"`, `-`,
        // or a clipboard register) also records the deletion in register 1;
        // the black-hole register returned above remains wholly isolated.
        let rotates_numbered = requested.is_some()
            || matches!(
                class,
                DeletionClass::Large | DeletionClass::Exceptional { .. }
            );
        let writes_small = requested.is_none()
            && matches!(
                class,
                DeletionClass::Small | DeletionClass::Exceptional { within_line: true }
            );
        if rotates_numbered {
            self.rotate_numbered(value.clone());
        }
        if writes_small {
            self.values.insert('-', StoredRegister::text(value.clone()));
        }
        self.values.insert('"', StoredRegister::text(unnamed));
        effect
    }

    pub(crate) fn macro_events(&self, name: char) -> Option<Vec<InputEvent>> {
        let normalized = normalize_named(name)?;
        let stored = self.values.get(&normalized)?;
        stored
            .replay
            .clone()
            .or_else(|| Some(register_value_as_macro_events(&stored.value)))
    }

    pub(crate) fn set_macro(&mut self, name: char, events: Vec<InputEvent>) {
        let Some(normalized) = normalize_named(name) else {
            return;
        };
        self.values
            .insert(normalized, StoredRegister::macro_program(events));
    }

    fn write_explicit(
        &mut self,
        requested: Option<char>,
        value: &RegisterValue,
    ) -> (RegisterValue, RegisterWriteEffect) {
        let Some(name) = requested else {
            return (value.clone(), RegisterWriteEffect::default());
        };
        if name.is_ascii_uppercase() {
            let destination = name.to_ascii_lowercase();
            let previous = self.values.get(&destination).cloned();
            let combined = previous.as_ref().map_or_else(
                || value.clone(),
                |existing| append_value(&existing.value, value),
            );
            let replay = previous.as_ref().and_then(|existing| {
                existing.replay.as_ref().map(|events| {
                    let mut events = events.clone();
                    events.extend(register_suffix_as_macro_events(
                        &combined,
                        existing.value.text.len(),
                    ));
                    events
                })
            });
            self.values.insert(
                destination,
                StoredRegister {
                    value: combined.clone(),
                    replay,
                },
            );
            (combined, RegisterWriteEffect::default())
        } else if let Some(target) = ClipboardTarget::from_register(name) {
            (
                value.clone(),
                RegisterWriteEffect {
                    clipboard: Some(ClipboardWriteRequest::from_register(target, value.clone())),
                },
            )
        } else if is_writable(name) {
            self.values
                .insert(name, StoredRegister::text(value.clone()));
            (value.clone(), RegisterWriteEffect::default())
        } else {
            // `validate_write` rejects read-only destinations before an edit
            // is prepared. Retain this defensive fallback for compatibility
            // call sites while keeping special registers out of stored slots.
            (value.clone(), RegisterWriteEffect::default())
        }
    }

    fn rotate_numbered(&mut self, value: RegisterValue) {
        for number in (2_u8..=9).rev() {
            let previous = char::from(b'0' + number - 1);
            let current = char::from(b'0' + number);
            if let Some(contents) = self.values.get(&previous).cloned() {
                self.values.insert(current, contents);
            } else {
                self.values.remove(&current);
            }
        }
        self.values.insert('1', StoredRegister::text(value));
    }
}

fn append_value(existing: &RegisterValue, appended: &RegisterValue) -> RegisterValue {
    if existing.text.is_empty() {
        return appended.clone();
    }

    let kind = if existing.kind == RegisterKind::Linewise || appended.kind == RegisterKind::Linewise
    {
        RegisterKind::Linewise
    } else {
        // Vim preserves the destination's character/block shape when neither
        // side is linewise.
        existing.kind
    };
    let mut text = existing.text.clone();
    let mut hard_break_offsets = existing.hard_break_offsets.clone();
    if existing.kind == RegisterKind::Blockwise || appended.kind == RegisterKind::Linewise {
        let trailing_is_semantic = text
            .len()
            .checked_sub(1)
            .is_some_and(|offset| existing.is_hard_break(offset));
        if !text.ends_with('\n') || (kind == RegisterKind::Linewise && !trailing_is_semantic) {
            if kind == RegisterKind::Linewise {
                hard_break_offsets.push(text.len());
            }
            text.push('\n');
        }
    }
    let appended_at = text.len();
    text.push_str(&appended.text);
    hard_break_offsets.extend(appended.hard_break_offsets.iter().map(|offset| {
        appended_at
            .checked_add(*offset)
            .expect("combined register length is representable")
    }));
    let trailing_is_semantic = text
        .len()
        .checked_sub(1)
        .is_some_and(|offset| hard_break_offsets.binary_search(&offset).is_ok());
    if kind == RegisterKind::Linewise && (!text.ends_with('\n') || !trailing_is_semantic) {
        hard_break_offsets.push(text.len());
        text.push('\n');
    }
    if kind == RegisterKind::Blockwise {
        // Once Vim keeps the destination's blockwise shape, every U+000A in
        // its flat representation separates display rows. It must not later
        // be serialized as a document hard break merely because the appended
        // characterwise fragment originally carried that meaning.
        hard_break_offsets.clear();
    }
    RegisterValue::try_new(text, kind, hard_break_offsets)
        .expect("appending validated register values preserves their invariants")
}

fn normalize_named(name: char) -> Option<char> {
    name.is_ascii_alphabetic()
        .then(|| name.to_ascii_lowercase())
}

fn register_value_as_macro_events(value: &RegisterValue) -> Vec<InputEvent> {
    value
        .text
        .char_indices()
        .map(|(offset, character)| {
            if character == '\n' && value.is_hard_break(offset) {
                InputEvent::Key(Key::Enter)
            } else {
                InputEvent::Key(Key::Char(character))
            }
        })
        .collect()
}

fn register_suffix_as_macro_events(value: &RegisterValue, start: usize) -> Vec<InputEvent> {
    debug_assert!(value.text.is_char_boundary(start));
    value.text[start..]
        .char_indices()
        .map(|(relative, character)| {
            let offset = start + relative;
            if character == '\n' && value.is_hard_break(offset) {
                InputEvent::Key(Key::Enter)
            } else {
                InputEvent::Key(Key::Char(character))
            }
        })
        .collect()
}

fn macro_events_as_put_value(events: &[InputEvent]) -> Option<RegisterValue> {
    let mut text = String::new();
    for event in events {
        match event {
            InputEvent::Text(value) => text.push_str(value),
            InputEvent::Key(Key::Char(character)) => text.push(*character),
            InputEvent::Key(Key::Tab) => text.push('\t'),
            // Vim records Enter as a carriage-return byte. In the formatted
            // register projection it is literal content, not a semantic hard
            // break; a destination line-ending stage remains responsible for
            // rejecting any ambiguous reverse projection.
            InputEvent::Key(Key::Enter) => text.push('\r'),
            InputEvent::Key(Key::Escape) => text.push('\u{1b}'),
            InputEvent::Key(Key::Ctrl(character)) => {
                text.push(ctrl_key_as_c0(*character)?);
            }
            InputEvent::Key(
                Key::ModifiedNavigation { .. } | Key::Function { .. }
                | Key::ShiftEnter
                | Key::BackTab
                | Key::Backspace
                | Key::Delete
                | Key::Left
                | Key::Right
                | Key::WordLeft
                | Key::WordRight
                | Key::ParagraphStart | Key::ParagraphEnd | Key::NextParagraph
                | Key::Up
                | Key::Down
                | Key::Home
                | Key::End
                | Key::DocumentStart
                | Key::DocumentEnd
                | Key::SelectAll | Key::CopySelection | Key::PasteClipboard
                | Key::PageUp
                | Key::PageDown,
            ) => return None,
        }
    }
    Some(
        RegisterValue::try_new(text, RegisterKind::Characterwise, Vec::new())
            .expect("macro control characters form a valid literal text register"),
    )
}

fn ctrl_key_as_c0(character: char) -> Option<char> {
    if character.is_ascii_alphabetic() {
        let byte = character.to_ascii_uppercase() as u8 - b'@';
        Some(char::from(byte))
    } else if character == '[' {
        Some('\u{1b}')
    } else {
        None
    }
}

fn macro_events_as_register_value(events: &[InputEvent]) -> RegisterValue {
    let mut text = String::new();
    for event in events {
        match event {
            InputEvent::Text(value) => text.push_str(value),
            InputEvent::Key(Key::Char(character)) => text.push(*character),
            InputEvent::Key(Key::Enter) => text.push_str("<Enter>"),
            InputEvent::Key(Key::ShiftEnter) => text.push_str("<S-Enter>"),
            InputEvent::Key(Key::Tab) => text.push_str("<Tab>"),
            InputEvent::Key(Key::BackTab) => text.push_str("<S-Tab>"),
            InputEvent::Key(Key::Escape) => text.push_str("<Esc>"),
            InputEvent::Key(Key::Backspace) => text.push_str("<BS>"),
            InputEvent::Key(Key::Delete) => text.push_str("<Del>"),
            InputEvent::Key(Key::Left) => text.push_str("<Left>"),
            InputEvent::Key(Key::Right) => text.push_str("<Right>"),
            InputEvent::Key(Key::WordLeft) => text.push_str("<C-Left>"),
            InputEvent::Key(Key::WordRight) => text.push_str("<C-Right>"),
            InputEvent::Key(Key::ParagraphStart) => text.push_str("<ParagraphStart>"),
            InputEvent::Key(Key::ParagraphEnd) => text.push_str("<ParagraphEnd>"),
            InputEvent::Key(Key::NextParagraph) => text.push_str("<NextParagraph>"),
            InputEvent::Key(Key::Up) => text.push_str("<Up>"),
            InputEvent::Key(Key::Down) => text.push_str("<Down>"),
            InputEvent::Key(Key::Home) => text.push_str("<Home>"),
            InputEvent::Key(Key::End) => text.push_str("<End>"),
            InputEvent::Key(Key::DocumentStart) => text.push_str("<C-Home>"),
            InputEvent::Key(Key::DocumentEnd) => text.push_str("<C-End>"),
            InputEvent::Key(Key::SelectAll) => text.push_str("<SelectAll>"),
            InputEvent::Key(Key::CopySelection) => text.push_str("<CopySelection>"),
            InputEvent::Key(Key::PasteClipboard) => text.push_str("<PasteClipboard>"),
            InputEvent::Key(Key::PageUp) => text.push_str("<PageUp>"),
            InputEvent::Key(Key::PageDown) => text.push_str("<PageDown>"),
            InputEvent::Key(key @ (Key::Function { .. } | Key::ModifiedNavigation { .. })) => text.push_str(&super::literal_input::literal_key_text(*key).unwrap()),
            InputEvent::Key(Key::Ctrl(character)) => {
                text.push_str("<C-");
                text.push(*character);
                text.push('>');
            }
        }
    }
    RegisterValue::try_new(text, RegisterKind::Characterwise, Vec::new())
        .expect("macro inspection notation has valid literal line feeds")
}

fn normalize_read_name(name: char) -> Option<char> {
    if matches!(name, '"' | '-' | '_' | '.') || name.is_ascii_digit() {
        Some(name)
    } else if name.is_ascii_alphabetic() {
        Some(name.to_ascii_lowercase())
    } else {
        None
    }
}

pub(crate) fn is_valid_register(name: char) -> bool {
    matches!(name, '"' | '-' | '_' | '+' | '*' | '.' | '%')
        || name.is_ascii_digit()
        || name.is_ascii_alphabetic()
}

fn is_writable(name: char) -> bool {
    name == '"' || name == '-' || name.is_ascii_digit() || name.is_ascii_alphabetic()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uppercase_append_uses_vim_kind_transitions_and_publishes_the_full_value() {
        let cases = [
            (
                RegisterValue::characterwise("char"),
                RegisterValue::linewise("line\n"),
                RegisterValue::linewise("char\nline\n"),
            ),
            (
                RegisterValue::linewise("line\n"),
                RegisterValue::characterwise("char"),
                RegisterValue::linewise("line\nchar\n"),
            ),
            (
                RegisterValue::blockwise("aa\nbb"),
                RegisterValue::characterwise("c"),
                RegisterValue::blockwise("aa\nbb\nc"),
            ),
            (
                RegisterValue::characterwise("c"),
                RegisterValue::blockwise("aa\nbb"),
                RegisterValue::try_new("caa\nbb", RegisterKind::Characterwise, Vec::new()).unwrap(),
            ),
            (
                RegisterValue::blockwise("aa\nbb"),
                RegisterValue::blockwise("cc\ndd"),
                RegisterValue::blockwise("aa\nbb\ncc\ndd"),
            ),
        ];

        for (initial, appended, expected) in cases {
            let mut registers = Registers::default();
            registers.yank(Some('a'), initial);
            registers.yank(Some('A'), appended);
            assert_eq!(registers.get('a'), Some(&expected));
            assert_eq!(registers.get('"'), Some(&expected));
        }
    }

    #[test]
    fn explicit_break_offsets_distinguish_literal_lf_and_validate_structure() {
        let literal =
            RegisterValue::try_new("a\nb", RegisterKind::Characterwise, Vec::new()).unwrap();
        let semantic =
            RegisterValue::try_new("a\nb", RegisterKind::Characterwise, vec![1]).unwrap();
        assert_ne!(literal, semantic);
        assert!(literal.hard_break_offsets().is_empty());
        assert_eq!(semantic.hard_break_offsets(), &[1]);

        assert_eq!(
            RegisterValue::try_new("abc", RegisterKind::Characterwise, vec![1]),
            Err(RegisterValueError::BreakOffsetIsNotLineFeed { offset: 1 })
        );
        assert_eq!(
            RegisterValue::try_new("\n", RegisterKind::Blockwise, vec![0]),
            Err(RegisterValueError::BlockwiseSemanticBreak { offset: 0 })
        );
    }

    #[test]
    fn uppercase_append_rebases_markers_without_promoting_block_row_delimiters() {
        let mut registers = Registers::default();
        registers.yank(
            Some('a'),
            RegisterValue::try_new("literal\n", RegisterKind::Characterwise, Vec::new()).unwrap(),
        );
        registers.yank(Some('A'), RegisterValue::linewise("line\n"));
        let appended = registers.get('a').unwrap();
        assert_eq!(appended.text, "literal\n\nline\n");
        assert_eq!(appended.hard_break_offsets(), &[8, 13]);

        registers.yank(Some('b'), RegisterValue::blockwise("aa\nbb"));
        registers.yank(Some('B'), RegisterValue::characterwise("tail"));
        let block = registers.get('b').unwrap();
        assert_eq!(block.text, "aa\nbb\ntail");
        assert!(block.hard_break_offsets().is_empty());

        registers.yank(Some('B'), RegisterValue::characterwise("x\ny"));
        let block = registers.get('b').unwrap();
        assert_eq!(block.text, "aa\nbb\ntail\nx\ny");
        assert!(block.hard_break_offsets().is_empty());
    }

    #[test]
    fn uppercase_delete_append_keeps_full_destination_unnamed_but_rotates_only_fragment() {
        let mut registers = Registers::default();
        registers.yank(Some('a'), RegisterValue::characterwise("head"));
        registers.delete(
            Some('A'),
            RegisterValue::linewise("body\n"),
            DeletionClass::Large,
        );

        let expected = RegisterValue::linewise("head\nbody\n");
        assert_eq!(registers.get('a'), Some(&expected));
        assert_eq!(registers.get('"'), Some(&expected));
        assert_eq!(registers.get('1'), Some(&RegisterValue::linewise("body\n")));
    }

    #[test]
    fn deletion_class_drives_small_and_numbered_register_policy() {
        let mut registers = Registers::default();
        registers.values.insert(
            '-',
            StoredRegister::text(RegisterValue::characterwise("old-small")),
        );
        registers.values.insert(
            '1',
            StoredRegister::text(RegisterValue::linewise("old-large\n")),
        );

        registers.delete(
            Some('a'),
            RegisterValue::characterwise("named-small"),
            DeletionClass::Small,
        );
        assert_eq!(registers.get('-').unwrap().text, "old-small");
        assert_eq!(registers.get('1').unwrap().text, "named-small");
        assert_eq!(registers.get('2').unwrap().text, "old-large\n");

        registers.delete(
            Some('"'),
            RegisterValue::characterwise("quoted-small"),
            DeletionClass::Small,
        );
        assert_eq!(registers.get('-').unwrap().text, "old-small");
        assert_eq!(registers.get('1').unwrap().text, "quoted-small");
        assert_eq!(registers.get('2').unwrap().text, "named-small");
        assert_eq!(registers.get('3').unwrap().text, "old-large\n");

        registers.delete(
            None,
            RegisterValue::characterwise("ordinary-small"),
            DeletionClass::Small,
        );
        assert_eq!(registers.get('-').unwrap().text, "ordinary-small");
        assert_eq!(registers.get('1').unwrap().text, "quoted-small");

        registers.delete(
            Some('b'),
            RegisterValue::linewise("named-large\n"),
            DeletionClass::Large,
        );
        assert_eq!(registers.get('1').unwrap().text, "named-large\n");
        assert_eq!(registers.get('2').unwrap().text, "quoted-small");
        assert_eq!(registers.get('3').unwrap().text, "named-small");
        assert_eq!(registers.get('4').unwrap().text, "old-large\n");
        assert_eq!(registers.get('b').unwrap().text, "named-large\n");
    }

    #[test]
    fn explicit_numbered_destination_is_written_before_vim_rotation() {
        for (requested, expected_two, expected_three) in [
            ('1', "deleted\n", "old-two\n"),
            ('3', "old-one\n", "old-two\n"),
        ] {
            let mut registers = Registers::default();
            registers.values.insert(
                '1',
                StoredRegister::text(RegisterValue::linewise("old-one\n")),
            );
            registers.values.insert(
                '2',
                StoredRegister::text(RegisterValue::linewise("old-two\n")),
            );
            registers.values.insert(
                '3',
                StoredRegister::text(RegisterValue::linewise("old-three\n")),
            );

            registers.delete(
                Some(requested),
                RegisterValue::linewise("deleted\n"),
                DeletionClass::Large,
            );

            assert_eq!(registers.get('1').unwrap().text, "deleted\n");
            assert_eq!(registers.get('2').unwrap().text, expected_two);
            assert_eq!(registers.get('3').unwrap().text, expected_three);
            assert_eq!(registers.get('"').unwrap().text, "deleted\n");
        }
    }

    #[test]
    fn black_hole_does_not_touch_any_automatic_register() {
        let mut registers = Registers::default();
        registers.yank(None, RegisterValue::characterwise("kept"));
        registers.delete(
            None,
            RegisterValue::characterwise("old-small"),
            DeletionClass::Small,
        );
        registers.delete(
            None,
            RegisterValue::linewise("old-large\n"),
            DeletionClass::Large,
        );
        registers.delete(
            Some('_'),
            RegisterValue::characterwise("discarded"),
            DeletionClass::Exceptional { within_line: true },
        );
        registers.yank(Some('_'), RegisterValue::characterwise("discarded yank"));

        assert_eq!(registers.get('0').unwrap().text, "kept");
        assert_eq!(registers.get('-').unwrap().text, "old-small");
        assert_eq!(registers.get('1').unwrap().text, "old-large\n");
        assert_eq!(registers.get('"').unwrap().text, "old-large\n");
    }

    #[test]
    fn macro_program_is_authoritative_and_inspection_notation_is_not_put_text() {
        let mut registers = Registers::default();
        let events = vec![
            InputEvent::Text("<Left>\nliteral".to_owned()),
            InputEvent::Key(Key::Left),
            InputEvent::Key(Key::Enter),
        ];
        registers.set_macro('a', events.clone());

        let inspected = registers.get('a').unwrap();
        assert_eq!(inspected.text, "<Left>\nliteral<Left><Enter>");
        assert!(
            inspected.hard_break_offsets().is_empty(),
            "inspection text never invents a semantic hard break"
        );
        assert_eq!(registers.macro_events('a'), Some(events));

        let clipboard = ClipboardCommandContext::new();
        assert_eq!(
            registers.read_for_put('a', RegisterReadContext::new(&clipboard, None)),
            Err(RegisterReadError::MacroContainsNonTextKeys { register: 'a' })
        );
    }

    #[test]
    fn literal_angle_notation_remains_literal_text_and_overwrites_macro_program() {
        let mut registers = Registers::default();
        registers.set_macro('a', vec![InputEvent::Key(Key::Left)]);
        registers.yank(Some('a'), RegisterValue::characterwise("<Left>"));

        let clipboard = ClipboardCommandContext::new();
        assert_eq!(
            registers
                .read_for_put('a', RegisterReadContext::new(&clipboard, None))
                .unwrap(),
            Some(RegisterValue::characterwise("<Left>"))
        );
        assert_eq!(
            registers.macro_events('a'),
            Some("<Left>".chars().map(InputEvent::key).collect())
        );
    }

    #[test]
    fn text_only_macro_can_be_put_without_reparsing_angle_notation() {
        let mut registers = Registers::default();
        let events = vec![InputEvent::Text("<Left>\nliteral".to_owned())];
        registers.set_macro('a', events.clone());

        let clipboard = ClipboardCommandContext::new();
        let value = registers
            .read_for_put('a', RegisterReadContext::new(&clipboard, None))
            .unwrap()
            .unwrap();
        assert_eq!(value.text, "<Left>\nliteral");
        assert!(value.hard_break_offsets().is_empty());
        assert_eq!(registers.macro_events('a'), Some(events));
    }

    #[test]
    fn macro_controls_materialize_as_literal_scalars_without_hard_breaks() {
        let mut registers = Registers::default();
        registers.set_macro(
            'a',
            vec![
                InputEvent::Key(Key::Tab),
                InputEvent::Key(Key::Enter),
                InputEvent::Key(Key::Escape),
                InputEvent::Key(Key::Ctrl('w')),
                InputEvent::Key(Key::Ctrl('[')),
            ],
        );

        assert_eq!(
            registers.get('a').unwrap().text,
            "<Tab><Enter><Esc><C-w><C-[>"
        );
        let clipboard = ClipboardCommandContext::new();
        let put = registers
            .read_for_put('a', RegisterReadContext::new(&clipboard, None))
            .unwrap()
            .unwrap();
        assert_eq!(put.text, "\t\r\u{1b}\u{17}\u{1b}");
        assert!(put.hard_break_offsets().is_empty());
    }

    #[test]
    fn uppercase_text_append_preserves_the_existing_macro_program() {
        let mut registers = Registers::default();
        registers.set_macro('a', vec![InputEvent::Key(Key::Left)]);
        registers.yank(Some('A'), RegisterValue::linewise("tail\n"));

        assert_eq!(registers.get('a').unwrap().text, "<Left>\ntail\n");
        assert_eq!(
            registers.macro_events('a'),
            Some(vec![
                InputEvent::Key(Key::Left),
                InputEvent::Key(Key::Enter),
                InputEvent::key('t'),
                InputEvent::key('a'),
                InputEvent::key('i'),
                InputEvent::key('l'),
                InputEvent::Key(Key::Enter),
            ])
        );
    }
}
