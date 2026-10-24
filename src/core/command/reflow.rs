//! `gq`/`gw` operator glue. Literal documents count hard lines; Preview keeps
//! the selected visible fragment and rewrites only its Markdown source wrapping.
use super::*;
use crate::document::{
    reflow_edits, ReflowRequest, TextWidthSetting,
};

impl CommandInterpreter {
    /// The buffer's `textwidth` state as installed into this view.
    pub fn text_width(&self) -> TextWidthSetting {
        self.text_width
    }

    /// Application default propagation. An explicit local override survives.
    pub fn set_text_width_default(&mut self, width: u32) {
        self.text_width.set_default(width);
    }

    /// Canonical language name used to select a declarative comment profile.
    /// Independent of syntax coverage, styles, and highlighter availability.
    pub fn set_reflow_language(&mut self, language: Option<String>) {
        if self.reflow_language != language {
            self.generated_indent = None;
        }
        self.reflow_options.set_language(language.as_deref());
        self.reflow_language = language;
    }

    pub fn reflow_language(&self) -> Option<&str> {
        self.reflow_language.as_deref()
    }

    /// Preview formatting selects visible rows even when a view otherwise uses
    /// physical source lines. Resolve layout before the edit: source wrapping
    /// must never choose the Preview caret from newly inserted invisible breaks.
    pub(super) fn preview_format_requires_layout(&self, event: &InputEvent) -> bool {
        match self.pending {
            Pending::Operator(pending) if pending.operator.is_format() => {
                // Counts and grammar prefixes do not yet consume geometry.
                !matches!(event, InputEvent::Key(Key::Char('1'..='9' | 'i' | 'a' | 'f' | 'F' | 't' | 'T' | '\'' | '`' | '[' | ']' | '/' | '?')))
                    && !matches!(event, InputEvent::Key(Key::Char('0')) if pending.motion_count.is_some())
                    && !matches!(event, InputEvent::Key(Key::Char('g')) if !pending.g_prefix)
            }
            Pending::TextObject { operator, .. }
            | Pending::OperatorFind { operator, .. }
            | Pending::OperatorMark { operator, .. }
            | Pending::OperatorBracket { operator, .. } => operator.operator.is_format(),
            Pending::G { .. } if matches!(self.mode, Mode::VisualCharacter | Mode::VisualLine) =>
                matches!(event, InputEvent::Key(Key::Char('q' | 'w'))),
            _ if self.mode == Mode::CommandLine => matches!(event, InputEvent::Key(Key::Enter))
                && self.command_line_state.as_ref().and_then(|state| state.operator)
                    .is_some_and(|pending| pending.operator.is_format()),
            Pending::None if matches!(event, InputEvent::Key(Key::Char('.'))) => match &self.last_repeat {
                Some(RepeatAction::Operator { command, .. }) => command.operator.is_format(),
                Some(RepeatAction::VisualOperator { command, .. }) => command.operator.is_format(),
                Some(RepeatAction::VisualBlock(command)) => matches!(command.action,
                    VisualBlockRepeatAction::Operator { operator, .. } if operator.is_format()),
                _ => false,
            },
            _ => false,
        }
    }

    /// Source-only Preview reflow keeps the checked visible extent authoritative.
    /// In particular, two wrapped rows in one long source line must not silently
    /// expand to the whole paragraph. The document verifies an unchanged Preview,
    /// so the original visible caret coordinates remain exact after publication.
    pub(super) fn apply_preview_format_operator(
        &mut self,
        document: &mut Document,
        keep_cursor: bool,
        range: Range<usize>,
        last_visual_row: Option<usize>,
    ) -> Result<CommandOutput, DocumentError> {
        document.require_modifiable()?;
        let Some(snapshot) = self.line_layout.as_ref() else {
            return Ok(layout_required("Preview reflow"));
        };
        let last = last_visual_row.unwrap_or_else(|| if range.is_empty() { range.start } else {
            document.previous_grapheme_boundary(range.end).unwrap_or(range.start)
        });
        let target = if keep_cursor {
            VisualPosition { text_offset: self.cursor, affinity: self.boundary_affinity }
        } else {
            match layout_motion::g_caret_for_document(document, snapshot, VisualPosition {
                text_offset: last,
                affinity: BoundaryAffinity::Downstream,
            }) {
                Ok(position) => position,
                Err(error) => return Ok(layout_error(error)),
            }
        };
        let old_cursor = (self.cursor, self.boundary_affinity);
        let changed = document.reflow_markdown(range, self.text_width.effective(), self.indentation_options().tabstop)?;
        self.cursor = target.text_offset;
        self.boundary_affinity = target.affinity;
        self.physical_cursor = None;
        self.visual_position = Some(target);
        self.desired_x = None;
        Ok(CommandOutput {
            document_changed: changed,
            cursor_moved: old_cursor != (self.cursor, self.boundary_affinity),
            ..CommandOutput::complete()
        })
    }

