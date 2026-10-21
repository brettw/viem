//! Resolve session-dependent Ex addresses before document-level planning.
use super::ex::{AddressBase, ExAction, ExAddress, ExCommand, ExRange, RangeSeparator};
use super::ex_execute::ExExecuteError;
use super::search_regex::{CompiledRegex, RegexInput, RegexLimits, RegexWork};
use super::{CommandInterpreter, SearchDirection};
use crate::document::Document;

use super::{layout_motion, LayoutMotionError, LineMode, VisualPosition};
use crate::document::{BoundaryAffinity, DocumentId, Revision};
use std::ops::Range;
use std::cell::RefCell;

/// The units of an address belong to its base, not to its spelling after
/// resolution. In particular, `23+2` must never become a visual-row offset.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExLineDomain {
    PhysicalSource,
    Visual,
    /// A document-only caller has no display geometry. Contextual addresses
    /// retain the formatted hard-line fallback in that explicit environment.
    Formatted,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedExLine {
    pub domain: ExLineDomain,
    /// Physical/formatted ordinal, or a snapshot-local visual-row index.
    pub index: usize,
    pub content: Range<usize>,
    /// Includes a real following hard break, but never invents a soft break.
    pub text: Range<usize>,
    pub source: Option<Range<usize>>,
    pub source_content: Option<Range<usize>>,
    pub position: VisualPosition,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedExRange {
    pub document: DocumentId,
    pub revision: Revision,
    pub start: ResolvedExLine,
    pub end: ResolvedExLine,
    pub text: Range<usize>,
    /// Present only when both endpoints address physical source lines.
    /// A mixed range must not manufacture a source hull for a visual endpoint.
    pub source: Option<Range<usize>>,
    /// Exact local displayed units. Physical ranges stay indexed and lazy.
    pub lines: Vec<ResolvedExLine>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedExAddresses {
    pub document: DocumentId,
    pub revision: Revision,
    pub range: Option<ResolvedExRange>,
    /// `None` is the explicit boundary before the first line (`0`).
    pub destination: Option<ResolvedExLine>,
    pub destination_before_first: bool,
    pub navigation: Option<ResolvedExLine>,
    pub(super) search: Option<(SearchDirection, String)>,
}

/// Disposable preparation geometry, never success-dependent command state.
/// A bounded endpoint set lets distant range edges resolve in separate bands.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ExAddressEndpointCache {
    command: ExCommand,
    document: DocumentId,
    revision: Revision,
    origin: VisualPosition,
    configuration: crate::layout::ViewConfigurationGeneration,
    environment: crate::layout::MeasurementEnvironmentId,
    metrics: crate::layout::MetricsGeneration,
    units: Vec<(VisualPosition, ResolvedExLine)>,
}

#[derive(Debug)]
pub(super) enum ExAddressResolutionError {
    Execute(ExExecuteError),
    Layout(LayoutMotionError),
}

impl From<ExExecuteError> for ExAddressResolutionError {
    fn from(value: ExExecuteError) -> Self { Self::Execute(value) }
}
impl From<crate::document::DocumentError> for ExAddressResolutionError {
    fn from(value: crate::document::DocumentError) -> Self {
        Self::Execute(ExExecuteError::Document(value))
    }
}
impl From<LayoutMotionError> for ExAddressResolutionError {
    fn from(value: LayoutMotionError) -> Self { Self::Layout(value) }
}

impl CommandInterpreter {
    /// Bind addresses to exact source/display units without changing the parsed
    /// syntax or publishing search history. No global visual ordinal is needed.
    pub(super) fn resolve_ex_addresses(
        &self,
        document: &Document,
        command: &ExCommand,
        limits: RegexLimits,
    ) -> Result<ResolvedExAddresses, ExAddressResolutionError> {
        AddressResolver::new(self, document, limits).resolve(command)
    }

    pub(super) fn resolve_ex_addresses_with_layout(
        &self, document: &Document, command: &ExCommand,
        layout: Option<&crate::layout::LayoutSnapshot>, limits: RegexLimits,
    ) -> Result<ResolvedExAddresses, ExAddressResolutionError> {
        let mut resolver = AddressResolver::new(self, document, limits);
        resolver.layout = layout;
        resolver.retained_visual_range = self.command_line_state.as_ref()
            .and_then(|state| state.retained_visual_range.as_ref());
        resolver.resolve(command)
    }

    pub(super) fn prepare_ex_addresses_with_visual_range(
        &mut self, document: &Document, command: &ExCommand,
        retained: Option<&ResolvedExRange>, limits: RegexLimits,
    ) -> Result<ResolvedExAddresses, ExAddressResolutionError> {
        let mut resolver = AddressResolver::new(self, document, limits);
        resolver.retained_visual_range = retained;
        resolver.endpoints = RefCell::new(self.ex_address_endpoints.clone().filter(|cache|
            resolver.layout.is_some_and(|snapshot| cache.command == *command
                && cache.document == document.id() && cache.revision == document.revision()
                && cache.origin == resolver.raw_position()
                && cache.configuration == snapshot.configuration_generation
                && cache.environment == snapshot.measurement_environment_id
                && cache.metrics == snapshot.metrics_generation)));
        if resolver.endpoints.borrow().is_none() && self.line_mode == LineMode::Visual {
            if let Some(snapshot) = resolver.layout {
                *resolver.endpoints.borrow_mut() = Some(ExAddressEndpointCache {
                    command: command.clone(), document: document.id(), revision: document.revision(),
                    origin: resolver.raw_position(), configuration: snapshot.configuration_generation,
                    environment: snapshot.measurement_environment_id, metrics: snapshot.metrics_generation,
                    units: Vec::new(),
                });
            }
        }
        let result = resolver.resolve(command);
        let prepared = resolver.endpoints.into_inner();
        self.ex_address_endpoints = if matches!(result, Err(ExAddressResolutionError::Layout(
            LayoutMotionError::OutsideMaterializedCoverage(_)))) { prepared } else { None };
        result
    }

    pub(super) fn capture_visual_ex_range(&self, document: &Document,
        anchor: VisualPosition, active: VisualPosition)
        -> Result<ResolvedExRange, ExAddressResolutionError> {
        let resolver = AddressResolver::new(self, document, RegexLimits::default());
        let first = resolver.contextual(anchor)?;
        let last = resolver.contextual(active)?;
        if first.text.start <= last.text.start { resolver.between(first, last) }
        else { resolver.between(last, first) }
    }

}

fn retained_visual_addresses(range: Option<&ExRange>) -> bool {
    matches!(range, Some(ExRange::Between { start: ExAddress {
        base: AddressBase::Mark('<'), offset: 0 }, end: ExAddress {
        base: AddressBase::Mark('>'), offset: 0 }, .. }))
}

/// Layout preflight follows the resolver's address consumers. Absolute line
/// bases and whole-document ranges stay independent of display geometry.
pub(super) fn ex_command_requires_visual_layout(command: &ExCommand, nested_global: bool) -> bool {
    let contextual = |address: &ExAddress| !matches!(address.base, AddressBase::Absolute(_));
    let contextual_range = match command.range.as_ref() {
        Some(ExRange::WholeFile) => false,
        Some(ExRange::Single(address)) => contextual(address),
        Some(ExRange::Between { start, end, .. }) => contextual(start) || contextual(end),
        None => !(matches!(command.action, ExAction::Sort(_) | ExAction::Retab { .. }
            | ExAction::Global { .. }) && !nested_global),
    };
    match &command.action {
        ExAction::GoToLine(address) => contextual(address),
        ExAction::Copy { destination } | ExAction::Move { destination } =>
            contextual_range || contextual(destination),
        ExAction::Write { .. } | ExAction::WriteQuit { .. } =>
            command.range.is_some() && contextual_range,
        ExAction::Delete(_) | ExAction::Yank(_) | ExAction::Put { .. }
        | ExAction::Join { .. } | ExAction::Sort(_) | ExAction::Normal { .. }
        | ExAction::Substitute(_) | ExAction::RepeatSubstitute { .. }
        | ExAction::Print { .. } | ExAction::Shift { .. } | ExAction::Retab { .. }
        | ExAction::Align { .. } | ExAction::Global { .. } | ExAction::Read { .. } => contextual_range,
        _ => false,
    }
}

/// Direct document planners have no view snapshot. They still interpret
/// numeric addresses physically, while contextual addresses use the explicitly
/// documented formatted-line fallback.
pub fn resolve_document_ex_addresses(
    document: &Document,
    command: &ExCommand,
    current_line: usize,
    search_options: super::search_regex::SearchOptions,
    last_search_pattern: Option<&str>,
) -> Result<ResolvedExAddresses, ExExecuteError> {
    let mut interpreter = CommandInterpreter::new();
    interpreter.cursor = document.hard_line_snapshot().line(current_line)
        .ok_or(ExExecuteError::CurrentLineOutOfBounds {
            current: current_line, line_count: document.line_count(),
        })?.content_range().start;
    interpreter.search_options = search_options;
    interpreter.last_search = last_search_pattern
        .map(|pattern| (SearchDirection::Forward, pattern.to_owned()));
    match interpreter.resolve_ex_addresses(document, command, RegexLimits::default()) {
        Ok(resolved) => Ok(resolved),
        Err(ExAddressResolutionError::Execute(error)) => Err(error),
        Err(ExAddressResolutionError::Layout(_)) => unreachable!("document-only resolver has no layout"),
    }
}

struct AddressResolver<'a> {
    interpreter: &'a CommandInterpreter,
    document: &'a Document,
    layout: Option<&'a crate::layout::LayoutSnapshot>,
    limits: RegexLimits,
    work: RegexWork,
    search: Option<(SearchDirection, String)>,
    search_changed: bool,
    retained_visual_range: Option<&'a ResolvedExRange>,
    endpoints: RefCell<Option<ExAddressEndpointCache>>,
}

