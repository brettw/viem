//! Execution planning for parsed Ex commands.
//!
//! This module deliberately separates preparation from commit. Preparation is
//! read-only and stages document mutations, register changes, view-option
//! changes, navigation, and frontend work in one [`ExPlan`]. The coordinator
//! can therefore validate every effect before committing the document change.
//!
//! Substitution uses the versioned portable Regex v1 language and semantic
//! hard-line assertions. Replacement captures retain their formatted content.

use std::fmt;
use std::ops::Range;

use super::regex_v1::{
    CompiledRegex, ExpandedFragment, RegexError, RegexInput, RegexLimits, RegexWork,
    ReplacementTemplate,
};
use unicode_segmentation::UnicodeSegmentation;

use super::ex::{
    AddressBase, ExAction, ExAddress, ExCommand, ExRange, OptionAction, OptionOperation,
    RangeSeparator, RepeatPattern, SetOperation, SetScope, Substitute, SubstituteFlags,
};
use crate::document::{
    ArtifactWriteCompletionStatus, BoundaryAffinity, CommittedModelTransaction, Document,
    DocumentError, DocumentId, FileFormat, FormattedPayloadEdit, FormattedPayloadEditRequest,
    FormattedPayloadError, FormattedTextPayload, HardLineSnapshot, HardLineTransfer,
    HistoryBoundary, HistoryChangeNumber, HistoryLocation, HistoryNavigationRequest, ModelRequest,
    ModelTransactionError, PreparedArtifactWrite, Revision, SourceToTextError, TextEdit,
};

/// An inclusive, zero-based range of hard lines.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HardLineRange {
    pub start: usize,
    pub end: usize,
}

impl HardLineRange {
    pub fn new(start: usize, end: usize) -> Result<Self, ExExecuteError> {
        if start > end {
            return Err(ExExecuteError::InvertedRange { start, end });
        }
        Ok(Self { start, end })
    }

    pub fn line_count(self) -> usize {
        self.end - self.start + 1
    }
}

/// View/buffer state needed to interpret an Ex command without reaching into
/// the frontend or the main command interpreter.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExExecutionContext {
    /// Zero-based current hard line.
    pub current_line: usize,
    pub wrap: bool,
    pub fileformats: Vec<FileFormat>,
    pub search_options: super::regex_v1::SearchOptions,
    /// Buffer-owned `textwidth` state.
    pub text_width: crate::document::TextWidthSetting,
    pub indentation: crate::document::IndentationSetting,
    pub visible_whitespace: super::VisibleWhitespaceSetting,
    pub last_search_pattern: Option<String>,
}

impl Default for ExExecutionContext {
    fn default() -> Self {
        Self {
            current_line: 0,
            wrap: false,
            fileformats: vec![FileFormat::Unix, FileFormat::Dos],
            search_options: super::regex_v1::SearchOptions::default(),
            text_width: crate::document::TextWidthSetting::default(),
            indentation: Default::default(),
            visible_whitespace: Default::default(),
            last_search_pattern: None,
        }
    }
}

/// Ex-specific persistent state. It is separate from a document because
/// substitute history is command/session state and is not restored by undo.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ExExecutionState {
    last_substitute: Option<StoredSubstitute>,
}

impl ExExecutionState {
    pub fn has_previous_substitute(&self) -> bool {
        self.last_substitute.is_some()
    }

