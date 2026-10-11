//! Two-pass global commands: select stable line identities, then replay Ex.
use super::ex::ExCommand;
use super::ex_addresses::{ExAddressResolutionError, ResolvedExAddresses, ResolvedExRange};
use super::search_regex::{CompiledRegex, RegexInput, RegexLimits, RegexWork};
use super::*;

const GLOBAL_TARGET_LIMIT: usize = 100_000;
pub(crate) const GLOBAL_REPLAY_TIME_BUDGET: std::time::Duration = std::time::Duration::from_millis(250);

pub(crate) fn work_budget_status(completed: usize) -> CommandStatus {
    CommandStatus::Unsupported(format!(":global stopped at its work limit after {completed} completed targets; the completed edits remain undoable. Use a narrower range to continue."))
}

pub(super) fn ex_error(error: ExExecuteError) -> CommandOutput {
    CommandOutput {
        status: CommandStatus::ExError(ExCommandError::Execute(error)),
        mode_changed: true,
        ..CommandOutput::complete()
    }
}

pub(super) fn allowed(action: &ExAction) -> bool {
    if matches!(action, ExAction::Substitute(value) if value.flags.confirm)
        || matches!(action, ExAction::RepeatSubstitute { flags, .. } if flags.confirm)
    {
        return false;
    }
    matches!(
        action,
        ExAction::Global { .. }
            | ExAction::Delete(_)
            | ExAction::Yank(_)
            | ExAction::Put { .. }
            | ExAction::Join { .. }
            | ExAction::Copy { .. }
            | ExAction::Move { .. }
            | ExAction::Normal { .. }
            | ExAction::Sort(_)
            | ExAction::Substitute(_)
            | ExAction::RepeatSubstitute { .. }
            | ExAction::GoToLine(_)
            | ExAction::GoToByte { .. }
            | ExAction::Marks { .. }
            | ExAction::Registers { .. }
            | ExAction::Jumps
            | ExAction::NoHighlight
            | ExAction::Set(_)
            | ExAction::Print { .. }
            | ExAction::Shift { .. }
            | ExAction::Retab { .. }
            | ExAction::Align { .. }
            | ExAction::DeleteMarks { .. }
    )
}