impl<'a> AddressResolver<'a> {
    fn new(interpreter: &'a CommandInterpreter, document: &'a Document, limits: RegexLimits) -> Self {
        Self {
            interpreter, document, layout: interpreter.line_layout.as_ref(), limits, work: RegexWork::new(limits),
            search: interpreter.last_search.clone(), search_changed: false,
            retained_visual_range: None,
            endpoints: RefCell::new(None),
        }
    }

    fn resolve(&mut self, command: &ExCommand) -> Result<ResolvedExAddresses, ExAddressResolutionError> {
        let mut result = ResolvedExAddresses {
            document: self.document.id(), revision: self.document.revision(),
            range: None, destination: None, destination_before_first: false,
            navigation: None, search: None,
        };
        let uses_range = matches!(&command.action,
            ExAction::Delete(_) | ExAction::Yank(_) | ExAction::Put { .. }
            | ExAction::Join { .. } | ExAction::Copy { .. } | ExAction::Move { .. }
            | ExAction::Sort(_) | ExAction::Normal { .. } | ExAction::Substitute(_)
            | ExAction::RepeatSubstitute { .. } | ExAction::Print { .. }
            | ExAction::Shift { .. } | ExAction::Retab { .. } | ExAction::Align { .. }
            | ExAction::Global { .. } | ExAction::Read { .. });
        // File/pane/byte-command pseudo-ranges are not line addresses.
        let uses_optional_range = matches!(&command.action,
            ExAction::Write { .. } | ExAction::WriteQuit { .. });
        if uses_range || (uses_optional_range && command.range.is_some()) {
            let zero_allowed = matches!(&command.action, ExAction::Put { .. } | ExAction::Read { .. });
            let zero_destination = if zero_allowed {
                if let Some(ExRange::Single(address)) = &command.range {
                    let current = if matches!(address.base, AddressBase::Absolute(_)) { self.raw_position() }
                        else { self.current_position()? };
                    let (destination, before_first) = self.destination_address(address, current)?;
                    if before_first { result.destination_before_first = true; }
                    if let Some(line) = &destination {
                        result.range = Some(self.between(line.clone(), line.clone())?);
                    }
                    result.destination = destination;
                    before_first
                } else { false }
            } else { false };
            if !zero_destination && result.range.is_none() {
                let whole_default = matches!(&command.action,
                    ExAction::Sort(_) | ExAction::Retab { .. } | ExAction::Global { .. })
                    && self.interpreter.global_replay_depth == 0;
                let range = if retained_visual_addresses(command.range.as_ref())
                    && self.retained_visual_range.is_some() {
                    let range = self.retained_visual_range.unwrap();
                    self.validate_range(range)?;
                    range.clone()
                } else if command.range.is_none() && whole_default {
                    self.range(Some(&ExRange::WholeFile))?
                } else {
                    self.range(command.range.as_ref())?
                };
                result.range = Some(self.apply_command_count(command, range)?);
            }
        }
        match &command.action {
            ExAction::Copy { destination } | ExAction::Move { destination } => {
                let current = if matches!(destination.base, AddressBase::Absolute(_)) { self.raw_position() }
                    else { self.current_position()? };
                let (destination, before_first) = self.destination_address(destination, current)?;
                result.destination = destination;
                result.destination_before_first = before_first;
            }
            ExAction::GoToLine(address) => {
                let current = if matches!(address.base, AddressBase::Absolute(_)) {
                    self.raw_position()
                } else { self.current_position()? };
                result.navigation = Some(match self.address(address, current, None) {
                    Err(ExAddressResolutionError::Execute(ExExecuteError::AddressOutOfBounds {
                        value, line_count, ..
                    })) if matches!(address.base, AddressBase::Absolute(_)) && value > line_count as i128 => {
                        self.physical(self.physical_count().saturating_sub(1))?
                    }
                    result => result?,
                });
            }
            _ => {}
        }
        if matches!(&command.action, ExAction::Normal { .. } | ExAction::Global { .. }) {
            if let Some(range) = &mut result.range {
                if range.source.is_none() { range.lines = self.range_units(range)?; }
            }
        }
        if self.search_changed { result.search = self.search.clone(); }
        Ok(result)
    }

