//! Platform-neutral marked-text (IME) composition.
//!
//! A composition is an overlay over one immutable formatted-document
//! revision. Updating marked text never mutates [`Document`]. Committing
//! prepares one revision-bound model transaction so the normal document
//! transaction path remains the only source of authoritative changes.

use std::fmt;
use std::ops::Range;

use unicode_segmentation::UnicodeSegmentation;

use crate::document::{
    Association, BoundaryAffinity, CommittedModelTransaction, DeletionRecovery, Document,
    DocumentError, DocumentId, FormattedPayloadEdit, FormattedTextPayload, FormattedTextTree,
    MappingOutcome, ModelRequest, ModelTransactionError, PositionError, PreparedModelTransaction,
    Revision, StyleProperty, StylePropertyValue, TextEdit, TextRange,
};

/// Normalized composition input emitted after a frontend has converted its
/// native offsets to UTF-8 byte boundaries in formatted text.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CompositionEvent {
    Begin(CompositionTarget),
    Update(CompositionUpdate),
    Commit,
    Cancel,
}

/// The formatted range replaced temporarily by marked text.
///
/// Targets are exact snapshot values, not persistent anchors. A target that
/// no longer names the document's current revision is rejected as stale.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompositionTarget {
    document_id: DocumentId,
    revision: Revision,
    replacement_range: Range<usize>,
}

impl CompositionTarget {
    /// Construct a target from an already checked formatted range.
    pub fn from_text_range(range: TextRange) -> Self {
        Self {
            document_id: range.start().document(),
            revision: range.start().revision(),
            replacement_range: range.start().offset()..range.end().offset(),
        }
    }

    /// Construct and validate a target in the current formatted snapshot.
    pub fn at_offsets(
        document: &Document,
        replacement_range: Range<usize>,
    ) -> Result<Self, CompositionError> {
        validate_document_range(document, &replacement_range)?;
        Ok(Self {
            document_id: document.id(),
            revision: document.revision(),
            replacement_range,
        })
    }

    pub fn document_id(&self) -> DocumentId {
        self.document_id
    }

    pub fn revision(&self) -> Revision {
        self.revision
    }

    pub fn replacement_range(&self) -> Range<usize> {
        self.replacement_range.clone()
    }
}

/// One normalized marked-text update.
///
/// `selected_range` is relative to `marked_text` and is expressed in UTF-8
/// bytes. Both endpoints must be extended-grapheme boundaries. The range may
/// be empty to represent the insertion caret within the marked text.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompositionUpdate {
    pub marked_text: String,
    pub selected_range: Range<usize>,
}

impl CompositionUpdate {
    pub fn new(marked_text: impl Into<String>, selected_range: Range<usize>) -> Self {
        Self {
            marked_text: marked_text.into(),
            selected_range,
        }
    }
}

/// Active composition state. This belongs to a view/controller, not to the
/// authoritative document or undo tree.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompositionSession {
    target: CompositionTarget,
    base_text: FormattedTextTree,
    marked_text: String,
    selected_range: Range<usize>,
    generation: u64,
}

impl CompositionSession {
    /// Begin a session over a validated snapshot target.
    pub fn begin(document: &Document, target: CompositionTarget) -> Result<Self, CompositionError> {
        validate_target(document, &target)?;
        Ok(Self {
            target,
            base_text: document.projection().text_tree().clone(),
            marked_text: String::new(),
            selected_range: 0..0,
            generation: 0,
        })
    }

    /// Convenience constructor for a range in the current document revision.
    pub fn begin_at_offsets(
        document: &Document,
        replacement_range: Range<usize>,
    ) -> Result<Self, CompositionError> {
        Self::begin(
            document,
            CompositionTarget::at_offsets(document, replacement_range)?,
        )
    }

    pub fn document_id(&self) -> DocumentId {
        self.target.document_id
    }

    pub fn base_revision(&self) -> Revision {
        self.target.revision
    }