impl CommandInterpreter {
    pub(super) fn execute_ex_global(
        &mut self,
        document: &mut Document,
        command: &ExCommand,
        addresses: &ResolvedExAddresses,
    ) -> CommandOutput {
        let ExAction::Global {
            pattern,
            command: body,
            invert,
        } = &command.action
        else {
            unreachable!()
        };
        let body_command = match parse_ex(body) {
            Ok(command) => command,
            Err(error) => {
                return CommandOutput {
                    status: CommandStatus::ExError(ExCommandError::Parse(error)),
                    ..CommandOutput::complete()
                }
            }
        };
        if !allowed(&body_command.action)
            || (self.global_replay_depth > 0 && command.range.is_some())
        {
            return ex_error(ExExecuteError::UnsupportedGlobalCommand);
        }
        if self.compound_replay_depth >= COMPOUND_REPLAY_LIMIT {
            return CommandOutput::unsupported(":global recursion limit reached");
        }
        let pattern = if pattern.is_empty() {
            match &self.last_search {
                Some((_, pattern)) => pattern.clone(),
                None => return ex_error(ExExecuteError::NoPreviousSearch),
            }
        } else {
            pattern.clone()
        };
        let selected = (|| -> Result<_, ExExecuteError> {
            let range = addresses.range.as_ref().ok_or(ExExecuteError::UnresolvedAddress)?;
            let units = self.resolved_ex_range_units(document, range).map_err(|error| match error {
                ExAddressResolutionError::Execute(error) => error,
                ExAddressResolutionError::Layout(_) => ExExecuteError::UnsupportedCommand(
                    ":global range requires exact displayed rows".into()),
            })?;
            let limits = RegexLimits::default();
            let regex = CompiledRegex::compile(
                &pattern,
                self.search_options.case_insensitive(&pattern)?,
                limits,
            )?;
            let snapshot = document.hard_line_snapshot();
            let input = RegexInput::new(&snapshot);
            let mut work = RegexWork::new(limits);
            let mut matching = std::collections::BTreeSet::new();
            let scope = super::ex_execute::addressed_match_scope(document, range)?;
            let mut next = scope.start;
            while next <= scope.end {
                let Some(matched) = regex.find(&input, next, snapshot.text_length(), &mut work)?
                else {
                    break;
                };
                let bounds = matched.range();
                if bounds.start > scope.end {
                    break;
                }
                if bounds.end <= scope.end && super::ex_execute::addressed_match_eligible(document, range, bounds.clone())? {
                    // The prepared units define replay identity. Mixed ranges
                    // may contain local visual/formatted rows whose indexes
                    // differ from the source line containing the match.
                    let unit = if range.start.domain == ex_addresses::ExLineDomain::PhysicalSource
                        && range.end.domain == ex_addresses::ExLineDomain::PhysicalSource {
                        let source_line = super::ex_execute::addressed_match_line(document, range, bounds.start)?;
                        units.binary_search_by_key(&source_line, |unit| unit.index).ok()
                    } else {
                        let index = units.partition_point(|unit|
                            unit.content.end <= bounds.start && unit.content.start < bounds.start);
                        (!units.is_empty()).then_some(index.min(units.len().saturating_sub(1)))
                    };
                    if let Some(unit) = unit { matching.insert(unit); }
                }
                if matching.len() > GLOBAL_TARGET_LIMIT {
                    return Err(ExExecuteError::Regex(
                        search_regex::RegexError::RegexResourceLimit("global target lines"),
                    ));
                }
                next = snapshot.next_grapheme_boundary(bounds.start)
                    .unwrap_or_else(|| snapshot.text_length().saturating_add(1));
            }
            let inverse = *invert ^ command.bang;
            let mut targets = Vec::new();
            let mut selected_lines = Vec::new();
            for (index, unit) in units.into_iter().enumerate() {
                if matching.contains(&index) == inverse {
                    continue;
                }
                if targets.len() >= GLOBAL_TARGET_LIMIT {
                    return Err(ExExecuteError::Regex(
                        search_regex::RegexError::RegexResourceLimit("global target lines"),
                    ));
                }
                selected_lines.push(unit.clone());
                let request = ExNormalRequest {
                    range: ResolvedExRange {
                        document: document.id(), revision: document.revision(),
                        start: unit.clone(), end: unit.clone(), text: unit.text.clone(),
                        source: unit.source.clone(), lines: vec![unit],
                    },
                    commands: String::new(),
                    literal: true,
                };
                let bound = self
                    .ex_normal_targets(document, &request)
                    .ok_or(ExExecuteError::AddressOverflow)?;
                targets.extend(bound);
            }
            Ok((targets, selected_lines))
        })();
        let (targets, selected_lines) = match selected {
            Ok(targets) => targets,
            Err(error) => return ex_error(error),
        };
        let event_count = body.chars().count().saturating_add(2);
        if targets
            .len()
            .checked_mul(event_count)
            .map_or(true, |total| total > MACRO_REPLAY_EVENT_LIMIT)
        {
            return CommandOutput::count_error(CountError::ReplayEventBudgetExceeded {
                count: targets.len(),
                events_per_iteration: event_count,
                limit: MACRO_REPLAY_EVENT_LIMIT,
            });
        }
        // Vim exposes the global pattern to an empty substitute/search pattern.
        self.last_search = Some((SearchDirection::Forward, pattern.clone()));
        self.search_highlight_suppressed = false;
        self.ex_state.remember_global_pattern(&pattern);
        if targets.is_empty() {
            return if self.global_replay_depth > 0 {
                CommandOutput::complete()
            } else {
                ex_error(ExExecuteError::PatternNotFound(pattern))
            };
        }
        if self.global_replay_depth == 0 {
            if let Some(first) = targets.first().copied().flatten() {
                self.record_jump(document, self.cursor, first.anchor.offset());
            }
        }
        if let Some(result) = self.try_batch_literal_global_delete(document, &body_command, &selected_lines) {
            return result.unwrap_or_else(|error| CommandOutput {
                status: CommandStatus::Error(error.to_string()), ..CommandOutput::complete()
            });
        }
        if let Some(output) = self.try_batch_rich_global_delete(document, &body_command, &selected_lines) {
            return output;
        }
        if let Some(output) = self.try_batch_global_substitute(document, &body_command, &selected_lines) {
            return output;
        }
        let events: Vec<_> = std::iter::once(InputEvent::Key(Key::Char(':')))
            .chain(body.chars().map(|c| InputEvent::Key(Key::Char(c))))
            .chain([InputEvent::Key(Key::Enter)])
            .collect();
        if self.plan_compound_replay {
            self.pending_replay = Some(ReplayPlan::ExNormal(ExNormalReplayPlan {
                global: true,
                global_map: None,
                literal: true,
                events,
                targets,
            }));
            return CommandOutput::complete();
        }
        let mut replay = ExNormalReplayPlan {
            global: true,
            global_map: None,
            literal: true,
            events,
            targets,
        };
        let original_cursor = self.cursor;
        let original_revision = document.revision();
        let mut output = CommandOutput::complete();
        let mut first_error = None;
        self.compound_replay_depth += 1;
        self.global_replay_depth += 1;
        document.begin_edit_group();
        let started = std::time::Instant::now();
        for index in 0..replay.targets.len() {
            let target = match replay.bound_target(index) {
                Ok(target) => target,
                Err(error) => {
                    first_error.get_or_insert(CommandStatus::Error(error.to_string()));
                    break;
                }
            };
            let Some(position) =
                target.and_then(|target| ex_normal_target_position(document, target))
            else {
                continue;
            };
            self.prepare_ex_normal_target(document, target.unwrap(), position);
            let (line, map) = document.capture_position_maps(|document| {
                let mut line = CommandOutput::complete();
                for event in &replay.events {
                    let next = self
                        .handle(document, event.clone())
                        .unwrap_or_else(|error| CommandOutput {
                            status: CommandStatus::Error(error.to_string()),
                            ..CommandOutput::complete()
                        });
                    let stop = command_status_stops_compound(&next.status);
                    line.merge(next);
                    if stop {
                        break;
                    }
                }
                Ok::<_, std::convert::Infallible>(line)
            });
            let mut line = line.unwrap();
            if command_status_stops_compound(&line.status) {
                first_error.get_or_insert(line.status.clone());
                line.status = CommandStatus::Complete;
            }
            output.merge(line);
            let cleanup = self.abort_incomplete_replay(document);
            output.merge(cleanup);
            let composed = replay
                .global_map
                .as_ref()
                .map_or_else(|| Ok(map.clone()), |previous| previous.then(&map));
            match composed {
                Ok(map) => replay.global_map = Some(Box::new(map)),
                Err(error) => {
                    first_error.get_or_insert(CommandStatus::Error(format!(
                        ":global could not rebase its remaining lines: {error}"
                    )));
                    break;
                }
            }
            if index + 1 < replay.targets.len() && started.elapsed() >= GLOBAL_REPLAY_TIME_BUDGET {
                first_error.get_or_insert(work_budget_status(index + 1));
                break;
            }
        }
        document.end_edit_group();
        self.global_replay_depth -= 1;
        self.compound_replay_depth -= 1;
        output.document_changed |= original_revision != document.revision();
        output.cursor_moved |= original_cursor != self.cursor;
        output.status = first_error.unwrap_or(CommandStatus::Complete);
        output
    }