    fn current_position(&self) -> Result<VisualPosition, ExAddressResolutionError> {
        if self.interpreter.line_mode == LineMode::Visual {
            if let Some(snapshot) = &self.layout {
            self.validate_snapshot(snapshot)?;
            return Ok(self.interpreter.current_visual_position(snapshot).unwrap_or(self.raw_position()));
            }
        }
        Ok(self.raw_position())
    }

    fn destination_address(&mut self, address: &ExAddress, current: VisualPosition)
        -> Result<(Option<ResolvedExLine>, bool), ExAddressResolutionError> {
        match self.address(address, current, None) {
            Ok(line) => Ok((Some(line), false)),
            Err(ExAddressResolutionError::Execute(ExExecuteError::AddressOutOfBounds { value: 0, .. }))
                => Ok((None, true)),
            Err(error) => Err(error),
        }
    }

    fn raw_position(&self) -> VisualPosition {
        self.interpreter.visual_position.unwrap_or(VisualPosition {
            text_offset: self.interpreter.cursor, affinity: self.interpreter.boundary_affinity,
        })
    }

    fn validate_snapshot(&self, snapshot: &crate::layout::LayoutSnapshot) -> Result<(), ExAddressResolutionError> {
        if snapshot.document_id != self.document.id() {
            return Err(ExExecuteError::WrongDocument.into());
        }
        if snapshot.document_revision != self.document.revision() {
            return Err(ExExecuteError::StalePlan {
                expected: snapshot.document_revision, actual: self.document.revision(),
            }.into());
        }
        Ok(())
    }

    fn contextual(&self, position: VisualPosition) -> Result<ResolvedExLine, ExAddressResolutionError> {
        let position = if position.text_offset == self.document.projection().text_tree().byte_len()
            && self.document.addressable_hard_line_count() < self.document.line_count()
        {
            let line = self.document.hard_line_snapshot().line(self.document.addressable_hard_line_count() - 1)
                .ok_or(ExExecuteError::AddressOverflow)?;
            VisualPosition { text_offset: line.content_range().end, affinity: BoundaryAffinity::Upstream }
        } else { position };
        if self.interpreter.line_mode == LineMode::PhysicalSource {
            let index = if position.text_offset == self.interpreter.cursor {
                self.interpreter.physical_source_line_index(self.document)?
            } else {
                let source = self.document.projection().source_insertion_point(position.text_offset, true)
                    .ok_or(ExExecuteError::AddressOverflow)?;
                self.document.physical_line_index_at_source(source)?
            };
            return self.physical(index.min(self.physical_count().saturating_sub(1)));
        }
        if let Some(snapshot) = &self.layout {
            self.validate_snapshot(snapshot)?;
            if let Some(line) = self.endpoints.borrow().as_ref().and_then(|cache|
                cache.units.iter().find(|(cached, _)| *cached == position).map(|(_, line)| line.clone())) {
                return Ok(line);
            }
            if !snapshot.coverage.contains_text_offset(position.text_offset) {
                return Err(self.endpoint_demand(snapshot, position.text_offset)?.into());
            }
            let index = match layout_motion::command_row_span(snapshot, position, 0) {
                Ok(span) => span.start,
                Err(LayoutMotionError::PositionNotInLayout(_)) => {
                    let alternate = VisualPosition { text_offset: position.text_offset,
                        affinity: if position.affinity == BoundaryAffinity::Downstream {
                            BoundaryAffinity::Upstream
                        } else { BoundaryAffinity::Downstream } };
                    match layout_motion::command_row_span(snapshot, alternate, 0) {
                        Ok(span) => span.start,
                        Err(LayoutMotionError::PositionNotInLayout(_)) => {
                            // Addresses name logical line ownership. A mark
                            // inside an indivisible shaping cluster still names
                            // its exact containing row without moving the mark.
                            self.document.text_point(position.text_offset)?;
                            snapshot.rows.iter().position(|row| row.text_range.start <= position.text_offset
                                && (position.text_offset < row.text_range.end
                                    || row.text_range.is_empty() && position.text_offset == row.text_range.start))
                                .ok_or(LayoutMotionError::PositionNotInLayout(position))?
                        }
                        Err(error) => return Err(error.into()),
                    }
                }
                Err(error) => return Err(error.into()),
            };
            let line = self.visual(index)?;
            if let Some(cache) = self.endpoints.borrow_mut().as_mut() {
                const ENDPOINT_BUDGET: usize = 8;
                if cache.units.len() == ENDPOINT_BUDGET { cache.units.remove(0); }
                cache.units.push((position, line.clone()));
            }
            return Ok(line);
        }
        let index = self.document.hard_line_at_offset(position.text_offset)
            .ok_or(ExExecuteError::AddressOverflow)?;
        self.formatted(index)
    }

    fn physical_count(&self) -> usize { self.document.physical_source_line_number_count() }

    fn presentation_line(&self, at: usize) -> Result<usize, ExAddressResolutionError> {
        self.document.projection().presentation_line_at_offset(at)
            .ok_or_else(|| ExExecuteError::AddressOverflow.into())
    }

    fn endpoint_demand(&self, snapshot: &crate::layout::LayoutSnapshot, at: usize)
        -> Result<LayoutMotionError, ExAddressResolutionError> {
        let line = self.presentation_line(at)?;
        Ok(layout_motion::exact_ex_address_demand(snapshot, line..line + 1, at))
    }

    fn visual_span(&self, snapshot: &crate::layout::LayoutSnapshot, position: VisualPosition,
        delta: isize) -> Result<Range<usize>, ExAddressResolutionError> {
        if !snapshot.coverage.contains_text_offset(position.text_offset) {
            return Err(self.endpoint_demand(snapshot, position.text_offset)?.into());
        }
        match layout_motion::command_row_span(snapshot, position, delta) {
            Err(LayoutMotionError::OutsideMaterializedCoverage(demand)) => {
                let line = self.presentation_line(position.text_offset)?;
                let giant_line = self.document.projection().presentation_line_range(line)
                    .is_some_and(|range| range.len() > crate::layout::MAX_LONG_LINE_LAYOUT_SLICE_BYTES);
                if giant_line {
                    // Wrapped slices must retain their bounded continuation
                    // path; complete geometry for a giant line is unnecessary.
                    return Err(LayoutMotionError::OutsideMaterializedCoverage(demand).into());
                }
                Err(layout_motion::exact_ex_address_demand(snapshot, demand.requested_hard_lines(),
                    position.text_offset).into())
            }
            result => result.map_err(Into::into),
        }
    }

