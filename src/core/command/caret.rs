//! What the caret denotes in the document model.
//!
//! A frontend draws the caret, but it must not decide what the caret *means*.
//! That decision belongs to the model: the mode determines whether the cursor
//! names a character or an insertion point, and the document determines
//! whether a character actually exists there. Exporting the decision keeps one
//! authority and makes the two representations impossible to drift apart.
use super::{CommandInterpreter, Mode};
use crate::document::{BoundaryAffinity, Document};
use std::ops::Range;

/// The exact thing the caret occupies.
///
/// The two variants are kept apart in the type on purpose. `affinity` exists
/// only to choose which visual row an insertion point belongs to when a soft
/// wrap puts one offset at the end of one row and the start of the next. A
/// character cell needs no such choice, because a grapheme lies on exactly one
/// row. Making affinity unreachable from `Cell` means a frontend cannot use it
/// to pick the cell, which is what drew the block caret one grapheme early
/// after `$` and `<End>`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CaretTarget {
    /// The caret covers exactly this grapheme of hard-line content. Normal,
    /// Visual, and Replace modes address characters, not gaps between them.
    Cell { range: Range<usize> },
    /// The caret sits between graphemes. Insert and command-line modes
    /// address gaps; so does a character mode wherever no character exists,
    /// such as an empty line or the end of the document.
    Boundary {
        offset: usize,
        affinity: BoundaryAffinity,
    },
}

impl CaretTarget {
    /// The formatted-text offset the caret is anchored to. For a cell this is
    /// the first byte of the covered grapheme.
    pub fn offset(&self) -> usize {
        match self {
            Self::Cell { range } => range.start,
            Self::Boundary { offset, .. } => *offset,
        }
    }

    /// The covered range. A boundary covers nothing, so its range is empty.
    pub fn range(&self) -> Range<usize> {
        match self {
            Self::Cell { range } => range.clone(),
            Self::Boundary { offset, .. } => *offset..*offset,
        }
    }

    /// The row-disambiguating affinity. A cell lies on exactly one row, so it
    /// reports the neutral downstream value rather than a meaningful choice.
    pub fn affinity(&self) -> BoundaryAffinity {
        match self {
            Self::Cell { .. } => BoundaryAffinity::Downstream,
            Self::Boundary { affinity, .. } => *affinity,
        }
    }

    pub fn is_cell(&self) -> bool {
        matches!(self, Self::Cell { .. })
    }
}

impl Mode {
    /// Whether this mode's cursor names a character rather than a gap between
    /// characters. Vim's Normal, Visual, and Replace cursors sit *on* a
    /// character; Insert and the command line sit *between* characters.
    pub fn addresses_characters(self) -> bool {
        match self {
            Self::Normal
            | Self::Replace
            | Self::VisualCharacter
            | Self::VisualLine
            | Self::VisualBlock => true,
            Self::Insert | Self::CommandLine => false,
        }
    }
}

impl CommandInterpreter {
    /// What this view's caret occupies in `document`.
    ///
    /// This is the single authority for caret placement. Frontends resolve the
    /// returned target to geometry; they must not re-derive it from the mode,
    /// the cursor offset, and the boundary affinity, because those inputs do
    /// not by themselves say whether the caret names a character or a gap.
    pub fn caret_target(&self, document: &Document) -> CaretTarget {
        let (offset, affinity) = match self.visual_block() {
            // The moving corner of a Visual Block is the presented caret.
            Some(block) => (block.active.text_offset, block.active.affinity),
            None => (self.cursor(), self.boundary_affinity()),
        };
        let boundary = CaretTarget::Boundary { offset, affinity };
        if self.is_text_selection() || !self.mode().addresses_characters() {
            return boundary;
        }
        let lines = document.hard_line_snapshot();
        let Ok(line) = lines.line_at_offset(offset) else {
            return boundary;
        };
        let content = line.content_range();
        // A hard-line separator and the end of the document are gaps, not
        // characters: an empty line has no grapheme for a cell to cover.
        if offset < content.start || offset >= content.end {
            return boundary;
        }
        match lines.grapheme_range_at(offset) {
            Some(range) if range.end <= content.end => CaretTarget::Cell { range },
            _ => boundary,
        }
    }
}
