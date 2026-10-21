//! Ex mutations over exact source lines and displayed rows.
use super::*;
use crate::command::ex::ExAlignment;
use crate::command::ex_addresses::{
    ExLineDomain, ResolvedExAddresses, ResolvedExLine, ResolvedExRange,
};
use crate::document::{FragmentEdit, ReplacementFragment, SourcePatch};
use std::collections::BTreeMap;

pub(super) fn prepare<R: ExRegisterReader + ?Sized>(
    document: &Document,
    _state: &ExExecutionState,
    context: &ExExecutionContext,
    command: &ExCommand,
    registers: &R,
    addresses: &ResolvedExAddresses,
) -> Result<Option<ExPlan>, ExExecuteError> {
    if !matches!(
        command.action,
        ExAction::Delete(_)
            | ExAction::Yank(_)
            | ExAction::Put { .. }
            | ExAction::Join { .. }
            | ExAction::Copy { .. }
            | ExAction::Move { .. }
            | ExAction::Sort(_)
            | ExAction::Shift { .. }
            | ExAction::Retab { .. }
            | ExAction::Align { .. }
    ) {
        return Ok(None);
    }
    if addresses.document != document.id() {
        return Err(ExExecuteError::WrongDocument);
    }
    if addresses.revision != document.revision() {
        return Err(ExExecuteError::StalePlan {
            expected: addresses.revision,
            actual: document.revision(),
        });
    }
    let mut plan = ExPlan::empty(document);
    plan.navigation_is_candidate = true;
    if let ExAction::Put { register } = command.action {
        let value = registers
            .read(register)
            .ok_or(ExExecuteError::EmptyRegister(register))?;
        prepare_put(document, addresses, command.bang, &value, &mut plan)?;
        return Ok(Some(plan));
    }
    let range = addresses
        .range
        .as_ref()
        .ok_or(ExExecuteError::AddressOverflow)?;
    match &command.action {
        ExAction::Delete(arguments) | ExAction::Yank(arguments) => {
            let source = source_range(document, range)?;
            let value = if let Some(source) = &source {
                register_from_payload(document.physical_source_payload(source.clone())?)?
            } else {
                capture_linewise_register(document, range.text.clone())?
            };
            if matches!(command.action, ExAction::Delete(_)) {
                if let Some(source) = source {
                    if range.start.domain != range.end.domain {
                        plan.mutation = ExMutation::Model(ModelRequest::DeleteAddressedLines {
                            document: document.id(),
                            revision: document.revision(),
                            range: range.text.clone(),
                            source,
                            physical_start: range.start.domain == ExLineDomain::PhysicalSource,
                            physical_end: range.end.domain == ExLineDomain::PhysicalSource,
                        });
                        plan.outcome.navigation = Some(ExNavigation::TextOffset(range.text.start));
                    } else {
                        let deletion = source_deletion_range(document, source.clone())?;
                        let destination = source_deletion_cursor(document, &source, &deletion)?;
                        stage_source(
                            document,
                            vec![SourcePatch::primary(deletion, Vec::new())],
                            &mut plan,
                        )?;
                        plan.outcome.navigation =
                            Some(ExNavigation::PhysicalSourceOffset(destination));
                    }
                } else if let Some(lines) = exact_hard_lines(document, range)? {
                    let deletion = hard_line_deletion_range(document, lines)?;
                    let last_start = document.hard_line_snapshot().line(lines.end)
                        .ok_or(ExExecuteError::AddressOverflow)?.content_range().start;
                    let touches_list = document.projection().blocks_for_region(&deletion)
                        .iter().any(|block| (block.range.start < deletion.end
                            && (deletion.start < block.range.end
                                || block.range.is_empty() && deletion.start <= block.range.start)
                            || block.range == (last_start..last_start))
                            && block.containers.iter().any(|member| matches!(member.container.kind,
                                crate::document::ContainerKind::ListItem)));
                    // At EOF the deletion includes the preceding separator.
                    // A table's terminal caret is not selected cell content
                    // when all addressed lines start after that table.
                    let follows_table = document.projection().table_at(deletion.start)
                        .is_some_and(|table| table.range.end == deletion.start
                            && range.text.start > table.range.end);
                    if document.projection().range_intersects_table(&deletion)
                        || touches_list && !follows_table {
                        // A table hard line names one cell, not the displayed
                        // row matched by :global. Keep the text-range policy:
                        // its synthetic boundary requires an explicit row action.
                        // List rows likewise retain their existing join policy
                        // for unselected continuations and nested children.
                        plan.stage_text_edits(document, vec![TextEdit::new(deletion, "")]);
                    } else {
                        // Complete displayed lines own their block syntax. A
                        // generic text deletion can retain a heading prefix on
                        // the next table or carry code treatment into prose.
                        plan.mutation = ExMutation::Model(ModelRequest::DeleteLines {
                            document: document.id(),
                            revision: document.revision(),
                            range: deletion,
                        });
                    }
                    plan.outcome.navigation = Some(ExNavigation::TextOffset(
                        line_start_after_deletion(document, lines)?,
                    ));
                } else {
                    plan.mutation = ExMutation::Model(ModelRequest::DeleteLines {
                        document: document.id(),
                        revision: document.revision(),
                        range: range.text.clone(),
                    });
                    plan.outcome.navigation = Some(ExNavigation::TextOffset(range.text.start));
                }
            }
            plan.outcome.register_effects.push(ExRegisterEffect {
                kind: if matches!(command.action, ExAction::Delete(_)) {
                    ExRegisterEffectKind::Delete
                } else {
                    ExRegisterEffectKind::Yank
                },
                requested: arguments.register,
                range: range.text.clone(),
                value,
            });
        }
        ExAction::Join { .. } => prepare_join(document, context, range, command.bang, &mut plan)?,
        ExAction::Copy { .. } | ExAction::Move { .. } => {
            prepare_transfer(
                document,
                range,
                addresses,
                matches!(command.action, ExAction::Move { .. }),
                &mut plan,
            )?;
        }
        ExAction::Sort(options) => {
            prepare_sort(document, context, range, options, command.bang, &mut plan)?
        }
        ExAction::Shift {
            right,
            amount,
            print,
            number,
            list,
            ..
        } => {
            prepare_whitespace(
                document,
                context,
                range,
                WhitespaceAction::Shift {
                    right: *right,
                    amount: *amount,
                },
                &mut plan,
            )?;
            if *print {
                let last = range.end.clone();
                plan.outcome.frontend_requests.push(ExFrontendRequest::Info(
                    ExInfoRequest::PrintLines {
                        range: ResolvedExRange {
                            document: document.id(),
                            revision: document.revision(),
                            start: last.clone(),
                            end: last.clone(),
                            text: last.text.clone(),
                            source: last.source.clone(),
                            lines: vec![last],
                        },
                        number: *number,
                        list: *list,
                    },
                ));
            }
        }
        ExAction::Retab {
            tabstop,
            indent_only,
        } => {
            let old = context.indentation.effective();
            let width = tabstop
                .filter(|width| *width != 0)
                .unwrap_or(u64::from(old.tabstop));
            if !(1..=1024).contains(&width) {
                return Err(ExExecuteError::InvalidOptionValue {
                    option: "tabstop".into(),
                    value: width.to_string(),
                });
            }
            let mut new = old;
            new.tabstop = width as u32;
            prepare_whitespace(
                document,
                context,
                range,
                WhitespaceAction::Retab {
                    old,
                    new,
                    indent_only: *indent_only,
                    bang: command.bang,
                },
                &mut plan,
            )?;
            if tabstop.is_some_and(|width| width != 0) {
                let mut updated = context.indentation;
                updated.local.tabstop = Some(new.tabstop);
                plan.outcome.option_effects.push(ExOptionEffect {
                    name: ExOptionName::TabStop,
                    scope: SetScope::GlobalAndLocal,
                    old_value: ExOptionValue::Indentation(context.indentation),
                    new_value: ExOptionValue::Indentation(updated),
                });
            }
        }
        ExAction::Align { alignment, width } => {
            if document.format().is_markdown() {
                return Err(ExExecuteError::UnsupportedCommand("Paragraph alignment is not supported in Markdown Preview; turn Preview off for whitespace alignment".into()));
            }
            let default = if *alignment == ExAlignment::Left {
                0
            } else {
                match context.text_width.effective() {
                    0 => 80,
                    width => width as u64,
                }
            };
            let width = width
                .filter(|width| *width != 0 || *alignment == ExAlignment::Left)
                .unwrap_or(default);
            let width = usize::try_from(width).map_err(|_| ExExecuteError::AddressOverflow)?;
            if width > 1_000_000 {
                return Err(ExExecuteError::InvalidCount(width as u64));
            }
            prepare_whitespace(
                document,
                context,
                range,
                WhitespaceAction::Align {
                    alignment: *alignment,
                    width,
                },
                &mut plan,
            )?;
        }
        _ => unreachable!("addressed mutation filtered above"),
    }
    Ok(Some(plan))
}