    pub fn replacement_range(&self) -> Range<usize> {
        self.target.replacement_range.clone()
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Replace the temporary marked payload and return the exact overlay to
    /// display. Validation is completed before session state changes.
    pub fn update(
        &mut self,
        document: &Document,
        update: CompositionUpdate,
    ) -> Result<CompositionOverlay, CompositionError> {
        self.validate_document(document)?;
        validate_range(
            &update.marked_text,
            &update.selected_range,
            CompositionBoundary::MarkedText,
        )?;

        self.marked_text = update.marked_text;
        self.selected_range = update.selected_range;
        self.generation = self.generation.saturating_add(1);
        Ok(self.overlay_unchecked())
    }

    /// Return the current temporary formatted overlay without changing either
    /// the session or document.
    pub fn overlay(&self, document: &Document) -> Result<CompositionOverlay, CompositionError> {
        self.validate_document(document)?;
        Ok(self.overlay_unchecked())
    }

    /// Remove the overlay and describe the exact immutable formatted snapshot
    /// that is visible again. No source mutation is necessary or performed.
    pub fn cancel(self, document: &Document) -> Result<CompositionRestoration, CompositionError> {
        self.validate_document(document)?;
        let original_range_text = self
            .base_text
            .slice(self.target.replacement_range.clone())
            .map_err(DocumentError::FormattedTextStorage)
            .map_err(CompositionError::Document)?;
        Ok(CompositionRestoration {
            document_id: self.target.document_id,
            revision: self.target.revision,
            replacement_range: self.target.replacement_range,
            original_range_text,
            original_formatted_text: self.base_text,
        })
    }

    /// Prepare one atomic model request. Preparation does not end the session
    /// and does not mutate the document, so a failed model commit can leave the
    /// marked overlay available for policy handling or cancellation.
    pub fn prepare_commit(
        &self,
        document: &Document,
    ) -> Result<CompositionCommitRequest, CompositionError> {
        self.validate_document(document)?;
        let edit = TextEdit::new(
            self.target.replacement_range.clone(),
            self.marked_text.clone(),
        );
        let prepared = document
            .prepare_model_request(ModelRequest::ApplyTextEdits {
                document: self.target.document_id,
                revision: self.target.revision,
                edits: vec![edit.clone()],
            })
            .map_err(composition_model_error)?;

        // The model's grapheme-closed map supplies the legal target boundary.
        // This also handles joins with adjacent text without flattening the
        // immutable formatted tree merely to inspect a local boundary.
        let point = document
            .text_point(edit.range.start)
            .map_err(CompositionError::Document)?;
        let caret_offset = match prepared.text_position_map().map_text_point(
            point,
            Association::AfterInsertion,
            BoundaryAffinity::Downstream,
            DeletionRecovery::PreferFollowingThenPreceding,
        )? {
            MappingOutcome::Exact(point)
            | MappingOutcome::Moved(point)
            | MappingOutcome::CollapsedByDeletion(point)
            | MappingOutcome::RecoveredFromProvenance(point) => point.offset(),
            MappingOutcome::Ambiguous(_) | MappingOutcome::Unresolvable(_) => {
                return Err(CompositionError::UnresolvableCommitCaret)
            }
        };
        Ok(CompositionCommitRequest {
            document_id: self.target.document_id,
            expected_revision: self.target.revision,
            generation: self.generation,
            edit,
            caret_offset,
            prepared,
        })
    }

    /// Preserve the invoking caret's pending character declarations when a
    /// native IME commits its marked overlay. Preparation remains read-only;
    /// text and formatting share the same exact map and atomic publication.
    pub(crate) fn prepare_commit_with_typing_properties(
        &self,
        document: &Document,
        values: &[(StyleProperty, StylePropertyValue)],
        affinity: BoundaryAffinity,
    ) -> Result<CompositionCommitRequest, CompositionError> {
        if values.is_empty() || self.marked_text.is_empty() {
            return self.prepare_commit(document);
        }
        self.validate_document(document)?;
        let edit = TextEdit::new(
            self.target.replacement_range.clone(),
            self.marked_text.clone(),
        );
        let value = super::external_text_register_value(document, &self.marked_text);
        let payload = FormattedTextPayload::new(
            &document.hard_line_snapshot(),
            self.marked_text.clone(),
            value.hard_break_offsets().to_vec(),
        )
        .expect("composition literal text has validated semantic break offsets");
        let (prepared, caret_offset) = document
            .prepare_insertion_with_typing_properties(
                FormattedPayloadEdit::new(edit.range.clone(), payload)
                    .with_boundary_affinity(affinity),
                values,
            )
            .map_err(composition_model_error)?;
        Ok(CompositionCommitRequest {
            document_id: self.target.document_id,
            expected_revision: self.target.revision,
            generation: self.generation,
            edit,
            caret_offset,
            prepared,
        })
    }

    fn validate_document(&self, document: &Document) -> Result<(), CompositionError> {
        validate_target(document, &self.target)
    }

    fn overlay_unchecked(&self) -> CompositionOverlay {
        CompositionOverlay {
            document_id: self.target.document_id,
            revision: self.target.revision,
            generation: self.generation,
            base_text: self.base_text.clone(),
            replacement_range: self.target.replacement_range.clone(),
            marked_text: self.marked_text.clone(),
            selected_range: self.selected_range.clone(),
        }
    }
}

/// A disposable formatted projection with the marked range and selection
/// called out explicitly for layout and drawing.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompositionOverlay {
    document_id: DocumentId,
    revision: Revision,
    generation: u64,
    base_text: FormattedTextTree,
    replacement_range: Range<usize>,
    marked_text: String,
    selected_range: Range<usize>,
}