    fn global_batch_lines_match_mode(&self, lines: &[ex_addresses::ResolvedExLine]) -> bool {
        if self.line_mode != LineMode::Visual || !self.wrap { return true; }
        let Some(snapshot) = self.line_layout.as_ref() else { return true; };
        let complete_rows: std::collections::BTreeSet<_> = snapshot.rows.iter()
            .filter(|row| row.fragment_index == 0 && !row.wraps_to_next)
            .map(|row| (row.text_range.start, row.text_range.end)).collect();
        lines.iter().all(|line| complete_rows.contains(&(line.content.start, line.content.end)))
    }

    /// Simple paragraph deletions are independent. Only the final nine rich
    /// register values survive the ordered numbered-register rotation; capturing
    /// earlier values as plain text avoids exporting the whole source per row.
    fn try_batch_rich_global_delete(&mut self, document: &mut Document, command: &ExCommand,
        lines: &[ex_addresses::ResolvedExLine]) -> Option<CommandOutput> {
        let ExAction::Delete(arguments) = &command.action else { return None; };
        if !document.format().is_wysiwyg() || self.line_mode != LineMode::Visual
            || !self.global_batch_lines_match_mode(lines) || command.range.is_some()
            || arguments.count.is_some() || arguments.register.is_some_and(|name| name.is_uppercase() || matches!(name, '+' | '*')) {
            return None;
        }
        if let Err(output) = self.require_register_write(arguments.register) { return Some(output); }
        let snapshot = document.hard_line_snapshot();
        let mut ranges: Vec<Range<usize>> = Vec::new();
        let mut values = Vec::new();
        for (index, line) in lines.iter().enumerate() {
            let hard = snapshot.line_at_offset(line.content.start).ok()?;
            if hard.content_range() != line.content { return None; }
            let extent = MotionExtent { range: hard.linewise_range(), kind: MotionKind::Linewise };
            let value = if index + 9 >= lines.len() { register_value(document, &snapshot, &extent, arguments.register) }
                else { RegisterValue::linewise(format!("{}\n", snapshot.capture(hard.content_range()).ok()?.text())) };
            values.push(value);
            let mut range = extent.range;
            if range.end == snapshot.text_length() && range.end == hard.content_range().end && range.start > 0 {
                range.start -= 1;
            }
            if let Some(previous) = ranges.last_mut().filter(|previous| previous.end >= range.start) {
                previous.end = previous.end.max(range.end);
            } else { ranges.push(range); }
        }
        let origin = ranges.last()?.start;
        let cursor = document.text_anchor(document.text_point(origin).ok()?, Association::BeforeInsertion,
            BoundaryAffinity::Downstream, DeletionRecovery::PreferFollowingThenPreceding).ok()?;
        let prepared = document.prepare_independent_paragraph_deletions(ranges).ok().flatten()?;
        let map = prepared.text_position_map();
        let next_cursor = mapped_anchor(map, cursor).ok()??;
        let jumps = self.jumps.iter().map(|offset| {
            let anchor = document.text_anchor(document.text_point(*offset)?, Association::AfterInsertion,
                BoundaryAffinity::Downstream, DeletionRecovery::PreferFollowingThenPreceding)
                .map_err(command_position_document_error)?;
            mapped_anchor(map, anchor).map_err(command_position_document_error)?
                .ok_or(DocumentError::AmbiguousProjection)
        }).collect::<Result<Vec<_>, _>>().ok()?;
        let changed = !prepared.is_no_op();
        document.commit_model_transaction(prepared).ok()?;
        self.jumps = jumps;
        self.jumps_revision = Some(document.revision());
        for value in values { self.delete_register(arguments.register, value, DeletionClass::Large); }
        let line = line_start(&document.hard_line_snapshot(), next_cursor);
        self.cursor = first_nonblank_document(document, &document.hard_line_snapshot(), line);
        self.preferred_column = None;
        self.visual_position = None;
        self.desired_x = None;
        Some(CommandOutput { document_changed: changed, cursor_moved: true, ..CommandOutput::complete() })
    }