fn source_range(
    document: &Document,
    range: &ResolvedExRange,
) -> Result<Option<Range<usize>>, ExExecuteError> {
    if range.start.domain != ExLineDomain::PhysicalSource
        && range.end.domain != ExLineDomain::PhysicalSource
    {
        return Ok(None);
    }
    Ok(Some(document.addressed_source_range(
        range.text.clone(),
        range.start.source.as_ref().map(|source| source.start),
        range.end.source.as_ref().map(|source| source.end),
    )?))
}

fn register_from_payload(payload: FormattedTextPayload) -> Result<ExRegisterValue, ExExecuteError> {
    let mut text = payload.text().to_owned();
    let mut breaks = payload.break_offsets().to_vec();
    if !breaks
        .last()
        .is_some_and(|offset| *offset + 1 == text.len())
    {
        breaks.push(text.len());
        text.push('\n');
    }
    Ok(
        ExRegisterValue::try_new(text, ExRegisterKind::Linewise, breaks)
            .expect("physical payload and synthetic final break are validated"),
    )
}

fn stage_source(
    document: &Document,
    patches: Vec<SourcePatch>,
    plan: &mut ExPlan,
) -> Result<(), ExExecuteError> {
    let mut changed = Vec::with_capacity(patches.len());
    for patch in patches {
        if document.source_bytes_in(patch.range())? != patch.replacement() {
            changed.push(patch);
        }
    }
    if !changed.is_empty() {
        plan.mutation = ExMutation::Model(ModelRequest::ApplyPhysicalSourcePatches {
            document: document.id(),
            revision: document.revision(),
            patches: changed,
        });
    }
    Ok(())
}

fn stage_fragments(document: &Document, edits: Vec<FragmentEdit>, plan: &mut ExPlan) {
    plan.mutation = ExMutation::Model(ModelRequest::ApplyFragmentEdits {
        document: document.id(),
        revision: document.revision(),
        edits,
    });
}