    /// Hard lines intersected by a checked extent. Exclusive characterwise
    /// ends were already resolved, so an end at a line start excludes it.
    pub(super) fn hard_lines_covered(
        lines: &HardLineSnapshot,
        range: &Range<usize>,
    ) -> Range<usize> {
        let first = lines
            .line_at_offset(range.start.min(lines.text_length()))
            .expect("an operator extent starts on a hard line")
            .index();
        let last_offset = if range.end > range.start {
            lines
                .previous_grapheme_boundary(range.end.min(lines.text_length()))
                .unwrap_or(range.start)
        } else {
            range.start
        };
        let last = lines
            .line_at_offset(last_offset)
            .expect("an operator extent ends on a hard line")
            .index();
        first..last.max(first) + 1
    }

    /// Format the hard lines `lines` as one atomic transaction. `keep_cursor`
    /// selects `gw` (restore the original text location through an anchor)
    /// instead of `gq` (first nonblank of the last formatted line).
    pub(super) fn apply_format_operator(
        &mut self,
        document: &mut Document,
        keep_cursor: bool,
        lines: Range<usize>,
    ) -> Result<CommandOutput, DocumentError> {
        let snapshot = document.hard_line_snapshot();
        document.validate_hard_line_snapshot(&snapshot)?;
        let edits = match reflow_edits(&ReflowRequest {
            snapshot: &snapshot,
            format: document.format(),
            lines: lines.clone(),
            text_width: self.text_width.effective(),
            tabstop: self.indentation_options().tabstop,
            options: &self.reflow_options,
            autoindent: self.indentation_options().autoindent,
        }) {
            Ok(edits) => edits,
            Err(error) => {
                return Ok(CommandOutput {
                    status: CommandStatus::Error(format!("reflow failed: {error}")),
                    ..CommandOutput::complete()
                })
            }
        };
        let old_cursor = self.cursor;
        let old_count = snapshot.line_count();
        let last_index = lines.end.saturating_sub(1).max(lines.start);
        if edits.is_empty() {
            if !keep_cursor {
                let start = snapshot
                    .line(last_index)
                    .map_or(old_cursor, |line| line.content_range().start);
                self.cursor = first_nonblank_document(document, &snapshot, start);
            }
            return Ok(CommandOutput {
                cursor_moved: self.cursor != old_cursor,
                ..CommandOutput::complete()
            });
        }
        let anchor = keep_cursor
            .then(|| {
                document.text_point(old_cursor).ok().and_then(|point| {
                    document
                        .text_anchor(
                            point,
                            Association::AfterInsertion,
                            BoundaryAffinity::Downstream,
                            DeletionRecovery::PreferFollowingThenPreceding,
                        )
                        .ok()
                })
            })
            .flatten();
        let (result, map) = document.capture_position_maps(|document| document.apply_edits(edits));
        result?;
        let lines_after = document.hard_line_snapshot();
        if keep_cursor {
            let mapped = anchor.and_then(|anchor| mapped_anchor(&map, anchor).ok().flatten());
            self.cursor = normalize_normal_cursor_document(
                document,
                &lines_after,
                mapped.unwrap_or(old_cursor),
            );
        } else {
            let new_count = lines_after.line_count();
            let last_after = (last_index + new_count).saturating_sub(old_count);
            let start = lines_after
                .line(last_after.min(new_count.saturating_sub(1)))
                .map_or(0, |line| line.content_range().start);
            self.cursor = first_nonblank_document(document, &lines_after, start);
        }
        Ok(CommandOutput {
            document_changed: true,
            cursor_moved: self.cursor != old_cursor,
            ..CommandOutput::complete()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Encoding, Format};
    use std::time::Instant;

    fn event(commands: &mut CommandInterpreter, document: &mut Document, event: InputEvent) {
        let previous = document.projection().clone();
        let output = commands.handle(document, event.clone()).unwrap();
        assert!(
            !matches!(output.status, CommandStatus::Error(_)),
            "{event:?}: {:?}",
            output.status
        );
        assert!(
            !previous.compatibility_text_is_materialized(),
            "{event:?} flattened its input snapshot"
        );
        assert!(
            !document.projection().compatibility_text_is_materialized(),
            "{event:?} flattened its result snapshot"
        );
    }

    fn keys(commands: &mut CommandInterpreter, document: &mut Document, text: &str) {
        for character in text.chars() {
            event(commands, document, InputEvent::Key(Key::Char(character)));
        }
    }

    #[test]
    fn small_reflow_in_a_million_line_document_stays_local_and_never_flattens() {
        let bytes = b"// alpha beta\n// gamma\nint x;\n\n".repeat(250_000);
        let mut document = Document::from_bytes(bytes, Encoding::Utf8, Format::Code).unwrap();
        assert!(document.line_count() >= 1_000_000);
        // A fresh revision has no compatibility flat string.
        document.replace(0..0, " ").unwrap();
        let mut commands = CommandInterpreter::new();
        commands.set_reflow_language(Some("cpp".into()));
        let middle = document.line_start(600_000).unwrap();
        assert!(commands.set_cursor(&document, middle));
        // A cold distant edit cannot prove lexical context by looking back a
        // fixed number of rows. Reject without changing the command's state.
        let revision = document.revision();
        keys(&mut commands, &mut document, "2gq");
        let rejected = commands.handle(&mut document, InputEvent::Key(Key::Char('q'))).unwrap();
        assert!(matches!(rejected.status, CommandStatus::Error(_)));
        assert_eq!(document.revision(), revision);
        assert_eq!(commands.cursor(), middle);
        // Each query performs one bounded slice and retains verified lexical
        // checkpoints. Simulate optional offscreen warming before local edits.
        let snapshot = document.hard_line_snapshot();
        let mut warm = false;
        for _ in 0..256 {
            if !snapshot.comment_context(Some("cpp"), 600_000).is_unknown() {
                warm = true;
                break;
            }
        }
        assert!(warm, "bounded slices eventually establish distant context");
        let started = Instant::now();
        keys(&mut commands, &mut document, "2gqq");
        // Plain vertical motions are shared machinery; only reflow is asserted
        // not to flatten, so reposition through the exact line index.
        assert!(commands.set_cursor(&document, document.line_start(600_003).unwrap()));
        keys(&mut commands, &mut document, "2gww");
        assert!(commands.set_cursor(&document, document.line_start(600_006).unwrap()));
        keys(&mut commands, &mut document, ".");
        keys(&mut commands, &mut document, "3gqgq");
        assert!(
            started.elapsed().as_secs() < 20,
            "local reflow took {:?}",
            started.elapsed()
        );
        let snapshot = document.hard_line_snapshot();
        for index in [600_000, 600_003, 600_006] {
            let line = snapshot.line(index).unwrap().content_range();
            assert_eq!(
                snapshot.slice_utf8(line).unwrap(),
                "// alpha beta gamma",
                "{index}"
            );
        }
        assert_eq!(snapshot.line_count(), 1_000_001 - 3);
    }

    #[test]
    fn doubled_forms_do_not_flatten_small_documents() {
        for input in ["gqq", "gwgw", "gww", "2gqgq", "3gqq"] {
            let mut document = Document::from_bytes(
                b"alpha beta\ngamma\n\ndelta\n".to_vec(),
                Encoding::Utf8,
                Format::Code,
            )
            .unwrap();
            document.replace(0..0, " ").unwrap();
            let mut commands = CommandInterpreter::new();
            keys(&mut commands, &mut document, input);
        }
    }
}
