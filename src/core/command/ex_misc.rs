//! Line-oriented Ex commands sharing the document's hard-line and style APIs.
use super::*;
use crate::command::ex::ExAlignment;

pub(super) fn prepare(
    document: &Document,
    context: &ExExecutionContext,
    command: &ExCommand,
    plan: &mut ExPlan,
) -> Result<(), ExExecuteError> {
    match &command.action {
        ExAction::Print {
            count,
            number,
            list,
        } => {
            let range = effective_counted_range(document, context, command.range.as_ref(), *count)?;
            plan.outcome.frontend_requests.push(ExFrontendRequest::Info(
                ExInfoRequest::PrintLines {
                    range,
                    number: *number,
                    list: *list,
                },
            ));
            stage_line_edits(document, range.end, Vec::new(), plan)?;
        }
        ExAction::Shift {
            right,
            amount,
            count,
            print,
            number,
            list,
        } => {
            let lines = effective_counted_range(document, context, command.range.as_ref(), *count)?;
            let range = hard_line_text_range(document, lines)?;
            let operator = if *right {
                super::super::Operator::Indent
            } else {
                super::super::Operator::Outdent
            };
            let edits = super::super::indent_edits(
                &document.hard_line_snapshot(),
                &range,
                operator,
                *amount,
                context.indentation.effective(),
            )
            .map_err(|_| ExExecuteError::AddressOverflow)?;
            stage_line_edits(document, lines.end, edits, plan)?;
            if *print {
                plan.outcome.frontend_requests.push(ExFrontendRequest::Info(
                    ExInfoRequest::PrintLines {
                        range: HardLineRange {
                            start: lines.end,
                            end: lines.end,
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
            let lines = resolve_range(
                Some(command.range.as_ref().unwrap_or(&ExRange::WholeFile)),
                context.current_line,
                document.line_count(),
            )?;
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
            let snapshot = document.hard_line_snapshot();
            let mut edits = Vec::new();
            for line in snapshot
                .lines(lines.start..lines.end + 1)
                .expect("validated Ex line range")
            {
                let range = line.content_range();
                let text = snapshot
                    .slice_utf8(range.clone())
                    .expect("hard-line boundaries are UTF-8 boundaries");
                let mut at = 0;
                let mut column = 0;
                while at < text.len() {
                    let rest = &text[at..];
                    if rest.starts_with([' ', '\t']) {
                        let len = rest
                            .bytes()
                            .take_while(|byte| matches!(byte, b' ' | b'\t'))
                            .count();
                        let whitespace = &rest[..len];
                        let end = crate::document::indentation_end_column(
                            column,
                            whitespace,
                            old.tabstop as usize,
                        );
                        if whitespace.contains('\t') || (command.bang && len > 1) {
                            let replacement = new
                                .try_whitespace(column, end)
                                .map_err(|_| ExExecuteError::AddressOverflow)?;
                            if replacement != whitespace {
                                edits.push(TextEdit::new(
                                    range.start + at..range.start + at + len,
                                    replacement,
                                ));
                            }
                        }
                        column = end;
                        at += len;
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
            if !edits.is_empty() {
                plan.stage_text_edits(document, edits);
            }
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
            let lines = resolve_range(
                command.range.as_ref(),
                context.current_line,
                document.line_count(),
            )?;
            if document.format().is_wysiwyg() {
                return Err(ExExecuteError::UnsupportedCommand("Paragraph alignment is not supported in formatted Markdown; switch to Markdown Source for whitespace alignment".into()));
            } else {
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
                let options = context.indentation.effective();
                let snapshot = document.hard_line_snapshot();
                let mut edits = Vec::new();
                for line in snapshot
                    .lines(lines.start..lines.end + 1)
                    .expect("validated Ex line range")
                {
                    let range = line.content_range();
                    let text = snapshot
                        .slice_utf8(range.clone())
                        .expect("hard-line boundaries are UTF-8 boundaries");
                    let whitespace = crate::document::leading_whitespace_len(&text);
                    if whitespace == text.len() {
                        if *alignment == ExAlignment::Left && whitespace > 0 {
                            edits.push(TextEdit::new(range.clone(), ""));
                        }
                        continue;
                    }
                    let body = text[whitespace..].trim_end_matches([' ', '\t']);
                    let indent = match alignment {
                        ExAlignment::Left => width,
                        ExAlignment::Center => width.saturating_sub(options.columns(body)) / 2,
                        ExAlignment::Right => {
                            // Tabs in the body depend on the starting column. Find the
                            // rightmost indent that still fits, without allocating padding.
                            let mut low = 0;
                            let mut high = width;
                            while low < high {
                                let middle = low + (high - low).div_ceil(2);
                                if crate::document::indentation_end_column(
                                    middle,
                                    body,
                                    options.tabstop as usize,
                                ) <= width
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
                    if replacement != text[..whitespace] {
                        edits.push(TextEdit::new(
                            range.start..range.start + whitespace,
                            replacement,
                        ));
                    }
                }
                stage_line_edits(document, context.current_line, edits, plan)?;
            }
        }
        _ => unreachable!("only miscellaneous line commands are dispatched here"),
    }
    Ok(())
}

fn stage_line_edits(
    document: &Document,
    cursor_line: usize,
    edits: Vec<TextEdit>,
    plan: &mut ExPlan,
) -> Result<(), ExExecuteError> {
    let snapshot = document.hard_line_snapshot();
    let range = snapshot.line(cursor_line).unwrap().content_range();
    let text = snapshot
        .slice_utf8(range.clone())
        .expect("hard-line boundaries are UTF-8 boundaries");
    let mut cursor = range.start + crate::document::leading_whitespace_len(&text);
    for edit in edits.iter().rev() {
        if edit.range.end <= cursor {
            cursor = cursor - (edit.range.end - edit.range.start) + edit.replacement.len();
        }
    }
    if !edits.is_empty() {
        plan.stage_text_edits(document, edits);
    }
    plan.outcome.navigation = Some(ExNavigation::TextOffset(cursor));
    Ok(())
}