fn exact_hard_lines(
    document: &Document,
    range: &ResolvedExRange,
) -> Result<Option<HardLineRange>, ExExecuteError> {
    let snapshot = document.hard_line_snapshot();
    let first = snapshot
        .line_at_offset(range.text.start)
        .map_err(|_| ExExecuteError::AddressOverflow)?;
    let end = if range.text.end > range.text.start {
        snapshot.previous_grapheme_boundary(range.text.end)
            .ok_or(ExExecuteError::AddressOverflow)?
    } else {
        range.text.end
    };
    let last = snapshot
        .line_at_offset(end)
        .map_err(|_| ExExecuteError::AddressOverflow)?;
    let lines = HardLineRange {
        start: first.index(),
        end: last.index(),
    };
    if hard_line_text_range(document, lines)? != range.text {
        return Ok(None);
    }
    if let Some(source) = source_range(document, range)? {
        if document
            .source_byte_range_for_hard_lines(lines.start..lines.end + 1)
            .ok()
            .as_ref()
            != Some(&source)
        {
            return Ok(None);
        }
    }
    Ok(Some(lines))
}

fn formatted_units(
    document: &Document,
    range: &ResolvedExRange,
) -> Result<Vec<ResolvedExLine>, ExExecuteError> {
    if !range.lines.is_empty() {
        return Ok(range.lines.clone());
    }
    let snapshot = document.hard_line_snapshot();
    let mut result = Vec::new();
    for index in range.start.index..=range.end.index {
        let line = snapshot
            .line(index)
            .ok_or(ExExecuteError::AddressOverflow)?;
        let content = line.content_range();
        let text = content.start
            ..line
                .separator_range()
                .map_or(content.end, |separator| separator.end);
        result.push(ResolvedExLine {
            domain: ExLineDomain::Formatted,
            index,
            content: content.clone(),
            text,
            source: None,
            source_content: None,
            position: super::super::VisualPosition {
                text_offset: content.start,
                affinity: BoundaryAffinity::Downstream,
            },
        });
    }
    Ok(result)
}

struct SourceUnit {
    content: Range<usize>,
    separator: Range<usize>,
    text: String,
    boundaries: BTreeMap<usize, usize>,
}

fn source_units(
    document: &Document,
    range: Range<usize>,
) -> Result<Vec<SourceUnit>, ExExecuteError> {
    let first = document.physical_line_index_at_source(range.start)?;
    let last = document.physical_line_index_at_source(if range.is_empty() {
        range.end
    } else {
        range.end - 1
    })?;
    let mut units = Vec::with_capacity(last - first + 1);
    for index in first..=last {
        let (source, content) = document.physical_line_ranges(index)?;
        let start = content.start.max(range.start).min(range.end);
        let end = content.end.min(range.end).max(start);
        let content = start..end;
        let separator = end..source.end.min(range.end).max(end);
        let decoded = document
            .encoding()
            .decode_region(&document.source_bytes_in(content.clone())?, content.start)?;
        let mut boundaries = BTreeMap::new();
        boundaries.insert(0, content.start);
        boundaries.insert(decoded.text.len(), content.end);
        for span in decoded.scalar_spans() {
            boundaries.insert(span.decoded.start, span.source.start);
            boundaries.insert(span.decoded.end, span.source.end);
        }
        units.push(SourceUnit {
            content,
            separator,
            text: decoded.text,
            boundaries,
        });
    }
    Ok(units)
}

fn source_deletion_range(
    document: &Document,
    range: Range<usize>,
) -> Result<Range<usize>, ExExecuteError> {
    if range.end != document.source_byte_len() || range.is_empty() {
        return Ok(range);
    }
    let first = document.physical_line_index_at_source(range.start)?;
    let last = document.physical_line_index_at_source(range.end - 1)?;
    let (last_source, last_content) = document.physical_line_ranges(last)?;
    let (first_source, _) = document.physical_line_ranges(first)?;
    if first > 0 && first_source.start == range.start && last_source.end == last_content.end {
        let (_, preceding) = document.physical_line_ranges(first - 1)?;
        return Ok(preceding.end..range.end);
    }
    Ok(range)
}

pub(super) fn source_first_nonblank(
    document: &Document,
    line: usize,
) -> Result<usize, ExExecuteError> {
    Ok(document.physical_first_nonblank_source(line)?)
}

fn source_deletion_cursor(
    document: &Document,
    range: &Range<usize>,
    deletion: &Range<usize>,
) -> Result<usize, ExExecuteError> {
    if range.end < document.source_byte_len() {
        let next = document.physical_line_index_at_source(range.end)?;
        return Ok(source_first_nonblank(document, next)?.saturating_sub(deletion.len()));
    }
    let first = document.physical_line_index_at_source(range.start)?;
    if first > 0 {
        source_first_nonblank(document, first - 1)
    } else {
        Ok(document.physical_line_ranges(0)?.0.start)
    }
}

