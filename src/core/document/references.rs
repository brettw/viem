//! Reference settings preserve authored spelling and separate a use from its definition.
//! The disposable, revision-bound index reads only projected reference owners, never
//! the whole source. Parsing and projection happen outside the cache lock.
use super::*;
use pulldown_cmark::{Event, LinkType, Parser, Tag};
use std::sync::{Arc, Mutex};

pub(super) type ReferenceCache = Mutex<Option<(Revision, Arc<ReferenceIndex>)>>;
const MAX_REFERENCE_BYTES: usize = 2 * 1024 * 1024;
const MAX_REFERENCE_ITEMS: usize = 4096;

#[derive(Clone, Debug)]
struct Entry {
    range: Range<usize>,
    source: Range<usize>,
    raw: String,
    displayed: String,
    destination_source: Option<Range<usize>>,
    name: String,
    text: String,
    destination: String,
    title: String,
    definition: bool,
    image: bool,
    target: Option<usize>,
    label: Range<usize>,
}
#[derive(Default, Clone)]
pub(super) struct ReferenceIndex {
    entries: Vec<Entry>,
    complete: bool,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct ReferenceChoice {
    pub name: String,
    pub destination: String,
    pub offset: usize,
}
#[derive(Clone, Debug, serde::Serialize)]
pub struct ReferenceUse {
    pub text: String,
    pub offset: usize,
    pub image: bool,
}
#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReferenceContext {
    pub start: usize,
    pub end: usize,
    pub name: String,
    pub text: String,
    pub destination: String,
    pub title: String,
    pub definition: bool,
    pub definition_offset: Option<usize>,
    pub inactive: bool,
    pub links: usize,
    pub images: usize,
    pub complete: bool,
    pub editable: bool,
    pub choices: Vec<ReferenceChoice>,
    pub uses: Vec<ReferenceUse>,
}
#[derive(Clone, Debug, serde::Deserialize)]
#[serde(tag = "action", rename_all = "camelCase", deny_unknown_fields)]
pub enum ReferenceRequest {
    Insert { offset: usize, end: usize, text: String, name: String, destination: String, title: String },
    EditUse {
        offset: usize,
        text: String,
        name: String,
    },
    CreateDefinition {
        offset: usize,
        destination: String,
        title: String,
    },
    EditDefinition {
        offset: usize,
        destination: String,
        title: String,
    },
    Rename {
        offset: usize,
        name: String,
    },
    ConvertInline {
        offset: usize,
    },
    Remove {
        offset: usize,
    },
    DeleteDefinition {
        offset: usize,
    },
    GoToDefinition {
        offset: usize,
    },
    GoToUse {
        offset: usize,
        target: usize,
    },
}
impl ReferenceRequest {
    pub fn offset(&self) -> usize {
        match self {
            Self::Insert { offset, .. } | Self::EditUse { offset, .. }
            | Self::CreateDefinition { offset, .. }
            | Self::EditDefinition { offset, .. }
            | Self::Rename { offset, .. }
            | Self::ConvertInline { offset }
            | Self::Remove { offset }
            | Self::DeleteDefinition { offset }
            | Self::GoToDefinition { offset }
            | Self::GoToUse { offset, .. } => *offset,
        }
    }
}
fn is_reference(kind: LinkType) -> bool {
    matches!(
        kind,
        LinkType::Reference
            | LinkType::Collapsed
            | LinkType::Shortcut
            | LinkType::ReferenceUnknown
            | LinkType::CollapsedUnknown
            | LinkType::ShortcutUnknown
    )
}
fn name_key(name: &str) -> String {
    name.split_whitespace().collect::<Vec<_>>().join(" ")
}
fn same_name(left: &str, right: &str) -> bool {
    let definition = format!("[{left}]: x");
    Parser::new(&definition)
        .reference_definitions()
        .get(&name_key(right))
        .is_some()
}
fn valid_name(name: &str) -> bool {
    !name.trim().is_empty()
        && name.chars().count() <= 999
        && !name.contains(['[', ']', '\n', '\r'])
        && !name.chars().any(char::is_control)
}
fn quoted(value: &str) -> String {
    format!(
        "\"{}\"",
        value
            .replace('&', "&amp;")
            .replace('\\', "\\\\")
            .replace('"', "&quot;")
            .replace('\n', "&#10;")
            .replace('\r', "&#13;")
    )
}
fn inline(entry: &Entry) -> String {
    format!(
        "{}[{}](<{}>{})",
        if entry.image { "!" } else { "" },
        entry.text,
        super::links::escape_destination(&entry.destination),
        if entry.title.is_empty() {
            String::new()
        } else {
            format!(" {}", quoted(&entry.title))
        }
    )
}
impl Document {
    fn reference_raw(&self, source: Range<usize>) -> Result<String, DocumentError> {
        let bytes = self
            .state()
            .source
            .bytes_in(source.clone())
            .ok_or(DocumentError::VerificationFailed)?;
        Ok(self.encoding().decode_region(&bytes, source.start)?.text)
    }
    fn missing_reference_at(&self, offset: usize) -> Result<Option<Entry>, DocumentError> {
        let tree = self.projection().text_tree();
        let line = tree
            .hard_line_at_byte(offset)
            .map_err(DocumentError::FormattedTextStorage)?;
        let start = tree
            .hard_line_start(line)
            .map_err(DocumentError::FormattedTextStorage)?;
        let end = tree
            .hard_line_end(line)
            .map_err(DocumentError::FormattedTextStorage)?;
        if end - start > 8192 {
            return Ok(None);
        }
        let text = tree
            .slice(start..end)
            .map_err(DocumentError::FormattedTextStorage)?;
        if !text.contains('[') || !text.contains("][") {
            return Ok(None);
        }
        let mut callback = |_: pulldown_cmark::BrokenLink<'_>| Some(("missing".into(), "".into()));
        for (event, mut span) in Parser::new_with_broken_link_callback(
            &text,
            pulldown_cmark::Options::empty(),
            Some(&mut callback),
        )
        .into_offset_iter()
        {
            let Event::Start(Tag::Link {
                link_type: LinkType::ReferenceUnknown | LinkType::CollapsedUnknown,
                id,
                ..
            }) = event
            else {
                continue;
            };
            if text[span.end..].starts_with("[]") {
                span.end += 2;
            }
            let range = start + span.start..start + span.end;
            if !range.contains(&offset) || !self.can_insert_link(range.clone()) {
                continue;
            }
            let Some(source) = self.projection().source_range(range.clone()) else {
                continue;
            };
            let raw = self.reference_raw(source.clone())?;
            if raw != text[span] {
                continue;
            }
            let Some(label_end) = super::links::markdown_label_end(&raw, 0, raw.len()) else {
                continue;
            };
            return Ok(Some(Entry {
                range,
                source,
                text: raw[1..label_end].to_owned(),
                displayed: raw.clone(),
                destination_source: None,
                raw,
                name: id.to_string(),
                destination: String::new(),
                title: String::new(),
                definition: false,
                image: false,
                target: None,
                label: 1..label_end,
            }));
        }
        Ok(None)
    }
    fn reference_index_at(&self, offset: usize) -> Result<Arc<ReferenceIndex>, DocumentError> {
        let index = self.reference_index()?;
        if index
            .entries
            .iter()
            .any(|e| !e.image && e.range.contains(&offset))
        {
            return Ok(index);
        }
        if let Some(entry) = self.missing_reference_at(offset)? {
            let mut index = (*index).clone();
            index.entries.push(entry);
            return Ok(Arc::new(index));
        }
        Ok(index)
    }
    fn reference_index(&self) -> Result<Arc<ReferenceIndex>, DocumentError> {
        if let Some((revision, index)) = &*self
            .reference_cache
            .lock()
            .unwrap_or_else(|p| p.into_inner())
        {
            if *revision == self.revision() {
                return Ok(index.clone());
            }
        }
        let mut regions: Vec<(Range<usize>, Range<usize>, bool)> = Vec::new();
        let mut complete = true;
        let mut bytes = 0;
        // Style traversal is bounded. Ordinary caret queries never enter this path
        // unless their local projection has reference treatment.
        for (visited, span) in self.projection().style_span_items().enumerate() {
            if visited >= 65536 {
                complete = false;
                break;
            }
            if span.application != StyleApplication::Automatic("Markdown reference".into()) {
                continue;
            }
            if self.projection().inline_references_for_region(&span.range).iter().any(|r| r.range == span.range) { continue; }
            let Some(source) = self.projection().source_range(span.range.clone()) else {
                continue;
            };
            bytes += source.len();
            if bytes > MAX_REFERENCE_BYTES || regions.len() >= MAX_REFERENCE_ITEMS {
                complete = false;
                break;
            }
            // Definitions may have a destination/title on following physical rows.
            if let Some((previous_range, previous_source, false)) = regions.last_mut() {
                if span.range.start == previous_range.end + 1 && source.start >= previous_source.end
                {
                    let prior = self.reference_raw(previous_source.clone())?;
                    if prior.trim_start().starts_with('[')
                        && prior.contains("]:")
                        && !self
                            .reference_raw(source.clone())?
                            .trim_start()
                            .starts_with('[')
                    {
                        previous_range.end = span.range.end;
                        previous_source.end = source.end;
                        continue;
                    }
                }
            }
            regions.push((span.range, source, false));
        }
        for (visited, image) in self.projection().inline_image_items().enumerate() {
            if visited >= MAX_REFERENCE_ITEMS {
                complete = false;
                break;
            }
            if image.inline || image.html {
                continue;
            }
            bytes += image.source.len();
            if bytes > MAX_REFERENCE_BYTES || regions.len() >= MAX_REFERENCE_ITEMS {
                complete = false;
                break;
            }
            regions.push((image.range, image.source, true));
        }
        for (visited, reference) in self.projection().inline_reference_items().enumerate() {
            bytes += reference.source.len();
            if visited >= MAX_REFERENCE_ITEMS || bytes > MAX_REFERENCE_BYTES || regions.len() >= MAX_REFERENCE_ITEMS {
                complete = false; break;
            }
            regions.push((reference.range, reference.source, false));
        }
        regions.sort_by_key(|(_, source, _)| source.start);
        let mut entries = Vec::new();
        let mut definitions = String::new();
        let mut definition_ranges = Vec::new();
        for (range, source, image) in regions {
            let raw = self.reference_raw(source.clone())?;
            // Every region is already recognized by the document parser. Parse its
            // spelling to get field boundaries, never to activate syntax in code.
            let rendered_reference = self.projection().inline_references_for_region(&range).iter().any(|r| r.source == source);
            let displayed = if image || rendered_reference {
                raw.clone()
            } else {
                self.projection()
                    .text_tree()
                    .slice(range.clone())
                    .map_err(DocumentError::FormattedTextStorage)?
            };
            let parser = Parser::new(&displayed);
            let mut defs: Vec<_> = parser.reference_definitions().iter().collect();
            defs.sort_by_key(|(_, d)| d.span.start);
            if !defs.is_empty() && !image && !rendered_reference {
                for (name, def) in defs {
                    let logical = range.start + def.span.start..range.start + def.span.end;
                    let source = self
                        .projection()
                        .source_range(logical.clone())
                        .ok_or(DocumentError::AmbiguousProjection)?;
                    let spelling = self.reference_raw(source.clone())?;
                    let shown = &displayed[def.span.clone()];
                    let label_end = super::links::markdown_label_end(&spelling, 0, spelling.len())
                        .ok_or(DocumentError::VerificationFailed)?;
                    let shown_label_end = super::links::markdown_label_end(shown, 0, shown.len())
                        .ok_or(DocumentError::VerificationFailed)?;
                    let base = shown_label_end + 2;
                    let wrapped = format!("[x]({})", &shown[base..]);
                    let destination_source =
                        super::links::markdown_inline_at(&wrapped, 0, wrapped.len())
                            .and_then(|link| link.destination_range)
                            .and_then(|r| {
                                self.projection().source_range(
                                    logical.start + base + r.start - 4
                                        ..logical.start + base + r.end - 4,
                                )
                            });
                    definitions.push_str("\n\n");
                    let begin = definitions.len();
                    definitions.push_str(shown);
                    definition_ranges.push((begin..definitions.len(), entries.len()));
                    entries.push(Entry {
                        range: logical,
                        source,
                        displayed: shown.to_owned(),
                        destination_source,
                        name: name.to_owned(),
                        text: name.to_owned(),
                        destination: def.dest.to_string(),
                        title: def.title.as_deref().unwrap_or("").to_owned(),
                        definition: true,
                        image: false,
                        target: None,
                        label: 1..label_end,
                        raw: spelling,
                    });
                }
            } else {
                entries.push(Entry {
                    range,
                    source,
                    raw,
                    displayed,
                    destination_source: None,
                    name: String::new(),
                    text: String::new(),
                    destination: String::new(),
                    title: String::new(),
                    definition: false,
                    image,
                    target: None,
                    label: 0..0,
                });
            }
        }
        let parser = Parser::new(&definitions);
        let defs = parser.reference_definitions();
        for entry in &mut entries {
            if !entry.definition {
                let input = format!("x {}", entry.displayed);
                let mut resolve = |link: pulldown_cmark::BrokenLink<'_>| {
                    defs.get(&link.reference).map(|definition| {
                        (
                            definition.dest.clone(),
                            definition.title.clone().unwrap_or_else(|| "".into()),
                        )
                    })
                };
                let found = Parser::new_with_broken_link_callback(
                    &input,
                    pulldown_cmark::Options::empty(),
                    Some(&mut resolve),
                )
                .into_offset_iter()
                .find_map(|(event, span)| match event {
                    Event::Start(
                        Tag::Link {
                            link_type,
                            id,
                            dest_url,
                            title,
                            ..
                        }
                        | Tag::Image {
                            link_type,
                            id,
                            dest_url,
                            title,
                            ..
                        },
                    ) if span.start == 2 && is_reference(link_type) => {
                        Some((id.to_string(), dest_url.to_string(), title.to_string()))
                    }
                    _ => None,
                });
                let Some((name, destination, title)) = found else {
                    continue;
                };
                let open = if entry.image { 1 } else { 0 };
                let Some(end) = super::links::markdown_label_end(&entry.raw, open, entry.raw.len())
                else {
                    continue;
                };
                entry.label = open + 1..end;
                let Some(shown_end) =
                    super::links::markdown_label_end(&entry.displayed, open, entry.displayed.len())
                else {
                    continue;
                };
                entry.text = entry.displayed[open + 1..shown_end].to_owned();
                entry.name = name;
                entry.destination = destination;
                entry.title = title;
            }
            entry.target = defs.get(&name_key(&entry.name)).and_then(|def| {
                definition_ranges
                    .iter()
                    .find(|(r, _)| r.contains(&def.span.start))
                    .map(|(_, i)| *i)
            });
        }
        let index = Arc::new(ReferenceIndex { entries, complete });
        *self
            .reference_cache
            .lock()
            .unwrap_or_else(|p| p.into_inner()) = Some((self.revision(), index.clone()));
        Ok(index)
    }
    pub fn reference_context_at(
        &self,
        point: TextPoint,
    ) -> Result<Option<ReferenceContext>, DocumentError> {
        if point.document() != self.id() {
            return Err(DocumentError::WrongDocument);
        }
        if point.revision() != self.revision() {
            return Err(DocumentError::WrongSnapshot {
                expected: self.revision(),
                actual: point.revision(),
            });
        }
        self.text_point(point.offset())?;
        if self.format() != Format::Markdown {
            return Ok(None);
        }
        if !self
            .projection()
            .style_spans_for_region(&(point.offset()..point.offset().saturating_add(1)))
            .iter()
            .any(|s| {
                s.range.contains(&point.offset())
                    && s.application == StyleApplication::Automatic("Markdown reference".into())
            })
            && !self.projection().inline_references_for_region(&(point.offset()..point.offset())).iter().any(|r| r.range.is_empty() && r.range.start == point.offset())
            && self.missing_reference_at(point.offset())?.is_none()
        {
            return Ok(None);
        }
        let index = self.reference_index_at(point.offset())?;
        let Some((own, entry)) = index
            .entries
            .iter()
            .enumerate()
            .find(|(_, e)| !e.image && !e.name.is_empty() && (e.range.contains(&point.offset()) || e.range.is_empty() && e.range.start == point.offset()))
        else {
            return Ok(None);
        };
        let target = entry.target;
        let uses: Vec<_> = index
            .entries
            .iter()
            .filter(|e| !e.definition && e.target == target && target.is_some())
            .map(|e| ReferenceUse {
                text: e.text.clone(),
                offset: e.range.start,
                image: e.image,
            })
            .collect();
        Ok(Some(ReferenceContext {
            start: entry.range.start,
            end: entry.range.end,
            name: entry.name.clone(),
            text: if !entry.definition && entry.target.is_some() {
                self.projection().text_tree().slice(entry.range.clone()).map_err(DocumentError::FormattedTextStorage)?
            } else { entry.text.clone() },
            destination: entry.destination.clone(),
            title: entry.title.clone(),
            definition: entry.definition,
            definition_offset: target.map(|i| index.entries[i].range.start),
            inactive: entry.definition && target != Some(own),
            links: uses.iter().filter(|u| !u.image).count(),
            images: uses.iter().filter(|u| u.image).count(),
            complete: index.complete,
            editable: index.complete && self.is_modifiable() && !self.is_read_only(),
            uses,
            choices: index
                .entries
                .iter()
                .enumerate()
                .filter(|(i, e)| e.definition && e.target == Some(*i))
                .map(|(_, e)| ReferenceChoice {
                    name: e.name.clone(),
                    destination: e.destination.clone(),
                    offset: e.range.start,
                })
                .collect(),
        }))
    }
    pub fn reference_jump_target(
        &self,
        request: &ReferenceRequest,
    ) -> Result<usize, DocumentError> {
        let context = self
            .reference_context_at(self.text_point(request.offset())?)?
            .ok_or(DocumentError::UnsupportedFormatting)?;
        match request {
            ReferenceRequest::GoToDefinition { .. } => context
                .definition_offset
                .ok_or(DocumentError::UnsupportedFormatting),
            ReferenceRequest::GoToUse { target, .. }
                if context.uses.iter().any(|u| u.offset == *target) =>
            {
                Ok(*target)
            }
            _ => Err(DocumentError::UnsupportedFormatting),
        }
    }
    fn reference_patch(
        &self,
        entry: &Entry,
        range: Range<usize>,
        replacement: &str,
    ) -> Result<SourcePatch, DocumentError> {
        let bytes = self
            .state()
            .source
            .bytes_in(entry.source.clone())
            .ok_or(DocumentError::VerificationFailed)?;
        let decoded = self.encoding().decode_region(&bytes, entry.source.start)?;
        let start = decoded
            .source_boundary(range.start)
            .ok_or(DocumentError::AmbiguousProjection)?;
        let end = decoded
            .source_boundary(range.end)
            .ok_or(DocumentError::AmbiguousProjection)?;
        Ok(SourcePatch::primary(
            start..end,
            self.encoding().encode_fragment(replacement)?,
        ))
    }
    pub fn prepare_reference_edit(
        &self,
        document: DocumentId,
        revision: Revision,
        request: &ReferenceRequest,
    ) -> Result<(PreparedModelTransaction, usize), ModelTransactionError> {
        self.require_modifiable()?;
        if document != self.id() {
            return Err(DocumentError::WrongDocument.into());
        }
        if revision != self.revision() {
            return Err(DocumentError::WrongSnapshot {
                expected: self.revision(),
                actual: revision,
            }
            .into());
        }
        if let ReferenceRequest::Insert { offset, end, text, name, destination, title } = request {
            return self.prepare_reference_insertion(*offset..*end, text, name, destination, title);
        }
        let context = self
            .reference_context_at(self.text_point(request.offset())?)?
            .ok_or(DocumentError::UnsupportedFormatting)?;
        if !context.editable {
            return Err(DocumentError::UnsupportedFormatting.into());
        }
        let index = self.reference_index_at(request.offset())?;
        let (own, entry) = index
            .entries
            .iter()
            .enumerate()
            .find(|(_, e)| e.range.start == context.start && !e.image)
            .ok_or(DocumentError::UnsupportedFormatting)?;
        let mut patches = Vec::new();
        let target = entry.target.unwrap_or(own);
        let definition = &index.entries[if entry.definition { own } else { target }];
        let mut expected_use: Option<(String, String)> = None;
        let mut expected_definition: Option<(String, String)> = None;
        match request {
            ReferenceRequest::CreateDefinition {
                destination, title, ..
            } if entry.target.is_none() && valid_name(&entry.name) => {
                if destination.chars().any(char::is_control) {
                    return Err(DocumentError::UnsupportedFormatting.into());
                }
                let newline = self.file_format().spelling();
                let spelling = format!(
                    "{newline}{newline}[{}]: <{}>{}",
                    entry.name,
                    super::links::escape_destination(destination),
                    if title.is_empty() {
                        String::new()
                    } else {
                        format!(" {}", quoted(title))
                    }
                );
                patches.push(SourcePatch::primary(
                    self.source_byte_len()..self.source_byte_len(),
                    self.encoding().encode_fragment(&spelling)?,
                ));
                expected_use = Some((super::links::plain_label(&entry.text), destination.clone()));
            }
            ReferenceRequest::EditUse { text, name, .. } if !entry.definition => {
                let choice = context
                    .choices
                    .iter()
                    .find(|c| same_name(&c.name, name))
                    .ok_or(DocumentError::UnsupportedFormatting)?;
                if text.contains(['\n', '\r']) {
                    return Err(DocumentError::UnsupportedFormatting.into());
                }
                if *text != context.text {
                    if entry.target.is_some() {
                        let label = self.reference_label_replacement(entry.range.clone(), text)?;
                        let decoded = self.encoding().decode_region(&label, 0)?.text;
                        patches.push(self.reference_patch(entry, entry.label.clone(), &decoded)?);
                    } else {
                        patches.push(self.reference_patch(entry, entry.label.clone(), &super::links::escape_label(text))?);
                    }
                }
                if *name != entry.name
                    || *text != context.text && entry.raw[entry.label.end + 1..].is_empty()
                    || *text != context.text && entry.raw[entry.label.end + 1..] == *"[]"
                {
                    patches.push(self.reference_patch(
                        entry,
                        entry.label.end + 1..entry.raw.len(),
                        &format!("[{name}]"),
                    )?);
                }
                expected_use = Some((text.clone(), choice.destination.clone()));
            }
            ReferenceRequest::EditDefinition {
                destination, title, ..
            } if entry.target.is_some() => {
                if destination.chars().any(char::is_control) {
                    return Err(DocumentError::UnsupportedFormatting.into());
                }
                let destination_source = definition
                    .destination_source
                    .clone()
                    .ok_or(DocumentError::UnsupportedFormatting)?;
                if *destination != definition.destination {
                    patches.push(SourcePatch::primary(
                        destination_source.clone(),
                        self.encoding().encode_fragment(&format!(
                            "<{}>",
                            super::links::escape_destination(destination)
                        ))?,
                    ));
                }
                if *title != definition.title {
                    let suffix = if title.is_empty() {
                        String::new()
                    } else {
                        format!(" {}", quoted(title))
                    };
                    patches.push(SourcePatch::primary(
                        destination_source.end..definition.source.end,
                        self.encoding().encode_fragment(&suffix)?,
                    ));
                }
                expected_definition = Some((destination.clone(), title.clone()));
            }
            ReferenceRequest::Rename { name, .. } if entry.target.is_some() && valid_name(name) => {
                if name == &definition.name {
                    return Ok((
                        self.prepare_reprojected_source_patches(Vec::new())?,
                        context.start,
                    ));
                }
                if context
                    .choices
                    .iter()
                    .any(|c| c.offset != definition.range.start && same_name(&c.name, name))
                {
                    return Err(DocumentError::UnsupportedFormatting.into());
                }
                if context.inactive {
                    return Err(DocumentError::UnsupportedFormatting.into());
                }
                patches.push(self.reference_patch(definition, definition.label.clone(), name)?);
                for used in index
                    .entries
                    .iter()
                    .filter(|e| !e.definition && e.target == Some(target))
                {
                    patches.push(self.reference_patch(
                        used,
                        used.label.end + 1..used.raw.len(),
                        &format!("[{name}]"),
                    )?);
                }
            }
            ReferenceRequest::ConvertInline { .. }
                if !entry.definition && entry.target.is_some() =>
            {
                patches.push(self.reference_patch(entry, 0..entry.raw.len(), &inline(entry))?)
            }
            ReferenceRequest::Remove { .. } if !entry.definition => {
                // Retain the label's authored inline formatting.
                patches.push(self.reference_patch(
                    entry,
                    0..entry.raw.len(),
                    &super::links::protect_unlinked_label(&entry.raw[entry.label.clone()]),
                )?);
            }
            ReferenceRequest::DeleteDefinition { .. } if entry.definition => {
                if !context.inactive {
                    for used in index
                        .entries
                        .iter()
                        .filter(|e| !e.definition && e.target == Some(own))
                    {
                        patches.push(self.reference_patch(
                            used,
                            0..used.raw.len(),
                            &inline(used),
                        )?);
                    }
                }
                patches.push(self.reference_patch(entry, 0..entry.raw.len(), "")?);
            }
            _ => return Err(DocumentError::UnsupportedFormatting.into()),
        }
        let mut prepared = self.prepare_reprojected_source_patches(patches)?;
        self.attach_prepared_source_map(&mut prepared)?;
        if prepared.is_no_op() {
            return Ok((prepared, context.start));
        }
        let candidate = self.prepared_candidate_document(&prepared)?;
        let after = candidate.reference_index()?;
        if !after.complete {
            return Err(DocumentError::UnsupportedFormatting.into());
        }
        let mapped_source = |entry: &Entry| -> Result<usize, DocumentError> {
            let map = prepared
                .text_position_map()
                .source_position_map()
                .ok_or(DocumentError::VerificationFailed)?;
            let point = self
                .source_point(entry.source.start)
                .map_err(|_| DocumentError::AmbiguousProjection)?;
            Ok(map
                .map_source_point(
                    point,
                    Association::BeforeInsertion,
                    DeletionRecovery::PreferFollowingThenPreceding,
                )
                .map_err(|_| DocumentError::AmbiguousProjection)?
                .value()
                .ok_or(DocumentError::AmbiguousProjection)?
                .offset())
        };
        if let Some((text, destination)) = expected_use {
            let source = mapped_source(entry)?;
            if !after.entries.iter().any(|e| {
                e.source.start == source
                    && !e.definition
                    && !e.image
                    && candidate.projection().text_tree().slice(e.range.clone()).is_ok_and(|value| value == text)
                    && e.destination == destination
            }) {
                return Err(DocumentError::VerificationFailed.into());
            }
        }
        if let Some((destination, title)) = expected_definition {
            let source = mapped_source(definition)?;
            if !after.entries.iter().any(|e| {
                e.source.start == source
                    && e.definition
                    && e.name == definition.name
                    && e.destination == destination
                    && e.title == title
            }) {
                return Err(DocumentError::VerificationFailed.into());
            }
        }
        if let ReferenceRequest::Rename { name, .. } = request {
            for old in index.entries.iter().filter(|e| {
                e.source == definition.source || !e.definition && e.target == Some(target)
            }) {
                let source = mapped_source(old)?;
                if !after.entries.iter().any(|e| {
                    e.source.start == source
                        && e.definition == old.definition
                        && e.image == old.image
                        && same_name(&e.name, name)
                        && (old.definition || e.text == old.text)
                        && e.destination == old.destination
                        && e.title == old.title
                }) {
                    return Err(DocumentError::VerificationFailed.into());
                }
            }
        }
        if let ReferenceRequest::EditDefinition {
            destination, title, ..
        } = request
        {
            if !context.inactive {
                for old in index
                    .entries
                    .iter()
                    .filter(|e| !e.definition && e.target == Some(target))
                {
                    let source = mapped_source(old)?;
                    if !after.entries.iter().any(|e| {
                        e.source.start == source
                            && e.image == old.image
                            && e.text == old.text
                            && e.destination == *destination
                            && e.title == *title
                    }) {
                        return Err(DocumentError::VerificationFailed.into());
                    }
                }
            }
        }
        let caret = prepared
            .text_position_map()
            .map_text_point(
                self.text_point(context.start)?,
                Association::BeforeInsertion,
                BoundaryAffinity::Downstream,
                DeletionRecovery::PreferFollowingThenPreceding,
            )
            .map_err(|_| DocumentError::AmbiguousProjection)?
            .value()
            .ok_or(DocumentError::AmbiguousProjection)?
            .offset();
        candidate.text_point(caret)?;
        Ok((prepared, caret))
    }
}

