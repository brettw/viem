//! `gq`/`gw` operator glue: resolve the checked extent to hard lines, run the
//! portable formatter, then apply the command-specific cursor policy.
use super::*;
use crate::document::{
    comment_profile_for_language, reflow_edits, ReflowRequest, TextWidthSetting,
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
        self.reflow_language = language;
    }

    pub fn reflow_language(&self) -> Option<&str> {
        self.reflow_language.as_deref()
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
        let profile = self
            .reflow_language
            .as_deref()
            .and_then(comment_profile_for_language);
        let edits = match reflow_edits(&ReflowRequest {
            snapshot: &snapshot,
            format: document.format(),
            lines: lines.clone(),
            text_width: self.text_width.effective(),
            profile,
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