fn prepare_join(
    document: &Document,
    context: &ExExecutionContext,
    range: &ResolvedExRange,
    bang: bool,
    plan: &mut ExPlan,
) -> Result<(), ExExecuteError> {
    let profile = (document.format().is_code() && context.reflow_options.has('j')).then(|| context.reflow_options.profile());
    let snapshot = document.hard_line_snapshot();
    let mut comment_join = profile.as_ref().map(|profile|
        crate::document::CommentJoinContext::new(&snapshot, profile, context.reflow_options.language()));
    if let Some(source) = source_range(document, range)? {
        let units = source_units(document, source.clone())?;
        let mut patches = Vec::new();
        for (pair_index, pair) in units.windows(2).enumerate() {
            if pair[0].separator.is_empty() {
                continue;
            }
            let following = &pair[1];
            let leading = if bang {
                0
            } else if let Some(comment_join) = &mut comment_join {
                comment_join.prefix(range.start.index + pair_index, &pair[0].text, &following.text)
            } else {
                following.text.len() - following.text.trim_start_matches([' ', '\t']).len()
            };
            let end = *following
                .boundaries
                .get(&leading)
                .ok_or(ExExecuteError::AddressOverflow)?;
            let separator = if bang { "" } else { crate::command::options::join_separator(
                pair[0].text.chars().last(), following.text[leading..].chars().next(), context.vim_options.flag("joinspaces")) };
            patches.push(SourcePatch::primary(
                pair[0].separator.start..end,
                document.encoding().encode_fragment(separator)?,
            ));
        }
        stage_source(document, patches, plan)?;
        let first = units.first().ok_or(ExExecuteError::AddressOverflow)?;
        let cursor = first.boundaries[&crate::document::leading_whitespace_len(&first.text)];
        plan.outcome.navigation = Some(ExNavigation::PhysicalSourceOffset(cursor));
    } else {
        let units = formatted_units(document, range)?;
        let mut edits = Vec::new();
        for pair in units.windows(2) {
            if pair[0].content.end == pair[0].text.end {
                continue;
            }
            let following = snapshot
                .slice_utf8(pair[1].content.clone())
                .map_err(|_| ExExecuteError::AddressOverflow)?;
            let before = snapshot
                .slice_utf8(pair[0].content.clone())
                .map_err(|_| ExExecuteError::AddressOverflow)?;
            let leading = if bang { 0 }
                else if let Some(comment_join) = &mut comment_join { comment_join.prefix(snapshot.line_at_offset(pair[0].content.start).map_err(|_| ExExecuteError::AddressOverflow)?.index(), &before, &following) }
                else { following.len() - following.trim_start_matches([' ', '\t']).len() };
            let separator = if bang { "" } else { crate::command::options::join_separator(
                before.chars().last(), following[leading..].chars().next(), context.vim_options.flag("joinspaces")) };
            edits.push(TextEdit::new(
                pair[0].content.end..pair[1].content.start + leading,
                separator,
            ));
        }
        if !edits.is_empty() {
            plan.stage_text_edits(document, edits);
        }
        plan.outcome.navigation = Some(ExNavigation::TextOffset(range.text.start));
    }
    Ok(())
}

fn destination<'a>(addresses: &'a ResolvedExAddresses) -> Option<&'a ResolvedExLine> {
    addresses
        .destination
        .as_ref()
        .or_else(|| addresses.range.as_ref().map(|range| &range.end))
}

fn source_destination(
    document: &Document,
    addresses: &ResolvedExAddresses,
    before: bool,
) -> Result<usize, ExExecuteError> {
    if addresses.destination_before_first {
        return Ok(document.physical_line_ranges(0)?.0.start);
    }
    let line = destination(addresses).ok_or(ExExecuteError::AddressOverflow)?;
    if let Some(source) = &line.source {
        return Ok(if before { source.start } else { source.end });
    }
    let at = if before {
        line.text.start
    } else {
        line.text.end
    };
    let downstream = before;
    let source = document
        .projection()
        .source_insertion_point(at, downstream)
        .ok_or(DocumentError::AmbiguousProjection)?;
    let affinity = if downstream {
        BoundaryAffinity::Downstream
    } else {
        BoundaryAffinity::Upstream
    };
    let mapped = document
        .projection()
        .map_source_boundary(document.revision(), source, affinity)
        .map_err(|_| DocumentError::AmbiguousProjection)?;
    if mapped.formatted_offset != at {
        return Err(DocumentError::AmbiguousProjection.into());
    }
    Ok(source)
}