    fn physical(&self, index: usize) -> Result<ResolvedExLine, ExAddressResolutionError> {
        let (source, source_content) = self.document.physical_line_ranges(index)?;
        let start = self.document.visible_point_for_source(source.start, true)?;
        let content_end = self.document.visible_point_for_source(source_content.end, false)?;
        let end = self.document.visible_point_for_source(source.end, false)?;
        Ok(ResolvedExLine {
            domain: ExLineDomain::PhysicalSource, index,
            content: start..content_end.max(start),
            text: start..end.max(start),
            source: Some(source), source_content: Some(source_content),
            position: VisualPosition { text_offset: start, affinity: BoundaryAffinity::Downstream },
        })
    }

    fn formatted(&self, index: usize) -> Result<ResolvedExLine, ExAddressResolutionError> {
        let snapshot = self.document.hard_line_snapshot();
        let line = snapshot.line(index).ok_or(ExExecuteError::AddressOverflow)?;
        let content = line.content_range();
        let end = line.separator_range().map_or(content.end, |separator| separator.end);
        Ok(ResolvedExLine {
            domain: ExLineDomain::Formatted, index, text: content.start..end,
            position: VisualPosition { text_offset: content.start, affinity: BoundaryAffinity::Downstream },
            content, source: None, source_content: None,
        })
    }

    fn visual(&self, index: usize) -> Result<ResolvedExLine, ExAddressResolutionError> {
        let snapshot = self.layout.ok_or(LayoutMotionError::EmptyLayout)?;
        let row = snapshot.rows.get(index).ok_or(LayoutMotionError::EmptyLayout)?;
        if !snapshot.horizontal_text_is_materialized(row.text_range.start)
            || !snapshot.horizontal_text_is_materialized(row.text_range.end) {
            return Err(layout_motion::complete_horizontal_demand(snapshot, row.text_range.start).into());
        }
        let content = row.text_range.clone();
        let end = if !row.wraps_to_next {
            self.document.hard_line_snapshot().line_at_offset(content.end).ok()
                .and_then(|line| line.separator_range())
                .filter(|separator| separator.start == content.end)
                .map_or(content.end, |separator| separator.end)
        } else { content.end };
        Ok(ResolvedExLine {
            domain: ExLineDomain::Visual, index, text: content.start..end,
            position: VisualPosition { text_offset: content.start, affinity: BoundaryAffinity::Downstream },
            content, source: None, source_content: None,
        })
    }

    fn address(&mut self, address: &ExAddress, current: VisualPosition, current_source: Option<usize>) -> Result<ResolvedExLine, ExAddressResolutionError> {
        let contextual_current = |this: &Self| {
            if this.interpreter.line_mode == LineMode::PhysicalSource {
                if let Some(source) = current_source {
                    return this.physical(this.document.physical_line_index_at_source(source)?
                        .min(this.physical_count().saturating_sub(1)));
                }
            }
            this.contextual(current)
        };
        let base = match &address.base {
            AddressBase::Absolute(one_based) => {
                let value = i128::from(*one_based).checked_add(i128::from(address.offset))
                    .ok_or(ExExecuteError::AddressOverflow)?;
                let count = self.physical_count();
                if value < 1 || value > count as i128 {
                    return Err(ExExecuteError::AddressOutOfBounds {
                        value, line_count: count, zero_allowed: false,
                    }.into());
                }
                return self.physical(usize::try_from(value - 1).map_err(|_| ExExecuteError::AddressOverflow)?);
            }
            AddressBase::Current => contextual_current(self)?,
            AddressBase::Last => {
                if self.interpreter.line_mode == LineMode::PhysicalSource {
                    self.physical(self.physical_count().saturating_sub(1))?
                } else if let Some(snapshot) = &self.layout {
                    self.validate_snapshot(snapshot)?;
                    self.contextual(VisualPosition { text_offset: self.document.projection().text_tree().byte_len(),
                        affinity: BoundaryAffinity::Downstream })?
                } else {
                    self.formatted(self.document.addressable_hard_line_count().saturating_sub(1))?
                }
            }
            AddressBase::Mark(name) => {
                let offset = self.interpreter.mark_offset(*name)
                    .ok_or(ExExecuteError::MarkNotSet(*name))?;
                self.contextual(VisualPosition { text_offset: offset,
                    affinity: if *name == '>' { BoundaryAffinity::Upstream } else { BoundaryAffinity::Downstream } })?
            }
            AddressBase::Search { pattern, forward } => {
                let current = contextual_current(self)?;
                let at = self.search_address(pattern, *forward, &current)?;
                self.contextual(VisualPosition { text_offset: at, affinity: BoundaryAffinity::Downstream })?
            }
        };
        self.advance(base, address.offset, false)
    }

    fn advance(&self, line: ResolvedExLine, offset: i64, clamp_end: bool) -> Result<ResolvedExLine, ExAddressResolutionError> {
        if offset == 0 { return Ok(line); }
        match line.domain {
            ExLineDomain::PhysicalSource | ExLineDomain::Formatted => {
                let count = if line.domain == ExLineDomain::PhysicalSource {
                    self.physical_count()
                } else { self.document.addressable_hard_line_count() };
                let value = (line.index as i128 + 1).checked_add(i128::from(offset))
                    .ok_or(ExExecuteError::AddressOverflow)?;
                if value < 1 || (!clamp_end && value > count as i128) {
                    return Err(ExExecuteError::AddressOutOfBounds {
                        value, line_count: count, zero_allowed: false,
                    }.into());
                }
                let index = usize::try_from(value.min(count as i128) - 1)
                    .map_err(|_| ExExecuteError::AddressOverflow)?;
                if line.domain == ExLineDomain::PhysicalSource { self.physical(index) } else { self.formatted(index) }
            }
            ExLineDomain::Visual => {
                let snapshot = self.layout.ok_or(LayoutMotionError::EmptyLayout)?;
                let delta = isize::try_from(offset).map_err(|_| ExExecuteError::AddressOverflow)?;
                let origin = self.visual_span(snapshot, line.position, 0)?.start;
                let rows = self.visual_span(snapshot, line.position, delta)?;
                let mut target = if offset < 0 { rows.start } else { rows.end - 1 };
                if self.document.addressable_hard_line_count() < self.document.line_count()
                    && snapshot.rows[target].text_range == (self.document.projection().text_tree().byte_len()..self.document.projection().text_tree().byte_len())
                {
                    if !clamp_end {
                        return Err(ExExecuteError::AddressOutOfBounds {
                            value: line.index as i128 + 1 + i128::from(offset),
                            line_count: snapshot.rows.len().saturating_sub(1), zero_allowed: false,
                        }.into());
                    }
                    target = target.saturating_sub(1);
                }
                if !clamp_end && target.abs_diff(origin) != offset.unsigned_abs() as usize {
                    return Err(ExExecuteError::AddressOutOfBounds {
                        value: line.index as i128 + 1 + i128::from(offset),
                        line_count: snapshot.rows.len(), zero_allowed: false,
                    }.into());
                }
                self.visual(target)
            }
        }
    }

