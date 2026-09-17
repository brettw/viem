//! Resolve session-dependent Ex addresses before document-level planning.
use super::ex::{AddressBase, ExAction, ExAddress, ExCommand, ExRange, RangeSeparator};
use super::ex_execute::{resolve_address, ExExecuteError};
use super::search_regex::{CompiledRegex, RegexInput, RegexLimits, RegexWork};
use super::{CommandInterpreter, SearchDirection};
use crate::document::Document;

impl CommandInterpreter {
    /// Bind marks and pattern addresses against one exact source projection.
    /// The returned search state is published only if the command succeeds.
    pub(super) fn bind_ex_addresses(
        &self,
        document: &Document,
        command: &mut ExCommand,
    ) -> Result<Option<(SearchDirection, String)>, ExExecuteError> {
        self.bind_ex_addresses_with_limits(document, command, RegexLimits::default())
    }

    pub(super) fn bind_ex_addresses_with_limits(
        &self,
        document: &Document,
        command: &mut ExCommand,
        limits: RegexLimits,
    ) -> Result<Option<(SearchDirection, String)>, ExExecuteError> {
        let mut work = RegexWork::new(limits);
        let current = document
            .hard_line_at_offset(self.cursor)
            .ok_or(ExExecuteError::AddressOverflow)?;
        let mut search = self.last_search.clone();
        let mut changed = false;
        let mut bind = |address: &mut ExAddress, current| {
            self.bind_ex_address(
                document,
                address,
                current,
                &mut search,
                &mut changed,
                limits,
                &mut work,
            )
        };
        match command.range.as_mut() {
            Some(ExRange::Single(address)) => bind(address, current)?,
            Some(ExRange::Between {
                start,
                end,
                separator,
            }) => {
                bind(start, current)?;
                let current = if *separator == RangeSeparator::Semicolon {
                    resolve_address(start.clone(), current, document.line_count())?
                } else {
                    current
                };
                bind(end, current)?;
            }
            _ => {}
        }
        match &mut command.action {
            ExAction::Copy { destination } | ExAction::Move { destination } => {
                bind(destination, current)?
            }
            ExAction::GoToLine(address) => bind(address, current)?,
            _ => {}
        }
        Ok(if changed { search } else { None })
    }

    fn bind_ex_address(
        &self,
        document: &Document,
        address: &mut ExAddress,
        current: usize,
        search: &mut Option<(SearchDirection, String)>,
        changed: &mut bool,
        limits: RegexLimits,
        work: &mut RegexWork,
    ) -> Result<(), ExExecuteError> {
        let line = match &address.base {
            AddressBase::Mark(name) => {
                let offset = self
                    .marks
                    .get(name)
                    .copied()
                    .ok_or(ExExecuteError::MarkNotSet(*name))?;
                document
                    .hard_line_at_offset(offset)
                    .ok_or(ExExecuteError::MarkNotSet(*name))?
            }
            AddressBase::Search { pattern, forward } => {
                let pattern = if pattern.is_empty() {
                    search
                        .as_ref()
                        .map(|(_, pattern)| pattern.clone())
                        .ok_or(ExExecuteError::NoPreviousSearch)?
                } else {
                    pattern.clone()
                };
                let regex = CompiledRegex::compile(
                    &pattern,
                    self.search_options.case_insensitive(&pattern)?,
                    limits,
                )?;
                let snapshot = document.hard_line_snapshot();
                let input = RegexInput::new(&snapshot);
                let count = snapshot.line_count();
                let mut scan =
                    |from: usize, to: usize, last: bool| -> Result<Option<usize>, ExExecuteError> {
                        let mut next = from;
                        let mut found = None;
                        while next < to {
                            let start = snapshot
                                .line(next)
                                .ok_or(ExExecuteError::AddressOverflow)?
                                .content_range()
                                .start;
                            let Some(matched) =
                                regex.find(&input, start, snapshot.text_length(), work)?
                            else {
                                break;
                            };
                            let line = snapshot
                                .line_at_offset(matched.range().start)
                                .map_err(|_| ExExecuteError::AddressOverflow)?
                                .index();
                            if line >= to {
                                break;
                            }
                            found = Some(line);
                            if !last {
                                break;
                            }
                            next = line + 1;
                        }
                        Ok(found)
                    };
                // Pattern addresses start beyond the current hard line. Scan
                // each side once, preserving full-document assertion context.
                let mut found = if *forward {
                    scan(current + 1, count, false)?
                } else {
                    scan(0, current, true)?
                };
                if found.is_none() && self.search_options.wrapscan {
                    found = if *forward {
                        scan(0, current + 1, false)?
                    } else {
                        scan(current, count, true)?
                    };
                }
                let found =
                    found.ok_or_else(|| ExExecuteError::PatternNotFound(pattern.clone()))?;
                *search = Some((
                    if *forward {
                        SearchDirection::Forward
                    } else {
                        SearchDirection::Backward
                    },
                    pattern,
                ));
                *changed = true;
                found
            }
            AddressBase::Absolute(_) | AddressBase::Current | AddressBase::Last => return Ok(()),
        };
        address.base = AddressBase::Absolute(
            u64::try_from(line + 1).map_err(|_| ExExecuteError::AddressOverflow)?,
        );
        Ok(())
    }
}
