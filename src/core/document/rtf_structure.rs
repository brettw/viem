//! Structural RTF deletion consumes selected list labels and paragraph breaks
//! explicitly while leaving the surrounding groups and opaque destinations.
use super::line_endings::NormalizedText;
use super::rtf::{self, Kind};
use super::{BlockKind, Document, DocumentError, Revision};
use std::collections::BTreeSet;
use std::ops::Range;

pub(super) fn deletion_patches(
    document: &Document,
    input: &NormalizedText,
    range: &Range<usize>,
    whole_line: bool,
) -> Result<Option<Vec<(Range<usize>, String)>>, DocumentError> {
    if !whole_line
        && !document
            .projection()
            .text_tree()
            .slice(range.clone())
            .map_err(DocumentError::FormattedTextStorage)?
            .contains('\n')
    {
        return Ok(None);
    }
    let projection = document.projection();
    let paragraphs = projection
        .blocks_for_region(range)
        .into_iter()
        .filter(|block| range.start <= block.range.start && block.range.end <= range.end)
        .collect::<Vec<_>>();
    if !paragraphs
        .iter()
        .any(|block| matches!(block.kind, BlockKind::ListItem { .. }))
    {
        return Ok(None);
    }
    let Some(first) = paragraphs.first() else {
        return Ok(None);
    };
    let last = paragraphs.last().unwrap();
    if !((range.start == first.range.start || range.start + 1 == first.range.start)
        && (range.end == last.range.end || range.end == last.range.end + 1))
    {
        return Ok(None);
    }
    let selected = paragraphs
        .iter()
        .map(|block| block.id)
        .collect::<BTreeSet<_>>();
    let structure = projection.list_structure();
    let mut origins = Vec::new();
    let mut surviving = Vec::new();
    let converter = super::rich_text::Builder::new(input, Revision(0));
    let tokens = rtf::tokenize(input);
    let numbering_origins = ListOriginIndex::new(input);
    let mut stack = Vec::new();
    let mut pn_groups = Vec::new();
    let mut hidden_groups = Vec::new();
    for (index, token) in tokens.iter().enumerate() {
        match token.kind {
            Kind::Open => stack.push(index),
            Kind::Close => {
                if let Some(open) = stack.pop() {
                    if tokens[open+1..index].iter().take(2).any(|token|matches!(&token.kind,Kind::Symbol('*'))||matches!(&token.kind,Kind::Control(name,_) if matches!(name.as_str(),"fonttbl"|"colortbl"|"stylesheet"|"listtable"|"listoverridetable"|"pntext"|"listtext"|"info"|"pict"|"object"|"field"|"shp"))) {
                        hidden_groups.push(converter.source_range(tokens[open].range.start..token.range.end));
                    }
                    if tokens[open + 1..index]
                        .iter()
                        .take(2)
                        .any(|token| matches!(&token.kind,Kind::Control(name,_) if name=="pn"))
                    {
                        pn_groups.push(
                            converter.source_range(tokens[open].range.start..token.range.end),
                        );
                    }
                }
            }
            _ => {}
        }
    }
    hidden_groups.sort_by_key(|range| (range.start, range.end));
    let mut hidden: Vec<Range<usize>> = Vec::new();
    for range in hidden_groups {
        if let Some(previous) = hidden
            .last_mut()
            .filter(|previous| range.start <= previous.end)
        {
            previous.end = previous.end.max(range.end);
        } else {
            hidden.push(range);
        }
    }
    let mapped = tokens
        .iter()
        .map(|token| (converter.source_range(token.range.clone()), token))
        .collect::<Vec<_>>();
    let mut selectors = Vec::new();
    for block in &paragraphs {
        let first = super::rich_text::block_source_point(projection, block)?;
        let start = if block.range.start == 0 {
            0
        } else {
            projection
                .provenance_for_region(&(block.range.start - 1..block.range.start))
                .last()
                .map(|span| span.source.end)
                .ok_or(DocumentError::AmbiguousProjection)?
        };
        let index = mapped.partition_point(|(source, _)| source.end <= start);
        for (source, token) in mapped[index..]
            .iter()
            .take_while(|(source, _)| source.start < first)
        {
            let index = hidden.partition_point(|group| group.start <= source.start);
            let opaque = index > 0 && hidden[index - 1].end >= source.end;
            if start <= source.start
                && source.end <= first
                && !opaque
                && matches!(&token.kind,Kind::Control(name,_) if matches!(name.as_str(),"pard"|"s"|"ls"|"ilvl"|"li"|"fi"|"ri"|"sb"|"sa"|"sl"|"slmult"|"ql"|"qr"|"qc"|"qj"|"rtlpar"|"ltrpar"))
            {
                selectors.push(source.clone());
            }
        }
    }
    for list in &structure.lists {
        let mut modern_deleted = false;
        for item in &list.items {
            let removed = item.paragraph_ids.iter().any(|id| selected.contains(id));
            let body_at = super::rich_text::list_item_source_point(projection, item)?;
            let origin = numbering_origins
                .at(body_at)
                .ok_or(DocumentError::AmbiguousProjection)?;
            let legacy = pn_groups.iter().find(|group| group.end == origin);
            if removed {
                if !item.paragraph_ids.iter().all(|id| selected.contains(id)) {
                    return Err(DocumentError::AmbiguousProjection);
                }
                if let Some(group) = legacy {
                    origins.push(group.clone());
                } else {
                    modern_deleted = true;
                }
            } else if modern_deleted
                || selectors
                    .iter()
                    .any(|selector| selector.start < origin && origin <= selector.end)
            {
                let spans = item
                    .paragraph_ids
                    .iter()
                    .flat_map(|id| {
                        projection
                            .blocks()
                            .iter()
                            .filter(move |block| block.id == *id)
                    })
                    .flat_map(|block| projection.provenance_for_region(&block.range))
                    .filter(|span| !span.source.is_empty())
                    .collect::<Vec<_>>();
                let body = if let (Some(first), Some(last)) = (spans.first(), spans.last()) {
                    first.source.start..last.source.end
                } else {
                    body_at..body_at
                };
                surviving.push((body, Some(list.style), item.ordinal, Some(item.ordinal)));
            }
        }
    }
    // A shared old numbering destination cannot be erased while a surviving
    // paragraph still refers to it; scope an explicit neutral override instead.
    for origin in &origins {
        if structure
            .lists
            .iter()
            .flat_map(|list| &list.items)
            .filter(|item| !selected.contains(&item.paragraph_id))
            .any(|item| {
                super::rich_text::list_item_source_point(projection, item)
                    .ok()
                    .and_then(|at| numbering_origins.at(at))
                    == Some(origin.end)
            })
        {
            return Err(DocumentError::AmbiguousProjection);
        }
    }
    let mut ranges = origins;
    ranges.extend(selectors);
    for span in projection.provenance_for_region(range) {
        if span.formatted.is_empty() {
            continue;
        }
        if span.source.is_empty() {
            return Err(DocumentError::AmbiguousProjection);
        } else {
            ranges.push(span.source);
        }
    }
    ranges.sort_by_key(|range| (range.start, range.end));
    let mut merged: Vec<Range<usize>> = Vec::new();
    for range in ranges {
        if let Some(previous) = merged
            .last_mut()
            .filter(|previous| range.start <= previous.end)
        {
            previous.end = previous.end.max(range.end);
        } else {
            merged.push(range);
        }
    }
    let mut patches = merged
        .into_iter()
        .map(|range| (range, String::new()))
        .collect::<Vec<_>>();
    patches.extend(rtf::list_patches(input, &surviving, projection)?);
    Ok(Some(patches))
}