    fn range(&mut self, range: Option<&ExRange>) -> Result<ResolvedExRange, ExAddressResolutionError> {
        if matches!(range, Some(ExRange::WholeFile)) {
            let first = self.physical(0)?;
            let last = self.physical(self.physical_count().saturating_sub(1))?;
            let mut range = self.between(first, last)?;
            range.text = 0..self.document.projection().text_tree().byte_len();
            range.source = Some(range.start.source.as_ref().unwrap().start..self.document.source_byte_len());
            return Ok(range);
        }
        let absolute_only = match range {
            Some(ExRange::Single(address)) => matches!(address.base, AddressBase::Absolute(_)),
            Some(ExRange::Between { start, end, .. }) => matches!(start.base, AddressBase::Absolute(_))
                && matches!(end.base, AddressBase::Absolute(_)),
            _ => false,
        };
        let current = if absolute_only { self.raw_position() } else { self.current_position()? };
        let (start, end) = match range {
            None => { let line = self.contextual(current)?; (line.clone(), line) }
            Some(ExRange::Single(address)) => {
                let line = self.address(address, current, None)?; (line.clone(), line)
            }
            Some(ExRange::Between { start, end, separator }) => {
                let start = self.address(start, current, None)?;
                let end_current = if *separator == RangeSeparator::Semicolon { start.position } else { current };
                let end_source = (*separator == RangeSeparator::Semicolon)
                    .then(|| start.source.as_ref().map(|source| source.start)).flatten();
                let end = self.address(end, end_current, end_source)?;
                (start, end)
            }
            Some(ExRange::WholeFile) => unreachable!(),
        };
        self.between(start, end)
    }

    fn between(&self, mut start: ResolvedExLine, mut end: ResolvedExLine) -> Result<ResolvedExRange, ExAddressResolutionError> {
        let source = if start.domain == ExLineDomain::PhysicalSource && end.domain == ExLineDomain::PhysicalSource {
            if start.index > end.index {
                return Err(ExExecuteError::InvertedRange { start: start.index, end: end.index }.into());
            }
            Some(start.source.as_ref().unwrap().start..end.source.as_ref().unwrap().end)
        } else { None };
        if start.text.start > end.text.end {
            return Err(ExExecuteError::InvertedRange { start: start.text.start, end: end.text.end }.into());
        }
        let text = start.text.start..end.text.end;
        let mut lines = Vec::new();
        if start.domain == end.domain && start.domain != ExLineDomain::PhysicalSource {
            // Retained visual units are bounded by the installed snapshot.
            // Formatted units remain indexed and are generated by consumers.
            if start.domain == ExLineDomain::Visual {
                let snapshot = self.layout.ok_or(LayoutMotionError::EmptyLayout)?;
                // Distant endpoints may have been prepared in separate bands.
                // Consumers which need interior rows acquire them explicitly.
                if snapshot.coverage.contains_text_offset(start.position.text_offset)
                    && snapshot.coverage.contains_text_offset(end.position.text_offset) {
                    start.index = layout_motion::command_row_span(snapshot, start.position, 0)?.start;
                    end.index = layout_motion::command_row_span(snapshot, end.position, 0)?.start;
                    if start.index > end.index {
                        return Err(ExExecuteError::InvertedRange { start: start.index, end: end.index }.into());
                    }
                    for index in start.index..=end.index { lines.push(self.visual(index)?); }
                    if lines.windows(2).any(|pair| pair[0].text.end > pair[1].text.start) {
                        return Err(LayoutMotionError::TextDoesNotMatchLayout.into());
                    }
                    if lines.windows(2).any(|pair| pair[0].text.end != pair[1].text.start) {
                        lines.clear();
                    }
                }
            } else if start.index > end.index {
                return Err(ExExecuteError::InvertedRange { start: start.index, end: end.index }.into());
            }
        }
        Ok(ResolvedExRange { document: self.document.id(), revision: self.document.revision(),
            start, end, text, source, lines })
    }

    fn validate_range(&self, range: &ResolvedExRange) -> Result<(), ExAddressResolutionError> {
        if range.document != self.document.id() { return Err(ExExecuteError::WrongDocument.into()); }
        if range.revision != self.document.revision() {
            return Err(ExExecuteError::StalePlan { expected: range.revision,
                actual: self.document.revision() }.into());
        }
        Ok(())
    }

    fn range_units(&self, range: &ResolvedExRange) -> Result<Vec<ResolvedExLine>, ExAddressResolutionError> {
        self.validate_range(range)?;
        const MAX_REPLAY_UNITS: usize = 100_000;
        if range.start.domain == ExLineDomain::PhysicalSource && range.end.domain == ExLineDomain::PhysicalSource {
            let count = range.end.index - range.start.index + 1;
            if count > MAX_REPLAY_UNITS {
                return Err(ExExecuteError::Regex(super::search_regex::RegexError::RegexResourceLimit("global target lines")).into());
            }
            return (range.start.index..=range.end.index).map(|index| self.physical(index)).collect();
        }
        if !range.lines.is_empty() { return Ok(range.lines.clone()); }
        if self.interpreter.line_mode == LineMode::Visual && self.layout.is_some() {
            let snapshot = self.layout.unwrap();
            if !snapshot.coverage.contains_text_offset(range.start.position.text_offset)
                || !snapshot.coverage.contains_text_offset(range.end.position.text_offset) {
                let first = self.presentation_line(range.text.start)?;
                let last = self.presentation_line(range.end.content.end)?;
                if last.saturating_sub(first) >= 4_096 {
                    return Err(ExExecuteError::Regex(super::search_regex::RegexError::RegexResourceLimit(
                        "exact visual replay layout")).into());
                }
                return Err(layout_motion::exact_ex_address_demand(snapshot, first..last + 1,
                    range.text.start).into());
            }
            let first = self.contextual(range.start.position)?;
            let last = if range.end.domain == ExLineDomain::Visual { range.end.clone() }
                else { self.contextual(VisualPosition { text_offset: range.end.content.end,
                    affinity: if range.end.content.is_empty() { BoundaryAffinity::Downstream }
                        else { BoundaryAffinity::Upstream } })? };
            let units = self.between(first, last)?.lines;
            if units.is_empty() {
                let first = self.presentation_line(range.text.start)?;
                let last = self.presentation_line(range.end.content.end)?;
                if last.saturating_sub(first) >= 4_096 {
                    return Err(ExExecuteError::Regex(super::search_regex::RegexError::RegexResourceLimit(
                        "exact visual replay layout")).into());
                }
                return Err(layout_motion::exact_ex_address_demand(snapshot, first..last + 1,
                    range.text.start).into());
            }
            return Ok(units.into_iter().filter_map(|mut unit| {
                let start = unit.text.start.max(range.text.start);
                let end = unit.text.end.min(range.text.end);
                if start > end { return None; }
                unit.text = start..end;
                unit.content = unit.content.start.max(start)..unit.content.end.min(end).max(start);
                unit.position.text_offset = start;
                Some(unit)
            }).collect());
        }
        let first = self.document.hard_line_at_offset(range.text.start).ok_or(ExExecuteError::AddressOverflow)?;
        let last = self.document.hard_line_at_offset(range.end.content.end).ok_or(ExExecuteError::AddressOverflow)?;
        if last - first + 1 > MAX_REPLAY_UNITS {
            return Err(ExExecuteError::Regex(super::search_regex::RegexError::RegexResourceLimit("global target lines")).into());
        }
        (first..=last).map(|index| {
            let mut unit = self.formatted(index)?;
            let start = unit.text.start.max(range.text.start);
            let end = unit.text.end.min(range.text.end).max(start);
            unit.text = start..end;
            unit.content = unit.content.start.max(start)..unit.content.end.min(end).max(start);
            unit.position.text_offset = start;
            Ok(unit)
        }).collect()
    }