    fn try_batch_global_substitute(&mut self, document: &mut Document, command: &ExCommand,
        lines: &[ex_addresses::ResolvedExLine]) -> Option<CommandOutput> {
        if !document.format().is_literal() || !self.global_batch_lines_match_mode(lines) { return None; }
        let context = ExExecutionContext { search_options: self.search_options,
            last_search_pattern: self.last_search.as_ref().map(|(_, pattern)| pattern.clone()),
            ..ExExecutionContext::default() };
        let plan = ex_execute::prepare_global_substitutions(document, &self.ex_state,
            &context, command, &self.registers, lines).ok().flatten()?;
        let jumps = self.jumps.iter().map(|at| document.text_anchor(document.text_point(*at)?,
            Association::AfterInsertion, BoundaryAffinity::Downstream,
            DeletionRecovery::PreferFollowingThenPreceding).map_err(command_position_document_error))
            .collect::<Result<Vec<_>, _>>().ok()?;
        // Preparation failures use the ordinary prefix-preserving replay. No
        // partial batch state or history has been published at this point.
        let outcome = commit_ex(document, &mut self.ex_state, plan).ok()?;
        let map = outcome.model_transactions().last().expect("fragment model request publishes a transaction").text_position_map();
        self.jumps = match jumps.into_iter().map(|anchor| mapped_anchor(map, anchor)
            .map_err(command_position_document_error).and_then(|at| at.ok_or(DocumentError::AmbiguousProjection)))
            .collect::<Result<Vec<_>, _>>() {
            Ok(jumps) => jumps,
            Err(error) => return Some(CommandOutput { status: CommandStatus::Error(error.to_string()),
                ..CommandOutput::complete() }),
        };
        self.jumps_revision = Some(document.revision());
        if let Some(ExNavigation::TextOffset(at)) = outcome.navigation { self.cursor = at; }
        self.preferred_column = None;
        self.visual_position = None;
        self.desired_x = None;
        if let Some(pattern) = self.ex_state.previous_substitute_pattern() {
            let direction = self.last_search.as_ref().map_or(SearchDirection::Forward, |(direction, _)| *direction);
            self.last_search = Some((direction, pattern.to_owned()));
        }
        Some(CommandOutput { document_changed: outcome.document_changed, cursor_moved: true,
            ex_outcome: Some(outcome), ..CommandOutput::complete() })
    }
}