/// Retarget a legacy numbering destination in place so an enclosing override
/// cannot be shadowed by its original pnstart. Modern table-driven numbering
/// uses scoped supporting overrides at the caller.
pub(super) fn renumber_legacy_patches(
    input: &NormalizedText,
    marker_source: usize,
    ordinal: u64,
) -> Option<Vec<(Range<usize>, String)>> {
    let tokens = rtf::tokenize(input);
    let converter = super::rich_text::Builder::new(input, Revision(0));
    let mut opens = Vec::new();
    let mut cached = Vec::new();
    for (index, token) in tokens.iter().enumerate() {
        match token.kind {
            Kind::Open => opens.push(index),
            Kind::Close => {
                if let Some(open) = opens.pop() {
                    if tokens[open + 1..index]
                        .iter()
                        .take(1)
                        .any(|token| matches!(&token.kind,Kind::Control(name,_) if name=="pntext"))
                    {
                        cached.push((open, index, opens.last().copied()));
                    }
                    if converter.source_range(token.range.clone()).end != marker_source {
                        continue;
                    }
                    let pn = tokens[open + 1..index]
                        .iter()
                        .take(2)
                        .find(|token| matches!(&token.kind,Kind::Control(name,_) if name=="pn"))?;
                    let patch = if let Some(start) = tokens[open + 1..index].iter().find(
                        |token| matches!(&token.kind,Kind::Control(name,_) if name=="pnstart"),
                    ) {
                        (
                            converter.source_range(start.range.clone()),
                            format!("\\pnstart{ordinal} "),
                        )
                    } else {
                        (
                            converter.source_range(pn.range.end..pn.range.end),
                            format!("\\pnstart{ordinal} "),
                        )
                    };
                    let mut result = vec![patch];
                    if let Some((first, last, _)) = cached.iter().rev().find(|(_, last, parent)| {
                        *last < open
                            && *parent == opens.last().copied()
                            && !tokens[*last + 1..open].iter().any(
                                |token| matches!(&token.kind,Kind::Control(name,_) if name=="par"),
                            )
                    }) {
                        result.push((
                            converter
                                .source_range(tokens[*first].range.start..tokens[*last].range.end),
                            format!("{{\\pntext {ordinal}.\\tab}}"),
                        ));
                    }
                    return Some(result);
                }
            }
            _ => {}
        }
    }
    None
}