    fn apply_command_count(&self, command: &ExCommand, range: ResolvedExRange) -> Result<ResolvedExRange, ExAddressResolutionError> {
        let count = match &command.action {
            ExAction::Delete(arguments) | ExAction::Yank(arguments) => arguments.count,
            ExAction::Print { count, .. } | ExAction::Shift { count, .. }
            | ExAction::RepeatSubstitute { count, .. } | ExAction::Join { count } => *count,
            ExAction::Substitute(substitute) => substitute.count,
            _ => None,
        };
        if let Some(count) = count {
            if count == 0 { return Err(ExExecuteError::InvalidCount(count).into()); }
            let delta = i64::try_from(count - 1).map_err(|_| ExExecuteError::AddressOverflow)?;
            let first = range.end;
            let last = self.advance(first.clone(), delta, true)?;
            return self.between(first, last);
        }
        if matches!(command.action, ExAction::Join { .. })
            && matches!(command.range, None | Some(ExRange::Single(_))) {
            let last = self.advance(range.end, 1, true)?;
            return self.between(range.start, last);
        }
        Ok(range)
    }

    fn search_address(&mut self, entered: &str, forward: bool, current: &ResolvedExLine) -> Result<usize, ExAddressResolutionError> {
        let pattern = if entered.is_empty() {
            self.search.as_ref().map(|(_, pattern)| pattern.clone())
                .ok_or(ExExecuteError::NoPreviousSearch)?
        } else { entered.to_owned() };
        let case = self.interpreter.search_options.case_insensitive(&pattern)
            .map_err(ExExecuteError::from)?;
        let regex = CompiledRegex::compile(&pattern, case, self.limits)
            .map_err(ExExecuteError::from)?;
        let snapshot = self.document.hard_line_snapshot();
        let input = RegexInput::new(&snapshot);
        let length = snapshot.text_length();
        let mut scan = |from: usize, to: usize, last: bool| -> Result<Option<usize>, ExAddressResolutionError> {
            let mut next = from;
            let mut found = None;
            while next <= to {
                let Some(matched) = regex.find(&input, next, length, &mut self.work)
                    .map_err(ExExecuteError::from)? else { break; };
                let at = matched.range().start;
                if at >= to { break; }
                found = Some(at);
                if !last { break; }
                next = snapshot.next_grapheme_boundary(at)
                    .unwrap_or_else(|| length.saturating_add(1));
            }
            Ok(found)
        };
        let mut found = if forward { scan(current.text.end, length.saturating_add(1), false)? }
            else { scan(0, current.text.start, true)? };
        if found.is_none() && self.interpreter.search_options.wrapscan {
            found = if forward { scan(0, current.text.end.saturating_add(1), false)? }
                else { scan(current.text.start, length.saturating_add(1), true)? };
        }
        let found = found.ok_or_else(|| ExExecuteError::PatternNotFound(pattern.clone()))?;
        self.search = Some((if forward { SearchDirection::Forward } else { SearchDirection::Backward }, pattern));
        self.search_changed = true;
        Ok(found)
    }
}

impl CommandInterpreter {
    pub(super) fn resolved_ex_range_units(&self, document: &Document, range: &ResolvedExRange)
        -> Result<Vec<ResolvedExLine>, ExAddressResolutionError> {
        AddressResolver::new(self, document, RegexLimits::default()).range_units(range)
    }
}

#[cfg(test)]
mod domain_tests {
    use super::*;
    use crate::document::{Encoding, Format};
    use crate::layout::{LayoutEngine, MockTextMeasurementProvider, ViewLayout};

    fn displayed(document: &Document, width: f32) -> CommandInterpreter {
        let mut engine = LayoutEngine::new(MockTextMeasurementProvider::new());
        let mut view = ViewLayout::new(width, 500.0);
        engine.relayout(document, &mut view).unwrap();
        let mut interpreter = CommandInterpreter::new();
        interpreter.line_layout = Some(view.snapshot().unwrap().clone());
        interpreter
    }

    fn resolve(interpreter: &CommandInterpreter, document: &Document, input: &str)
        -> Result<ResolvedExAddresses, ExAddressResolutionError> {
        let command = super::super::parse_ex(input).unwrap();
        interpreter.resolve_ex_addresses(document, &command, RegexLimits::default())
    }

    #[test]
    fn numeric_bases_and_offsets_always_name_physical_lines() {
        let document = Document::from_bytes(b"alpha\nbeta\n\ngamma\ndelta".to_vec(),
            Encoding::Utf8, Format::Markdown).unwrap();
        let interpreter = displayed(&document, 55.0);
        let resolved = resolve(&interpreter, &document, ":1+3p").unwrap();
        let range = resolved.range.unwrap();
        assert_eq!(range.start.domain, ExLineDomain::PhysicalSource);
        assert_eq!(range.start.index, 3);
        assert_eq!(range.source, Some(document.physical_line_ranges(3).unwrap().0));
        assert!(range.lines.is_empty(), "physical ranges remain indexed");
    }

    #[test]
    fn contextual_offsets_count_displayed_rows_without_global_ordinals() {
        let document = Document::new("abc def ghi jkl mno pqr stu vwx yz\ntail");
        let interpreter = displayed(&document, 55.0);
        let rows = &interpreter.line_layout.as_ref().unwrap().rows;
        assert!(rows.len() > 4);
        let range = resolve(&interpreter, &document, ":.,.+2y").unwrap().range.unwrap();
        assert_eq!(range.start.domain, ExLineDomain::Visual);
        assert_eq!(range.text, rows[0].text_range.start..rows[2].text_range.end);
        assert_eq!(range.lines.len(), 3);
        assert!(range.source.is_none());
    }