fn prepare_put(
    document: &Document,
    addresses: &ResolvedExAddresses,
    before: bool,
    value: &ExRegisterValue,
    plan: &mut ExPlan,
) -> Result<(), ExExecuteError> {
    let physical = addresses.destination_before_first
        || destination(addresses).is_some_and(|line| line.domain == ExLineDomain::PhysicalSource);
    if physical {
        let at = source_destination(document, addresses, before)?;
        let lines = ex_register_logical_lines(value);
        let ending = document
            .encoding()
            .encode_fragment(document.file_format().spelling())?;
        let mut bytes = Vec::new();
        let content_start = document.physical_line_ranges(0)?.0.start;
        if at == document.source_byte_len()
            && at != content_start
            && !source_has_final_ending(document)?
        {
            bytes.extend_from_slice(&ending);
        }
        let mut last_start = bytes.len();
        for (index, line) in lines.iter().enumerate() {
            last_start = bytes.len();
            bytes.extend(document.encoding().encode_fragment(line)?);
            if index + 1 < lines.len()
                || at < document.source_byte_len()
                || source_has_final_ending(document)?
            {
                bytes.extend_from_slice(&ending);
            }
        }
        let indent = lines
            .last()
            .map_or(0, |line| crate::document::leading_whitespace_len(line));
        let last_indent = document
            .encoding()
            .encode_fragment(&lines.last().unwrap()[..indent])?
            .len();
        stage_source(document, vec![SourcePatch::primary(at..at, bytes)], plan)?;
        plan.outcome.navigation = Some(ExNavigation::PhysicalSourceOffset(
            at + last_start + last_indent,
        ));
    } else if let Some(line) = destination(addresses) {
        let snapshot = document.hard_line_snapshot();
        if let Some(hard) = exact_hard_lines(
            document,
            &ResolvedExRange {
                document: document.id(),
                revision: document.revision(),
                start: line.clone(),
                end: line.clone(),
                text: line.text.clone(),
                source: None,
                lines: vec![line.clone()],
            },
        )? {
            let index = if before { hard.start } else { hard.end + 1 };
            let (edit, cursor) = plan_put(document, index, value)?;
            plan.stage_formatted_payload_edits(document, vec![edit]);
            plan.outcome.navigation = Some(ExNavigation::TextOffset(cursor));
        } else {
            let at = if before {
                line.text.start
            } else {
                line.text.end
            };
            let lines = ex_register_logical_lines(value);
            let (mut text, mut breaks) = render_logical_lines(&lines)?;
            let last_nonblank = breaks.last().map_or(0, |offset| offset + 1)
                + crate::document::leading_whitespace_len(lines.last().unwrap());
            let hard = snapshot
                .line_at_offset(at)
                .map_err(|_| ExExecuteError::AddressOverflow)?;
            let hard_start = hard.content_range().start;
            let prefix =
                usize::from(at != hard_start && (before || line.content.end == line.text.end));
            if prefix != 0 {
                text.insert(0, '\n');
                for offset in &mut breaks {
                    *offset += 1;
                }
                breaks.insert(0, 0);
            }
            breaks.push(text.len());
            text.push('\n');
            let payload = FormattedTextPayload::new(&snapshot, text, breaks)?;
            plan.stage_formatted_payload_edits(
                document,
                vec![FormattedPayloadEdit::new(at..at, payload)],
            );
            plan.outcome.navigation = Some(ExNavigation::TextOffset(at + prefix + last_nonblank));
        }
    } else {
        return Err(ExExecuteError::AddressOverflow);
    }
    Ok(())
}

fn source_has_final_ending(document: &Document) -> Result<bool, ExExecuteError> {
    let last = document
        .physical_source_line_number_count()
        .saturating_sub(1);
    let (source, content) = document.physical_line_ranges(last)?;
    Ok(content.end < source.end)
}