impl CompositionOverlay {
    /// Persistent text for bounded layout capture. Only the marked payload
    /// allocates new leaves; the immutable document prefix/suffix remain shared.
    pub(crate) fn layout_text_tree(
        &self,
    ) -> Result<FormattedTextTree, crate::document::FormattedTextError> {
        self.base_text.splice_prevalidated_batch(&[(
            self.replacement_range.clone(),
            self.marked_text.as_str(),
        )])
    }

    pub fn document_id(&self) -> DocumentId {
        self.document_id
    }

    pub fn revision(&self) -> Revision {
        self.revision
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Explicitly materialize the unchanged prefix. Regional presentation
    /// code should prefer [`Self::text_in_range`].
    pub fn prefix(&self) -> String {
        self.base_text
            .slice(0..self.replacement_range.start)
            .expect("a validated composition target remains a valid tree range")
    }

    pub fn marked_text(&self) -> &str {
        &self.marked_text
    }

    /// Explicitly materialize the unchanged suffix. Regional presentation
    /// code should prefer [`Self::text_in_range`].
    pub fn suffix(&self) -> String {
        self.base_text
            .slice(self.replacement_range.end..self.base_text.byte_len())
            .expect("a validated composition target remains a valid tree range")
    }

    pub fn base_utf8_len(&self) -> usize {
        self.base_text.byte_len()
    }

    /// UTF-8 byte length of the disposable formatted projection.
    pub fn utf8_len(&self) -> usize {
        self.base_text.byte_len() - self.replacement_range.len() + self.marked_text.len()
    }

    /// Map a boundary in the immutable base projection to the corresponding
    /// boundary in this splice. Boundaries inside the replaced range have no
    /// unique image and are rejected.
    pub fn overlay_offset_for_base_boundary(
        &self,
        offset: usize,
        association: Association,
    ) -> Option<usize> {
        if offset < self.replacement_range.start
            || (offset == self.replacement_range.start
                && (self.replacement_range.start != self.replacement_range.end
                    || association == Association::BeforeInsertion))
        {
            return Some(offset);
        }
        if offset < self.replacement_range.end || offset > self.base_text.byte_len() {
            return None;
        }
        self.marked_range()
            .end
            .checked_add(offset - self.replacement_range.end)
    }

    /// Number of hard lines in the disposable projection. Counting is limited
    /// to the replaced and marked payloads; unchanged prefix/suffix topology is
    /// supplied by the source-backed formatted snapshot.
    pub fn hard_line_count(&self, base_hard_line_count: usize) -> Option<usize> {
        let removed = self.base_text.slice(self.replacement_range.clone()).ok()?;
        let removed_breaks = removed.bytes().filter(|byte| *byte == b'\n').count();
        let inserted_breaks = self
            .marked_text
            .bytes()
            .filter(|byte| *byte == b'\n')
            .count();
        base_hard_line_count
            .checked_sub(removed_breaks)?
            .checked_add(inserted_breaks)
    }

    /// Copy one scalar-aligned range without materializing the complete
    /// projection. Presentation adapters use this to fetch only text covered
    /// by their bounded layout snapshot.
    pub fn text_in_range(&self, range: Range<usize>) -> Option<String> {
        if range.start > range.end || range.end > self.utf8_len() {
            return None;
        }
        if !self.is_char_boundary(range.start) || !self.is_char_boundary(range.end) {
            return None;
        }

        let marked_start = self.replacement_range.start;
        let marked_end = marked_start + self.marked_text.len();
        let mut result = String::with_capacity(range.len());
        append_tree_intersection(&mut result, &self.base_text, 0, 0, marked_start, &range)?;
        append_intersection(&mut result, &self.marked_text, marked_start, &range);
        append_tree_intersection(
            &mut result,
            &self.base_text,
            self.replacement_range.end,
            marked_end,
            self.utf8_len(),
            &range,
        )?;
        Some(result)
    }

    fn is_char_boundary(&self, offset: usize) -> bool {
        let marked_start = self.replacement_range.start;
        let marked_end = marked_start + self.marked_text.len();
        if offset <= marked_start {
            self.base_text.is_char_boundary(offset).unwrap_or(false)
        } else if offset <= marked_end {
            self.marked_text.is_char_boundary(offset - marked_start)
        } else {
            self.base_text
                .is_char_boundary(self.replacement_range.end + offset - marked_end)
                .unwrap_or(false)
        }
    }

    /// Materialize the temporary formatted text. Layout code can instead use
    /// `prefix`, `marked_text`, and `suffix` to avoid this allocation.
    pub fn formatted_text(&self) -> String {
        let mut text = String::with_capacity(
            self.base_text.byte_len() - self.replacement_range.len() + self.marked_text.len(),
        );
        text.push_str(&self.prefix());
        text.push_str(&self.marked_text);
        text.push_str(&self.suffix());
        text
    }

    pub fn replacement_range(&self) -> Range<usize> {
        self.replacement_range.clone()
    }

    pub fn marked_range(&self) -> Range<usize> {
        self.replacement_range.start..self.replacement_range.start + self.marked_text.len()
    }

    pub fn selected_range_in_marked_text(&self) -> Range<usize> {
        self.selected_range.clone()
    }

    pub fn selected_range_in_overlay(&self) -> Range<usize> {
        let start = self.replacement_range.start + self.selected_range.start;
        let end = self.replacement_range.start + self.selected_range.end;
        start..end
    }
}

fn append_intersection(
    output: &mut String,
    segment: &str,
    segment_start: usize,
    requested: &Range<usize>,
) {
    let segment_end = segment_start + segment.len();
    let start = requested.start.max(segment_start);
    let end = requested.end.min(segment_end);
    if start < end {
        output.push_str(&segment[start - segment_start..end - segment_start]);
    }
}

fn append_tree_intersection(
    output: &mut String,
    tree: &FormattedTextTree,
    source_start: usize,
    projected_start: usize,
    projected_end: usize,
    requested: &Range<usize>,
) -> Option<()> {
    let start = requested.start.max(projected_start);
    let end = requested.end.min(projected_end);
    if start < end {
        let local_start = source_start.checked_add(start - projected_start)?;
        let local_end = source_start.checked_add(end - projected_start)?;
        output.push_str(&tree.slice(local_start..local_end).ok()?);
    }
    Some(())
}

/// The result of cancelling a session. It lets layout restore the exact base
/// snapshot while making explicit that the document itself was never changed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompositionRestoration {
    pub document_id: DocumentId,
    pub revision: Revision,
    pub replacement_range: Range<usize>,
    pub original_range_text: String,
    original_formatted_text: FormattedTextTree,
}