impl Document {
    /// A shortcut's label is also its lookup key. Preserve that key when visible
    /// typing changes the label by authoring an explicit reference suffix.
    pub(super) fn repair_reference_label_edits(&self, edits: &[TextEdit], patches: &mut Vec<SourcePatch>) -> Result<(), ModelTransactionError> {
        let mut handled = std::collections::HashSet::new();
        for edit in edits {
            for reference in self.projection().inline_references_for_region(&edit.range) {
                if !handled.insert(reference.source.start) { continue; }
                if edit.replacement.is_empty() && !reference.range.is_empty()
                    && edit.range.start <= reference.range.start && reference.range.end <= edit.range.end {
                    // Remove only uncovered delimiters; an existing source patch
                    // may already include one edge while crossing another owner.
                    for delimiter in [reference.source.start..reference.label_source.start, reference.label_source.end..reference.source.end] {
                        let mut remaining = vec![delimiter];
                        for patch in patches.iter() {
                            remaining = remaining.into_iter().flat_map(|range| {
                                let covered = patch.range();
                                if covered.end <= range.start || range.end <= covered.start { return vec![range]; }
                                let mut pieces = Vec::new();
                                if range.start < covered.start { pieces.push(range.start..covered.start); }
                                if covered.end < range.end { pieces.push(covered.end..range.end); }
                                pieces
                            }).collect();
                        }
                        patches.extend(remaining.into_iter().filter(|range| !range.is_empty()).map(|range| SourcePatch::primary(range, Vec::new())));
                    }
                    continue;
                }
                let label_touched = patches.iter().any(|p| p.range().start >= reference.label_source.start
                    && p.range().end <= reference.label_source.end);
                if !label_touched || patches.iter().any(|p| p.range().start <= reference.source.start && reference.source.end <= p.range().end) { continue; }
                let suffix = self.reference_raw(reference.label_source.end..reference.source.end)?;
                if suffix == "]" || suffix == "][]" {
                    let closing = self.encoding().encode_fragment("]")?.len();
                    let start = reference.label_source.end + closing;
                    if patches.iter().any(|p| p.range().start < reference.source.end && start < p.range().end) { continue; }
                    patches.push(SourcePatch::primary(start..reference.source.end, self.encoding().encode_fragment(&format!("[{}]", reference.name))?));
                }
            }
        }
        Ok(())
    }
}

