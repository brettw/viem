//! Mark names and publication of the last affected text range.
use super::*;

pub(super) fn valid_mark(name: char) -> bool {
    name.is_ascii_alphabetic() || matches!(name, '<' | '>' | '[' | ']' | '.' | '^' | '\'' | '`')
}
pub(super) fn canonical_mark(name: char) -> char {
    if name == '`' {
        '\''
    } else {
        name
    }
}

impl CommandInterpreter {
    pub(crate) fn note_affected_range(
        &mut self,
        document: &Document,
        range: Range<usize>,
        continuing_insert: bool,
        change: bool,
    ) {
        let lines = document.hard_line_snapshot();
        let end = if range.end > range.start {
            lines
                .previous_grapheme_boundary(range.end)
                .unwrap_or(range.start)
        } else {
            range.start
        };
        let continuing_insert = continuing_insert
            && self.affected_insert_unit == Some(document.edit_group_generation());
        self.affected_insert_unit = change.then(|| document.edit_group_generation());
        if !continuing_insert || !self.marks.contains_key(&'[') {
            self.marks.insert('[', range.start);
        }
        self.marks.insert(']', end);
        if change {
            self.marks.insert('.', range.start);
        }
    }
}

static NEXT_FILE_MARK: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
impl CommandInterpreter {
    pub(super) fn finish_set_mark(&mut self, key: Key) -> CommandOutput {
        self.pending = Pending::None;
        match key {
            Key::Char(name)
                if name.is_ascii_alphabetic() || matches!(name, '\'' | '`' | '[' | ']') =>
            {
                self.set_named_mark(name);
                CommandOutput::complete()
            }
            _ => CommandOutput::unsupported("invalid mark name"),
        }
    }
    pub(super) fn set_named_mark(&mut self, name: char) {
        if canonical_mark(name) == '\'' {
            self.previous_jump_anchor = None;
        }
        self.marks.insert(canonical_mark(name), self.cursor);
        if name.is_ascii_uppercase() {
            let generation = NEXT_FILE_MARK
                .fetch_update(
                    std::sync::atomic::Ordering::Relaxed,
                    std::sync::atomic::Ordering::Relaxed,
                    |value| value.checked_add(1),
                )
                .expect("file mark identities exhausted");
            self.file_mark_generations.insert(name, generation);
            self.file_mark_ownership |= 1 << (name as u32 - 'A' as u32);
        }
    }
    pub(crate) fn mark_offset(&self, name: char) -> Option<usize> {
        if name.is_ascii_uppercase()
            && self.file_mark_ownership & (1 << (name as u32 - 'A' as u32)) == 0
        {
            return None;
        }
        self.marks.get(&canonical_mark(name)).copied()
    }
    pub(crate) fn set_file_mark_ownership(&mut self, mask: u32) {
        self.file_mark_ownership = mask;
    }
    pub(super) fn file_mark_request(&self, name: char, linewise: bool) -> CommandOutput {
        if self.native_requests_blocked {
            return scripting::native_request_error();
        }
        let mut outcome = ExOutcome::default();
        outcome.frontend_requests.push(ExFrontendRequest::File(
            ex_execute::ExFileRequest::JumpFileMark { name, linewise },
        ));
        CommandOutput {
            ex_outcome: Some(outcome),
            ..CommandOutput::complete()
        }
    }
}
impl CommandInterpreter {
    pub(crate) fn mark_destination(
        &self,
        document: &Document,
        marked: usize,
        linewise: bool,
    ) -> usize {
        let lines = document.hard_line_snapshot();
        if linewise {
            first_nonblank_document(document, &lines, marked)
        } else {
            normalize_normal_cursor_document(document, &lines, marked)
        }
    }
    pub(crate) fn restore_file_mark(&mut self, name: char, generation: u64, offset: usize) {
        self.marks.insert(name, offset);
        let generation = if generation == 0 {
            NEXT_FILE_MARK
                .fetch_update(
                    std::sync::atomic::Ordering::Relaxed,
                    std::sync::atomic::Ordering::Relaxed,
                    |value| value.checked_add(1),
                )
                .expect("file mark identities exhausted")
        } else {
            generation
        };
        self.file_mark_generations.insert(name, generation);
        self.file_mark_ownership |= 1 << (name as u32 - 'A' as u32);
    }
    pub(crate) fn note_external_jump(&mut self, document: &Document, origin: usize) {
        self.record_jump(document, origin, self.cursor);
    }
}
impl CommandInterpreter {
    pub(super) fn capture_mark_anchors(
        &self,
        document: &Document,
    ) -> Result<BTreeMap<char, TextAnchor>, DocumentError> {
        if let Some(revision) = self
            .position_revision
            .filter(|revision| *revision != document.revision())
        {
            return Err(DocumentError::WrongSnapshot {
                expected: document.revision(),
                actual: revision,
            });
        }
        self.marks
            .iter()
            .map(|(name, offset)| {
                let point = document.text_point(*offset)?;
                document
                    .text_anchor(
                        point,
                        Association::AfterInsertion,
                        BoundaryAffinity::Downstream,
                        DeletionRecovery::PreferFollowingThenPreceding,
                    )
                    .map(|anchor| (*name, anchor))
                    .map_err(command_position_document_error)
            })
            .collect()
    }