    pub fn previous_substitute_pattern(&self) -> Option<&str> {
        self.last_substitute
            .as_ref()
            .map(|substitute| substitute.pattern.as_str())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct StoredSubstitute {
    pattern: String,
    replacement: String,
    flags: SubstituteFlags,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExRegisterKind {
    Characterwise,
    Linewise,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExRegisterValue {
    pub text: String,
    pub kind: ExRegisterKind,
    hard_break_offsets: Vec<usize>,
}

impl ExRegisterValue {
    pub fn characterwise(text: impl Into<String>) -> Self {
        let text = text.into();
        let hard_break_offsets = ex_line_feed_offsets(&text);
        Self::try_new(text, ExRegisterKind::Characterwise, hard_break_offsets)
            .expect("line-feed offsets derived from Ex register text are valid")
    }

    pub fn linewise(text: impl Into<String>) -> Self {
        let text = text.into();
        let hard_break_offsets = ex_line_feed_offsets(&text);
        Self::try_new(text, ExRegisterKind::Linewise, hard_break_offsets)
            .expect("line-feed offsets derived from Ex register text are valid")
    }

    pub fn try_new(
        text: impl Into<String>,
        kind: ExRegisterKind,
        hard_break_offsets: Vec<usize>,
    ) -> Result<Self, ExRegisterValueError> {
        let text = text.into();
        validate_ex_register_break_offsets(&text, &hard_break_offsets)?;
        Ok(Self {
            text,
            kind,
            hard_break_offsets,
        })
    }

    pub fn hard_break_offsets(&self) -> &[usize] {
        &self.hard_break_offsets
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExRegisterValueError {
    BreakOffsetOutOfBounds { offset: usize, text_length: usize },
    BreakOffsetIsNotLineFeed { offset: usize },
    BreakOffsetsNotStrictlyIncreasing { previous: usize, offset: usize },
}

impl fmt::Display for ExRegisterValueError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BreakOffsetOutOfBounds {
                offset,
                text_length,
            } => write!(
                formatter,
                "Ex register break offset {offset} is outside text length {text_length}"
            ),
            Self::BreakOffsetIsNotLineFeed { offset } => write!(
                formatter,
                "Ex register break offset {offset} does not name U+000A"
            ),
            Self::BreakOffsetsNotStrictlyIncreasing { previous, offset } => write!(
                formatter,
                "Ex register break offsets are not strictly increasing at {previous}, {offset}"
            ),
        }
    }
}

impl std::error::Error for ExRegisterValueError {}

fn ex_line_feed_offsets(text: &str) -> Vec<usize> {
    text.bytes()
        .enumerate()
        .filter_map(|(offset, byte)| (byte == b'\n').then_some(offset))
        .collect()
}

fn validate_ex_register_break_offsets(
    text: &str,
    hard_break_offsets: &[usize],
) -> Result<(), ExRegisterValueError> {
    let mut previous = None;
    for &offset in hard_break_offsets {
        if offset >= text.len() {
            return Err(ExRegisterValueError::BreakOffsetOutOfBounds {
                offset,
                text_length: text.len(),
            });
        }
        if let Some(previous) = previous {
            if previous >= offset {
                return Err(ExRegisterValueError::BreakOffsetsNotStrictlyIncreasing {
                    previous,
                    offset,
                });
            }
        }
        if text.as_bytes()[offset] != b'\n' {
            return Err(ExRegisterValueError::BreakOffsetIsNotLineFeed { offset });
        }
        previous = Some(offset);
    }
    Ok(())
}

/// Read-only register access used while preparing `:put`.
pub trait ExRegisterReader {
    fn read(&self, requested: Option<char>) -> Option<ExRegisterValue>;
}

impl ExRegisterReader for () {
    fn read(&self, _requested: Option<char>) -> Option<ExRegisterValue> {
        None
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExRegisterEffectKind {
    Yank,
    Delete,
}

/// A staged register update. Applying it is infallible in the command-layer
/// register implementation, so a coordinator can publish it with a successful
/// document commit without exposing partial state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExRegisterEffect {
    pub kind: ExRegisterEffectKind,
    pub requested: Option<char>,
    pub value: ExRegisterValue,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExOptionName {
    Wrap,
    FileFormat,
    FileFormats,
    IgnoreCase,
    SmartCase,
    WrapScan,
    TextWidth,
    AutoIndent, TabStop, ShiftWidth, SoftTabStop, ExpandTab, SmartTab,
    ContinueCommentsOnEnter, ContinueCommentsOnOpenLine,
    List, ListChars,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExOptionValue {
    Boolean(bool),
    String(String),
    Integer(i32),
    Indentation(crate::document::IndentationSetting),
    VisibleWhitespace(super::VisibleWhitespaceSetting),
    FileFormat(FileFormat),
    FileFormats(Vec<FileFormat>),
    /// A displayed numeric value, such as the effective `textwidth`.
    Number(u32),
    /// A buffer-local numeric override; `None` returns to inheritance.
    OptionalNumber(Option<u32>),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExOptionEffect {
    pub scope: SetScope,
    pub name: ExOptionName,
    pub old_value: ExOptionValue,
    pub new_value: ExOptionValue,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExOptionDisplay {
    pub name: ExOptionName,
    pub value: ExOptionValue,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExFileRequest {
    NavigateArgument {
        target: super::argument_list::ExArgumentTarget,
        force: bool,
        write_first: bool,
        path: Option<String>,
        /// One-based line; zero requests the last line.
        line: Option<u64>,
    },
    EditNewWindow {
        path: Option<String>,
    },
    PrintWorkingDirectory,
    CheckTime,
    ChangeDirectory {
        path: Option<String>,
    },
    Split {
        path: Option<String>,
        height: Option<usize>,
    },
    /// A new empty document in a new pane; the current buffer remains open.
    NewPane {
        height: Option<usize>,
    },
    Edit {
        path: Option<String>,
        force: bool,
    },
    New {
        force: bool,
    },
    Write {
        path: Option<String>,
        force: bool,
        range: Option<HardLineRange>,
    },
    SaveAs {
        path: String,
        force: bool,
    },
    Quit {
        force: bool,
    },
    QuitAll {
        force: bool,
    },
    WriteQuit {
        path: Option<String>,
        force: bool,
        range: Option<HardLineRange>,
    },
    Xit {
        path: Option<String>,
        force: bool,
    },
    WriteAll {
        force: bool,
    },
}

/// Immutable identity of the document snapshot against which an outbound Ex
/// request was prepared. Hosts must not infer either value from whichever
/// buffer happens to be current when asynchronous work completes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExRequestTag {
    document: DocumentId,
    revision: Revision,
}

impl ExRequestTag {
    pub(crate) fn new(document: DocumentId, revision: Revision) -> Self {
        Self { document, revision }
    }

    pub fn document(self) -> DocumentId {
        self.document
    }

    pub fn revision(self) -> Revision {
        self.revision
    }
}

/// A request that still belongs to the host/session layer, tagged with the
/// exact document snapshot that emitted it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaggedExFileRequest {
    tag: ExRequestTag,
    request: ExFileRequest,
}

impl TaggedExFileRequest {
    pub(crate) fn new(tag: ExRequestTag, request: ExFileRequest) -> Self {
        Self { tag, request }
    }

    pub fn tag(&self) -> ExRequestTag {
        self.tag
    }

    pub fn request(&self) -> &ExFileRequest {
        &self.request
    }

    pub fn into_request(self) -> ExFileRequest {
        self.request
    }
}

/// Source-authoritative storage work derived from one Ex write-family
/// request. The optional host request is released only after this exact write
/// succeeds and the originating document state has not changed meanwhile.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PreparedExArtifactWrite {
    tag: ExRequestTag,
    write: PreparedArtifactWrite,
    after_success: Option<TaggedExFileRequest>,
}

impl PreparedExArtifactWrite {
    pub(crate) fn new(
        tag: ExRequestTag,
        write: PreparedArtifactWrite,
        after_success: Option<TaggedExFileRequest>,
    ) -> Self {
        debug_assert_eq!(tag.document(), write.document());
        debug_assert_eq!(tag.revision(), write.revision());
        Self {
            tag,
            write,
            after_success,
        }
    }

    pub fn tag(&self) -> ExRequestTag {
        self.tag
    }

    pub fn write(&self) -> &PreparedArtifactWrite {
        &self.write
    }

    pub fn after_success(&self) -> Option<&TaggedExFileRequest> {
        self.after_success.as_ref()
    }
}

/// Result of adapting an outbound Ex file request at the serial core
/// boundary. Storage writes are ready for execution without holding a core
/// borrow; other variants deliberately remain host/session work.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PreparedExFileRequest {
    ArtifactWrite(PreparedExArtifactWrite),
    Host(TaggedExFileRequest),
}

/// Whether a host action attached to `:wq`/`:xit` may run after storage
/// completion. A failed write and an edit made while I/O was in flight both
/// retain the action for diagnostics but never authorize it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExPostWriteDisposition {
    NotRequested,
    Ready(TaggedExFileRequest),
    WriteFailed(TaggedExFileRequest),
    DocumentChanged {
        request: TaggedExFileRequest,
        captured_history: HistoryLocation,
        current_history: HistoryLocation,
        current_revision: Revision,
    },
}

impl ExPostWriteDisposition {
    pub fn ready_request(&self) -> Option<&TaggedExFileRequest> {
        match self {
            Self::Ready(request) => Some(request),
            _ => None,
        }
    }
}

/// Model publication plus any now-authorized host action for one completed Ex
/// artifact write.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompletedExArtifactWrite {
    status: ArtifactWriteCompletionStatus,
    post_write: ExPostWriteDisposition,
}

impl CompletedExArtifactWrite {
    pub(crate) fn new(
        status: ArtifactWriteCompletionStatus,
        post_write: ExPostWriteDisposition,
    ) -> Self {
        Self { status, post_write }
    }

    pub fn status(&self) -> &ArtifactWriteCompletionStatus {
        &self.status
    }

    pub fn post_write(&self) -> &ExPostWriteDisposition {
        &self.post_write
    }

    pub fn into_parts(self) -> (ArtifactWriteCompletionStatus, ExPostWriteDisposition) {
        (self.status, self.post_write)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExInfoRequest {
    /// Plain informational or warning text for a persistent, selectable output surface.
    Message(String),
    Marks(Vec<char>),
    Registers(Vec<char>),
    Jumps,
    Options(Vec<ExOptionDisplay>),
    PrintLines {
        range: HardLineRange,
        number: bool,
        list: bool,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExNormalRequest {
    pub range: HardLineRange,
    pub commands: String,
    /// `:normal!` requests literal built-in commands. Mappings are currently
    /// deferred, but retaining this bit avoids changing the interface later.
    pub literal: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExFrontendRequest {
    File(ExFileRequest),
    Info(ExInfoRequest),
    /// A `CTRL-W` window effect. The frontend owns pane geometry and focus;
    /// the core only resolves which effect the grammar named.
    Window(super::window::WindowRequest),
    /// Internal continuation staged for the main command interpreter. The
    /// interpreter consumes this before publishing an Ex outcome; it is kept
    /// in this compatibility enum only to avoid widening `ExPlan` while Ex
    /// execution is incrementally migrated to typed controller plans.
    Normal(ExNormalRequest),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExNavigation {
    TextOffset(usize),
    HistoryRestoration,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ExOutcome {
    pub document_changed: bool,
    /// Exact model publications produced by mutating Ex commands, in commit
    /// order.
    ///
    /// An ordinary Ex command contributes zero or one entry. Compound command
    /// replay may contribute several entries, and retaining each publication
    /// preserves its exact summary and position map. The entries are not
    /// collapsed into one Ex-only map because non-Ex edits may have committed
    /// between them; the outer command coordinator captures that complete
    /// event-level transition separately.
    model_transactions: Vec<CommittedModelTransaction>,
    pub navigation: Option<ExNavigation>,
    pub register_effects: Vec<ExRegisterEffect>,
    pub option_effects: Vec<ExOptionEffect>,
    pub frontend_requests: Vec<ExFrontendRequest>,
    pub substitutions: usize,
}

impl ExOutcome {
    /// Return the exact publication for an ordinary Ex outcome.
    ///
    /// Aggregate outcomes containing more than one publication deliberately
    /// return `None`; callers inspecting compound replay must use
    /// [`Self::model_transactions`] so no transaction is mistaken for the
    /// complete transition.
    pub fn model_transaction(&self) -> Option<&CommittedModelTransaction> {
        match self.model_transactions.as_slice() {
            [transaction] => Some(transaction),
            [] | [_, _, ..] => None,
        }
    }

    /// Return every exact model publication in commit order.
    pub fn model_transactions(&self) -> &[CommittedModelTransaction] {
        &self.model_transactions
    }

    /// Merge sequential command output without flattening model publications.
    ///
    /// Collection-valued effects retain execution order, navigation composes
    /// as sequential state (the last destination wins), and substitution
    /// counts remain exact. An overflow is detected before `self` is changed.
    /// This is the merge primitive intended for compound command output.
    pub fn try_merge(&mut self, mut next: Self) -> Result<(), ExOutcomeMergeError> {
        let substitutions = self
            .substitutions
            .checked_add(next.substitutions)
            .ok_or(ExOutcomeMergeError::SubstitutionCountOverflow)?;

        self.document_changed |= next.document_changed;
        self.model_transactions.append(&mut next.model_transactions);
        if next.navigation.is_some() {
            self.navigation = next.navigation;
        }
        self.register_effects.extend(next.register_effects);
        self.option_effects.extend(next.option_effects);
        self.frontend_requests.extend(next.frontend_requests);
        self.substitutions = substitutions;
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExOutcomeMergeError {
    SubstitutionCountOverflow,
}

impl fmt::Display for ExOutcomeMergeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SubstitutionCountOverflow => {
                formatter.write_str("compound Ex substitution count overflowed")
            }
        }
    }
}

impl std::error::Error for ExOutcomeMergeError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SubstitutePreview {
    pub pattern: String,
    pub replacement: String,
    pub matches: usize,
    pub edits: Vec<TextEdit>,
    /// The same replacements with semantic hard breaks distinguished from
    /// literal U+000A content, ready for a policy-approved model commit.
    pub payload_edits: Vec<FormattedPayloadEdit>,
    /// Authoritative replacement pieces preserving captured styles.
    pub fragment_edits: Vec<crate::document::FragmentEdit>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExPolicyRequest {
    ConfirmSubstitution(SubstitutePreview),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExCapability {
    /// Copying or moving formatted Markdown blocks must preserve source syntax,
    /// styles, objects, and provenance rather than flattening to plain text.
    SourcePreservingBlockTransfer,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExExecuteError {
    Document(DocumentError),
    FormattedPayload(FormattedPayloadError),
    CurrentLineOutOfBounds {
        current: usize,
        line_count: usize,
    },
    AddressOutOfBounds {
        value: i128,
        line_count: usize,
        zero_allowed: bool,
    },
    AddressOverflow,
    InvertedRange {
        start: usize,
        end: usize,
    },
    InvalidCount(u64),
    NoUndo,
    NoRedo,
    ReadOnly,
    EmptyRegister(Option<char>),
    NoPreviousSubstitute,
    NoPreviousSearch,
    InvalidSortArgument(String),
    InvalidRegex(String),
    UnsupportedRegexAtom(String),
    UnsupportedReplacementAtom(String),
    Regex(RegexError),
    PatternNotFound(String),
    DestinationInsideRange,
    InvalidOptionValue {
        option: String,
        value: String,
    },
    UnsupportedOption(String),
    UnsupportedOptionOperation(String),
    ConflictingOptionChanges(String),
    StalePlan {
        expected: Revision,
        actual: Revision,
    },
    WrongDocument,
    Model(ModelTransactionError),
    SourceMapping(SourceToTextError),
    NeedsPolicy(ExPolicyRequest),
    NeedsCapability(ExCapability),
}

impl From<RegexError> for ExExecuteError {
    fn from(value: RegexError) -> Self {
        match value {
            RegexError::InvalidRegex(error) => Self::InvalidRegex(error),
            RegexError::UnsupportedRegexAtom(atom) => Self::UnsupportedRegexAtom(atom),
            RegexError::UnsupportedReplacementAtom(atom) => Self::UnsupportedReplacementAtom(atom),
            other => Self::Regex(other),
        }
    }
}

impl From<DocumentError> for ExExecuteError {
    fn from(value: DocumentError) -> Self {
        Self::Document(value)
    }
}

impl From<FormattedPayloadError> for ExExecuteError {
    fn from(value: FormattedPayloadError) -> Self {
        Self::FormattedPayload(value)
    }
}

impl fmt::Display for ExExecuteError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Regex(error) => error.fmt(formatter),
            Self::Document(error) => error.fmt(formatter),
            Self::FormattedPayload(error) => error.fmt(formatter),
            Self::CurrentLineOutOfBounds {
                current,
                line_count,
            } => write!(
                formatter,
                "current hard line {} is outside a {line_count}-line document",
                current + 1
            ),
            Self::AddressOutOfBounds {
                value,
                line_count,
                zero_allowed,
            } => write!(
                formatter,
                "line address {value} is outside {}..={line_count}",
                if *zero_allowed { 0 } else { 1 }
            ),
            Self::AddressOverflow => formatter.write_str("line address arithmetic overflowed"),
            Self::InvertedRange { start, end } => {
                write!(
                    formatter,
                    "line range {}..={} is inverted",
                    start + 1,
                    end + 1
                )
            }
            Self::InvalidCount(count) => write!(formatter, "invalid line count {count}"),
            Self::ReadOnly => {
                formatter.write_str("E45: readonly option is set (use ! to override)")
            }
            Self::NoUndo => formatter.write_str("already at oldest change"),
            Self::NoRedo => formatter.write_str("already at newest change"),
            Self::EmptyRegister(name) => match name {
                Some(name) => write!(formatter, "register {name:?} is empty"),
                None => formatter.write_str("unnamed register is empty"),
            },
            Self::NoPreviousSubstitute => formatter.write_str("no previous substitute"),
            Self::NoPreviousSearch => formatter.write_str("no previous search pattern"),
            Self::InvalidSortArgument(error) => write!(formatter, "invalid sort argument: {error}"),
            Self::InvalidRegex(error) => write!(formatter, "invalid regular expression: {error}"),
            Self::UnsupportedRegexAtom(atom) => {
                write!(
                    formatter,
                    "unsupported Vim regular-expression atom {atom:?}"
                )
            }
            Self::UnsupportedReplacementAtom(atom) => {
                write!(
                    formatter,
                    "unsupported substitute replacement atom {atom:?}"
                )
            }
            Self::PatternNotFound(pattern) => write!(formatter, "pattern not found: {pattern}"),
            Self::DestinationInsideRange => {
                formatter.write_str("cannot move lines into themselves")
            }
            Self::InvalidOptionValue { option, value } => {
                write!(formatter, "invalid value {value:?} for option {option}")
            }
            Self::UnsupportedOption(option) => write!(formatter, "unsupported option {option}"),
            Self::UnsupportedOptionOperation(option) => {
                write!(formatter, "unsupported operation for option {option}")
            }
            Self::ConflictingOptionChanges(option) => {
                write!(formatter, "conflicting changes to option {option}")
            }
            Self::StalePlan { expected, actual } => write!(
                formatter,
                "stale Ex plan for revision {}; current revision is {}",
                expected.0, actual.0
            ),
            Self::WrongDocument => formatter.write_str("Ex plan belongs to another document"),
            Self::Model(error) => error.fmt(formatter),
            Self::SourceMapping(error) => error.fmt(formatter),
            Self::NeedsPolicy(_) => formatter.write_str("command requires a policy decision"),
            Self::NeedsCapability(capability) => {
                write!(
                    formatter,
                    "command requires unavailable capability {capability:?}"
                )
            }
        }
    }
}

impl std::error::Error for ExExecuteError {}

#[derive(Clone, Debug, PartialEq)]
pub enum ExMutation {
    None,
    Model(ModelRequest),
    FormattedPayload(FormattedPayloadEditRequest),
}

/// A revision-bound, fully validated command plan.
#[derive(Clone, Debug, PartialEq)]
pub struct ExPlan {
    document_id: DocumentId,
    expected_revision: Revision,
    pub mutation: ExMutation,
    pub outcome: ExOutcome,
    next_substitute: Option<StoredSubstitute>,
}

impl ExPlan {
    pub fn document_id(&self) -> DocumentId {
        self.document_id
    }

    pub fn expected_revision(&self) -> Revision {
        self.expected_revision
    }

    pub fn staged_register_effects(&self) -> &[ExRegisterEffect] {
        &self.outcome.register_effects
    }

    fn empty(document: &Document) -> Self {
        Self {
            document_id: document.id(),
            expected_revision: document.revision(),
            mutation: ExMutation::None,
            outcome: ExOutcome::default(),
            next_substitute: None,
        }
    }

    fn stage_text_edits(&mut self, document: &Document, edits: Vec<TextEdit>) {
        self.mutation = ExMutation::Model(ModelRequest::ApplyTextEdits {
            document: document.id(),
            revision: document.revision(),
            edits,
        });
    }

    fn stage_formatted_payload_edits(
        &mut self,
        document: &Document,
        edits: Vec<FormattedPayloadEdit>,
    ) {
        self.mutation = ExMutation::FormattedPayload(FormattedPayloadEditRequest::new(
            document.id(),
            document.revision(),
            edits,
        ));
    }

    fn stage_history_navigation(
        &mut self,
        document: &Document,
        navigation: HistoryNavigationRequest,
    ) {
        self.mutation = ExMutation::Model(ModelRequest::NavigateHistory {
            document: document.id(),
            revision: document.revision(),
            navigation,
        });
    }

    fn stage_file_format(&mut self, document: &Document, target: FileFormat) {
        self.mutation = ExMutation::Model(ModelRequest::SetFileFormat {
            document: document.id(),
            revision: document.revision(),
            target,
        });
    }

    fn stage_hard_line_transfer(
        &mut self,
        document: &Document,
        operation: HardLineTransfer,
        source_lines: Range<usize>,
        destination: usize,
    ) {
        self.mutation = ExMutation::Model(ModelRequest::TransferHardLines {
            document: document.id(),
            revision: document.revision(),
            operation,
            source_lines,
            destination,
        });
    }
}

/// Resolve an ordinary Ex line address to a zero-based hard-line index.
pub fn resolve_address(
    address: ExAddress,
    current_line: usize,
    line_count: usize,
) -> Result<usize, ExExecuteError> {
    resolve_address_inner(address, current_line, line_count, false)
}

/// Resolve an Ex destination address. A value of zero denotes the position
/// before the first hard line; other values are one-based line numbers.
pub fn resolve_destination(
    address: ExAddress,
    current_line: usize,
    line_count: usize,
) -> Result<usize, ExExecuteError> {
    let one_based = resolve_address_value(address, current_line, line_count)?;
    if !(0..=line_count as i128).contains(&one_based) {
        return Err(ExExecuteError::AddressOutOfBounds {
            value: one_based,
            line_count,
            zero_allowed: true,
        });
    }
    usize::try_from(one_based).map_err(|_| ExExecuteError::AddressOverflow)
}

pub fn resolve_range(
    range: Option<&ExRange>,
    current_line: usize,
    line_count: usize,
) -> Result<HardLineRange, ExExecuteError> {
    validate_current_line(current_line, line_count)?;
    match range {
        None => Ok(HardLineRange {
            start: current_line,
            end: current_line,
        }),
        Some(ExRange::WholeFile) => Ok(HardLineRange {
            start: 0,
            end: line_count - 1,
        }),
        Some(ExRange::Single(address)) => {
            let line = resolve_address(*address, current_line, line_count)?;
            Ok(HardLineRange {
                start: line,
                end: line,
            })
        }
        Some(ExRange::Between {
            start,
            end,
            separator,
        }) => {
            let start = resolve_address(*start, current_line, line_count)?;
            let end_current = match separator {
                RangeSeparator::Comma => current_line,
                RangeSeparator::Semicolon => start,
            };
            let end = resolve_address(*end, end_current, line_count)?;
            HardLineRange::new(start, end)
        }
    }
}

/// Resolve the last address of `:put` as a line-insertion boundary.
///
/// Unlike ordinary command ranges, `:put` accepts the special address zero,
/// meaning the boundary before the first line. Multi-address ranges still use
/// ordinary range validation; only a single put address gets this exception.
fn resolve_put_address(
    range: Option<&ExRange>,
    current_line: usize,
    line_count: usize,
) -> Result<usize, ExExecuteError> {
    validate_current_line(current_line, line_count)?;
    match range {
        None => current_line
            .checked_add(1)
            .ok_or(ExExecuteError::AddressOverflow),
        Some(ExRange::Single(address)) => resolve_destination(*address, current_line, line_count),
        Some(range) => resolve_range(Some(range), current_line, line_count)?
            .end
            .checked_add(1)
            .ok_or(ExExecuteError::AddressOverflow),
    }
}

/// Convert an inclusive hard-line range to the exact formatted range occupied
/// by those lines, including the following hard break when one exists.
pub fn hard_line_text_range(
    document: &Document,
    lines: HardLineRange,
) -> Result<Range<usize>, ExExecuteError> {
    let hard_lines = document.hard_line_snapshot();
    validate_line_range(lines, hard_lines.line_count())?;
    let end = lines
        .end
        .checked_add(1)
        .ok_or(ExExecuteError::AddressOverflow)?;
    hard_lines
        .linewise_extent(lines.start..end)
        .map_err(|_| ExExecuteError::AddressOverflow)
}

fn capture_linewise_register(
    document: &Document,
    range: Range<usize>,
) -> Result<ExRegisterValue, ExExecuteError> {
    let snapshot = document.hard_line_snapshot();
    let captured = snapshot.capture(range)?;
    let mut text = captured.text().to_owned();
    let mut hard_break_offsets = captured.break_offsets().to_vec();
    let ends_with_hard_break = hard_break_offsets
        .last()
        .is_some_and(|offset| offset.saturating_add(1) == text.len());
    if !ends_with_hard_break {
        hard_break_offsets.push(text.len());
        text.push('\n');
    }
    Ok(
        ExRegisterValue::try_new(text, ExRegisterKind::Linewise, hard_break_offsets)
            .expect("captured and synthetic Ex register breaks are valid"),
    )
}

/// Prepare a parsed command without mutating document, command, register, or
/// frontend state.
pub fn prepare_ex<R: ExRegisterReader + ?Sized>(
    document: &Document,
    state: &ExExecutionState,
    context: &ExExecutionContext,
    command: &ExCommand,
    registers: &R,
) -> Result<ExPlan, ExExecuteError> {
    validate_current_line(context.current_line, document.line_count())?;
    if document.is_read_only()
        && !command.bang
        && (matches!(
            &command.action,
            ExAction::Write { .. }
                | ExAction::NavigateArgument { write_first: true, .. }
                | ExAction::SaveAs { .. }
                | ExAction::WriteQuit { .. }
                | ExAction::WriteAll
        ) || matches!(&command.action,ExAction::Xit{path} if path.is_some()||document.is_dirty()))
    {
        return Err(ExExecuteError::ReadOnly);
    }
    let mut plan = ExPlan::empty(document);

    match &command.action {
        ExAction::NavigateArgument { target, write_first, path, line } => push_file(
            &mut plan,
            ExFileRequest::NavigateArgument {
                target: *target,
                force: command.bang,
                write_first: *write_first,
                path: path.clone(),
                line: *line,
            },
        ),
        ExAction::EditNewWindow { path } => push_file(
            &mut plan,
            ExFileRequest::EditNewWindow { path: path.clone() },
        ),
        ExAction::CheckTime => push_file(&mut plan, ExFileRequest::CheckTime),
        ExAction::PrintWorkingDirectory => {
            push_file(&mut plan, ExFileRequest::PrintWorkingDirectory)
        }
        ExAction::ChangeDirectory { path } => push_file(
            &mut plan,
            ExFileRequest::ChangeDirectory { path: path.clone() },
        ),
        ExAction::Update => {
            if document.is_dirty() {
                if document.is_read_only() && !command.bang {
                    return Err(ExExecuteError::ReadOnly);
                }
                push_file(
                    &mut plan,
                    ExFileRequest::Write {
                        path: None,
                        force: command.bang,
                        range: None,
                    },
                );
            }
        }
        ExAction::Split { path } => {
            push_file(&mut plan, ExFileRequest::Split { path: path.clone(), height: None })
        }
        ExAction::Edit { path } => push_file(
            &mut plan,
            ExFileRequest::Edit {
                path: path.clone(),
                force: command.bang,
            },
        ),
        ExAction::New => push_file(
            &mut plan,
            ExFileRequest::New {
                force: command.bang,
            },
        ),
        ExAction::Write { path } => {
            let range = resolve_optional_file_range(document, context, command.range.as_ref())?;
            push_file(
                &mut plan,
                ExFileRequest::Write {
                    path: path.clone(),
                    force: command.bang,
                    range,
                },
            );
        }
        ExAction::SaveAs { path } => push_file(
            &mut plan,
            ExFileRequest::SaveAs {
                path: path.clone(),
                force: command.bang,
            },
        ),
        ExAction::Quit => push_file(
            &mut plan,
            ExFileRequest::Quit {
                force: command.bang,
            },
        ),
        ExAction::QuitAll => push_file(
            &mut plan,
            ExFileRequest::QuitAll {
                force: command.bang,
            },
        ),
        ExAction::WriteQuit { path } => {
            let range = resolve_optional_file_range(document, context, command.range.as_ref())?;
            push_file(
                &mut plan,
                ExFileRequest::WriteQuit {
                    path: path.clone(),
                    force: command.bang,
                    range,
                },
            );
        }
        ExAction::Xit { path } => push_file(
            &mut plan,
            ExFileRequest::Xit {
                path: path.clone(),
                force: command.bang,
            },
        ),
        ExAction::WriteAll => push_file(
            &mut plan,
            ExFileRequest::WriteAll {
                force: command.bang,
            },
        ),
        ExAction::Undo { change } => {
            let navigation = change.map_or(HistoryNavigationRequest::Undo, |change| {
                HistoryNavigationRequest::SelectChange(HistoryChangeNumber::from_u64(change))
            });
            plan.stage_history_navigation(document, navigation);
            plan.outcome.navigation = Some(ExNavigation::HistoryRestoration);
        }
        ExAction::Redo => {
            plan.stage_history_navigation(document, HistoryNavigationRequest::Redo);
            plan.outcome.navigation = Some(ExNavigation::HistoryRestoration);
        }
        ExAction::Delete(arguments) => {
            let lines = effective_counted_range(
                document,
                context,
                command.range.as_ref(),
                arguments.count,
            )?;
            let register_range = hard_line_text_range(document, lines)?;
            let value = capture_linewise_register(document, register_range)?;
            let deletion = hard_line_deletion_range(document, lines)?;
            plan.stage_text_edits(document, vec![TextEdit::new(deletion, "")]);
            plan.outcome.register_effects.push(ExRegisterEffect {
                kind: ExRegisterEffectKind::Delete,
                requested: arguments.register,
                value,
            });
            plan.outcome.navigation = Some(ExNavigation::TextOffset(line_start_after_deletion(
                document, lines,
            )?));
        }
        ExAction::Yank(arguments) => {
            let lines = effective_counted_range(
                document,
                context,
                command.range.as_ref(),
                arguments.count,
            )?;
            let range = hard_line_text_range(document, lines)?;
            plan.outcome.register_effects.push(ExRegisterEffect {
                kind: ExRegisterEffectKind::Yank,
                requested: arguments.register,
                value: capture_linewise_register(document, range)?,
            });
        }
        ExAction::Put { register } => {
            let value = registers
                .read(*register)
                .ok_or(ExExecuteError::EmptyRegister(*register))?;
            let addressed_line = resolve_put_address(
                command.range.as_ref(),
                context.current_line,
                document.line_count(),
            )?;
            let insertion_index = if command.bang {
                addressed_line.saturating_sub(1)
            } else {
                addressed_line
            };
            let (edit, inserted_at) = plan_put(document, insertion_index, &value)?;
            plan.stage_formatted_payload_edits(document, vec![edit]);
            plan.outcome.navigation = Some(ExNavigation::TextOffset(inserted_at));
        }
        ExAction::Join { count } => {
            let mut lines = resolve_range(
                command.range.as_ref(),
                context.current_line,
                document.line_count(),
            )?;
            if let Some(count) = count {
                lines = range_with_count(lines.end, *count, document.line_count())?;
            } else if matches!(command.range.as_ref(), None | Some(ExRange::Single(_)))
                && lines.end + 1 < document.line_count()
            {
                lines.end += 1;
            }
            let edits = plan_join(document, lines, command.bang)?;
            if !edits.is_empty() {
                plan.outcome.navigation = Some(ExNavigation::TextOffset(
                    hard_line_first_nonblank_offset(document, lines.start)?,
                ));
            }
            plan.stage_text_edits(document, edits);
        }
        ExAction::Copy { destination } => {
            let lines = resolve_range(
                command.range.as_ref(),
                context.current_line,
                document.line_count(),
            )?;
            let destination =
                resolve_destination(*destination, context.current_line, document.line_count())?;
            let source = lines.start..lines.end + 1;
            let cursor = transfer_cursor_offset(
                document,
                HardLineTransfer::Copy,
                source.clone(),
                destination,
            )?;
            plan.stage_hard_line_transfer(document, HardLineTransfer::Copy, source, destination);
            plan.outcome.navigation = Some(ExNavigation::TextOffset(cursor));
        }
        ExAction::Move { destination } => {
            let lines = resolve_range(
                command.range.as_ref(),
                context.current_line,
                document.line_count(),
            )?;
            let destination =
                resolve_destination(*destination, context.current_line, document.line_count())?;
            let source = lines.start..lines.end + 1;
            let cursor = transfer_cursor_offset(
                document,
                HardLineTransfer::Move,
                source.clone(),
                destination,
            )?;
            plan.stage_hard_line_transfer(document, HardLineTransfer::Move, source, destination);
            plan.outcome.navigation = Some(ExNavigation::TextOffset(cursor));
        }
        ExAction::Sort(options) => {
            let lines = if command.range.is_none() {
                HardLineRange {
                    start: 0,
                    end: document.line_count() - 1,
                }
            } else {
                resolve_range(
                    command.range.as_ref(),
                    context.current_line,
                    document.line_count(),
                )?
            };
            let (source_lines, order) =
                super::sort::ordered_lines(document, context, lines, options, command.bang)?;
            if !source_lines.is_empty() {
                let first = order.first().copied().unwrap_or(source_lines.start);
                let snapshot = document.hard_line_snapshot();
                let destination = snapshot
                    .line(source_lines.start)
                    .unwrap()
                    .content_range()
                    .start;
                let content = snapshot.line(first).unwrap().content_range();
                let indent = snapshot.text()[content]
                    .char_indices()
                    .find(|(_, c)| !c.is_whitespace())
                    .map_or(0, |(at, _)| at);
                plan.outcome.navigation = Some(ExNavigation::TextOffset(destination + indent));
                plan.mutation = ExMutation::Model(ModelRequest::ReorderHardLines {
                    document: document.id(),
                    revision: document.revision(),
                    source_lines,
                    order,
                });
            }
        }
        ExAction::Normal { commands } => {
            let range = resolve_range(
                command.range.as_ref(),
                context.current_line,
                document.line_count(),
            )?;
            plan.outcome
                .frontend_requests
                .push(ExFrontendRequest::Normal(ExNormalRequest {
                    range,
                    commands: commands.clone(),
                    literal: command.bang,
                }));
        }
        ExAction::Substitute(substitute) => {
            prepare_substitute(document, state, context, command, substitute, &mut plan)?;
        }
        ExAction::RepeatSubstitute {
            pattern,
            flags,
            count,
        } => {
            prepare_repeat_substitute(
                document, state, context, command, *pattern, flags, *count, &mut plan,
            )?;
        }
        ExAction::GoToLine(address) => {
            let line = resolve_address(*address, context.current_line, document.line_count())?;
            let hard_lines = document.hard_line_snapshot();
            plan.outcome.navigation = Some(ExNavigation::TextOffset(
                hard_lines
                    .line(line)
                    .map(|line| line.content_range().start)
                    .ok_or(ExExecuteError::AddressOverflow)?,
            ));
        }
        ExAction::GoToByte { count } => {
            let count = effective_goto_count(
                command.range.as_ref(),
                *count,
                context.current_line,
                document.line_count(),
            )?;
            let source_offset = goto_source_offset(count, document.source_byte_len());
            let source = document
                .source_point(source_offset)
                .map_err(|_| ExExecuteError::AddressOverflow)?;
            let mapped = document
                .map_source_point(source, BoundaryAffinity::Downstream)
                .map_err(ExExecuteError::SourceMapping)?;
            plan.outcome.navigation = Some(ExNavigation::TextOffset(mapped.text().offset()));
        }
        ExAction::Marks { names } => plan
            .outcome
            .frontend_requests
            .push(ExFrontendRequest::Info(ExInfoRequest::Marks(names.clone()))),
        ExAction::Registers { names } => {
            plan.outcome
                .frontend_requests
                .push(ExFrontendRequest::Info(ExInfoRequest::Registers(
                    names.clone(),
                )))
        }
        ExAction::Jumps => plan
            .outcome
            .frontend_requests
            .push(ExFrontendRequest::Info(ExInfoRequest::Jumps)),
        ExAction::Set(set) => prepare_set(document, context, set.scope, &set.operation, &mut plan)?,
    }
    Ok(plan)
}

/// Commit a previously prepared plan. Revision validation happens before any
/// mutation, and Ex history is updated only after the document operation
/// succeeds.
pub fn commit_ex(
    document: &mut Document,
    state: &mut ExExecutionState,
    mut plan: ExPlan,
) -> Result<ExOutcome, ExExecuteError> {
    if document.id() != plan.document_id {
        return Err(ExExecuteError::WrongDocument);
    }
    if document.revision() != plan.expected_revision {
        return Err(ExExecuteError::StalePlan {
            expected: plan.expected_revision,
            actual: document.revision(),
        });
    }

    let before = document.revision();
    let model_transaction = match plan.mutation {
        ExMutation::None => None,
        ExMutation::Model(request) => {
            let substitution_cursor = matches!(request, ModelRequest::ApplyFragmentEdits { .. })
                .then(|| match plan.outcome.navigation {
                    Some(ExNavigation::TextOffset(at)) => Some(at),
                    _ => None,
                })
                .flatten();
            let navigation = match &request {
                ModelRequest::NavigateHistory { navigation, .. } => Some(*navigation),
                _ => None,
            };
            let prepared = document
                .prepare_model_request(request)
                .map_err(|error| ex_model_error(error, navigation))?;
            if let Some(at) = substitution_cursor {
                let cursor = super::prepared_cursor(
                    document,
                    &prepared,
                    at,
                    crate::document::Association::BeforeInsertion,
                )
                .map_err(ExExecuteError::Document)?;
                plan.outcome.navigation = Some(ExNavigation::TextOffset(cursor));
            }
            Some(
                document
                    .commit_model_transaction(prepared)
                    .map_err(|error| ex_model_error(error, navigation))?,
            )
        }
        ExMutation::FormattedPayload(request) => {
            let prepared = document
                .prepare_formatted_payload_request(request)
                .map_err(|error| ex_model_error(error, None))?;
            Some(
                document
                    .commit_model_transaction(prepared)
                    .map_err(|error| ex_model_error(error, None))?,
            )
        }
    };

    if let Some(next) = plan.next_substitute.take() {
        state.last_substitute = Some(next);
    }
    plan.outcome.document_changed = document.revision() != before;
    plan.outcome.model_transactions.extend(model_transaction);
    Ok(plan.outcome)
}

fn ex_model_error(
    error: ModelTransactionError,
    navigation: Option<HistoryNavigationRequest>,
) -> ExExecuteError {
    match error {
        ModelTransactionError::Document(error) => ExExecuteError::Document(error),
        ModelTransactionError::WrongDocument { .. } => ExExecuteError::WrongDocument,
        ModelTransactionError::StaleRevision { expected, actual } => {
            ExExecuteError::StalePlan { expected, actual }
        }
        ModelTransactionError::History(crate::document::HistoryError::Boundary(
            HistoryBoundary::Oldest,
        )) if navigation == Some(HistoryNavigationRequest::Undo) => ExExecuteError::NoUndo,
        ModelTransactionError::History(crate::document::HistoryError::Boundary(
            HistoryBoundary::NoPreferredRedo,
        )) if navigation == Some(HistoryNavigationRequest::Redo) => ExExecuteError::NoRedo,
        other => ExExecuteError::Model(other),
    }
}

/// Convenience for hosts that do not need to inspect staged side effects.
pub fn execute_ex<R: ExRegisterReader + ?Sized>(
    document: &mut Document,
    state: &mut ExExecutionState,
    context: &ExExecutionContext,
    command: &ExCommand,
    registers: &R,
) -> Result<ExOutcome, ExExecuteError> {
    let plan = prepare_ex(document, state, context, command, registers)?;
    commit_ex(document, state, plan)
}

fn push_file(plan: &mut ExPlan, request: ExFileRequest) {
    plan.outcome
        .frontend_requests
        .push(ExFrontendRequest::File(request));
}

fn resolve_optional_file_range(
    document: &Document,
    context: &ExExecutionContext,
    range: Option<&ExRange>,
) -> Result<Option<HardLineRange>, ExExecuteError> {
    range
        .map(|range| resolve_range(Some(range), context.current_line, document.line_count()))
        .transpose()
}

fn resolve_address_inner(
    address: ExAddress,
    current_line: usize,
    line_count: usize,
    zero_allowed: bool,
) -> Result<usize, ExExecuteError> {
    let one_based = resolve_address_value(address, current_line, line_count)?;
    let lower = if zero_allowed { 0 } else { 1 };
    if !(lower..=line_count as i128).contains(&one_based) {
        return Err(ExExecuteError::AddressOutOfBounds {
            value: one_based,
            line_count,
            zero_allowed,
        });
    }
    usize::try_from(one_based - 1).map_err(|_| ExExecuteError::AddressOverflow)
}

fn resolve_address_value(
    address: ExAddress,
    current_line: usize,
    line_count: usize,
) -> Result<i128, ExExecuteError> {
    validate_current_line(current_line, line_count)?;
    let base = match address.base {
        AddressBase::Absolute(line) => i128::from(line),
        AddressBase::Current => {
            i128::try_from(current_line + 1).map_err(|_| ExExecuteError::AddressOverflow)?
        }
        AddressBase::Last => {
            i128::try_from(line_count).map_err(|_| ExExecuteError::AddressOverflow)?
        }
    };
    base.checked_add(i128::from(address.offset))
        .ok_or(ExExecuteError::AddressOverflow)
}

fn effective_goto_count(
    range: Option<&ExRange>,
    explicit_count: Option<u64>,
    current_line: usize,
    line_count: usize,
) -> Result<u64, ExExecuteError> {
    if let Some(count) = explicit_count {
        return Ok(count);
    }
    let Some(range) = range else {
        return Ok(1);
    };
    let current = i128::try_from(current_line)
        .ok()
        .and_then(|line| line.checked_add(1))
        .ok_or(ExExecuteError::AddressOverflow)?;
    let last = i128::try_from(line_count).map_err(|_| ExExecuteError::AddressOverflow)?;
    let value = match range {
        ExRange::WholeFile => last,
        ExRange::Single(address) => resolve_goto_address(*address, current, last)?,
        ExRange::Between {
            start,
            end,
            separator,
        } => {
            let start = resolve_goto_address(*start, current, last)?;
            let end_current = match separator {
                RangeSeparator::Comma => current,
                RangeSeparator::Semicolon => start,
            };
            resolve_goto_address(*end, end_current, last)?
        }
    };
    u64::try_from(value).map_err(|_| ExExecuteError::AddressOverflow)
}

fn resolve_goto_address(
    address: ExAddress,
    current: i128,
    last: i128,
) -> Result<i128, ExExecuteError> {
    let base = match address.base {
        AddressBase::Absolute(value) => i128::from(value),
        AddressBase::Current => current,
        AddressBase::Last => last,
    };
    let value = base
        .checked_add(i128::from(address.offset))
        .ok_or(ExExecuteError::AddressOverflow)?;
    if value < 0 || value > i128::from(u64::MAX) {
        return Err(ExExecuteError::AddressOverflow);
    }
    Ok(value)
}

/// Vim treats a zero or oversized `:goto` count as end-of-file. The mapping
/// layer itself remains strictly checked; this command policy computes one
/// legal source boundary before invoking it.
fn goto_source_offset(count: u64, source_len: usize) -> usize {
    if count == 0 {
        return source_len;
    }
    usize::try_from(count - 1)
        .unwrap_or(source_len)
        .min(source_len)
}

fn validate_current_line(current: usize, line_count: usize) -> Result<(), ExExecuteError> {
    if line_count == 0 || current >= line_count {
        Err(ExExecuteError::CurrentLineOutOfBounds {
            current,
            line_count,
        })
    } else {
        Ok(())
    }
}

fn validate_line_range(range: HardLineRange, line_count: usize) -> Result<(), ExExecuteError> {
    if range.start > range.end {
        return Err(ExExecuteError::InvertedRange {
            start: range.start,
            end: range.end,
        });
    }
    if range.end >= line_count {
        return Err(ExExecuteError::AddressOutOfBounds {
            value: range.end as i128 + 1,
            line_count,
            zero_allowed: false,
        });
    }
    Ok(())
}

fn range_with_count(
    start: usize,
    count: u64,
    line_count: usize,
) -> Result<HardLineRange, ExExecuteError> {
    if count == 0 {
        return Err(ExExecuteError::InvalidCount(count));
    }
    let count = usize::try_from(count).map_err(|_| ExExecuteError::AddressOverflow)?;
    let end = start
        .checked_add(count - 1)
        .ok_or(ExExecuteError::AddressOverflow)?;
    // Vim accepts a finite count that runs beyond the last line and operates
    // through EOF. Keep the arithmetic checked first so a nonsensical count
    // cannot wrap into a small, destructive range.
    let end = end.min(
        line_count
            .checked_sub(1)
            .ok_or(ExExecuteError::AddressOverflow)?,
    );
    let result = HardLineRange { start, end };
    validate_line_range(result, line_count)?;
    Ok(result)
}

fn effective_counted_range(
    document: &Document,
    context: &ExExecutionContext,
    range: Option<&ExRange>,
    count: Option<u64>,
) -> Result<HardLineRange, ExExecuteError> {
    let range = resolve_range(range, context.current_line, document.line_count())?;
    match count {
        Some(count) => range_with_count(range.end, count, document.line_count()),
        None => Ok(range),
    }
}

fn hard_line_deletion_range(
    document: &Document,
    lines: HardLineRange,
) -> Result<Range<usize>, ExExecuteError> {
    let hard_lines = document.hard_line_snapshot();
    validate_line_range(lines, hard_lines.line_count())?;
    let first = hard_lines
        .line(lines.start)
        .ok_or(ExExecuteError::AddressOverflow)?;
    if lines.end + 1 < hard_lines.line_count() {
        let after = hard_lines
            .line(lines.end + 1)
            .ok_or(ExExecuteError::AddressOverflow)?;
        return Ok(first.content_range().start..after.content_range().start);
    }
    if lines.start > 0 {
        let preceding = hard_lines
            .line(lines.start - 1)
            .and_then(|line| line.separator_range())
            .ok_or(ExExecuteError::AddressOverflow)?;
        Ok(preceding.start..hard_lines.text_length())
    } else {
        Ok(0..hard_lines.text_length())
    }
}

fn line_start_after_deletion(
    document: &Document,
    deleted: HardLineRange,
) -> Result<usize, ExExecuteError> {
    let hard_lines = document.hard_line_snapshot();
    let line_count = hard_lines.line_count();
    let following = deleted.end.checked_add(1).filter(|&line| line < line_count);
    let (target_line, rebased_start) = if let Some(following) = following {
        let rebased_start = hard_lines
            .line(deleted.start)
            .map(|line| line.content_range().start)
            .ok_or(ExExecuteError::AddressOverflow)?;
        (following, rebased_start)
    } else if deleted.start > 0 {
        let preceding = deleted.start - 1;
        let rebased_start = hard_lines
            .line(preceding)
            .map(|line| line.content_range().start)
            .ok_or(ExExecuteError::AddressOverflow)?;
        (preceding, rebased_start)
    } else {
        return Ok(0);
    };
    let target = hard_lines
        .line(target_line)
        .ok_or(ExExecuteError::AddressOverflow)?;
    let target_range=target.content_range();
    let target_offset=super::first_nonblank_document(document,&hard_lines,target_range.start)-target_range.start;
    rebased_start
        .checked_add(target_offset)
        .ok_or(ExExecuteError::AddressOverflow)
}

fn plan_put(
    document: &Document,
    insertion_line: usize,
    register: &ExRegisterValue,
) -> Result<(FormattedPayloadEdit, usize), ExExecuteError> {
    let hard_lines = document.hard_line_snapshot();
    let lines = logical_line_contents(&hard_lines)?;
    if insertion_line > lines.len() {
        return Err(ExExecuteError::AddressOutOfBounds {
            value: insertion_line as i128,
            line_count: lines.len(),
            zero_allowed: true,
        });
    }
    let inserted = ex_register_logical_lines(register);
    let insertion_offset = line_insertion_offset(&hard_lines, insertion_line)?;
    let (payload, relative_content_start) =
        line_insertion_payload(&hard_lines, &inserted, insertion_line, lines.len())?;
    let last_line_start = inserted
        .iter()
        .take(inserted.len().saturating_sub(1))
        .try_fold(relative_content_start, |offset, line| {
            offset.checked_add(line.len())?.checked_add(1)
        })
        .ok_or(ExExecuteError::AddressOverflow)?;
    let last_line_indent = inserted
        .last()
        .map_or(0, |line| first_nonblank_relative(line));
    let inserted_at = insertion_offset
        .checked_add(last_line_start)
        .and_then(|offset| offset.checked_add(last_line_indent))
        .ok_or(ExExecuteError::AddressOverflow)?;
    let edit = FormattedPayloadEdit::new(insertion_offset..insertion_offset, payload);
    Ok((edit, inserted_at))
}

fn hard_line_first_nonblank_offset(
    document: &Document,
    line_index: usize,
) -> Result<usize, ExExecuteError> {
    let hard_lines = document.hard_line_snapshot();
    let line = hard_lines
        .line(line_index)
        .ok_or(ExExecuteError::AddressOverflow)?;
    let range = line.content_range();
    Ok(super::first_nonblank_document(document,&hard_lines,range.start))
}

fn first_nonblank_relative(text: &str) -> usize {
    text.grapheme_indices(true)
        .find(|(_, grapheme)| !grapheme.chars().all(char::is_whitespace))
        .map_or(0, |(offset, _)| offset)
}

/// Converts only explicitly marked semantic breaks into line boundaries.
/// Literal U+000A values remain inside their logical line. A linewise
/// register's conventional final semantic terminator describes its shape and
/// is not an extra empty line inserted by `:put`.
fn ex_register_logical_lines(register: &ExRegisterValue) -> Vec<String> {
    let mut text_end = register.text.len();
    let mut breaks = register.hard_break_offsets();
    if register.kind == ExRegisterKind::Linewise
        && breaks
            .last()
            .is_some_and(|offset| offset.saturating_add(1) == text_end)
    {
        text_end -= 1;
        breaks = &breaks[..breaks.len() - 1];
    }

    let mut lines = Vec::with_capacity(breaks.len() + 1);
    let mut start = 0;
    for &hard_break in breaks {
        if hard_break >= text_end {
            break;
        }
        lines.push(register.text[start..hard_break].to_owned());
        start = hard_break + 1;
    }
    lines.push(register.text[start..text_end].to_owned());
    lines
}

fn plan_join(
    document: &Document,
    lines: HardLineRange,
    bang: bool,
) -> Result<Vec<TextEdit>, ExExecuteError> {
    let hard_lines = document.hard_line_snapshot();
    validate_line_range(lines, hard_lines.line_count())?;
    if lines.start == lines.end {
        return Ok(Vec::new());
    }
    let mut edits = Vec::with_capacity(lines.line_count() - 1);
    for line in lines.start..lines.end {
        let current = hard_lines
            .line(line)
            .ok_or(ExExecuteError::AddressOverflow)?;
        let Some(separator_range) = current.separator_range() else {
            break;
        };
        if bang {
            edits.push(TextEdit::new(separator_range, ""));
            continue;
        }
        let following = hard_lines
            .line(line + 1)
            .ok_or(ExExecuteError::AddressOverflow)?;
        let following_range = following.content_range();
        let following_start = following_range.start;
        let following_end = following_range.end;
        let end = super::document_prefix_end(document,following_start..following_end,char::is_whitespace);
        let before = (!current.content_range().is_empty()).then(||super::document_char_before(document,current.content_range().end)).flatten();
        let after = (end<following_end).then(||super::document_char_at(document,end)).flatten();
        let separator = if before.is_none()
            || after.is_none()
            || before.is_some_and(char::is_whitespace)
            || after.is_some_and(char::is_whitespace)
        {
            ""
        } else {
            " "
        };
        edits.push(TextEdit::new(separator_range.start..end, separator));
    }
    Ok(edits)
}

/// Resolve the Normal-mode cursor destination for a source-backed line
/// transfer. Ex addresses denote insertion gaps in the old line order. Vim
/// leaves the cursor on the first non-blank character of the last transferred
/// line.
fn transfer_cursor_offset(
    document: &Document,
    operation: HardLineTransfer,
    source: Range<usize>,
    destination: usize,
) -> Result<usize, ExExecuteError> {
    let hard_lines = document.hard_line_snapshot();
    let mut lines = logical_line_contents(&hard_lines)?;
    if source.start >= source.end || source.end > lines.len() {
        return Err(ExExecuteError::AddressOutOfBounds {
            value: source.end as i128,
            line_count: lines.len(),
            zero_allowed: false,
        });
    }
    if destination > lines.len() {
        return Err(ExExecuteError::AddressOutOfBounds {
            value: destination as i128,
            line_count: lines.len(),
            zero_allowed: true,
        });
    }

    let count = source.end - source.start;
    if operation == HardLineTransfer::Move && source.start < destination && destination < source.end
    {
        return Err(ExExecuteError::DestinationInsideRange);
    }

    let last_line = match operation {
        HardLineTransfer::Copy => {
            let copied = lines[source.clone()].to_vec();
            lines.splice(destination..destination, copied);
            destination
                .checked_add(count - 1)
                .ok_or(ExExecuteError::AddressOverflow)?
        }
        HardLineTransfer::Move if destination == source.start || destination == source.end => {
            source.end - 1
        }
        HardLineTransfer::Move => {
            let moved = lines.drain(source.clone()).collect::<Vec<_>>();
            let insertion = if destination > source.end {
                destination
                    .checked_sub(count)
                    .ok_or(ExExecuteError::AddressOverflow)?
            } else {
                destination
            };
            lines.splice(insertion..insertion, moved);
            insertion
                .checked_add(count - 1)
                .ok_or(ExExecuteError::AddressOverflow)?
        }
    };
    let start = logical_line_start(&lines, last_line).ok_or(ExExecuteError::AddressOverflow)?;
    let indentation = lines[last_line]
        .grapheme_indices(true)
        .find(|(_, grapheme)| !grapheme.chars().all(char::is_whitespace))
        .map_or(0, |(offset, _)| offset);
    start
        .checked_add(indentation)
        .ok_or(ExExecuteError::AddressOverflow)
}

fn line_insertion_offset(
    snapshot: &HardLineSnapshot,
    insertion_line: usize,
) -> Result<usize, ExExecuteError> {
    if insertion_line == snapshot.line_count() {
        Ok(snapshot.text_length())
    } else {
        snapshot
            .line(insertion_line)
            .map(|line| line.content_range().start)
            .ok_or(ExExecuteError::AddressOverflow)
    }
}

fn line_insertion_payload(
    snapshot: &HardLineSnapshot,
    inserted: &[String],
    insertion_line: usize,
    existing_line_count: usize,
) -> Result<(FormattedTextPayload, usize), ExExecuteError> {
    if inserted.is_empty() || insertion_line > existing_line_count {
        return Err(ExExecuteError::AddressOverflow);
    }
    let (mut text, mut hard_breaks) = render_logical_lines(inserted)?;
    let relative_content_start = if insertion_line == existing_line_count && insertion_line != 0 {
        text.insert(0, '\n');
        for offset in &mut hard_breaks {
            *offset = offset
                .checked_add(1)
                .ok_or(ExExecuteError::AddressOverflow)?;
        }
        hard_breaks.insert(0, 0);
        1
    } else {
        hard_breaks.push(text.len());
        text.push('\n');
        0
    };
    Ok((
        FormattedTextPayload::new(snapshot, text, hard_breaks)?,
        relative_content_start,
    ))
}

fn logical_line_contents(snapshot: &HardLineSnapshot) -> Result<Vec<String>, ExExecuteError> {
    snapshot
        .lines(0..snapshot.line_count())
        .map_err(|_| ExExecuteError::AddressOverflow)
        .map(|lines| {
            lines
                .into_iter()
                .map(|line| snapshot.text()[line.content_range()].to_owned())
                .collect()
        })
}

fn render_logical_lines(lines: &[String]) -> Result<(String, Vec<usize>), ExExecuteError> {
    let text_length = lines
        .iter()
        .try_fold(0usize, |length, line| length.checked_add(line.len()))
        .ok_or(ExExecuteError::AddressOverflow)?;
    let text_length = text_length
        .checked_add(lines.len().saturating_sub(1))
        .ok_or(ExExecuteError::AddressOverflow)?;
    let mut text = String::with_capacity(text_length);
    let mut hard_breaks = Vec::with_capacity(lines.len().saturating_sub(1));
    for (index, line) in lines.iter().enumerate() {
        text.push_str(line);
        if index + 1 < lines.len() {
            hard_breaks.push(text.len());
            text.push('\n');
        }
    }
    Ok((text, hard_breaks))
}

/// Resolve a semantic line ordinal without inspecting U+000A values inside
/// line content. The separators introduced by `render_logical_lines` are one
/// byte each in the normalized formatted projection.
fn logical_line_start(lines: &[String], line: usize) -> Option<usize> {
    if line > lines.len() {
        return None;
    }
    if line == lines.len() {
        let content = lines
            .iter()
            .try_fold(0usize, |total, item| total.checked_add(item.len()))?;
        return content.checked_add(lines.len().saturating_sub(1));
    }
    lines.iter().take(line).try_fold(0usize, |offset, content| {
        offset.checked_add(content.len())?.checked_add(1)
    })
}

fn prepare_substitute(
    document: &Document,
    state: &ExExecutionState,
    context: &ExExecutionContext,
    command: &ExCommand,
    substitute: &Substitute,
    plan: &mut ExPlan,
) -> Result<(), ExExecuteError> {
    let pattern = if substitute.pattern.is_empty() {
        context
            .last_search_pattern
            .clone()
            .ok_or(ExExecuteError::NoPreviousSearch)?
    } else {
        substitute.pattern.clone()
    };
    let flags = effective_flags(&substitute.flags, state.last_substitute.as_ref());
    let stored = StoredSubstitute {
        pattern,
        replacement: substitute.replacement.clone(),
        flags: flags.clone(),
    };
    plan_substitution(
        document,
        context,
        command.range.as_ref(),
        substitute.count,
        stored,
        plan,
    )
}

#[allow(clippy::too_many_arguments)]
fn prepare_repeat_substitute(
    document: &Document,
    state: &ExExecutionState,
    context: &ExExecutionContext,
    command: &ExCommand,
    pattern_source: RepeatPattern,
    flags: &SubstituteFlags,
    count: Option<u64>,
    plan: &mut ExPlan,
) -> Result<(), ExExecuteError> {
    let previous = state
        .last_substitute
        .as_ref()
        .ok_or(ExExecuteError::NoPreviousSubstitute)?;
    let pattern = match pattern_source {
        RepeatPattern::LastSubstitute => previous.pattern.clone(),
        RepeatPattern::LastSearch => context
            .last_search_pattern
            .clone()
            .ok_or(ExExecuteError::NoPreviousSearch)?,
    };
    let stored = StoredSubstitute {
        pattern,
        replacement: previous.replacement.clone(),
        flags: effective_flags(flags, Some(previous)),
    };
    plan_substitution(
        document,
        context,
        command.range.as_ref(),
        count,
        stored,
        plan,
    )
}

fn effective_flags(
    requested: &SubstituteFlags,
    previous: Option<&StoredSubstitute>,
) -> SubstituteFlags {
    let mut result = if requested.use_previous_flags {
        previous
            .map(|value| value.flags.clone())
            .unwrap_or_default()
    } else {
        SubstituteFlags::default()
    };
    result.global |= requested.global;
    result.confirm |= requested.confirm;
    result.print |= requested.print;
    result.number |= requested.number;
    result.list |= requested.list;
    result.suppress_errors |= requested.suppress_errors;
    result.use_previous_flags = requested.use_previous_flags;
    if requested.ignore_case.is_some() {
        result.ignore_case = requested.ignore_case;
    }
    if requested.occurrence.is_some() {
        result.occurrence = requested.occurrence;
    }
    result
}

fn plan_substitution(
    document: &Document,
    context: &ExExecutionContext,
    range: Option<&ExRange>,
    count: Option<u64>,
    stored: StoredSubstitute,
    plan: &mut ExPlan,
) -> Result<(), ExExecuteError> {
    let lines = effective_counted_range(document, context, range, count)?;
    let limits = RegexLimits::default();
    let regex = CompiledRegex::compile(
        &stored.pattern,
        stored
            .flags
            .ignore_case
            .unwrap_or(context.search_options.case_insensitive(&stored.pattern)?),
        limits,
    )?;
    let replacement = ReplacementTemplate::compile(&stored.replacement, &regex)?;
    let (edits, payload_edits, fragment_edits, substitutions) =
        substitution_edits(document, lines, &regex, &replacement, &stored, limits)?;
    if substitutions == 0 && !stored.flags.suppress_errors {
        return Err(ExExecuteError::PatternNotFound(stored.pattern));
    }
    if stored.flags.confirm && substitutions != 0 {
        return Err(ExExecuteError::NeedsPolicy(
            ExPolicyRequest::ConfirmSubstitution(SubstitutePreview {
                pattern: stored.pattern,
                replacement: stored.replacement,
                matches: substitutions,
                edits,
                payload_edits,
                fragment_edits,
            }),
        ));
    }

    plan.outcome.substitutions = substitutions;
    if let Some(first) = edits.first() {
        plan.outcome.navigation = Some(ExNavigation::TextOffset(first.range.start));
    }
    if stored.flags.print || stored.flags.number || stored.flags.list {
        plan.outcome
            .frontend_requests
            .push(ExFrontendRequest::Info(ExInfoRequest::PrintLines {
                range: lines,
                number: stored.flags.number,
                list: stored.flags.list,
            }));
    }
    plan.mutation = ExMutation::Model(ModelRequest::ApplyFragmentEdits {
        document: document.id(),
        revision: document.revision(),
        edits: fragment_edits,
    });
    plan.next_substitute = Some(stored);
    Ok(())
}

fn substitution_edits(
    document: &Document,
    lines: HardLineRange,
    regex: &CompiledRegex,
    replacement: &ReplacementTemplate,
    substitute: &StoredSubstitute,
    limits: RegexLimits,
) -> Result<
    (
        Vec<TextEdit>,
        Vec<FormattedPayloadEdit>,
        Vec<crate::document::FragmentEdit>,
        usize,
    ),
    ExExecuteError,
> {
    use crate::document::{FragmentEdit, ReplacementFragment};
    let snapshot = document.hard_line_snapshot();
    validate_line_range(lines, snapshot.line_count())?;
    let start = snapshot.line(lines.start).unwrap().content_range().start;
    let end = snapshot.line(lines.end).unwrap().content_range().end;
    let input = RegexInput::new(&snapshot);
    let mut work = RegexWork::new(limits);
    let matches = regex.find_all(&input, start..end, &mut work)?;
    let occurrence = substitute.flags.occurrence.unwrap_or(1);
    if occurrence == 0 {
        return Err(ExExecuteError::InvalidCount(0));
    }
    let mut per_line = std::collections::HashMap::<usize, u64>::new();
    let mut selected = Vec::new();
    for matched in matches {
        let line = snapshot
            .line_at_offset(matched.range().start)
            .unwrap()
            .index();
        let ordinal = per_line.entry(line).or_default();
        *ordinal += 1;
        if if substitute.flags.global {
            *ordinal >= occurrence
        } else {
            *ordinal == occurrence
        } {
            matched.validate_edit(&snapshot)?;
            selected.push(matched);
        }
    }
    // Every selected extent is validated before building any source transaction.
    let mut edits = Vec::new();
    let mut payload_edits = Vec::new();
    let mut fragment_edits = Vec::new();
    for matched in selected {
        let mut text = String::new();
        let mut breaks = Vec::new();
        let mut fragments = Vec::new();
        for fragment in replacement.expand(&matched, &input)? {
            match fragment {
                ExpandedFragment::Literal {
                    text: part,
                    break_offsets,
                } => {
                    breaks.extend(break_offsets.iter().map(|at| text.len() + at));
                    text.push_str(&part);
                    fragments.push(ReplacementFragment::Literal(FormattedTextPayload::new(
                        &snapshot,
                        part,
                        break_offsets,
                    )?));
                }
                ExpandedFragment::Capture(range) => {
                    breaks.extend(
                        input
                            .hard_break_offsets(range.clone())
                            .map(|at| text.len() + at - range.start),
                    );
                    text.push_str(&snapshot.slice_utf8(range.clone()).expect("validated capture boundaries"));
                    fragments.push(ReplacementFragment::Capture(range));
                }
            }
        }
        edits.push(TextEdit::new(matched.range(), text.clone()));
        payload_edits.push(FormattedPayloadEdit::new(
            matched.range(),
            FormattedTextPayload::new(&snapshot, text, breaks)?,
        ));
        fragment_edits.push(FragmentEdit {
            range: matched.range(),
            fragments,
        });
    }
    let total = edits.len();
    Ok((edits, payload_edits, fragment_edits, total))
}

fn prepare_set(
    document: &Document,
    context: &ExExecutionContext,
    scope: SetScope,
    operation: &SetOperation,
    plan: &mut ExPlan,
) -> Result<(), ExExecuteError> {
    let mut values = PendingOptions {
        wrap: context.wrap,
        file_format: document.file_format(),
        fileformats: context.fileformats.clone(),
        search_options: context.search_options,
        text_width: context.text_width,
        indentation: context.indentation,
        visible_whitespace: context.visible_whitespace.clone(),
    };
    match operation {
        SetOperation::ShowChanged => {
            let mut shown = Vec::new();
            if values.visible_whitespace.enabled.is_some() {
                shown.push(display(ExOptionName::List, ExOptionValue::Boolean(values.visible_whitespace.enabled())));
            }
            if values.visible_whitespace.listchars.is_some() { shown.push(display(ExOptionName::ListChars, ExOptionValue::String(values.visible_whitespace.listchars().into()))); }
            if values.search_options.ignorecase {
                shown.push(display(
                    ExOptionName::IgnoreCase,
                    ExOptionValue::Boolean(true),
                ));
            }
            if values.search_options.smartcase {
                shown.push(display(
                    ExOptionName::SmartCase,
                    ExOptionValue::Boolean(true),
                ));
            }
            if !values.search_options.wrapscan {
                shown.push(display(
                    ExOptionName::WrapScan,
                    ExOptionValue::Boolean(false),
                ));
            }
            if values.wrap {
                shown.push(display(ExOptionName::Wrap, ExOptionValue::Boolean(true)));
            }
            if values.text_width.local_override().is_some() {
                shown.push(display(
                    ExOptionName::TextWidth,
                    ExOptionValue::Number(values.text_width.effective()),
                ));
            }
            let defaults = { let mut d = values.indentation; d.local = Default::default(); indentation_options(d) };
            shown.extend(indentation_options(values.indentation).into_iter().zip(defaults).filter_map(|(value, default)| (value != default).then_some(value)));
            if document.format() != crate::document::Format::Rtf
                && values.file_format != FileFormat::Unix
            {
                shown.push(display(
                    ExOptionName::FileFormat,
                    ExOptionValue::FileFormat(values.file_format),
                ));
            }
            if values.fileformats != [FileFormat::Unix, FileFormat::Dos] {
                shown.push(display(
                    ExOptionName::FileFormats,
                    ExOptionValue::FileFormats(values.fileformats.clone()),
                ));
            }
            plan.outcome
                .frontend_requests
                .push(ExFrontendRequest::Info(ExInfoRequest::Options(shown)));
        }
        SetOperation::ShowAll => {
            plan.outcome
                .frontend_requests
                .push(ExFrontendRequest::Info(ExInfoRequest::Options(
                    all_option_values(&values)
                        .into_iter()
                        .filter(|option| {
                            document.format() != crate::document::Format::Rtf
                                || option.name != ExOptionName::FileFormat
                        })
                        .collect(),
                )));
        }
        SetOperation::Options(operations) => {
            let original_file_format = values.file_format;
            for operation in operations {
                if document.format() == crate::document::Format::Rtf
                    && matches!(operation.name.as_str(), "fileformat" | "ff")
                {
                    return Err(ExExecuteError::UnsupportedOption(operation.name.clone()));
                }
                apply_option_operation(scope, operation, &mut values, plan)?;
            }
            if values.file_format != original_file_format {
                if !matches!(plan.mutation, ExMutation::None) {
                    return Err(ExExecuteError::ConflictingOptionChanges(
                        "fileformat".to_owned(),
                    ));
                }
                plan.stage_file_format(document, values.file_format);
            }
        }
    }
    Ok(())
}

#[derive(Clone, Debug)]
struct PendingOptions {
    wrap: bool,
    file_format: FileFormat,
    fileformats: Vec<FileFormat>,
    search_options: super::regex_v1::SearchOptions,
    text_width: crate::document::TextWidthSetting,
    indentation: crate::document::IndentationSetting,
    visible_whitespace: super::VisibleWhitespaceSetting,
}

fn all_option_values(values: &PendingOptions) -> Vec<ExOptionDisplay> {
    let mut result = vec![
        display(ExOptionName::List, ExOptionValue::Boolean(values.visible_whitespace.enabled())),
        display(ExOptionName::ListChars, ExOptionValue::String(values.visible_whitespace.listchars().into())),
        display(
            ExOptionName::IgnoreCase,
            ExOptionValue::Boolean(values.search_options.ignorecase),
        ),
        display(
            ExOptionName::SmartCase,
            ExOptionValue::Boolean(values.search_options.smartcase),
        ),
        display(
            ExOptionName::WrapScan,
            ExOptionValue::Boolean(values.search_options.wrapscan),
        ),
        display(ExOptionName::Wrap, ExOptionValue::Boolean(values.wrap)),
        display(
            ExOptionName::TextWidth,
            ExOptionValue::Number(values.text_width.effective()),
        ),
        display(
            ExOptionName::FileFormat,
            ExOptionValue::FileFormat(values.file_format),
        ),
        display(
            ExOptionName::FileFormats,
            ExOptionValue::FileFormats(values.fileformats.clone()),
        ),
    ];
    result.extend(indentation_options(values.indentation));
    result
}

fn display(name: ExOptionName, value: ExOptionValue) -> ExOptionDisplay {
    ExOptionDisplay { name, value }
}

fn indentation_options(setting: crate::document::IndentationSetting) -> Vec<ExOptionDisplay> {
    let value = setting.effective();
    vec![
        display(ExOptionName::AutoIndent, ExOptionValue::Boolean(value.autoindent)),
        display(ExOptionName::TabStop, ExOptionValue::Number(value.tabstop)),
        display(ExOptionName::ShiftWidth, ExOptionValue::Number(value.shiftwidth)),
        display(ExOptionName::SoftTabStop, ExOptionValue::Integer(value.softtabstop)),
        display(ExOptionName::ExpandTab, ExOptionValue::Boolean(value.expandtab)),
        display(ExOptionName::SmartTab, ExOptionValue::Boolean(value.smarttab)),
        display(ExOptionName::ContinueCommentsOnEnter, ExOptionValue::Boolean(value.continue_comments_on_enter)),
        display(ExOptionName::ContinueCommentsOnOpenLine, ExOptionValue::Boolean(value.continue_comments_on_open_line)),
    ]
}
fn apply_indentation_option(scope: SetScope, operation: &OptionOperation,
    values: &mut crate::document::IndentationSetting, plan: &mut ExPlan) -> Result<bool, ExExecuteError> {
    let name = operation.name.to_ascii_lowercase();
    let key = match name.as_str() {
        "autoindent" | "ai" => ExOptionName::AutoIndent,
        "tabstop" | "ts" => ExOptionName::TabStop,
        "shiftwidth" | "sw" => ExOptionName::ShiftWidth,
        "softtabstop" | "sts" => ExOptionName::SoftTabStop,
        "expandtab" | "et" => ExOptionName::ExpandTab,
        "smarttab" | "sta" => ExOptionName::SmartTab,
        "continuecommentsonenter" => ExOptionName::ContinueCommentsOnEnter,
        "continuecommentsonopenline" => ExOptionName::ContinueCommentsOnOpenLine,
        _ => return Ok(false),
    };
    if operation.action == OptionAction::Query {
        let display = indentation_options(*values).into_iter().find(|d| d.name == key).unwrap();
        show_option(plan, key, display.value);
        return Ok(true);
    }
    let old = *values;
    let effective = values.effective();
    let boolean = match key {
        ExOptionName::AutoIndent => Some((&mut values.local.autoindent, effective.autoindent)),
        ExOptionName::ExpandTab => Some((&mut values.local.expandtab, effective.expandtab)),
        ExOptionName::SmartTab => Some((&mut values.local.smarttab, effective.smarttab)),
        ExOptionName::ContinueCommentsOnEnter => Some((&mut values.local.continue_comments_on_enter, effective.continue_comments_on_enter)),
        ExOptionName::ContinueCommentsOnOpenLine => Some((&mut values.local.continue_comments_on_open_line, effective.continue_comments_on_open_line)),
        _ => None,
    };
    if let Some((local, value)) = boolean {
        *local = match &operation.action {
            OptionAction::Enable => Some(true), OptionAction::Disable => Some(false),
            OptionAction::Toggle => Some(!value), OptionAction::Inherit | OptionAction::Reset => None,
            _ => return Err(ExExecuteError::UnsupportedOptionOperation(name)),
        };
    } else {
        let number = match &operation.action {
            OptionAction::Assign(value) => Some(value.parse::<i32>().map_err(|_| ExExecuteError::InvalidOptionValue { option: name.clone(), value: value.clone() })?),
            OptionAction::Inherit | OptionAction::Reset => None,
            _ => return Err(ExExecuteError::UnsupportedOptionOperation(name)),
        };
        if number.is_some_and(|n| match key { ExOptionName::TabStop => !(1..=1024).contains(&n), ExOptionName::ShiftWidth => !(0..=1024).contains(&n), _ => !(-1..=1024).contains(&n) }) {
            return Err(ExExecuteError::InvalidOptionValue { option: name, value: number.unwrap().to_string() });
        }
        match key {
            ExOptionName::TabStop => values.local.tabstop = number.map(|v| v as u32),
            ExOptionName::ShiftWidth => values.local.shiftwidth = number.map(|v| v as u32),
            ExOptionName::SoftTabStop => values.local.softtabstop = number,
            _ => unreachable!(),
        }
    }
    if old != *values {
        plan.outcome.option_effects.push(ExOptionEffect { scope, name: key,
            old_value: ExOptionValue::Indentation(old), new_value: ExOptionValue::Indentation(*values) });
    }
    Ok(true)
}

fn show_listchars(plan: &mut ExPlan, setting: &super::VisibleWhitespaceSetting) {
    show_option(plan, ExOptionName::ListChars, ExOptionValue::String(setting.listchars().into()));
}

/// Vim's comma-option modifiers match one complete contiguous token sequence.
/// Category parsing happens after modification; repeated category names are
/// legal and are not independently removed or deduplicated here.
fn modify_listchars(current: &str, value: &str, action: &OptionAction) -> String {
    if value.is_empty() { return current.to_owned(); }
    let found = current.match_indices(value).find_map(|(start, _)| {
        let end = start + value.len();
        ((start == 0 || current.as_bytes()[start - 1] == b',')
            && (end == current.len() || current.as_bytes()[end] == b','))
            .then_some(start..end)
    });
    match action {
        OptionAction::Remove(_) => {
            let Some(mut range) = found else { return current.to_owned(); };
            if current.as_bytes().get(range.end) == Some(&b',') { range.end += 1; }
            else if range.start > 0 { range.start -= 1; }
            format!("{}{}", &current[..range.start], &current[range.end..])
        }
        OptionAction::Append(_) | OptionAction::Prepend(_) if found.is_some() => current.to_owned(),
        OptionAction::Append(_) => {
            let separator = if current.is_empty() || current.ends_with(',') { "" } else { "," };
            format!("{current}{separator}{value}")
        }
        OptionAction::Prepend(_) => {
            let separator = if current.is_empty() { "" } else { "," };
            format!("{value}{separator}{current}")
        }
        _ => unreachable!("listchars modifier helper accepts only +=, ^=, and -="),
    }
}

fn apply_whitespace_option(scope: SetScope, operation: &OptionOperation, setting: &mut super::VisibleWhitespaceSetting, plan: &mut ExPlan) -> Result<bool, ExExecuteError> {
    let name = operation.name.to_ascii_lowercase();
    let key = match name.as_str() { "list" => ExOptionName::List, "listchars" | "lcs" => ExOptionName::ListChars, _ => return Ok(false) };
    let old = setting.clone();
    if key == ExOptionName::List {
        match &operation.action {
            OptionAction::Query => { show_option(plan, key, ExOptionValue::Boolean(setting.enabled())); return Ok(true); }
            OptionAction::Enable => setting.enabled = Some(true),
            OptionAction::Disable => setting.enabled = Some(false),
            OptionAction::Toggle => setting.enabled = Some(!setting.enabled()),
            OptionAction::Reset | OptionAction::Inherit => setting.enabled = None,
            _ => return Err(ExExecuteError::UnsupportedOptionOperation(name)),
        }
    } else {
        match &operation.action {
            OptionAction::Query => { show_listchars(plan, setting); return Ok(true); }
            OptionAction::Reset | OptionAction::Inherit => setting.listchars = None,
            OptionAction::Assign(value) => setting.listchars = Some(value.clone()),
            OptionAction::Append(value) | OptionAction::Prepend(value) | OptionAction::Remove(value) => {
                setting.listchars = Some(modify_listchars(setting.listchars(), value, &operation.action));
            }
            _ => return Err(ExExecuteError::UnsupportedOptionOperation(name)),
        }
        crate::layout::ListChars::parse(setting.listchars()).map_err(|_| ExExecuteError::InvalidOptionValue { option: name, value: setting.listchars().into() })?;
    }
    if old != *setting { plan.outcome.option_effects.push(ExOptionEffect { scope, name: key, old_value: ExOptionValue::VisibleWhitespace(old), new_value: ExOptionValue::VisibleWhitespace(setting.clone()) }); }
    Ok(true)
}

fn apply_option_operation(
    scope: SetScope,
    operation: &OptionOperation,
    values: &mut PendingOptions,
    plan: &mut ExPlan,
) -> Result<(), ExExecuteError> {
    let name = operation.name.to_ascii_lowercase();
    if apply_indentation_option(scope, operation, &mut values.indentation, plan)? { return Ok(()); }
    if apply_whitespace_option(scope, operation, &mut values.visible_whitespace, plan)? { return Ok(()); }
    match name.as_str() {
        "ignorecase" | "ic" => apply_boolean_option(
            scope,
            ExOptionName::IgnoreCase,
            false,
            &operation.action,
            &mut values.search_options.ignorecase,
            plan,
        ),
        "smartcase" | "sc" => apply_boolean_option(
            scope,
            ExOptionName::SmartCase,
            false,
            &operation.action,
            &mut values.search_options.smartcase,
            plan,
        ),
        "wrapscan" | "ws" => apply_boolean_option(
            scope,
            ExOptionName::WrapScan,
            true,
            &operation.action,
            &mut values.search_options.wrapscan,
            plan,
        ),
        "wrap" => apply_boolean_option(
            scope,
            ExOptionName::Wrap,
            false,
            &operation.action,
            &mut values.wrap,
            plan,
        ),
        "textwidth" | "tw" => {
            // Both `:set` and `:setlocal` forms set a buffer override; the
            // application default is owned by Settings, never by Ex.
            let old = values.text_width.local_override();
            match &operation.action {
                OptionAction::Query => {
                    show_option(
                        plan,
                        ExOptionName::TextWidth,
                        ExOptionValue::Number(values.text_width.effective()),
                    );
                    return Ok(());
                }
                OptionAction::Assign(value) => {
                    values
                        .text_width
                        .set_local(Some(parse_text_width(&name, value)?));
                }
                OptionAction::Inherit | OptionAction::Reset => values.text_width.set_local(None),
                _ => return Err(ExExecuteError::UnsupportedOptionOperation(name)),
            }
            let new = values.text_width.local_override();
            if new != old {
                plan.outcome.option_effects.push(ExOptionEffect {
                    scope,
                    name: ExOptionName::TextWidth,
                    old_value: ExOptionValue::OptionalNumber(old),
                    new_value: ExOptionValue::OptionalNumber(new),
                });
            }
            Ok(())
        }
        "fileformat" | "ff" => {
            let old = values.file_format;
            match &operation.action {
                OptionAction::Query => show_option(
                    plan,
                    ExOptionName::FileFormat,
                    ExOptionValue::FileFormat(old),
                ),
                OptionAction::Assign(value) => {
                    values.file_format = parse_file_format(&name, value)?
                }
                OptionAction::Reset => values.file_format = FileFormat::Unix,
                _ => return Err(ExExecuteError::UnsupportedOptionOperation(name)),
            }
            if values.file_format != old {
                plan.outcome.option_effects.push(ExOptionEffect {
                    scope,
                    name: ExOptionName::FileFormat,
                    old_value: ExOptionValue::FileFormat(old),
                    new_value: ExOptionValue::FileFormat(values.file_format),
                });
            }
            Ok(())
        }
        "fileformats" | "ffs" => {
            if scope == SetScope::Local {
                return Err(ExExecuteError::UnsupportedOptionOperation(name));
            }
            let old = values.fileformats.clone();
            match &operation.action {
                OptionAction::Query => show_option(
                    plan,
                    ExOptionName::FileFormats,
                    ExOptionValue::FileFormats(old.clone()),
                ),
                OptionAction::Assign(value) => {
                    values.fileformats = parse_file_formats(&name, value)?
                }
                OptionAction::Append(value) => {
                    for format in parse_file_formats(&name, value)? {
                        if !values.fileformats.contains(&format) {
                            values.fileformats.push(format);
                        }
                    }
                }
                OptionAction::Prepend(value) => {
                    let additions = parse_file_formats(&name, value)?;
                    values
                        .fileformats
                        .retain(|format| !additions.contains(format));
                    let mut next = additions;
                    next.append(&mut values.fileformats);
                    values.fileformats = next;
                }
                OptionAction::Remove(value) => {
                    let removals = parse_file_formats(&name, value)?;
                    values
                        .fileformats
                        .retain(|format| !removals.contains(format));
                }
                OptionAction::Reset => values.fileformats = vec![FileFormat::Unix, FileFormat::Dos],
                _ => return Err(ExExecuteError::UnsupportedOptionOperation(name)),
            }
            if values.fileformats != old {
                plan.outcome.option_effects.push(ExOptionEffect {
                    scope,
                    name: ExOptionName::FileFormats,
                    old_value: ExOptionValue::FileFormats(old),
                    new_value: ExOptionValue::FileFormats(values.fileformats.clone()),
                });
            }
            Ok(())
        }
        _ => Err(ExExecuteError::UnsupportedOption(operation.name.clone())),
    }
}

fn apply_boolean_option(
    scope: SetScope,
    name: ExOptionName,
    default: bool,
    action: &OptionAction,
    current: &mut bool,
    plan: &mut ExPlan,
) -> Result<(), ExExecuteError> {
    let old = *current;
    match action {
        OptionAction::Enable => *current = true,
        OptionAction::Disable => *current = false,
        OptionAction::Toggle => *current = !*current,
        OptionAction::Reset => *current = default,
        OptionAction::Query => {
            show_option(plan, name, ExOptionValue::Boolean(*current));
            return Ok(());
        }
        _ => {
            return Err(ExExecuteError::UnsupportedOptionOperation(format!(
                "{name:?}"
            )))
        }
    }
    if *current != old {
        plan.outcome.option_effects.push(ExOptionEffect {
            scope,
            name,
            old_value: ExOptionValue::Boolean(old),
            new_value: ExOptionValue::Boolean(*current),
        });
    }
    Ok(())
}

fn show_option(plan: &mut ExPlan, name: ExOptionName, value: ExOptionValue) {
    plan.outcome
        .frontend_requests
        .push(ExFrontendRequest::Info(ExInfoRequest::Options(vec![
            display(name, value),
        ])));
}

/// A positive unsigned 32-bit integer. Zero, signs, fractions, malformed
/// text, and overflow are rejected without changing the prior value.
fn parse_text_width(option: &str, value: &str) -> Result<u32, ExExecuteError> {
    let invalid = || ExExecuteError::InvalidOptionValue {
        option: option.to_owned(),
        value: value.to_owned(),
    };
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(invalid());
    }
    match value.parse::<u32>() {
        Ok(width) if width > 0 => Ok(width),
        _ => Err(invalid()),
    }
}

fn parse_file_format(option: &str, value: &str) -> Result<FileFormat, ExExecuteError> {
    match value.to_ascii_lowercase().as_str() {
        "unix" => Ok(FileFormat::Unix),
        "dos" => Ok(FileFormat::Dos),
        "mac" => Ok(FileFormat::Mac),
        _ => Err(ExExecuteError::InvalidOptionValue {
            option: option.to_owned(),
            value: value.to_owned(),
        }),
    }
}

fn parse_file_formats(option: &str, value: &str) -> Result<Vec<FileFormat>, ExExecuteError> {
    if value.is_empty() {
        return Ok(Vec::new());
    }
    let mut formats = Vec::new();
    for item in value.split(',') {
        let format = parse_file_format(option, item)?;
        if formats.contains(&format) {
            return Err(ExExecuteError::InvalidOptionValue {
                option: option.to_owned(),
                value: value.to_owned(),
            });
        }
        formats.push(format);
    }
    Ok(formats)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::command::ex::parse_ex;
    use crate::document::{Encoding, Format, MappingOutcome, TextRange};

    #[derive(Default)]
    struct TestRegisters(BTreeMap<char, ExRegisterValue>);

    impl ExRegisterReader for TestRegisters {
        fn read(&self, requested: Option<char>) -> Option<ExRegisterValue> {
            self.0.get(&requested.unwrap_or('"')).cloned()
        }
    }

    fn context(current_line: usize) -> ExExecutionContext {
        ExExecutionContext {
            current_line,
            ..ExExecutionContext::default()
        }
    }

    fn execute(
        document: &mut Document,
        state: &mut ExExecutionState,
        current_line: usize,
        command: &str,
    ) -> Result<ExOutcome, ExExecuteError> {
        execute_ex(
            document,
            state,
            &context(current_line),
            &parse_ex(command).unwrap(),
            &(),
        )
    }

    #[test]
    fn address_ranges_are_one_based_and_semicolon_rebinds_dot() {
        let document = Document::new("a\nb\nc\nd");
        assert_eq!(
            resolve_range(
                parse_ex(":1;.+2join").unwrap().range.as_ref(),
                3,
                document.line_count()
            )
            .unwrap(),
            HardLineRange { start: 0, end: 2 }
        );
        assert!(matches!(
            resolve_address(
                ExAddress {
                    base: AddressBase::Absolute(0),
                    offset: 0
                },
                0,
                document.line_count()
            ),
            Err(ExExecuteError::AddressOutOfBounds { .. })
        ));
    }

    #[test]
    fn counted_line_commands_start_at_the_last_addressed_line() {
        let mut deleted = Document::new("one\ntwo\nthree\nfour\nfive\nsix");
        execute(
            &mut deleted,
            &mut ExExecutionState::default(),
            5,
            ":1+1,2+1delete 2",
        )
        .unwrap();
        assert_eq!(deleted.text(), "one\ntwo\nfive\nsix");

        let mut yanked = Document::new("one\ntwo\nthree\nfour\nfive\nsix");
        let yank = execute(
            &mut yanked,
            &mut ExExecutionState::default(),
            5,
            ":2;.+1yank a 2",
        )
        .unwrap();
        assert_eq!(yanked.text(), "one\ntwo\nthree\nfour\nfive\nsix");
        assert_eq!(yank.register_effects[0].requested, Some('a'));
        assert_eq!(yank.register_effects[0].value.text, "three\nfour\n");

        let mut substituted = Document::new("x\nx\nx\nx\nx\nx");
        execute(
            &mut substituted,
            &mut ExExecutionState::default(),
            5,
            ":1+1,2+1substitute/x/y/g 2",
        )
        .unwrap();
        assert_eq!(substituted.text(), "x\nx\ny\ny\nx\nx");

        let mut joined = Document::new("one\ntwo\nthree\nfour");
        execute(
            &mut joined,
            &mut ExExecutionState::default(),
            3,
            ":1;.+1join 2",
        )
        .unwrap();
        assert_eq!(joined.text(), "one\ntwo three\nfour");
    }

    #[test]
    fn finite_counted_line_commands_clamp_their_endpoint_to_eof() {
        let mut deleted = Document::new("one\ntwo\nthree\nfour\nfive");
        execute(
            &mut deleted,
            &mut ExExecutionState::default(),
            0,
            ":4delete 3",
        )
        .unwrap();
        assert_eq!(deleted.text(), "one\ntwo\nthree");

        let mut yanked = Document::new("one\ntwo\nthree\nfour\nfive");
        let yank = execute(
            &mut yanked,
            &mut ExExecutionState::default(),
            0,
            ":2,4yank a 3",
        )
        .unwrap();
        assert_eq!(yank.register_effects[0].requested, Some('a'));
        assert_eq!(yank.register_effects[0].value.text, "four\nfive\n");

        let mut substituted = Document::from_bytes(
            b"**x**\n\n__x__\n\n*x*\n\n_x_\n\n_x_".to_vec(),
            Encoding::Latin1,
            Format::Markdown,
        )
        .unwrap();
        execute(
            &mut substituted,
            &mut ExExecutionState::default(),
            0,
            ":4substitute/x/y/g 3",
        )
        .unwrap();
        assert_eq!(
            substituted.source_bytes(),
            b"**x**\n\n__x__\n\n*x*\n\n_y_\n\n_y_"
        );

        let mut joined = Document::new("one\ntwo\nthree\nfour\nfive");
        execute(&mut joined, &mut ExExecutionState::default(), 0, ":4join 3").unwrap();
        assert_eq!(joined.text(), "one\ntwo\nthree\nfour five");
    }

    #[test]
    fn counted_range_endpoint_arithmetic_is_checked() {
        let mut document = Document::new("one\ntwo\nthree\nfour");
        let result = execute(
            &mut document,
            &mut ExExecutionState::default(),
            0,
            ":2,3delete 18446744073709551615",
        );
        assert!(matches!(result, Err(ExExecuteError::AddressOverflow)));
        assert_eq!(document.text(), "one\ntwo\nthree\nfour");
    }

    #[test]
    fn preparation_stages_delete_and_register_effect_before_commit() {
        let mut document = Document::new("one\ntwo\nthree");
        let mut state = ExExecutionState::default();
        let command = parse_ex(":2delete a").unwrap();
        let plan = prepare_ex(&document, &state, &context(0), &command, &()).unwrap();
        assert_eq!(document.text(), "one\ntwo\nthree");
        assert_eq!(plan.staged_register_effects()[0].requested, Some('a'));
        assert_eq!(plan.staged_register_effects()[0].value.text, "two\n");

        let outcome = commit_ex(&mut document, &mut state, plan).unwrap();
        assert_eq!(document.text(), "one\nthree");
        assert!(outcome.document_changed);
    }

    #[test]
    fn deleting_the_unterminated_last_line_removes_the_preceding_break() {
        let mut document = Document::new("one\ntwo");
        execute(
            &mut document,
            &mut ExExecutionState::default(),
            1,
            ":delete",
        )
        .unwrap();
        assert_eq!(document.text(), "one");
    }

    #[test]
    fn put_before_and_after_are_single_undoable_edits() {
        let mut registers = TestRegisters::default();
        registers
            .0
            .insert('"', ExRegisterValue::linewise("middle\n"));
        let mut state = ExExecutionState::default();
        let mut document = Document::new("first\nlast");
        let command = parse_ex(":1put").unwrap();
        execute_ex(&mut document, &mut state, &context(0), &command, &registers).unwrap();
        assert_eq!(document.text(), "first\nmiddle\nlast");
        assert!(document.undo());
        assert_eq!(document.text(), "first\nlast");

        let command = parse_ex(":1put!").unwrap();
        execute_ex(&mut document, &mut state, &context(0), &command, &registers).unwrap();
        assert_eq!(document.text(), "middle\nfirst\nlast");
    }

    #[test]
    fn multiline_put_targets_the_last_inserted_lines_first_nonblank() {
        let mut registers = TestRegisters::default();
        registers
            .0
            .insert('"', ExRegisterValue::linewise("  alpha\n   beta\n"));
        let mut document = Document::from_bytes_with_file_format(
            b"start\rend".to_vec(),
            Encoding::Latin1,
            Format::PlainText,
            FileFormat::Mac,
        )
        .unwrap();
        let outcome = execute_ex(
            &mut document,
            &mut ExExecutionState::default(),
            &context(0),
            &parse_ex(":1put").unwrap(),
            &registers,
        )
        .unwrap();

        assert_eq!(document.text(), "start\n  alpha\n   beta\nend");
        assert_eq!(document.source_bytes(), b"start\r  alpha\r   beta\rend");
        assert_eq!(
            outcome.navigation,
            Some(ExNavigation::TextOffset(
                document.text().find("beta").unwrap()
            ))
        );
    }

    #[test]
    fn put_bang_uses_the_last_range_address_and_zero_is_put_specific() {
        let mut registers = TestRegisters::default();
        registers
            .0
            .insert('"', ExRegisterValue::linewise("inserted\n"));

        for command in [":1+1,2+1put!", ":1;.+2put!"] {
            let mut document = Document::new("one\ntwo\nthree\nfour");
            execute_ex(
                &mut document,
                &mut ExExecutionState::default(),
                &context(3),
                &parse_ex(command).unwrap(),
                &registers,
            )
            .unwrap();
            assert_eq!(
                document.text(),
                "one\ntwo\ninserted\nthree\nfour",
                "{command}"
            );
        }

        let mut at_start = Document::new("one\ntwo");
        execute_ex(
            &mut at_start,
            &mut ExExecutionState::default(),
            &context(1),
            &parse_ex(":0put").unwrap(),
            &registers,
        )
        .unwrap();
        assert_eq!(at_start.text(), "inserted\none\ntwo");

        let mut ordinary_range = Document::new("one\ntwo");
        let error = execute(
            &mut ordinary_range,
            &mut ExExecutionState::default(),
            1,
            ":0delete",
        )
        .unwrap_err();
        assert!(matches!(
            error,
            ExExecuteError::AddressOutOfBounds {
                zero_allowed: false,
                ..
            }
        ));
        assert_eq!(ordinary_range.text(), "one\ntwo");
    }

    #[test]
    fn join_and_join_bang_have_distinct_whitespace_rules() {
        let mut normal = Document::new("one\n   two\nthree");
        execute(&mut normal, &mut ExExecutionState::default(), 0, ":1,2join").unwrap();
        assert_eq!(normal.text(), "one two\nthree");

        let mut bang = Document::new("one\n   two");
        execute(&mut bang, &mut ExExecutionState::default(), 0, ":1,2join!").unwrap();
        assert_eq!(bang.text(), "one   two");
    }

    #[test]
    fn single_address_join_uses_the_implicit_following_line_and_first_nonblank_cursor() {
        let mut document = Document::new("one\ntwo\n  three\n   four\nfive");
        let outcome =
            execute(&mut document, &mut ExExecutionState::default(), 0, ":3join").unwrap();
        assert_eq!(document.text(), "one\ntwo\n  three four\nfive");
        assert_eq!(
            outcome.navigation,
            Some(ExNavigation::TextOffset(
                document.text().find("three").unwrap()
            ))
        );

        let mut explicit_single_line = Document::new("one\ntwo\nthree\nfour");
        let outcome = execute(
            &mut explicit_single_line,
            &mut ExExecutionState::default(),
            0,
            ":3,3join",
        )
        .unwrap();
        assert_eq!(explicit_single_line.text(), "one\ntwo\nthree\nfour");
        assert!(!outcome.document_changed);
    }

    #[test]
    fn delete_targets_the_first_nonblank_of_the_surviving_line() {
        let mut middle = Document::new("  one\n  two\n   three");
        let outcome =
            execute(&mut middle, &mut ExExecutionState::default(), 0, ":2delete").unwrap();
        assert_eq!(middle.text(), "  one\n   three");
        assert_eq!(
            outcome.navigation,
            Some(ExNavigation::TextOffset(
                middle.text().find("three").unwrap()
            ))
        );

        let mut last = Document::new("  one\n  two\n   three");
        let outcome = execute(&mut last, &mut ExExecutionState::default(), 0, ":$delete").unwrap();
        assert_eq!(last.text(), "  one\n  two");
        assert_eq!(
            outcome.navigation,
            Some(ExNavigation::TextOffset(last.text().find("two").unwrap()))
        );
    }

    #[test]
    fn copy_and_move_are_atomic_for_plain_text() {
        let mut state = ExExecutionState::default();
        let mut copied = Document::new("a\nb\nc");
        execute(&mut copied, &mut state, 0, ":2copy 3").unwrap();
        assert_eq!(copied.text(), "a\nb\nc\nb");
        assert!(copied.undo());
        assert_eq!(copied.text(), "a\nb\nc");

        let mut moved = Document::new("a\nb\nc\nd");
        execute(&mut moved, &mut state, 0, ":2,3move 0").unwrap();
        assert_eq!(moved.text(), "b\nc\na\nd");
        assert!(moved.undo());
        assert_eq!(moved.text(), "a\nb\nc\nd");
    }

    #[test]
    fn ex_ranges_use_semantic_hard_breaks_not_literal_lf() {
        let mut document = Document::from_bytes_with_file_format(
            b"a\rb\nc".to_vec(),
            Encoding::Utf8,
            Format::PlainText,
            FileFormat::Mac,
        )
        .unwrap();
        assert_eq!(document.text(), "a\nb\nc");
        assert_eq!(document.line_count(), 2);
        assert_eq!(
            hard_line_text_range(&document, HardLineRange { start: 1, end: 1 }).unwrap(),
            2..5
        );

        let yank = execute(&mut document, &mut ExExecutionState::default(), 1, ":yank").unwrap();
        assert_eq!(yank.register_effects[0].value.text, "b\nc\n");
        assert_eq!(
            yank.register_effects[0].value.hard_break_offsets(),
            &[3],
            "the literal LF stays unmarked and the linewise terminator is semantic"
        );

        execute(
            &mut document,
            &mut ExExecutionState::default(),
            1,
            ":substitute/c/C/",
        )
        .unwrap();
        assert_eq!(document.text(), "a\nb\nC");
        assert_eq!(document.line_count(), 2);
        assert_eq!(document.source_bytes(), b"a\rb\nC");

        execute(
            &mut document,
            &mut ExExecutionState::default(),
            0,
            ":1,2join!",
        )
        .unwrap();
        assert_eq!(document.text(), "ab\nC");
        assert_eq!(document.line_count(), 1);
        assert_eq!(document.source_bytes(), b"ab\nC");
    }

    #[test]
    fn copy_and_move_preserve_literal_lf_with_mac_line_endings() {
        let new_document = || {
            Document::from_bytes_with_file_format(
                b"a\rb\nc".to_vec(),
                Encoding::Utf8,
                Format::PlainText,
                FileFormat::Mac,
            )
            .unwrap()
        };

        let mut copied = new_document();
        execute(&mut copied, &mut ExExecutionState::default(), 1, ":2copy 1").unwrap();
        assert_eq!(copied.text(), "a\nb\nc\nb\nc");
        assert_eq!(copied.line_count(), 3);
        assert_eq!(copied.source_bytes(), b"a\rb\nc\rb\nc");
        assert!(copied.undo());
        assert_eq!(copied.source_bytes(), b"a\rb\nc");

        let mut moved = new_document();
        execute(&mut moved, &mut ExExecutionState::default(), 1, ":2move 0").unwrap();
        assert_eq!(moved.text(), "b\nc\na");
        assert_eq!(moved.line_count(), 2);
        assert_eq!(moved.source_bytes(), b"b\nc\ra");
        assert!(moved.undo());
        assert_eq!(moved.source_bytes(), b"a\rb\nc");
    }

    #[test]
    fn put_spells_semantic_breaks_with_the_document_file_format() {
        let mut registers = TestRegisters::default();
        registers
            .0
            .insert('"', ExRegisterValue::linewise("middle\n"));
        let mut document = Document::from_bytes_with_file_format(
            b"first\rlast".to_vec(),
            Encoding::Utf8,
            Format::PlainText,
            FileFormat::Mac,
        )
        .unwrap();
        execute_ex(
            &mut document,
            &mut ExExecutionState::default(),
            &context(0),
            &parse_ex(":1put").unwrap(),
            &registers,
        )
        .unwrap();
        assert_eq!(document.text(), "first\nmiddle\nlast");
        assert_eq!(document.line_count(), 3);
        assert_eq!(document.source_bytes(), b"first\rmiddle\rlast");
    }

    #[test]
    fn put_splits_only_marked_register_breaks_and_preserves_literal_lf() {
        let mut registers = TestRegisters::default();
        registers.0.insert(
            '"',
            ExRegisterValue::try_new(
                "left\nliteral\nright\n",
                ExRegisterKind::Linewise,
                vec![12, 18],
            )
            .unwrap(),
        );
        let mut document = Document::from_bytes_with_file_format(
            b"first\rlast".to_vec(),
            Encoding::Utf8,
            Format::PlainText,
            FileFormat::Mac,
        )
        .unwrap();
        execute_ex(
            &mut document,
            &mut ExExecutionState::default(),
            &context(0),
            &parse_ex(":1put").unwrap(),
            &registers,
        )
        .unwrap();
        assert_eq!(document.text(), "first\nleft\nliteral\nright\nlast");
        assert_eq!(document.line_count(), 4);
        assert_eq!(
            document.source_bytes(),
            b"first\rleft\nliteral\rright\rlast"
        );
        assert!(document.undo());
        assert_eq!(document.source_bytes(), b"first\rlast");
    }

    #[test]
    fn markdown_copy_and_move_preserve_source_syntax_and_target_the_last_line() {
        let source = b"# H\n\n  **one**\n\n_two_\n\ntail";
        let mut copied = Document::from_bytes(
            source.to_vec(),
            crate::document::Encoding::Utf8,
            Format::Markdown,
        )
        .unwrap();
        let copied_outcome = execute(
            &mut copied,
            &mut ExExecutionState::default(),
            0,
            ":1,2copy 4",
        )
        .unwrap();
        assert_eq!(
            copied.source_bytes(),
            b"# H\n\n  **one**\n\n_two_\n\ntail\n\n# H\n\n  **one**"
        );
        assert_eq!(copied.text(), "H\n  one\ntwo\ntail\nH\n  one");
        assert_eq!(
            copied_outcome.navigation,
            Some(ExNavigation::TextOffset("H\n  one\ntwo\ntail\nH\n  ".len()))
        );
        assert_eq!(
            copied_outcome
                .model_transaction()
                .unwrap()
                .summary()
                .source_patches()
                .len(),
            1
        );
        assert!(copied.undo());
        assert_eq!(copied.source_bytes(), source);

        let mut moved = Document::from_bytes(
            source.to_vec(),
            crate::document::Encoding::Utf8,
            Format::Markdown,
        )
        .unwrap();
        let moved_outcome = execute(
            &mut moved,
            &mut ExExecutionState::default(),
            0,
            ":1,2move 4",
        )
        .unwrap();
        assert_eq!(moved.source_bytes(), b"_two_\n\ntail\n\n# H\n\n  **one**");
        assert_eq!(moved.text(), "two\ntail\nH\n  one");
        assert_eq!(
            moved_outcome.navigation,
            Some(ExNavigation::TextOffset("two\ntail\nH\n  ".len()))
        );
        assert_eq!(
            moved_outcome
                .model_transaction()
                .unwrap()
                .summary()
                .source_patches()
                .len(),
            2
        );
        assert!(moved.undo());
        assert_eq!(moved.source_bytes(), source);
    }

    #[test]
    fn move_at_either_source_boundary_is_a_history_free_no_op() {
        for command in [":1,2move 0", ":1,2move 2"] {
            let mut document = Document::new("a\n  b\nc");
            let revision = document.revision();
            let history = document.history_status();
            let outcome =
                execute(&mut document, &mut ExExecutionState::default(), 0, command).unwrap();
            assert_eq!(document.text(), "a\n  b\nc", "{command}");
            assert_eq!(document.revision(), revision, "{command}");
            assert_eq!(document.history_status(), history, "{command}");
            assert_eq!(
                outcome.navigation,
                Some(ExNavigation::TextOffset("a\n  ".len())),
                "{command}"
            );
            assert!(!outcome.document_changed, "{command}");
        }
    }

    #[test]
    fn substitute_uses_unicode_regexes_ranges_and_is_one_undo_step() {
        let mut document = Document::new("Café 12\nCAFÉ 34\ncafé 56");
        let mut state = ExExecutionState::default();
        let outcome = execute(&mut document, &mut state, 0, ":%s/café ([0-9]+)/&-\\1/gi").unwrap();
        assert_eq!(outcome.substitutions, 3);
        assert_eq!(document.text(), "Café 12-12\nCAFÉ 34-34\ncafé 56-56");
        assert!(document.undo());
        assert_eq!(document.text(), "Café 12\nCAFÉ 34\ncafé 56");
    }

    #[test]
    fn substitute_exposes_exact_discontiguous_model_map() {
        let mut document = Document::new("a\nmiddle\na");
        let middle = TextRange::new(
            document.text_point(2).unwrap(),
            document.text_point(8).unwrap(),
        )
        .unwrap();
        let outcome = execute(
            &mut document,
            &mut ExExecutionState::default(),
            0,
            ":%s/a/alpha/g",
        )
        .unwrap();

        assert_eq!(document.text(), "alpha\nmiddle\nalpha");
        let transaction = outcome
            .model_transaction()
            .expect("a source-changing substitute publishes a model transaction");
        assert_eq!(transaction.summary().formatted_splices().len(), 2);
        assert_eq!(
            transaction.summary().formatted_splices()[0].old_range(),
            0..1
        );
        assert_eq!(
            transaction.summary().formatted_splices()[1].old_range(),
            9..10
        );
        let MappingOutcome::Moved(mapped_middle) = transaction
            .text_position_map()
            .map_text_range(middle)
            .unwrap()
        else {
            panic!("unchanged content between substitutions must retain its identity");
        };
        assert_eq!(mapped_middle.segments().len(), 1);
        assert_eq!(mapped_middle.segments()[0].start().offset(), 6);
        assert_eq!(mapped_middle.segments()[0].end().offset(), 12);
    }

    #[test]
    fn compound_outcome_retains_ordered_model_publications_and_all_effects() {
        let mut document = Document::new("a\na");
        let mut state = ExExecutionState::default();

        let mut aggregate = execute(&mut document, &mut state, 0, ":s/a/x/").unwrap();
        let first_transaction = aggregate.model_transaction().unwrap().clone();
        let delete = execute(&mut document, &mut state, 1, ":2delete a").unwrap();
        let second_transaction = delete.model_transaction().unwrap().clone();
        aggregate.try_merge(delete).unwrap();
        aggregate
            .try_merge(execute(&mut document, &mut state, 0, ":set wrap").unwrap())
            .unwrap();
        aggregate
            .try_merge(execute(&mut document, &mut state, 0, ":marks a").unwrap())
            .unwrap();

        assert!(aggregate.document_changed);
        assert_eq!(aggregate.substitutions, 1);
        assert_eq!(aggregate.register_effects.len(), 1);
        assert_eq!(aggregate.register_effects[0].value.text, "a\n");
        assert_eq!(aggregate.option_effects.len(), 1);
        assert_eq!(aggregate.option_effects[0].name, ExOptionName::Wrap);
        assert!(matches!(
            aggregate.frontend_requests.as_slice(),
            [ExFrontendRequest::Info(ExInfoRequest::Marks(names))] if names == &['a']
        ));
        assert_eq!(
            aggregate.navigation,
            Some(ExNavigation::TextOffset(0)),
            "the last navigation destination remains the final sequential state"
        );

        assert!(
            aggregate.model_transaction().is_none(),
            "a multi-publication outcome must not masquerade as one transaction"
        );
        assert_eq!(
            aggregate.model_transactions(),
            [first_transaction, second_transaction]
        );
        assert_eq!(
            aggregate.model_transactions()[0].after_revision(),
            aggregate.model_transactions()[1].before_revision()
        );
        assert_eq!(
            aggregate.model_transactions()[0]
                .summary()
                .formatted_splices()
                .len(),
            1
        );
        assert_eq!(
            aggregate.model_transactions()[1]
                .summary()
                .source_patches()
                .len(),
            1
        );
    }

    #[test]
    fn compound_outcome_allows_non_ex_publications_between_exact_ex_maps() {
        let mut document = Document::new("a\nb\nc");
        let mut state = ExExecutionState::default();
        let mut aggregate = execute(&mut document, &mut state, 0, ":1delete").unwrap();
        let first_target = aggregate.model_transaction().unwrap().after_revision();

        document
            .apply_model_request(ModelRequest::ApplyTextEdits {
                document: document.id(),
                revision: document.revision(),
                edits: vec![TextEdit::new(0..1, "B")],
            })
            .unwrap();

        let next = execute(&mut document, &mut state, 0, ":1delete").unwrap();
        let second_source = next.model_transaction().unwrap().before_revision();
        assert_ne!(
            first_target, second_source,
            "the omitted ordinary edit creates an intentional map-chain gap"
        );

        aggregate.try_merge(next).unwrap();
        assert_eq!(aggregate.model_transactions().len(), 2);
        assert_eq!(document.text(), "c");
    }

    #[test]
    fn compound_outcome_substitution_overflow_is_atomic() {
        let mut aggregate = ExOutcome {
            navigation: Some(ExNavigation::TextOffset(7)),
            substitutions: usize::MAX,
            ..ExOutcome::default()
        };
        let before = aggregate.clone();
        let next = ExOutcome {
            document_changed: true,
            navigation: Some(ExNavigation::HistoryRestoration),
            register_effects: vec![ExRegisterEffect {
                kind: ExRegisterEffectKind::Yank,
                requested: Some('a'),
                value: ExRegisterValue::characterwise("kept"),
            }],
            substitutions: 1,
            ..ExOutcome::default()
        };

        assert_eq!(
            aggregate.try_merge(next),
            Err(ExOutcomeMergeError::SubstitutionCountOverflow)
        );
        assert_eq!(aggregate, before);
    }

    #[test]
    fn substitute_repeat_and_suppressed_no_match_are_stateful_but_undo_is_not() {
        let mut document = Document::new("cat\ncat");
        let mut state = ExExecutionState::default();
        execute(&mut document, &mut state, 0, ":s/cat/dog/").unwrap();
        assert!(state.has_previous_substitute());
        execute(&mut document, &mut state, 1, ":&").unwrap();
        assert_eq!(document.text(), "dog\ndog");
        execute(&mut document, &mut state, 0, ":s/missing/x/e").unwrap();
        assert!(document.undo());
        assert_eq!(document.text(), "dog\ncat");
        assert_eq!(state.previous_substitute_pattern(), Some("missing"));
    }

    #[test]
    fn confirm_substitute_returns_a_policy_request_without_mutation() {
        let document = Document::new("one one");
        let error = prepare_ex(
            &document,
            &ExExecutionState::default(),
            &context(0),
            &parse_ex(":s/one/two/gc").unwrap(),
            &(),
        )
        .unwrap_err();
        match error {
            ExExecuteError::NeedsPolicy(ExPolicyRequest::ConfirmSubstitution(preview)) => {
                assert_eq!(preview.matches, 2);
                assert_eq!(preview.edits.len(), 2);
                assert_eq!(preview.fragment_edits.len(), 2);
            }
            other => panic!("unexpected error: {other:?}"),
        }
        assert_eq!(document.text(), "one one");
    }

    #[test]
    fn incompatible_vim_regex_atoms_are_not_silently_reinterpreted() {
        let document = Document::new("word");
        let error = prepare_ex(
            &document,
            &ExExecutionState::default(),
            &context(0),
            &parse_ex(r#":s/\(word\)/x/"#).unwrap(),
            &(),
        )
        .unwrap_err();
        assert_eq!(
            error,
            ExExecuteError::UnsupportedRegexAtom(r#"\("#.to_owned())
        );

        let error = prepare_ex(
            &document,
            &ExExecutionState::default(),
            &context(0),
            &parse_ex(r#":s/word/\U&/"#).unwrap(),
            &(),
        )
        .unwrap_err();
        assert_eq!(
            error,
            ExExecuteError::UnsupportedReplacementAtom(r#"\U"#.to_owned())
        );
    }

    #[test]
    fn unrepresentable_substitution_is_atomic_for_latin1() {
        let mut document = Document::from_bytes(
            vec![b'c', b'a', b'f', 0xe9],
            crate::document::Encoding::Latin1,
            Format::PlainText,
        )
        .unwrap();
        let revision = document.revision();
        let mut state = ExExecutionState::default();
        let error = execute(&mut document, &mut state, 0, ":s/café/😀/").unwrap_err();
        assert!(matches!(
            error,
            ExExecuteError::Document(DocumentError::UnrepresentableCharacter { .. })
        ));
        assert_eq!(document.text(), "café");
        assert_eq!(document.revision(), revision);
        assert!(!state.has_previous_substitute());
    }

    #[test]
    fn stale_plan_is_rejected_before_any_effect() {
        let mut document = Document::new("one\ntwo");
        let command = parse_ex(":delete").unwrap();
        let plan = prepare_ex(
            &document,
            &ExExecutionState::default(),
            &context(0),
            &command,
            &(),
        )
        .unwrap();
        document.insert(0, "x").unwrap();
        let before = document.text().to_owned();
        assert!(matches!(
            commit_ex(&mut document, &mut ExExecutionState::default(), plan),
            Err(ExExecuteError::StalePlan { .. })
        ));
        assert_eq!(document.text(), before);
    }

    #[test]
    fn stale_substitute_plan_does_not_publish_ex_state() {
        let mut document = Document::new("cat\nmiddle\ncat");
        let mut state = ExExecutionState::default();
        let plan = prepare_ex(
            &document,
            &state,
            &context(0),
            &parse_ex(":%s/cat/dog/g").unwrap(),
            &(),
        )
        .unwrap();
        assert!(!state.has_previous_substitute());

        document.insert(0, "live ").unwrap();
        let source_after_live_edit = document.source_bytes();
        assert!(matches!(
            commit_ex(&mut document, &mut state, plan),
            Err(ExExecuteError::StalePlan { .. })
        ));
        assert_eq!(document.source_bytes(), source_after_live_edit);
        assert!(!state.has_previous_substitute());
    }

    #[test]
    fn undo_redo_and_line_navigation_execute_in_core() {
        let mut document = Document::new("a\nb");
        document.insert(1, "x").unwrap();
        let mut state = ExExecutionState::default();
        execute(&mut document, &mut state, 0, ":undo").unwrap();
        assert_eq!(document.text(), "a\nb");
        execute(&mut document, &mut state, 0, ":redo").unwrap();
        assert_eq!(document.text(), "ax\nb");
        let outcome = execute(&mut document, &mut state, 0, ":2").unwrap();
        assert_eq!(outcome.navigation, Some(ExNavigation::TextOffset(3)));
    }

    #[test]
    fn undo_selects_an_exact_change_number_without_replaying_edits() {
        let mut document = Document::new("a");
        document.insert(1, "b").unwrap();
        let first = document.history_status().current.change;
        document.insert(2, "c").unwrap();
        document.insert(3, "d").unwrap();

        let outcome = execute(
            &mut document,
            &mut ExExecutionState::default(),
            0,
            &format!(":undo {}", first.as_u64()),
        )
        .unwrap();
        assert_eq!(document.text(), "ab");
        assert_eq!(document.history_status().current.change, first);
        assert_eq!(outcome.navigation, Some(ExNavigation::HistoryRestoration));

        let revision = document.revision();
        let history = document.history_status();
        assert!(matches!(
            execute(
                &mut document,
                &mut ExExecutionState::default(),
                0,
                ":undo 999999",
            ),
            Err(ExExecuteError::Model(ModelTransactionError::History(
                crate::document::HistoryError::ChangeNotFound(_)
            )))
        ));
        assert_eq!(document.revision(), revision);
        assert_eq!(document.history_status(), history);
    }

    #[test]
    fn removed_word_boundary_options_are_not_configurable_or_listed() {
        let document = Document::new("one two");
        let state = ExExecutionState::default();
        for option in ["linebreak", "nolinebreak", "lbr", "nolbr", "linebreak?"] {
            assert!(matches!(
                prepare_ex(
                    &document, &state, &context(0),
                    &parse_ex(&format!(":set {option}")).unwrap(), &(),
                ),
                Err(ExExecuteError::UnsupportedOption(_))
            ));
        }
        let plan = prepare_ex(
            &document, &state, &context(0), &parse_ex(":set all").unwrap(), &(),
        ).unwrap();
        let ExFrontendRequest::Info(ExInfoRequest::Options(options)) =
            &plan.outcome.frontend_requests[0] else { panic!("expected options") };
        assert_eq!(options.len(), 17);
        assert!(options.iter().any(|option| option.name == ExOptionName::TextWidth));
    }

    #[test]
    fn set_stages_view_options_and_commits_fileformat() {
        let mut document = Document::new("a\nb");
        let mut state = ExExecutionState::default();
        let command = parse_ex(":set wrap ff=dos").unwrap();
        let plan = prepare_ex(&document, &state, &context(0), &command, &()).unwrap();
        assert_eq!(document.file_format(), FileFormat::Unix);
        assert_eq!(plan.outcome.option_effects.len(), 2);
        let outcome = commit_ex(&mut document, &mut state, plan).unwrap();
        assert!(outcome.document_changed);
        assert_eq!(document.file_format(), FileFormat::Dos);
        assert_eq!(document.source_bytes(), b"a\r\nb");
    }

    #[test]
    fn file_info_and_internal_normal_continuations_are_typed() {
        let document = Document::new("a\nb");
        let state = ExExecutionState::default();
        let write = prepare_ex(
            &document,
            &state,
            &context(0),
            &parse_ex(":1,2write! copy.txt").unwrap(),
            &(),
        )
        .unwrap();
        assert!(matches!(
            &write.outcome.frontend_requests[0],
            ExFrontendRequest::File(ExFileRequest::Write {
                force: true,
                range: Some(HardLineRange { start: 0, end: 1 }),
                ..
            })
        ));

        let normal = prepare_ex(
            &document,
            &state,
            &context(0),
            &parse_ex(":%normal! A!").unwrap(),
            &(),
        )
        .unwrap();
        assert_eq!(
            normal.outcome.frontend_requests[0],
            ExFrontendRequest::Normal(ExNormalRequest {
                range: HardLineRange { start: 0, end: 1 },
                commands: "A!".to_owned(),
                literal: true,
            })
        );

        let goto = prepare_ex(
            &document,
            &state,
            &context(0),
            &parse_ex(":goto 2").unwrap(),
            &(),
        )
        .unwrap();
        assert_eq!(goto.outcome.navigation, Some(ExNavigation::TextOffset(1)));
    }

    #[test]
    fn argument_navigation_is_a_host_effect_without_edits_or_history_changes() {
        use crate::command::argument_list::ExArgumentTarget;
        let mut document = Document::new("one line");
        document.replace(0..3, "changed").unwrap();
        let mut state = ExExecutionState::default();
        let before = document.history_status().current;
        for (command, target, write_first, force) in [
            (":3n", ExArgumentTarget::Next(3), false, false),
            (":2N!", ExArgumentTarget::Previous(2), false, true),
            (":wn!", ExArgumentTarget::Next(1), true, true),
        ] {
            let outcome = execute(&mut document, &mut state, 0, command).unwrap();
            assert!(!outcome.document_changed);
            assert!(outcome.register_effects.is_empty());
            assert_eq!(document.history_status().current, before);
            assert_eq!(outcome.frontend_requests, [ExFrontendRequest::File(ExFileRequest::NavigateArgument {
                target, force, write_first, path: None, line: None,
            })]);
        }
    }

    #[test]
    fn write_next_obeys_read_only_policy_before_emitting_host_work() {
        let mut document = Document::new("body");
        document.set_read_only(true);
        let state = ExExecutionState::default();
        for command in [":wn", ":wN", ":2wprevious copy"] {
            assert!(matches!(prepare_ex(&document, &state, &context(0),
                &parse_ex(command).unwrap(), &()), Err(ExExecuteError::ReadOnly)));
        }
        for command in [":next", ":wn!", ":wprevious!"] {
            assert!(prepare_ex(&document, &state, &context(0), &parse_ex(command).unwrap(), &()).is_ok());
        }
    }

    #[test]
    fn goto_uses_one_based_source_bytes_ranges_and_vim_eof_policy() {
        let document = Document::new("abcdef");
        let state = ExExecutionState::default();
        let revision = document.revision();
        let source = document.source_bytes();
        let cases = [
            (":goto", 0),
            (":goto 1", 0),
            (":goto 2", 1),
            (":5goto", 4),
            (":1,5goto", 4),
            (":1,5goto 3", 2),
            (":999goto", 6),
            (":goto 0", 6),
            (":goto 999999999999", 6),
        ];
        for (input, expected) in cases {
            let plan = prepare_ex(
                &document,
                &state,
                &context(0),
                &parse_ex(input).unwrap(),
                &(),
            )
            .unwrap();
            assert_eq!(
                plan.outcome.navigation,
                Some(ExNavigation::TextOffset(expected)),
                "{input}"
            );
        }
        assert_eq!(document.revision(), revision);
        assert_eq!(document.source_bytes(), source);
    }

    #[test]
    fn goto_maps_encoded_and_hidden_source_without_guessing_interiors() {
        let state = ExExecutionState::default();
        let latin1 = Document::from_bytes(
            vec![0xe9, b'x'],
            crate::document::Encoding::Latin1,
            Format::PlainText,
        )
        .unwrap();
        let plan = prepare_ex(
            &latin1,
            &state,
            &context(0),
            &parse_ex(":goto 2").unwrap(),
            &(),
        )
        .unwrap();
        assert_eq!(
            plan.outcome.navigation,
            Some(ExNavigation::TextOffset("é".len()))
        );

        let utf16 = Document::from_bytes(
            vec![0xff, 0xfe, b'A', 0, b'B', 0],
            crate::document::Encoding::Utf16Le,
            Format::PlainText,
        )
        .unwrap();
        assert!(matches!(
            prepare_ex(
                &utf16,
                &state,
                &context(0),
                &parse_ex(":goto 2").unwrap(),
                &(),
            ),
            Err(ExExecuteError::SourceMapping(
                SourceToTextError::InteriorBom { .. }
            ))
        ));
        let plan = prepare_ex(
            &utf16,
            &state,
            &context(0),
            &parse_ex(":goto 5").unwrap(),
            &(),
        )
        .unwrap();
        assert_eq!(plan.outcome.navigation, Some(ExNavigation::TextOffset(1)));

        let markdown = Document::from_bytes(
            b"**x**".to_vec(),
            crate::document::Encoding::Utf8,
            Format::Markdown,
        )
        .unwrap();
        assert!(matches!(
            prepare_ex(
                &markdown,
                &state,
                &context(0),
                &parse_ex(":goto 2").unwrap(),
                &(),
            ),
            Err(ExExecuteError::SourceMapping(
                SourceToTextError::InteriorHiddenSyntax { .. }
            ))
        ));
        let plan = prepare_ex(
            &markdown,
            &state,
            &context(0),
            &parse_ex(":goto 3").unwrap(),
            &(),
        )
        .unwrap();
        assert_eq!(plan.outcome.navigation, Some(ExNavigation::TextOffset(0)));
    }
}