impl CommandInterpreter {
    /// Independent literal line deletions have no intermediate semantic
    /// dependencies. Publish their local source patches together, retaining
    /// the same ordered register writes and one global undo unit.
    fn try_batch_literal_global_delete(
        &mut self, document: &mut Document, command: &ExCommand,
        lines: &[ex_addresses::ResolvedExLine],
    ) -> Option<Result<CommandOutput, DocumentError>> {
        let ExAction::Delete(arguments) = &command.action else { return None; };
        if !(document.format().is_literal() || document.format().is_source_view()) || command.range.is_some() || arguments.count.is_some()
            || lines.iter().any(|line| line.source.is_none()) {
            return None;
        }
        if self.line_mode == LineMode::Visual && self.wrap {
            if let Some(snapshot) = self.line_layout.as_ref() {
                let complete_rows: std::collections::BTreeSet<_> = snapshot.rows.iter()
                    .filter(|row| row.fragment_index == 0 && !row.wraps_to_next)
                    .map(|row| (row.text_range.start, row.text_range.end)).collect();
                // A contextual body still acts on displayed rows. Only use
                // physical patches when each target is proven to be exactly
                // one complete row; unseen or wrapped rows retain replay.
                if lines.iter().any(|line| !complete_rows.contains(&(line.content.start, line.content.end))) {
                    return None;
                }
            }
        }
        Some((|| {
            if let Err(output) = self.require_register_write(arguments.register) { return Ok(output); }
            let mut deletions: Vec<Range<usize>> = Vec::new();
            let mut values = Vec::new();
            for line in lines {
                let range = line.source.as_ref().expect("source lines checked").clone();
                let payload = document.physical_source_payload(range.clone())?;
                let mut text = payload.text().to_owned();
                let mut breaks = payload.break_offsets().to_vec();
                if !breaks.last().is_some_and(|offset| offset + 1 == text.len()) {
                    breaks.push(text.len()); text.push('\n');
                }
                values.push(RegisterValue::try_new(text, RegisterKind::Linewise, breaks)
                    .map_err(|_| DocumentError::FormattedPayloadCannotReproject)?);
                let deletion = range;
                if let Some(previous) = deletions.last_mut().filter(|previous| previous.end >= deletion.start) {
                    previous.end = previous.end.max(deletion.end);
                } else { deletions.push(deletion); }
            }
            // Widen only the final merged run. Widening each last-line
            // target first can absorb the preceding blank line, then widen
            // again across an unrelated target when the run is finalized.
            if let Some(last) = deletions.last_mut() {
                if last.end == document.source_byte_len() && last.start > 0 {
                    let terminal = document.physical_line_at_source(last.end)?;
                    if terminal.content_range.end == terminal.source_range.end && !terminal.content_range.is_empty() {
                        let first = document.physical_line_at_source(last.start)?.index;
                        if first > 0 { last.start = document.physical_line(first - 1)?.content_range.end; }
                    }
                }
            }
            let last = deletions.last().map_or(0, |range| range.start);
            let old = document.source_point(last).map_err(|_| DocumentError::AmbiguousProjection)?;
            let prepared = document.prepare_model_request(ModelRequest::ApplyPhysicalSourcePatches {
                document: document.id(), revision: document.revision(),
                patches: deletions.into_iter().map(|range| crate::document::SourcePatch::primary(range, Vec::new())).collect(),
            }).map_err(command_document_error)?;
            let at = prepared.text_position_map().source_position_map()
                .and_then(|map| map.map_source_point(old, Association::BeforeInsertion, DeletionRecovery::PreferFollowingThenPreceding).ok())
                .and_then(|mapped| mapped.value().copied()).ok_or(DocumentError::AmbiguousProjection)?;
            let changed = !prepared.is_no_op();
            let map = prepared.text_position_map().clone();
            let jumps = self.jumps.iter().map(|offset| {
                document.text_anchor(document.text_point(*offset)?, Association::AfterInsertion,
                    BoundaryAffinity::Downstream, DeletionRecovery::PreferFollowingThenPreceding)
                    .map_err(command_position_document_error)
            }).collect::<Result<Vec<_>, _>>()?;
            document.commit_model_transaction(prepared).map_err(command_document_error)?;
            self.jumps = jumps.into_iter().map(|anchor| {
                mapped_anchor(&map, anchor).map_err(command_position_document_error)?
                    .ok_or(DocumentError::AmbiguousProjection)
            }).collect::<Result<Vec<_>, _>>()?;
            self.jumps_revision = Some(document.revision());
            for value in values { self.delete_register(arguments.register, value, DeletionClass::Large); }
            let line = document.physical_line_at_source(at.offset())?.index;
            let source = document.physical_first_nonblank_source(line)?;
            self.cursor = document.visible_point_for_source(source, true)?;
            self.remember_physical_cursor(document, source);
            self.preferred_column = None;
            self.visual_position = None;
            self.desired_x = None;
            Ok(CommandOutput { document_changed: changed, cursor_moved: true, ..CommandOutput::complete() })
        })())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn global_and_pattern_addresses_keep_large_projection_unmaterialized_and_bound_targets() {
        let mut document = Document::new(format!("needle\n{}needle", "row\n".repeat(20_000)));
        let mut commands = CommandInterpreter::new();
        let global = parse_ex(":g/needle/y a").unwrap();
        let addresses = commands.resolve_ex_addresses(&document, &global, RegexLimits::default()).unwrap();
        assert_eq!(
            commands.execute_ex_global(&mut document, &global, &addresses).status,
            CommandStatus::Complete
        );
        assert!(!document.projection().compatibility_text_is_materialized());
        assert_eq!(commands.register('a').unwrap().text, "needle\n");
        assert_eq!(
            commands
                .execute_ex_command(&mut document, ":?needle?")
                .status,
            CommandStatus::Complete
        );
        assert_eq!(commands.cursor(), 0);
        assert!(!document.projection().compatibility_text_is_materialized());
        let mut excessive = Document::new("x\n".repeat(GLOBAL_TARGET_LIMIT + 1));
        let mut commands = CommandInterpreter::new();
        let global = parse_ex(":g/^/d").unwrap();
        let addresses = commands.resolve_ex_addresses(&excessive, &global, RegexLimits::default()).unwrap();
        assert!(format!(
            "{:?}",
            commands.execute_ex_global(&mut excessive, &global, &addresses).status
        )
        .contains("global target lines"));
        assert!(!excessive.undo());
        assert!(!excessive.projection().compatibility_text_is_materialized());
    }

    #[test]
    fn visual_global_tests_each_selected_wrapped_row_and_replays_exact_anchors() {
        let original = "abc def ghi jkl mno pqr stu vwx yz\ntail";
        let mut core = crate::Core::new(Document::new(original));
        let view = core.add_view(crate::layout::MockTextMeasurementProvider::new(), 55.0, 500.0);
        let snapshot = core.layout(view).unwrap().snapshot().unwrap().clone();
        for character in ":.,.+2g/./normal iX".chars() {
            core.handle(view, crate::CoreEvent::Input(InputEvent::Key(Key::Char(character)))).unwrap();
        }
        let output = core.handle(view, crate::CoreEvent::Input(InputEvent::Key(Key::Enter))).unwrap();
        assert_eq!(output.command.unwrap().status, CommandStatus::Complete);
        let mut expected = original.to_owned();
        for row in snapshot.rows[..3].iter().rev() { expected.insert(row.text_range.start, 'X'); }
        assert_eq!(core.document().text(), expected);
        core.handle(view, crate::CoreEvent::Input(InputEvent::Key(Key::Char('u')))).unwrap();
        assert_eq!(core.document().text(), original);
    }

    #[test]
    fn headless_mixed_global_matches_prepared_formatted_units() {
        let source = b"one\ncontinuation\n\nsecond\n\nthird";
        let mut document = Document::from_bytes(source.to_vec(),
            crate::document::Encoding::Utf8, crate::document::Format::Markdown).unwrap();
        let mut commands = CommandInterpreter::new();
        let output = commands.execute_ex_command(&mut document, ":2,$g/third/normal iX");
        assert_eq!(output.status, CommandStatus::Complete);
        assert_eq!(document.text(), "one continuation\nsecond\nXthird");
        assert_eq!(document.source_bytes(), b"one\ncontinuation\n\nsecond\n\nXthird");
        assert!(document.undo());
        assert_eq!(document.source_bytes(), source);
    }
}