    pub(super) fn rebase_unchanged_marks(
        &mut self,
        before: &Self,
        anchors: &BTreeMap<char, TextAnchor>,
        map: &PositionMap,
    ) -> Result<bool, PositionError> {
        let mut marks = self.marks.clone();
        for (name, anchor) in anchors {
            let Some(old_offset) = before.marks.get(name) else {
                continue;
            };
            if self.marks.get(name) != Some(old_offset) {
                continue;
            }
            let Some(mapped) = mapped_anchor(map, *anchor)? else {
                return Ok(false);
            };
            marks.insert(*name, mapped);
        }

        // A global command can record its jump before publishing an edit.
        // That newly assigned mark is absent from the pre-command mark set;
        // retain its exact source anchor until this transition is published.
        if let Some(anchor) = self.previous_jump_anchor.filter(|anchor| {
            anchor.revision() == map.source_revision()
                && self.marks.get(&'\'') == Some(&anchor.offset())
        }) {
            let Some(mapped) = mapped_anchor(map, anchor)? else {
                return Ok(false);
            };
            marks.insert('\'', mapped);
        }
        self.marks = marks;
        self.previous_jump_anchor = None;
        Ok(true)
    }

    pub(super) fn track_headless_marks(
        &mut self,
        document: &mut Document,
        action: impl FnOnce(&mut Self, &mut Document) -> Result<CommandOutput, DocumentError>,
    ) -> Result<CommandOutput, DocumentError> {
        if self.plan_compound_replay || self.tracking_positions {
            return action(self, document);
        }
        let before = self.clone();
        let checkpoint = document.begin_command_checkpoint();
        let anchors = self.capture_mark_anchors(document).ok();
        self.tracking_positions = true;
        let (result, map) = document.capture_position_maps(|document| action(self, document));
        self.tracking_positions = false;
        if let Ok(output) = &result {
            if map.source_revision() != map.target_revision() {
                if let Some(anchors) = anchors {
                    if !self
                        .rebase_unchanged_marks(&before, &anchors, &map)
                        .unwrap_or(false)
                    {
                        document.rollback_command_checkpoint(checkpoint);
                        *self = before;
                        return Err(DocumentError::AmbiguousProjection);
                    }
                }
                let mut ranges = map.replacements().map(|(_, new)| new);
                if let Some(first) = ranges.next() {
                    let range = ranges.fold(first, |all, next| {
                        all.start.min(next.start)..all.end.max(next.end)
                    });
                    self.note_affected_range(
                        document,
                        range,
                        matches!(before.mode, Mode::Insert | Mode::Replace),
                        !output.history_navigation,
                    );
                }
            }
        }
        document.commit_command_checkpoint(checkpoint);
        result
    }
}