    #[test]
    fn explicit_counts_extend_in_the_end_address_domain() {
        let document = Document::new("abc def ghi jkl mno pqr stu vwx yz\ntail\nlast");
        let interpreter = displayed(&document, 55.0);
        let range = resolve(&interpreter, &document, ":.,.+2delete 2").unwrap().range.unwrap();
        assert_eq!((range.start.index, range.end.index), (2, 3));
        assert_eq!(range.start.domain, ExLineDomain::Visual);
        let range = resolve(&interpreter, &document, ":1delete 2").unwrap().range.unwrap();
        assert_eq!((range.start.index, range.end.index), (0, 1));
        assert_eq!(range.start.domain, ExLineDomain::PhysicalSource);
    }

    #[test]
    fn semicolon_rebases_context_in_the_selected_mode() {
        let document = Document::from_bytes(b"alpha\nbeta\n\ngamma".to_vec(),
            Encoding::Utf8, Format::Markdown).unwrap();
        let mut interpreter = displayed(&document, 55.0);
        let range = resolve(&interpreter, &document, ":2;.+1p").unwrap().range.unwrap();
        assert_eq!(range.start.domain, ExLineDomain::PhysicalSource);
        assert_eq!(range.end.domain, ExLineDomain::Visual);
        assert!(range.source.is_none());
        interpreter.line_mode = LineMode::PhysicalSource;
        let range = resolve(&interpreter, &document, ":2;.+1p").unwrap().range.unwrap();
        assert_eq!((range.start.index, range.end.index), (1, 2));
        assert!(range.source.is_some());
    }

    #[test]
    fn contextual_last_and_numeric_last_exclude_terminal_caret_row() {
        let document = Document::new("a\n");
        let mut interpreter = displayed(&document, 500.0);
        let range = resolve(&interpreter, &document, ":$p").unwrap().range.unwrap();
        assert_eq!((range.start.domain, range.start.index), (ExLineDomain::Visual, 0));
        interpreter.line_mode = LineMode::PhysicalSource;
        let range = resolve(&interpreter, &document, ":$p").unwrap().range.unwrap();
        assert_eq!((range.start.domain, range.start.index), (ExLineDomain::PhysicalSource, 0));
        assert!(matches!(resolve(&interpreter, &document, ":2p"),
            Err(ExAddressResolutionError::Execute(ExExecuteError::AddressOutOfBounds { .. }))));
    }

    #[test]
    fn zero_destination_accepts_offset_forms_and_ranges_still_reject_zero() {
        let document = Document::new("alpha\nbeta");
        let interpreter = displayed(&document, 500.0);
        for input in [":0put", ":1-1put", ":.-1put", ":1copy 1-1", ":0read file"] {
            assert!(resolve(&interpreter, &document, input).unwrap().destination_before_first, "{input}");
        }
        assert!(matches!(resolve(&interpreter, &document, ":0p"),
            Err(ExAddressResolutionError::Execute(ExExecuteError::AddressOutOfBounds { .. }))));
    }

    #[test]
    fn stale_visual_geometry_is_rejected_but_physical_addresses_do_not_use_it() {
        let mut document = Document::new("abc def ghi\ntail");
        let interpreter = displayed(&document, 55.0);
        document.insert(0, "X").unwrap();
        assert!(matches!(resolve(&interpreter, &document, ":.p"),
            Err(ExAddressResolutionError::Execute(ExExecuteError::StalePlan { .. }))));
        assert!(resolve(&interpreter, &document, ":2p").is_ok());
    }

    #[test]
    fn late_numeric_ranges_do_not_enumerate_or_flatten_large_documents() {
        let mut document = Document::new("line text\n".repeat(100_000));
        document.insert(0, "X").unwrap();
        let interpreter = CommandInterpreter::new();
        assert!(!document.projection().compatibility_text_is_materialized());
        let range = resolve(&interpreter, &document, ":90000,90001y").unwrap().range.unwrap();
        assert_eq!((range.start.index, range.end.index), (89_999, 90_000));
        assert!(range.lines.is_empty());
        assert!(!document.projection().compatibility_text_is_materialized());
    }

    #[test]
    fn whole_file_range_keeps_the_encoding_bom_outside_editing() {
        let document = Document::from_bytes(b"\xef\xbb\xbfalpha\nbeta\n".to_vec(),
            Encoding::Utf8, Format::PlainText).unwrap();
        let interpreter = CommandInterpreter::new();
        let range = resolve(&interpreter, &document, ":%d").unwrap().range.unwrap();
        assert_eq!(range.source, Some(3..document.source_byte_len()));
    }

    #[test]
    fn unrelated_numeric_pseudo_ranges_do_not_request_line_layout() {
        let document = Document::new("alpha\nbeta");
        let interpreter = CommandInterpreter::new();
        for input in [":800goto", ":40split", ":set number"] {
            let resolved = resolve(&interpreter, &document, input).unwrap();
            assert!(resolved.range.is_none());
            assert!(resolved.navigation.is_none());
        }
    }

    #[test]
    fn command_submission_preflight_only_acquires_consumed_visual_geometry() {
        let document = Document::new("alpha beta gamma\ntail");
        let mut interpreter = CommandInterpreter::new();
        interpreter.enter_command_line(super::super::CommandLineKind::Ex);
        for (input, needs_layout) in [
            (".,.+2p", true), ("$p", true), ("p", true), ("1copy .", true),
            ("1,2p", false), ("2+1p", false), ("%p", false), ("sort", false),
            ("800goto", false), ("40split", false), ("set number", false),
        ] {
            interpreter.command_line_state.as_mut().unwrap().buffer.set(input.into());
            assert_eq!(interpreter.requires_layout_for_input(&document,
                &super::super::InputEvent::Key(super::super::Key::Enter), None), needs_layout, "{input}");
        }
        interpreter.line_mode = LineMode::PhysicalSource;
        interpreter.command_line_state.as_mut().unwrap().buffer.set(".,.+2p".into());
        assert!(!interpreter.requires_layout_for_input(&document,
            &super::super::InputEvent::Key(super::super::Key::Enter), None));
    }