impl CompositionRestoration {
    pub fn formatted_text(&self) -> String {
        self.original_formatted_text.flatten()
    }
}

/// One revision-bound semantic request. Applying it delegates to the regular
/// document transaction, including encoding, reverse projection, verification,
/// history creation, and all-or-nothing failure behavior.
pub struct CompositionCommitRequest {
    document_id: DocumentId,
    expected_revision: Revision,
    generation: u64,
    edit: TextEdit,
    caret_offset: usize,
    prepared: PreparedModelTransaction,
}

impl fmt::Debug for CompositionCommitRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CompositionCommitRequest")
            .field("document_id", &self.document_id)
            .field("expected_revision", &self.expected_revision)
            .field("generation", &self.generation)
            .field("edit", &self.edit)
            .field("caret_offset", &self.caret_offset)
            .field("prepared", &self.prepared)
            .finish()
    }
}

impl CompositionCommitRequest {
    pub fn document_id(&self) -> DocumentId {
        self.document_id
    }

    pub fn expected_revision(&self) -> Revision {
        self.expected_revision
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn edit(&self) -> &TextEdit {
        &self.edit
    }

    pub fn caret_offset(&self) -> usize {
        self.caret_offset
    }

    pub fn prepared_model_transaction(&self) -> &PreparedModelTransaction {
        &self.prepared
    }

    /// Apply exactly one text edit as one document transaction and undo unit.
    pub fn apply(self, document: &mut Document) -> Result<CompositionCommit, CompositionError> {
        self.apply_with_model_transaction(document)
            .map(|(composition, _)| composition)
    }

    pub(crate) fn apply_with_model_transaction(
        self,
        document: &mut Document,
    ) -> Result<(CompositionCommit, CommittedModelTransaction), CompositionError> {
        validate_document_revision(document, self.document_id, self.expected_revision)?;
        let committed = document
            .commit_model_transaction(self.prepared)
            .map_err(composition_model_error)?;
        let composition = CompositionCommit {
            document_id: self.document_id,
            before_revision: committed.before_revision(),
            after_revision: committed.after_revision(),
            caret_offset: self.caret_offset,
        };
        Ok((composition, committed))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CompositionCommit {
    pub document_id: DocumentId,
    pub before_revision: Revision,
    pub after_revision: Revision,
    pub caret_offset: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompositionBoundary {
    Replacement,
    MarkedText,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CompositionError {
    AlreadyActive,
    NoActiveSession,
    WrongDocument {
        expected: DocumentId,
        actual: DocumentId,
    },
    StaleRevision {
        expected: Revision,
        actual: Revision,
    },
    InvalidRange {
        boundary: CompositionBoundary,
        start: usize,
        end: usize,
        length: usize,
    },
    InvalidGraphemeBoundary {
        boundary: CompositionBoundary,
        offset: usize,
    },
    UnresolvableCommitCaret,
    Document(DocumentError),
    Position(PositionError),
    Transaction(ModelTransactionError),
}

impl fmt::Display for CompositionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyActive => {
                formatter.write_str("a marked-text composition is already active")
            }
            Self::NoActiveSession => formatter.write_str("no marked-text composition is active"),
            Self::WrongDocument { expected, actual } => write!(
                formatter,
                "composition belongs to document {}, not {}",
                expected.0, actual.0
            ),
            Self::StaleRevision { expected, actual } => write!(
                formatter,
                "composition is based on revision {}; current revision is {}",
                expected.0, actual.0
            ),
            Self::InvalidRange {
                boundary,
                start,
                end,
                length,
            } => write!(
                formatter,
                "invalid {boundary:?} range {start}..{end} for length {length}"
            ),
            Self::InvalidGraphemeBoundary { boundary, offset } => write!(
                formatter,
                "{boundary:?} offset {offset} splits an extended grapheme cluster"
            ),
            Self::UnresolvableCommitCaret => {
                formatter.write_str("the committed composition caret could not be resolved")
            }
            Self::Document(error) => error.fmt(formatter),
            Self::Position(error) => error.fmt(formatter),
            Self::Transaction(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for CompositionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Document(error) => Some(error),
            Self::Position(error) => Some(error),
            Self::Transaction(error) => Some(error),
            _ => None,
        }
    }
}

impl From<PositionError> for CompositionError {
    fn from(error: PositionError) -> Self {
        Self::Position(error)
    }
}

fn composition_model_error(error: ModelTransactionError) -> CompositionError {
    match error {
        ModelTransactionError::Document(error) => CompositionError::Document(error),
        ModelTransactionError::Position(error) => CompositionError::Position(error),
        ModelTransactionError::WrongDocument { expected, actual } => {
            // Model commit reports the receiving document as `expected` and
            // the prepared transaction as `actual`; composition diagnostics
            // phrase those roles in the opposite order.
            CompositionError::WrongDocument {
                expected: actual,
                actual: expected,
            }
        }
        ModelTransactionError::StaleRevision { expected, actual } => {
            CompositionError::StaleRevision { expected, actual }
        }
        other => CompositionError::Transaction(other),
    }
}

fn validate_target(
    document: &Document,
    target: &CompositionTarget,
) -> Result<(), CompositionError> {
    validate_document_revision(document, target.document_id, target.revision)?;
    validate_document_range(document, &target.replacement_range)
}

fn validate_document_range(
    document: &Document,
    range: &Range<usize>,
) -> Result<(), CompositionError> {
    let length = document.projection().text_tree().byte_len();
    if range.start > range.end || range.end > length {
        return Err(CompositionError::InvalidRange {
            boundary: CompositionBoundary::Replacement,
            start: range.start,
            end: range.end,
            length,
        });
    }
    for offset in [range.start, range.end] {
        match document.text_point(offset) {
            Ok(_) => {}
            Err(DocumentError::NotGraphemeBoundary(_)) => {
                return Err(CompositionError::InvalidGraphemeBoundary {
                    boundary: CompositionBoundary::Replacement,
                    offset,
                })
            }
            Err(error) => return Err(CompositionError::Document(error)),
        }
    }
    Ok(())
}

fn validate_document_revision(
    document: &Document,
    expected_document: DocumentId,
    expected_revision: Revision,
) -> Result<(), CompositionError> {
    if document.id() != expected_document {
        return Err(CompositionError::WrongDocument {
            expected: expected_document,
            actual: document.id(),
        });
    }
    if document.revision() != expected_revision {
        return Err(CompositionError::StaleRevision {
            expected: expected_revision,
            actual: document.revision(),
        });
    }
    Ok(())
}

fn validate_range(
    text: &str,
    range: &Range<usize>,
    boundary: CompositionBoundary,
) -> Result<(), CompositionError> {
    if range.start > range.end || range.end > text.len() {
        return Err(CompositionError::InvalidRange {
            boundary,
            start: range.start,
            end: range.end,
            length: text.len(),
        });
    }
    for offset in [range.start, range.end] {
        if !is_grapheme_boundary(text, offset) {
            return Err(CompositionError::InvalidGraphemeBoundary { boundary, offset });
        }
    }
    Ok(())
}

fn is_grapheme_boundary(text: &str, offset: usize) -> bool {
    offset == text.len()
        || text
            .grapheme_indices(true)
            .any(|(grapheme_offset, _)| grapheme_offset == offset)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Encoding, Format};

    #[test]
    fn update_produces_an_overlay_without_mutating_document() {
        let document = Document::new("hello");
        let original_revision = document.revision();
        let original_source = document.source_bytes();
        let mut session = CompositionSession::begin_at_offsets(&document, 1..4).unwrap();

        let overlay = session
            .update(&document, CompositionUpdate::new("é", 2..2))
            .unwrap();

        assert_eq!(overlay.prefix(), "h");
        assert_eq!(overlay.marked_text(), "é");
        assert_eq!(overlay.suffix(), "o");
        assert_eq!(overlay.formatted_text(), "héo");
        assert_eq!(overlay.marked_range(), 1..3);
        assert_eq!(overlay.selected_range_in_marked_text(), 2..2);
        assert_eq!(overlay.selected_range_in_overlay(), 3..3);
        assert_eq!(document.text(), "hello");
        assert_eq!(document.source_bytes(), original_source);
        assert_eq!(document.revision(), original_revision);
    }

    #[test]
    fn invalid_marked_selection_does_not_replace_the_previous_overlay() {
        let document = Document::new("base");
        let mut session = CompositionSession::begin_at_offsets(&document, 0..0).unwrap();
        session
            .update(&document, CompositionUpdate::new("ok", 2..2))
            .unwrap();

        let error = session
            .update(&document, CompositionUpdate::new("a\u{301}", 1..1))
            .unwrap_err();

        assert_eq!(
            error,
            CompositionError::InvalidGraphemeBoundary {
                boundary: CompositionBoundary::MarkedText,
                offset: 1,
            }
        );
        assert_eq!(session.overlay(&document).unwrap().marked_text(), "ok");
        assert_eq!(session.generation(), 1);
    }

    #[test]
    fn marked_selection_must_stay_within_marked_text() {
        let document = Document::new("");
        let mut session = CompositionSession::begin_at_offsets(&document, 0..0).unwrap();

        let error = session
            .update(&document, CompositionUpdate::new("é", 0..3))
            .unwrap_err();

        assert_eq!(
            error,
            CompositionError::InvalidRange {
                boundary: CompositionBoundary::MarkedText,
                start: 0,
                end: 3,
                length: 2,
            }
        );
        assert_eq!(session.generation(), 0);
    }

    #[test]
    fn replacement_target_must_be_a_grapheme_range() {
        let document = Document::new("a\u{301}b");
        let error = CompositionTarget::at_offsets(&document, 1..1).unwrap_err();
        assert_eq!(
            error,
            CompositionError::InvalidGraphemeBoundary {
                boundary: CompositionBoundary::Replacement,
                offset: 1,
            }
        );
    }

    #[test]
    fn cancel_restores_the_exact_base_projection_without_a_source_edit() {
        let document = Document::new("before");
        let revision = document.revision();
        let source = document.source_bytes();
        let mut session = CompositionSession::begin_at_offsets(&document, 0..6).unwrap();
        session
            .update(&document, CompositionUpdate::new("during", 6..6))
            .unwrap();

        let restoration = session.cancel(&document).unwrap();

        assert_eq!(restoration.formatted_text(), "before");
        assert_eq!(restoration.original_range_text, "before");
        assert_eq!(restoration.replacement_range, 0..6);
        assert_eq!(document.text(), "before");
        assert_eq!(document.source_bytes(), source);
        assert_eq!(document.revision(), revision);
    }

    #[test]
    fn commit_is_one_revision_bound_text_edit_and_one_undo_unit() {
        let mut document = Document::new("tea");
        let before = document.revision();
        let before_source = document.source_bytes();
        let before_history = document.history_status();
        let mut session = CompositionSession::begin_at_offsets(&document, 1..3).unwrap();
        session
            .update(&document, CompositionUpdate::new("eam", 3..3))
            .unwrap();
        let request = session.prepare_commit(&document).unwrap();

        assert_eq!(request.expected_revision(), before);
        assert_eq!(request.edit(), &TextEdit::new(1..3, "eam"));
        assert_eq!(document.source_bytes(), before_source);
        assert_eq!(document.history_status(), before_history);
        let commit = request.apply(&mut document).unwrap();

        assert_eq!(document.text(), "team");
        assert_eq!(commit.before_revision, before);
        assert_eq!(commit.after_revision, document.revision());
        assert_eq!(commit.caret_offset, 4);
        assert_ne!(commit.before_revision, commit.after_revision);
        assert!(document.undo());
        assert_eq!(document.text(), "tea");
        assert!(!document.undo());
    }

    #[test]
    fn sessions_reject_stale_revisions_and_wrong_documents() {
        let mut first = Document::new("one");
        let second = Document::new("two");
        let session = CompositionSession::begin_at_offsets(&first, 0..0).unwrap();

        assert!(matches!(
            session.overlay(&second),
            Err(CompositionError::WrongDocument { .. })
        ));

        first.insert(0, "x").unwrap();
        assert!(matches!(
            session.prepare_commit(&first),
            Err(CompositionError::StaleRevision { .. })
        ));
    }

    #[test]
    fn latin1_unrepresentable_commit_rolls_back_at_document_boundary() {
        let mut document =
            Document::from_bytes(vec![b'a', 0xe9], Encoding::Latin1, Format::PlainText).unwrap();
        let before_text = document.text().to_owned();
        let before_source = document.source_bytes();
        let before_revision = document.revision();
        let mut session = CompositionSession::begin_at_offsets(&document, 1..3).unwrap();
        session
            .update(&document, CompositionUpdate::new("😀", 4..4))
            .unwrap();
        let error = session.prepare_commit(&document).unwrap_err();

        assert!(matches!(
            error,
            CompositionError::Document(DocumentError::UnrepresentableCharacter {
                encoding: Encoding::Latin1,
                character: '😀',
            })
        ));
        assert_eq!(document.text(), before_text);
        assert_eq!(document.source_bytes(), before_source);
        assert_eq!(document.revision(), before_revision);
        assert!(!document.undo());
    }

    #[test]
    fn commit_preparation_handles_grapheme_joins_on_both_sides() {
        let mut left = Document::new("ab");
        let mut left_session = CompositionSession::begin_at_offsets(&left, 1..1).unwrap();
        left_session
            .update(&left, CompositionUpdate::new("\u{301}", 2..2))
            .unwrap();
        let left_request = left_session.prepare_commit(&left).unwrap();
        assert_eq!(left_request.caret_offset(), "a\u{301}".len());
        left_request.apply(&mut left).unwrap();
        assert_eq!(left.text(), "a\u{301}b");

        let regional_a = "\u{1f1e6}";
        let regional_b = "\u{1f1e7}";
        let mut right = Document::new(format!("{regional_b}x"));
        let mut right_session = CompositionSession::begin_at_offsets(&right, 0..0).unwrap();
        right_session
            .update(
                &right,
                CompositionUpdate::new(regional_a, regional_a.len()..regional_a.len()),
            )
            .unwrap();
        let right_request = right_session.prepare_commit(&right).unwrap();
        assert_eq!(
            right_request.caret_offset(),
            format!("{regional_a}{regional_b}").len()
        );
        right_request.apply(&mut right).unwrap();
        assert_eq!(right.text(), format!("{regional_a}{regional_b}x"));
    }

    #[test]
    fn commit_request_rechecks_revision_before_touching_the_model() {
        let mut document = Document::new("abc");
        let mut session = CompositionSession::begin_at_offsets(&document, 1..2).unwrap();
        session
            .update(&document, CompositionUpdate::new("B", 1..1))
            .unwrap();
        let request = session.prepare_commit(&document).unwrap();
        document.insert(3, "!").unwrap();
        let text_before_attempt = document.text().to_owned();
        let revision_before_attempt = document.revision();

        assert!(matches!(
            request.apply(&mut document),
            Err(CompositionError::StaleRevision { .. })
        ));
        assert_eq!(document.text(), text_before_attempt);
        assert_eq!(document.revision(), revision_before_attempt);
    }
}