/// Resolve the active numbering control from source grammar at an actual body
/// boundary. This replaces the former generated-label provenance dependency.
pub(super) struct ListOriginIndex {
    entries: Vec<(usize, Option<usize>)>,
}

impl ListOriginIndex {
    pub(super) fn new(input: &NormalizedText) -> Self {
        #[derive(Clone, Copy)]
        struct Context {
            origin: Option<usize>,
            hidden: bool,
            start: bool,
            numbering: bool,
        }
        let converter = super::rich_text::Builder::new(input, Revision(0));
        let mut state = Context {
            origin: None,
            hidden: false,
            start: true,
            numbering: false,
        };
        let mut stack = Vec::new();
        let mut entries = Vec::new();
        for token in rtf::tokenize(input) {
            let source = converter.source_range(token.range.clone());
            let previous_origin = state.origin;
            match token.kind {
                Kind::Open => {
                    stack.push(state);
                    state.start = true;
                    state.numbering = false;
                }
                Kind::Close => {
                    let numbering = state.numbering;
                    if let Some(parent) = stack.pop() {
                        state = parent;
                        if numbering && !state.hidden {
                            state.origin = Some(source.end);
                        }
                    }
                }
                Kind::Symbol('*') if state.start => state.hidden = true,
                Kind::Control(name, _) => {
                    if state.start
                        && name == "pn"
                        && stack.last().is_some_and(|parent| !parent.hidden)
                    {
                        state.numbering = true;
                    }
                    if state.start && rtf::non_body(&name) {
                        state.hidden = true;
                    }
                    if !matches!(name.as_str(), "rtf" | "ansi" | "mac" | "pc" | "pca") {
                        state.start = false;
                    }
                    if !state.hidden {
                        match name.as_str() {
                            "ls" | "ilvl" => state.origin = Some(source.end),
                            "pard" => state.origin = None,
                            _ => {}
                        }
                    }
                }
                Kind::Character('\r' | '\n') => {}
                _ => state.start = false,
            }
            if previous_origin != state.origin {
                entries.push((source.end, state.origin));
            }
        }
        Self { entries }
    }

    pub(super) fn at(&self, source_at: usize) -> Option<usize> {
        let end = self.entries.partition_point(|(at, _)| *at <= source_at);
        end.checked_sub(1).and_then(|index| self.entries[index].1)
    }
}

/// The original modern selector resumes after the new paragraph's terminator,
/// including sources that inherit ls and specify only a following ilvl.
pub(super) fn modern_selector_at(input: &NormalizedText, source_at: usize) -> Option<String> {
    let converter = super::rich_text::Builder::new(input, Revision(0));
    let mut state = (None, 0u8, false, true);
    let mut stack = Vec::new();
    for token in rtf::tokenize(input) {
        if converter.source_range(token.range).end > source_at {
            break;
        }
        match token.kind {
            Kind::Open => {
                stack.push(state);
                state.3 = true;
            }
            Kind::Close => state = stack.pop().unwrap_or((None, 0, false, true)),
            Kind::Symbol('*') if state.3 => state.2 = true,
            Kind::Control(name, number) => {
                if state.3 && rtf::non_body(&name) {
                    state.2 = true;
                }
                state.3 = false;
                if state.2 {
                    continue;
                }
                match name.as_str() {
                    "ls" => state.0 = number.filter(|n| (1..=2000).contains(n)),
                    "ilvl" => state.1 = number.filter(|n| (0..9).contains(n)).unwrap_or(0) as u8,
                    "pard" => {
                        state.0 = None;
                        state.1 = 0;
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
    state.0.map(|id| format!("\\ls{id}\\ilvl{} ", state.1))
}