fn prepare_transfer(
    document: &Document,
    range: &ResolvedExRange,
    addresses: &ResolvedExAddresses,
    moving: bool,
    plan: &mut ExPlan,
) -> Result<(), ExExecuteError> {
    let operation = if moving {
        HardLineTransfer::Move
    } else {
        HardLineTransfer::Copy
    };
    let source = source_range(document, range)?;
    if let Some(source) = &source {
        verify_mixed_fragment(document, range, source)?;
    }
    let physical_destination = addresses.destination_before_first
        || addresses
            .destination
            .as_ref()
            .is_some_and(|line| line.domain == ExLineDomain::PhysicalSource);
    if source.is_none() && (!physical_destination || addresses.destination_before_first) {
        if let Some(lines) = exact_hard_lines(document, range)? {
            let insertion_line = if addresses.destination_before_first {
                Some(0)
            } else if let Some(line) = addresses.destination.as_ref() {
                let single = ResolvedExRange {
                    document: document.id(),
                    revision: document.revision(),
                    start: line.clone(),
                    end: line.clone(),
                    text: line.text.clone(),
                    source: line.source.clone(),
                    lines: vec![line.clone()],
                };
                exact_hard_lines(document, &single)?.map(|lines| lines.end + 1)
            } else {
                None
            };
            if let Some(destination) = insertion_line {
                let source = lines.start..lines.end + 1;
                let cursor =
                    transfer_cursor_offset(document, operation, source.clone(), destination)?;
                plan.stage_hard_line_transfer(document, operation, source, destination);
                plan.outcome.navigation = Some(ExNavigation::TextOffset(cursor));
                return Ok(());
            }
        }
    }
    if let Some(source) = source {
        let at = source_destination(document, addresses, false)?;
        if moving && source.start < at && at < source.end {
            return Err(ExExecuteError::DestinationInsideRange);
        }
        let units = source_units(document, source.clone())?;
        if moving && (at == source.start || at == source.end) {
            let last = units.last().ok_or(ExExecuteError::AddressOverflow)?;
            let cursor = last.boundaries[&crate::document::leading_whitespace_len(&last.text)];
            plan.outcome.navigation = Some(ExNavigation::PhysicalSourceOffset(cursor));
            return Ok(());
        }
        let mut bytes = document.source_bytes_in(source.clone())?;
        let mut moved_source = source.clone();
        let ending = document
            .encoding()
            .encode_fragment(document.file_format().spelling())?;
        let first_content = document.physical_line_ranges(0)?.0.start;
        let mut prefix = 0;
        if at == document.source_byte_len()
            && at != first_content
            && !source_has_final_ending(document)?
        {
            bytes.splice(0..0, ending.clone());
            prefix = ending.len();
        }
        let last = units.last().ok_or(ExExecuteError::AddressOverflow)?;
        if at < document.source_byte_len() && last.separator.is_empty() {
            bytes.extend_from_slice(&ending);
        }
        if at == document.source_byte_len()
            && !source_has_final_ending(document)?
            && !last.separator.is_empty()
        {
            bytes.truncate(bytes.len() - last.separator.len());
            moved_source.end -= last.separator.len();
        }
        let mut moved_destination = at + prefix;
        let mut cursor = at
            + prefix
            + last.content.start.saturating_sub(source.start)
            + last
                .boundaries
                .get(&crate::document::leading_whitespace_len(&last.text))
                .copied()
                .ok_or(ExExecuteError::AddressOverflow)?
                .saturating_sub(last.content.start);
        let mut patches = vec![SourcePatch::primary(at..at, bytes)];
        if moving {
            let deletion = source_deletion_range(document, source.clone())?;
            if deletion.end <= at {
                cursor = cursor.saturating_sub(deletion.len());
                moved_destination = moved_destination.saturating_sub(deletion.len());
            }
            patches.push(SourcePatch::primary(deletion, Vec::new()));
            plan.mutation = ExMutation::Model(ModelRequest::MovePhysicalSource {
                document: document.id(),
                revision: document.revision(),
                patches,
                source: moved_source,
                destination: moved_destination,
            });
        } else {
            stage_source(document, patches, plan)?;
        }
        plan.outcome.navigation = Some(ExNavigation::PhysicalSourceOffset(cursor));
    } else {
        let at = if physical_destination {
            let source = source_destination(document, addresses, false)?;
            let visible = document
                .visible_point_for_source(source, true)
                .map_err(|_| {
                    ExExecuteError::UnsupportedCommand(
                        "This physical destination has no editable formatted boundary.".into(),
                    )
                })?;
            let point = document.text_point(visible)?;
            if !addresses.destination_before_first && document
                .source_point_for_insertion(point, BoundaryAffinity::Downstream)?
                .offset()
                != source
            {
                return Err(ExExecuteError::UnsupportedCommand(
                    "This physical destination is inside hidden formatting. Choose a visible line boundary for this visual transfer.".into(),
                ));
            }
            point.offset()
        } else {
            addresses
                .destination
                .as_ref()
                .ok_or(ExExecuteError::AddressOverflow)?
                .text
                .end
        };
        if moving && range.text.start < at && at < range.text.end {
            return Err(ExExecuteError::DestinationInsideRange);
        }
        if moving && (at == range.text.start || at == range.text.end) {
            return Ok(());
        }
        let snapshot = document.hard_line_snapshot();
        let hard = snapshot
            .line_at_offset(at)
            .map_err(|_| ExExecuteError::AddressOverflow)?;
        let prefix = usize::from(at != hard.content_range().start);
        let separator = FormattedTextPayload::new(&snapshot, "\n", vec![0])?;
        let mut fragments = Vec::new();
        if prefix != 0 {
            fragments.push(ReplacementFragment::Literal(separator.clone()));
        }
        fragments.push(ReplacementFragment::Capture(range.text.clone()));
        if range.end.content.end == range.end.text.end {
            fragments.push(ReplacementFragment::Literal(separator));
        }
        let mut edits = vec![FragmentEdit {
            range: at..at,
            fragments,
        }];
        if moving {
            edits.push(FragmentEdit {
                range: range.text.clone(),
                fragments: Vec::new(),
            });
        }
        stage_fragments(document, edits, plan);
        let last_text = snapshot
            .slice_utf8(range.end.content.clone())
            .map_err(|_| ExExecuteError::AddressOverflow)?;
        let cursor = at + prefix + range.end.content.start - range.text.start
            + crate::document::leading_whitespace_len(&last_text)
            - if moving && range.text.end <= at {
                range.text.len()
            } else {
                0
            };
        plan.outcome.navigation = Some(ExNavigation::TextOffset(cursor));
    }
    Ok(())
}