impl Document {
    /// Enumerated only when the reference insertion editor opens.
    pub fn reference_choices(&self) -> Result<Vec<ReferenceChoice>, DocumentError> {
        let index = self.reference_index()?;
        if !index.complete { return Err(DocumentError::UnsupportedFormatting); }
        Ok(index.entries.iter().enumerate().filter(|(i, e)| e.definition && e.target == Some(*i))
            .map(|(_, e)| ReferenceChoice { name: e.name.clone(), destination: e.destination.clone(), offset: e.range.start }).collect())
    }

    fn prepare_reference_insertion(&self, range: Range<usize>, text: &str, name: &str, destination: &str, title: &str)
        -> Result<(PreparedModelTransaction, usize), ModelTransactionError> {
        if !valid_name(name) || text.is_empty() || text.contains(['\n', '\r']) || destination.chars().any(char::is_control)
            || title.chars().any(char::is_control) { return Err(DocumentError::UnsupportedFormatting.into()); }
        let index = self.reference_index()?;
        if !index.complete { return Err(DocumentError::UnsupportedFormatting.into()); }
        let definition = index.entries.iter().enumerate().find(|(i, e)| e.definition && e.target == Some(*i) && same_name(&e.name, name));
        let destination = definition.map(|(_, e)| e.destination.as_str()).unwrap_or(destination);
        if definition.is_none() && destination.is_empty() { return Err(DocumentError::UnsupportedFormatting.into()); }
        // Reuse inline-link insertion for selection styling, source boundaries and
        // punctuation repairs, then compose the reference suffix in one transaction.
        let (inline, caret) = self.prepare_link_edit(self.id(), self.revision(), LinkEditIntent::Insert {
            range: range.clone(), text: text.into(), destination: destination.into(),
        })?;
        let mut composed = super::transaction::PatchComposition::new(self.source_byte_len());
        for patch in inline.summary().source_patches().iter().rev() { composed.splice(patch.range(), patch.replacement()); }
        let mut scratch = self.prepared_candidate_document(&inline)?;
        let link = scratch.link_snapshot_at(scratch.text_point(range.start)?)?.ok_or(DocumentError::VerificationFailed)?;
        let mut patches = vec![SourcePatch::primary(link.label_source.end..link.source.end,
            self.encoding().encode_fragment(&format!("][{name}]"))?)];
        if definition.is_none() {
            let newline = self.file_format().spelling();
            let spelling = format!("{newline}{newline}[{name}]: <{}>{}", super::links::escape_destination(destination),
                if title.is_empty() { String::new() } else { format!(" {}", quoted(title)) });
            patches.push(SourcePatch::primary(scratch.source_byte_len()..scratch.source_byte_len(), self.encoding().encode_fragment(&spelling)?));
        }
        let converted = scratch.prepare_reprojected_source_patches(patches)?;
        for patch in converted.summary().source_patches().iter().rev() { composed.splice(patch.range(), patch.replacement()); }
        scratch.commit_model_transaction(converted)?;
        let mut prepared = self.prepare_reprojected_source_patches(composed.source_patches())?;
        self.attach_prepared_source_map(&mut prepared)?;
        let candidate = self.prepared_candidate_document(&prepared)?;
        let reference = candidate.reference_context_at(candidate.text_point(range.start)?)?.ok_or(DocumentError::VerificationFailed)?;
        if reference.definition || reference.text != text || reference.destination != destination || !same_name(&reference.name, name) {
            return Err(DocumentError::VerificationFailed.into());
        }
        Ok((prepared, caret))
    }
}