    #[test]
    fn visual_ex_range_survives_reflow_without_changing_its_selected_units() {
        let document = Document::new("abc def ghi jkl mno pqr stu vwx yz\ntail");
        let mut interpreter = displayed(&document, 55.0);
        let rows = interpreter.line_layout.as_ref().unwrap().rows[..3].to_vec();
        interpreter.mode = super::super::Mode::VisualLine;
        interpreter.visual_anchor = Some(rows[0].text_range.start);
        interpreter.cursor = rows[2].text_range.start;
        assert_eq!(interpreter.enter_visual_ex(&document).status, super::super::CommandStatus::Pending);
        let retained = interpreter.command_line_state.as_ref().unwrap().retained_visual_range.clone().unwrap();
        interpreter.line_layout = displayed(&document, 500.0).line_layout;
        let command = super::super::parse_ex("'<,'>y").unwrap();
        let resolved = interpreter.prepare_ex_addresses_with_visual_range(&document, &command,
            Some(&retained), RegexLimits::default()).unwrap().range.unwrap();
        assert_eq!(resolved, retained);
        assert_eq!(resolved.text, rows[0].text_range.start..rows[2].text_range.end);
        interpreter.command_line_state.as_mut().unwrap().buffer.insert("s/def/X/");
        let preview = interpreter.search_presentation(&document);
        assert_eq!(preview.addressed_range, Some(retained));
    }

    #[test]
    fn replacing_a_stale_visual_range_with_numeric_addresses_uses_the_current_document() {
        let mut document = Document::new("head\ntail\nend");
        let mut interpreter = displayed(&document, 300.0);
        interpreter.mode = super::super::Mode::VisualLine;
        interpreter.visual_anchor = Some(0);
        assert_eq!(interpreter.enter_visual_ex(&document).status, super::super::CommandStatus::Pending);
        document.insert(0, "X").unwrap();
        let mut retained = interpreter.clone();
        retained.command_line_state.as_mut().unwrap().buffer.set("'<,'>s/head/H/".into());
        assert!(retained.search_presentation(&document).incremental_match.is_none());
        assert!(matches!(retained.submit_command_line(&mut document).unwrap().status,
            super::super::CommandStatus::ExError(super::super::ExCommandError::Execute(
                ExExecuteError::StalePlan { .. }))));
        assert_eq!(document.text(), "Xhead\ntail\nend");

        interpreter.command_line_state.as_mut().unwrap().buffer.set("2s/tail/T/".into());
        let preview = interpreter.search_presentation(&document);
        assert_eq!(preview.incremental_match.unwrap().matched_range, 6..10);
        assert_eq!(preview.addressed_range.unwrap().revision, document.revision());
        let mut adjusted = interpreter.clone();
        adjusted.line_layout = displayed(&document, 300.0).line_layout;
        adjusted.command_line_state.as_mut().unwrap().buffer.set("'<+1s/tail/T/".into());
        assert_eq!(adjusted.search_presentation(&document).incremental_match.unwrap().matched_range, 6..10,
            "altered mark addresses resolve freshly rather than reuse the retained range");
        interpreter.command_line_state.as_mut().unwrap().buffer.set("2d".into());
        assert_eq!(interpreter.submit_command_line(&mut document).unwrap().status,
            super::super::CommandStatus::Complete);
        assert_eq!(document.text(), "Xhead\nend");
    }

    #[test]
    fn core_contextual_print_uses_displayed_rows() {
        use crate::{Core, CoreEvent};
        use super::super::{InputEvent, Key, ExFrontendRequest, CommandStatus};
        use crate::command::ex_execute::ExInfoRequest;
        let mut core = Core::new(Document::new("abc def ghi jkl mno pqr stu vwx yz\ntail"));
        let view = core.add_view(MockTextMeasurementProvider::new(), 55.0, 500.0);
        let rows = core.layout(view).unwrap().snapshot().unwrap().rows[..3].to_vec();
        core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(Key::Char(':')))).unwrap();
        core.handle_with_layout(view, CoreEvent::Input(InputEvent::Text(".,.+2p".into()))).unwrap();
        let output = core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(Key::Enter)))
            .unwrap().command.unwrap();
        assert_eq!(output.status, CommandStatus::Complete);
        let outcome = output.ex_outcome.unwrap();
        let ExFrontendRequest::Info(ExInfoRequest::PrintLines { range, .. }) = &outcome.frontend_requests[0]
            else { panic!("print request"); };
        assert_eq!(range.text, rows[0].text_range.start..rows[2].text_range.end);
        assert_eq!(range.lines.len(), 3);
        assert_eq!(range.document, core.document().id());
        assert_eq!(range.revision, core.document().revision());
        let printed = super::super::ex_execute::addressed_text_lines(core.document(), range).unwrap();
        assert_eq!(printed.len(), 1, "soft wrapping does not create output line breaks");
        assert_eq!(printed[0].2, core.document().hard_line_snapshot()
            .slice_utf8(range.start.content.start..range.end.content.end).unwrap());
    }

    #[test]
    fn mixed_print_filters_physical_contributors_and_has_stable_grouping() {
        let document = Document::from_bytes(b"alpha\nbeta\ngamma\n\ntail".to_vec(),
            Encoding::Utf8, Format::Markdown).unwrap();
        let mut interpreter = displayed(&document, 55.0);
        let mut range = resolve(&interpreter, &document, ":2,$p").unwrap().range.unwrap();
        range.lines = vec![range.start.clone(), range.end.clone()];
        let with_units = super::super::ex_execute::addressed_text_lines(&document, &range).unwrap();
        let mut endpoint_only = range.clone();
        endpoint_only.lines.clear();
        let without_units = super::super::ex_execute::addressed_text_lines(&document, &endpoint_only).unwrap();
        assert_eq!(with_units, without_units);
        let output = with_units.iter().map(|(_, _, text)| text.as_str()).collect::<Vec<_>>().join("\n");
        assert!(!output.contains("alpha"), "preceding physical contributors stay outside the range");
        assert!(output.contains("beta"));
        assert!(output.contains("gamma"));
        assert!(output.contains("tail"));
        interpreter.line_layout = displayed(&document, 500.0).line_layout;
        assert_eq!(super::super::ex_execute::addressed_text_lines(&document, &range).unwrap(), with_units);
    }

    #[test]
    fn normal_replays_from_each_exact_original_visual_row_start() {
        let original = "abc def ghi jkl mno pqr stu vwx yz\ntail";
        let mut document = Document::new(original);
        let mut interpreter = displayed(&document, 55.0);
        let rows = interpreter.line_layout.as_ref().unwrap().rows[..3].to_vec();
        let range = resolve(&interpreter, &document, ":.,.+2normal iX").unwrap().range.unwrap();
        let output = interpreter.execute_ex_normal(&mut document, &super::super::ExNormalRequest {
            range, commands: "iX".into(), literal: true,
        });
        assert_eq!(output.status, super::super::CommandStatus::Complete);
        let mut expected = original.to_owned();
        for row in rows.iter().rev() { expected.insert(row.text_range.start, 'X'); }
        assert_eq!(document.text(), expected);
        assert!(document.undo());
        assert_eq!(document.text(), original);
    }
}