fn prepare_sort(
    document: &Document,
    context: &ExExecutionContext,
    range: &ResolvedExRange,
    options: &super::super::ex::SortOptions,
    reverse: bool,
    plan: &mut ExPlan,
) -> Result<(), ExExecuteError> {
    if let Some(source) = source_range(document, range)? {
        let units = source_units(document, source.clone())?;
        // A prefix prevents BOM-looking selected content from becoming the
        // temporary document's artifact BOM. Source bytes are never serialized
        // through this key-only projection.
        let mut bytes = document
            .encoding()
            .encode_fragment(&format!("X{}", document.file_format().spelling()))?;
        bytes.extend(document.source_bytes_in(source.clone())?);
        let scratch = Document::from_bytes_with_file_format(
            bytes,
            document.encoding(),
            crate::document::Format::PlainText,
            document.file_format(),
        )?;
        let snapshot = scratch.hard_line_snapshot();
        let records = (0..units.len())
            .map(|index| {
                snapshot
                    .line(index + 1)
                    .map(|line| (index, line.content_range()))
                    .ok_or(ExExecuteError::AddressOverflow)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let order =
            super::super::sort::ordered_ranges(&snapshot, &records, context, options, reverse)?;
        if order.iter().copied().eq(0..units.len()) {
            return Ok(());
        }
        verify_mixed_fragment(document, range, &source)?;
        let mut replacement = Vec::new();
        let mut relocations = Vec::with_capacity(order.len());
        for (position, index) in order.iter().copied().enumerate() {
            relocations.push((
                units[index].content.clone(),
                source.start + replacement.len(),
            ));
            replacement.extend(document.source_bytes_in(units[index].content.clone())?);
            let separator = if position + 1 == order.len() {
                &units.last().unwrap().separator
            } else {
                &units[position].separator
            };
            replacement.extend(document.source_bytes_in(separator.clone())?);
        }
        plan.mutation = ExMutation::Model(ModelRequest::ReorderPhysicalSource {
            document: document.id(),
            revision: document.revision(),
            patches: vec![SourcePatch::primary(source.clone(), replacement)],
            relocations,
        });
        let first = &units[order[0]];
        let cursor = source.start
            + first.boundaries[&crate::document::leading_whitespace_len(&first.text)]
            - first.content.start;
        plan.outcome.navigation = Some(ExNavigation::PhysicalSourceOffset(cursor));
    } else {
        let units = formatted_units(document, range)?;
        let snapshot = document.hard_line_snapshot();
        let records = units
            .iter()
            .enumerate()
            .map(|(index, line)| (index, line.content.clone()))
            .collect::<Vec<_>>();
        let order =
            super::super::sort::ordered_ranges(&snapshot, &records, context, options, reverse)?;
        if order.iter().copied().eq(0..units.len()) {
            return Ok(());
        }
        let first_text = snapshot
            .slice_utf8(units[order[0]].content.clone())
            .map_err(|_| ExExecuteError::AddressOverflow)?;
        let cursor = range.text.start + crate::document::leading_whitespace_len(&first_text);
        if let Some(lines) = exact_hard_lines(document, range)? {
            plan.mutation = ExMutation::Model(ModelRequest::ReorderHardLines {
                document: document.id(),
                revision: document.revision(),
                source_lines: lines.start..lines.end + 1,
                order: order.iter().map(|index| lines.start + index).collect(),
            });
            plan.outcome.navigation = Some(ExNavigation::TextOffset(cursor));
            return Ok(());
        }
        let mut fragments = Vec::new();
        for (position, index) in order.iter().copied().enumerate() {
            fragments.push(ReplacementFragment::Capture(units[index].content.clone()));
            let separator = if position + 1 == order.len() {
                units.last().unwrap()
            } else {
                &units[position]
            };
            if separator.content.end < separator.text.end {
                fragments.push(ReplacementFragment::Capture(
                    separator.content.end..separator.text.end,
                ));
            }
        }
        stage_fragments(
            document,
            vec![FragmentEdit {
                range: range.text.clone(),
                fragments,
            }],
            plan,
        );
        plan.outcome.navigation = Some(ExNavigation::TextOffset(cursor));
    }
    Ok(())
}

fn verify_mixed_fragment(
    document: &Document,
    range: &ResolvedExRange,
    source: &Range<usize>,
) -> Result<(), ExExecuteError> {
    if document.format().is_markdown()
        && range.start.domain != range.end.domain
        && !document
            .addressed_source_fragment_is_self_contained(range.text.clone(), source.clone())?
    {
        return Err(ExExecuteError::UnsupportedCommand(
            "This mixed physical/visual range cuts through a formatted paragraph. Use physical addresses at both ends, or visual addresses at both ends.".into(),
        ));
    }
    Ok(())
}

enum WhitespaceAction {
    Shift {
        right: bool,
        amount: usize,
    },
    Retab {
        old: crate::document::IndentationOptions,
        new: crate::document::IndentationOptions,
        indent_only: bool,
        bang: bool,
    },
    Align {
        alignment: ExAlignment,
        width: usize,
    },
}

fn whitespace_edits(
    text: &str,
    action: &WhitespaceAction,
    options: crate::document::IndentationOptions,
    round: bool,
) -> Result<Vec<TextEdit>, ExExecuteError> {
    let mut edits = Vec::new();
    match action {
        WhitespaceAction::Shift { right, amount } => {
            let length = crate::document::leading_whitespace_len(text);
            // Vim leaves truly empty lines alone, while whitespace-only lines
            // have real indentation that can be shifted.
            if text.is_empty() {
                return Ok(edits);
            }
            let columns = options.columns(&text[..length]);
            let end = crate::command::options::shifted_columns(columns, options.shift_width(), *amount, *right, round)
                .ok_or(ExExecuteError::AddressOverflow)?;
            let replacement = options
                .try_whitespace(0, end)
                .map_err(|_| ExExecuteError::AddressOverflow)?;
            if replacement != text[..length] {
                edits.push(TextEdit::new(0..length, replacement));
            }
        }
        WhitespaceAction::Retab {
            old,
            new,
            indent_only,
            bang,
        } => {
            let mut at = 0;
            let mut column = 0;
            while at < text.len() {
                let rest = &text[at..];
                if rest.starts_with([' ', '\t']) {
                    let length = rest
                        .bytes()
                        .take_while(|byte| matches!(byte, b' ' | b'\t'))
                        .count();
                    let whitespace = &rest[..length];
                    let end = crate::document::indentation_end_column(
                        column,
                        whitespace,
                        old.tabstop as usize,
                    );
                    if whitespace.contains('\t') || (*bang && length > 1) {
                        let replacement = new
                            .try_whitespace(column, end)
                            .map_err(|_| ExExecuteError::AddressOverflow)?;
                        if replacement != whitespace {
                            edits.push(TextEdit::new(at..at + length, replacement));
                        }
                    }
                    column = end;
                    at += length;
                } else {
                    if *indent_only {
                        break;
                    }
                    let grapheme = rest.graphemes(true).next().unwrap();
                    column = crate::document::indentation_end_column(
                        column,
                        grapheme,
                        old.tabstop as usize,
                    );
                    at += grapheme.len();
                }
            }
        }
        WhitespaceAction::Align { alignment, width } => {
            let leading = crate::document::leading_whitespace_len(text);
            if leading == text.len() {
                if *alignment == ExAlignment::Left && leading > 0 {
                    edits.push(TextEdit::new(0..leading, ""));
                }
                return Ok(edits);
            }
            let body = text[leading..].trim_end_matches([' ', '\t']);
            let indent = match alignment {
                ExAlignment::Left => *width,
                ExAlignment::Center => width.saturating_sub(options.columns(body)) / 2,
                ExAlignment::Right => {
                    let mut low = 0;
                    let mut high = *width;
                    while low < high {
                        let middle = low + (high - low).div_ceil(2);
                        if crate::document::indentation_end_column(
                            middle,
                            body,
                            options.tabstop as usize,
                        ) <= *width
                        {
                            low = middle;
                        } else {
                            high = middle - 1;
                        }
                    }
                    low
                }
            };
            let replacement = options
                .try_whitespace(0, indent)
                .map_err(|_| ExExecuteError::AddressOverflow)?;
            if replacement != text[..leading] {
                edits.push(TextEdit::new(0..leading, replacement));
            }
        }
    }
    Ok(edits)
}

fn prepare_whitespace(
    document: &Document,
    context: &ExExecutionContext,
    range: &ResolvedExRange,
    action: WhitespaceAction,
    plan: &mut ExPlan,
) -> Result<(), ExExecuteError> {
    let options = context.indentation.effective();
    if let Some(source) = source_range(document, range)? {
        let units = source_units(document, source.clone())?;
        let mut patches = Vec::new();
        for unit in &units {
            for edit in whitespace_edits(&unit.text, &action, options, context.vim_options.flag("shiftround"))? {
                let start = *unit
                    .boundaries
                    .get(&edit.range.start)
                    .ok_or(ExExecuteError::AddressOverflow)?;
                let end = *unit
                    .boundaries
                    .get(&edit.range.end)
                    .ok_or(ExExecuteError::AddressOverflow)?;
                patches.push(SourcePatch::primary(
                    start..end,
                    document.encoding().encode_fragment(&edit.replacement)?,
                ));
            }
        }
        let cursor = match action {
            WhitespaceAction::Retab { .. } => None,
            WhitespaceAction::Align { .. } => {
                let at = hard_line_first_nonblank_offset(document, context.current_line)?;
                document.projection().source_insertion_point(at, true)
            }
            WhitespaceAction::Shift { .. } => units
                .last()
                .map(|last| last.boundaries[&crate::document::leading_whitespace_len(&last.text)]),
        };
        if let Some(cursor) = cursor {
            let delta = patches
                .iter()
                .filter(|patch| patch.range().end <= cursor)
                .try_fold(0isize, |delta, patch| {
                    let added = isize::try_from(patch.replacement().len())
                        .map_err(|_| ExExecuteError::AddressOverflow)?;
                    let removed = isize::try_from(patch.range().len())
                        .map_err(|_| ExExecuteError::AddressOverflow)?;
                    delta
                        .checked_add(added)
                        .and_then(|delta| delta.checked_sub(removed))
                        .ok_or(ExExecuteError::AddressOverflow)
                })?;
            plan.outcome.navigation = Some(ExNavigation::PhysicalSourceOffset(
                cursor
                    .checked_add_signed(delta)
                    .ok_or(ExExecuteError::AddressOverflow)?,
            ));
        }
        stage_source(document, patches, plan)?;
    } else {
        let units = formatted_units(document, range)?;
        let snapshot = document.hard_line_snapshot();
        let mut edits = Vec::new();
        for unit in &units {
            let text = snapshot
                .slice_utf8(unit.content.clone())
                .map_err(|_| ExExecuteError::AddressOverflow)?;
            for edit in whitespace_edits(&text, &action, options, context.vim_options.flag("shiftround"))? {
                edits.push(TextEdit::new(
                    unit.content.start + edit.range.start..unit.content.start + edit.range.end,
                    edit.replacement,
                ));
            }
        }
        let cursor = match action {
            WhitespaceAction::Retab { .. } => None,
            WhitespaceAction::Align { .. } => Some(hard_line_first_nonblank_offset(
                document,
                context.current_line,
            )?),
            WhitespaceAction::Shift { .. } => {
                let last = units.last().ok_or(ExExecuteError::AddressOverflow)?;
                let text = snapshot
                    .slice_utf8(last.content.clone())
                    .map_err(|_| ExExecuteError::AddressOverflow)?;
                Some(last.content.start + crate::document::leading_whitespace_len(&text))
            }
        };
        if let Some(cursor) = cursor {
            let delta = edits
                .iter()
                .filter(|edit| edit.range.end <= cursor)
                .try_fold(0isize, |delta, edit| {
                    let added = isize::try_from(edit.replacement.len())
                        .map_err(|_| ExExecuteError::AddressOverflow)?;
                    let removed = isize::try_from(edit.range.len())
                        .map_err(|_| ExExecuteError::AddressOverflow)?;
                    delta
                        .checked_add(added)
                        .and_then(|delta| delta.checked_sub(removed))
                        .ok_or(ExExecuteError::AddressOverflow)
                })?;
            plan.outcome.navigation = Some(ExNavigation::TextOffset(
                cursor
                    .checked_add_signed(delta)
                    .ok_or(ExExecuteError::AddressOverflow)?,
            ));
        }
        if !edits.is_empty() {
            plan.stage_text_edits(document, edits);
        }
    }
    Ok(())
}
