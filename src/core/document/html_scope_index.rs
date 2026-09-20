//! Snapshot-local lexical HTML scopes. Semantic tree recovery remains the
//! projection's responsibility; these scopes deliberately match `stack_at`.
//!
//! Token ranges live in the persistent range tree, so a source insertion shifts
//! its suffix lazily. Contexts are parent-linked, immutable handles: a text token
//! retains two handles, not copies of all the opening syntax before it.
use super::html::{self, Tag, Token, TokenKind};
use super::line_endings::{self, NormalizedText};
use super::range_index::{OrderedRangeStore, RangeSpliceStats, RangedItem};
use super::source::SourceSnapshot;
use super::{Encoding, FileFormat, Revision, SourcePatch};
use std::ops::Range;
use std::sync::Arc;

#[derive(Debug)]
pub(super) struct HtmlScope {
    pub(super) tag: Tag,
    pub(super) opening: Arc<str>,
    parent: Option<Arc<HtmlScope>>,
}

impl HtmlScope {
    fn visit(scope: &Arc<Self>, visitor: &mut super::history_memory::MemoryVisitor<'_>) {
        visitor.arc(scope, |visitor| {
            visitor.arc(&scope.opening, |_| {});
            visitor.owned(
                Arc::as_ptr(scope) as usize,
                0,
                scope.tag.name.capacity()
                    + scope.tag.attributes.capacity() * std::mem::size_of::<(String, String)>()
                    + scope
                        .tag
                        .attributes
                        .iter()
                        .map(|(key, value)| key.capacity() + value.capacity())
                        .sum::<usize>(),
            );
            if let Some(parent) = &scope.parent {
                Self::visit(parent, visitor);
            }
        });
    }
}

impl Drop for HtmlScope {
    fn drop(&mut self) {
        let mut parent = self.parent.take();
        while let Some(scope) = parent {
            match Arc::try_unwrap(scope) {
                Ok(mut scope) => parent = scope.parent.take(),
                Err(_) => break,
            }
        }
    }
}

#[derive(Clone, Debug)]
enum Kind {
    Text,
    /// A literal less-than token could combine with an adjacent edit into a tag.
    UnsafeText,
    /// Opening metadata already belongs to its scope; a closing token needs
    /// only its name. Large attribute strings are never duplicated here.
    Tag {
        closing: Option<Arc<str>>,
        anchor: bool,
    },
    Opaque,
}

#[derive(Clone, Debug)]
struct Entry {
    range: Range<usize>,
    kind: Kind,
    before: Option<Arc<HtmlScope>>,
    after: Option<Arc<HtmlScope>>,
    link_before: Option<Arc<str>>,
    link_after: Option<Arc<str>>,
}

impl RangedItem for Entry {
    fn range(&self) -> &Range<usize> {
        &self.range
    }
    fn with_range(&self, range: Range<usize>) -> Self {
        Self {
            range,
            ..self.clone()
        }
    }
    fn visit_shared_memory(&self, visitor: &mut super::history_memory::MemoryVisitor<'_>) {
        if let Kind::Tag {
            closing: Some(name),
            ..
        } = &self.kind
        {
            visitor.arc(name, |_| {});
        }
        if let Some(scope) = &self.before {
            HtmlScope::visit(scope, visitor);
        }
        if let Some(scope) = &self.after {
            HtmlScope::visit(scope, visitor);
        }
        if let Some(link) = &self.link_before {
            visitor.arc(link, |_| {});
        }
        if let Some(link) = &self.link_after {
            visitor.arc(link, |_| {});
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct HtmlScopeIndex {
    revision: Revision,
    entries: OrderedRangeStore<Entry>,
}

fn same_context(a: &Option<Arc<HtmlScope>>, b: &Option<Arc<HtmlScope>>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => Arc::ptr_eq(a, b),
        _ => false,
    }
}

fn equivalent_context(mut a: Option<&Arc<HtmlScope>>, mut b: Option<&Arc<HtmlScope>>) -> bool {
    loop {
        match (a, b) {
            (None, None) => return true,
            (Some(left), Some(right)) => {
                if Arc::ptr_eq(left, right) {
                    return true;
                }
                // Reopening the same exact delimiter can create a new handle
                // while leaving the retained suffix's lexical state unchanged.
                if left.opening != right.opening {
                    return false;
                }
                a = left.parent.as_ref();
                b = right.parent.as_ref();
            }
            _ => return false,
        }
    }
}

fn transition(
    mut context: Option<Arc<HtmlScope>>,
    tag: &Tag,
    syntax: &str,
) -> Option<Arc<HtmlScope>> {
    if tag.end || super::html_paragraph::structural(&tag.name) {
        let mut ancestor = context.clone();
        while let Some(scope) = ancestor {
            if if tag.end {
                scope.tag.name == tag.name
            } else {
                html::heading_or_paragraph(&scope.tag.name)
            } {
                context = scope.parent.clone();
                break;
            }
            ancestor = scope.parent.clone();
        }
    }
    if !tag.end && !html::void(&tag.name) {
        super::work_statistics::record(|stats| {
            stats.html_scope_entries_copied += 1;
            stats.html_scope_syntax_bytes_copied += syntax.len();
        });
        context = Some(Arc::new(HtmlScope {
            tag: tag.clone(),
            opening: Arc::from(syntax),
            parent: context,
        }));
    }
    context
}

impl HtmlScopeIndex {
    pub(super) fn revision(&self) -> Revision {
        self.revision
    }

    pub(super) fn with_revision(&self, revision: Revision) -> Self {
        Self {
            revision,
            entries: self.entries.clone(),
        }
    }

    pub(super) fn from_tokens(
        input: &NormalizedText,
        revision: Revision,
        tokens: &[Token],
    ) -> Self {
        let (entries, _, _) = Self::entries(input, revision, tokens, None, None);
        Self {
            revision,
            entries: OrderedRangeStore::new(entries),
        }
    }

    fn entries(
        input: &NormalizedText,
        revision: Revision,
        tokens: &[Token],
        mut context: Option<Arc<HtmlScope>>,
        mut link: Option<Arc<str>>,
    ) -> (Vec<Entry>, Option<Arc<HtmlScope>>, Option<Arc<str>>) {
        let mapper = super::rich_text::Builder::new(input, revision);
        let mut entries = Vec::with_capacity(tokens.len());
        for token in tokens {
            let before = context.clone();
            let link_before = link.clone();
            let syntax = &input.text[token.range.clone()];
            let kind = match &token.kind {
                TokenKind::Tag(tag) => {
                    context = transition(context, tag, syntax);
                    if tag.name == "a" {
                        link = (!tag.end)
                            .then(|| tag.attribute("href").map(Arc::from))
                            .flatten();
                    }
                    Kind::Tag {
                        closing: tag.end.then(|| Arc::from(tag.name.as_str())),
                        anchor: tag.name == "a",
                    }
                }
                TokenKind::Text if !syntax.contains('<') => Kind::Text,
                TokenKind::Text => Kind::UnsafeText,
                _ => Kind::Opaque,
            };
            entries.push(Entry {
                range: mapper.source_range(token.range.clone()),
                kind,
                before,
                after: context.clone(),
                link_before,
                link_after: link.clone(),
            });
        }
        super::work_statistics::record(|stats| stats.html_index_entries_rebuilt += entries.len());
        (entries, context, link)
    }

    fn entry_at(&self, at: usize) -> Option<(usize, Entry)> {
        let (index, work) = self.entries.index_touching_point_with_stats(at);
        super::work_statistics::record(|stats| {
            stats.html_index_nodes_visited += work.nodes_visited
        });
        let index = index?;
        let (entry, work) = self.entries.get_with_stats(index);
        super::work_statistics::record(|stats| {
            stats.html_index_nodes_visited += work.nodes_visited
        });
        let entry = entry?;
        Some((index, entry))
    }

    fn context_at(&self, at: usize) -> Option<Arc<HtmlScope>> {
        self.entry_at(at).and_then(|(_, entry)| {
            if entry.range.end <= at {
                entry.after
            } else {
                entry.before
            }
        })
    }

    /// Offsets are used only against this immutable projection's source
    /// revision. A returned scope has no mutable/global source coordinate.
    pub(super) fn scopes_at(&self, source_offset: usize) -> Vec<Arc<HtmlScope>> {
        super::work_statistics::record(|stats| stats.html_scope_queries += 1);
        let mut scope = self.context_at(source_offset);
        let mut result = Vec::new();
        while let Some(current) = scope {
            scope = current.parent.clone();
            result.push(current);
        }
        super::work_statistics::record(|stats| stats.html_scope_entries_visited += result.len());
        result.reverse();
        result
    }

    pub(super) fn adjacent_closing_at(&self, source_offset: usize) -> Option<(String, usize)> {
        let (_, entry) = self.entry_at(source_offset)?;
        match entry.kind {
            Kind::Tag {
                closing: Some(name),
                ..
            } if entry.range.start == source_offset => Some((name.to_string(), entry.range.end)),
            _ => None,
        }
    }

    /// Anchor destination spelling follows the existing lexical link resolver,
    /// independently of paragraph recovery and the stack of open elements.
    pub(super) fn link_at_source(&self, source_offset: usize) -> Option<String> {
        let (_, entry) = self.entry_at(source_offset)?;
        if entry.range.end <= source_offset || matches!(entry.kind, Kind::Tag { anchor: true, .. })
        {
            return None;
        }
        entry.link_before.as_ref().map(|link| link.to_string())
    }

    pub(super) fn visit_retained_memory(
        &self,
        visitor: &mut super::history_memory::MemoryVisitor<'_>,
    ) {
        let before = visitor.retained_bytes();
        self.entries.visit_retained_memory(visitor);
        super::work_statistics::record(|stats| {
            stats.html_index_newly_retained_bytes = stats
                .html_index_newly_retained_bytes
                .max(visitor.retained_bytes() - before)
        });
    }

    /// Rebuild only the changed lexical region when its entry and exit scopes
    /// agree with the untouched tree. Non-converging structural/recovery edits
    /// deliberately use the full projector's ordinary rebuild instead.
    pub(super) fn updated(
        &self,
        source: &SourceSnapshot,
        revision: Revision,
        encoding: Encoding,
        file_format: FileFormat,
        patches: &[SourcePatch],
    ) -> Option<Self> {
        if patches.is_empty() {
            return Some(self.with_revision(revision));
        }
        let first = patches.first()?.range();
        let last = patches.last()?.range();
        let selected = first.start..last.end;
        let (mut first_index, mut first_entry) = self.entry_at(selected.start)?;
        if selected.start == first_entry.range.start
            && first_index > 0
            && matches!(self.entries.get(first_index - 1)?.kind, Kind::UnsafeText)
        {
            return None;
        }
        if selected.is_empty() && selected.start == first_entry.range.start && first_index > 0 {
            let previous = self.entries.get(first_index - 1)?;
            if matches!(previous.kind, Kind::Text) && previous.range.end == selected.start {
                first_index -= 1;
                first_entry = previous;
            }
        }
        let (mut last_index, mut last_entry) = self.entry_at(selected.end)?;
        if selected.is_empty()
            && selected.start == first_entry.range.end
            && matches!(first_entry.kind, Kind::Text)
        {
            last_index = first_index;
            last_entry = first_entry.clone();
        }
        let insert_before = selected.is_empty() && selected.start == first_entry.range.start;
        // A half-open edit ending at a new token's start leaves that token alone.
        if selected.end > selected.start
            && last_entry.range.start == selected.end
            && last_index > first_index
        {
            last_index -= 1;
            last_entry = self.entries.get(last_index)?;
        }
        let old_start = if insert_before || matches!(first_entry.kind, Kind::Text) {
            selected.start
        } else {
            first_entry.range.start
        };
        let old_end = if insert_before || matches!(last_entry.kind, Kind::Text) {
            selected.end
        } else {
            last_entry.range.end
        };
        let shift = patches.iter().try_fold(0i128, |sum, patch| {
            sum.checked_add(patch.replacement().len() as i128 - patch.range().len() as i128)
        })?;
        let new_end = usize::try_from(old_end as i128 + shift).ok()?;
        // Raw-text tokenizer state cannot be resumed by a lexical open stack.
        let before = if old_start < first_entry.range.end {
            first_entry.before.clone()
        } else {
            first_entry.after.clone()
        };
        let after = if old_end < last_entry.range.end {
            last_entry.before.clone()
        } else {
            last_entry.after.clone()
        };
        let link_before = if old_start < first_entry.range.end {
            first_entry.link_before.clone()
        } else {
            first_entry.link_after.clone()
        };
        let link_after = if old_end < last_entry.range.end {
            last_entry.link_before.clone()
        } else {
            last_entry.link_after.clone()
        };
        if !insert_before
            && (matches!(first_entry.kind, Kind::Opaque | Kind::UnsafeText)
                || matches!(last_entry.kind, Kind::Opaque | Kind::UnsafeText))
        {
            return None;
        }
        let mut ancestor = before.clone();
        while let Some(scope) = ancestor {
            if matches!(
                scope.tag.name.as_str(),
                "script"
                    | "style"
                    | "title"
                    | "textarea"
                    | "xmp"
                    | "iframe"
                    | "noembed"
                    | "noframes"
            ) {
                return None;
            }
            ancestor = scope.parent.clone();
        }
        let bytes = source.bytes_in(old_start..new_end)?;
        let decoded = encoding.decode_region(&bytes, old_start).ok()?;
        let input = line_endings::normalize(&decoded, file_format);
        let tokens = html::tokenize(&input.text);
        if tokens.iter().any(|token| {
            matches!(token.kind, TokenKind::Opaque)
                || matches!(token.kind, TokenKind::Text)
                    && input.text[token.range.clone()].contains('<')
        }) {
            return None;
        }
        let (mut replacement, exit, link_exit) =
            Self::entries(&input, revision, &tokens, before.clone(), link_before);
        if !equivalent_context(exit.as_ref(), after.as_ref()) || link_exit != link_after {
            return None;
        }
        if first_entry.range.start < old_start {
            replacement.insert(
                0,
                first_entry.with_range(first_entry.range.start..old_start),
            );
        }
        if !insert_before && old_end < last_entry.range.end {
            replacement.push(
                last_entry.with_range(
                    new_end..usize::try_from(last_entry.range.end as i128 + shift).ok()?,
                ),
            );
        }
        // Coalesce plain lexical runs, so continued typing does not retain one
        // index item per keystroke inside an otherwise enormous text token.
        let mut merged: Vec<Entry> = Vec::with_capacity(replacement.len());
        for entry in replacement {
            if let Some(previous) = merged.last_mut().filter(|previous| {
                matches!(previous.kind, Kind::Text)
                    && matches!(entry.kind, Kind::Text)
                    && previous.range.end == entry.range.start
                    && same_context(&previous.after, &entry.before)
            }) {
                previous.range.end = entry.range.end;
                previous.after = entry.after;
                previous.link_after = entry.link_after;
            } else {
                merged.push(entry);
            }
        }
        let mut work = RangeSpliceStats::default();
        let remove_end = if insert_before {
            first_index
        } else {
            last_index + 1
        };
        let entries =
            self.entries
                .splice(first_index..remove_end, merged, old_end, new_end, &mut work)?;
        super::work_statistics::record(|stats| {
            stats.html_index_nodes_visited += work.nodes_visited;
            stats.html_index_nodes_copied += work.nodes_copied;
        });
        Some(Self { revision, entries })
    }
}

#[cfg(test)]
mod tests {
    use super::super::{measure_document_work, Document, Format};
    use super::*;

    fn normalized(source: &[u8], encoding: Encoding) -> NormalizedText {
        line_endings::normalize(&encoding.decode(source).unwrap(), FileFormat::Unix)
    }

    fn assert_oracle(index: &HtmlScopeIndex, input: &NormalizedText) {
        let tokens = html::tokenize(&input.text);
        let links = super::super::links::html_links(&input.text);
        let mapper = super::super::rich_text::Builder::new(input, index.revision);
        for at in input
            .text
            .char_indices()
            .map(|(at, _)| at)
            .chain(std::iter::once(input.text.len()))
        {
            let source_at = mapper.source_range(at..at).start;
            let expected = super::super::html_paragraph::stack_at(&tokens, at)
                .into_iter()
                .map(|token| {
                    let TokenKind::Tag(tag) = &token.kind else {
                        unreachable!()
                    };
                    (
                        tag.name.clone(),
                        tag.attributes.clone(),
                        input.text[token.range.clone()].to_owned(),
                    )
                })
                .collect::<Vec<_>>();
            let actual = index
                .scopes_at(source_at)
                .into_iter()
                .map(|scope| {
                    (
                        scope.tag.name.clone(),
                        scope.tag.attributes.clone(),
                        scope.opening.to_string(),
                    )
                })
                .collect::<Vec<_>>();
            assert_eq!(
                actual, expected,
                "scope at {source_at} ({at}) in {:?}",
                input.text
            );
            let closing = tokens.iter().find_map(|token| match &token.kind {
                TokenKind::Tag(tag) if tag.end && token.range.start == at => Some((
                    tag.name.clone(),
                    mapper.source_range(token.range.clone()).end,
                )),
                _ => None,
            });
            assert_eq!(index.adjacent_closing_at(source_at), closing);
            let link = links
                .iter()
                .find(|link| link.label.contains(&at))
                .map(|link| link.destination.clone());
            assert_eq!(
                index.link_at_source(source_at),
                link,
                "link at {source_at} in {:?}",
                input.text
            );
        }
    }

    #[test]
    fn lexical_scope_index_matches_original_stack_for_recovery_and_encodings() {
        for source in [
            "<HTML><BODY><p a='1' a=2>A<b>B<i>C</b>D</i></p></BODY></HTML>",
            "<p>A<b>B<p>C</p>D<div>E<h2>F<section>G</section></div>",
            "<ul><li>A<li><a href='u&amp;v'>B</a><ul><li>C</ul></ul>",
            "<p>a<table><tr><td>b</td></tr>c</table>d</p>",
            "<p>a<script>'<b>opaque</b>'</script><!--<p>--><img src='x'>z</p>",
            "<p>a &amp; é < invalid <span class='unterminated",
            "<p></p><pre>\n A\n B</pre>",
            "<a href='outer'>a<a href=inner>b</a>c</a><a>d</a><a href='e'>e<p>f</p>g",
        ] {
            for encoding in [
                Encoding::Utf8,
                Encoding::Latin1,
                Encoding::Utf16Le,
                Encoding::Utf16Be,
            ] {
                let bytes = encoding.encode_fragment(source).unwrap();
                let input = normalized(&bytes, encoding);
                let index =
                    HtmlScopeIndex::from_tokens(&input, Revision(1), &html::tokenize(&input.text));
                assert_oracle(&index, &input);
            }
        }
    }

    #[test]
    fn bounded_updates_reuse_suffix_scopes_and_match_fresh_lexical_parse() {
        let mut source = SourceSnapshot::new(
            format!(
                "<body>{}<p>A<b>BC</b>D</p><p>tail</p></body>",
                "<p>unrelated</p>".repeat(10_000)
            )
            .into_bytes(),
        );
        let input = normalized(&source.bytes(), Encoding::Utf8);
        let mut index =
            HtmlScopeIndex::from_tokens(&input, Revision(1), &html::tokenize(&input.text));
        let tail = source.len() - "tail</p></body>".len();
        let original_scope = index.scopes_at(tail).pop().unwrap();
        let original = index.clone();
        for (revision, old, replacement) in [
            (2, "BC", "B&amp;C"),
            (3, "B&amp;C", "B<span class='new'>X</span>C"),
            (4, "<span class='new'>X</span>", "Q"),
            (5, "BC", "unused"),
        ] {
            let bytes = source.bytes();
            let text = std::str::from_utf8(&bytes).unwrap();
            let Some(at) = text.rfind(old) else { continue };
            let patch = SourcePatch::primary(at..at + old.len(), replacement.as_bytes().to_vec());
            source = source
                .replace(at, at + old.len(), replacement.as_bytes().to_vec())
                .unwrap();
            let (updated, work) = measure_document_work(|| {
                index.updated(
                    &source,
                    Revision(revision),
                    Encoding::Utf8,
                    FileFormat::Unix,
                    &[patch],
                )
            });
            index = updated.expect("balanced local wrapper edits converge");
            assert!(work.source_range_materialized_bytes < 256, "{work:?}");
            assert!(work.html_index_nodes_copied < 128, "{work:?}");
            assert!(index.entries.shared_leaf_count_with(&original.entries) > 400);
            let tail = source.len() - "tail</p></body>".len();
            assert!(Arc::ptr_eq(
                &original_scope,
                &index.scopes_at(tail).pop().unwrap()
            ));
            let input = normalized(&source.bytes(), Encoding::Utf8);
            let fresh =
                HtmlScopeIndex::from_tokens(&input, index.revision, &html::tokenize(&input.text));
            for at in source.len() - 100..source.len() {
                assert_eq!(
                    index
                        .scopes_at(at)
                        .iter()
                        .map(|scope| scope.opening.as_ref())
                        .collect::<Vec<_>>(),
                    fresh
                        .scopes_at(at)
                        .iter()
                        .map(|scope| scope.opening.as_ref())
                        .collect::<Vec<_>>()
                );
                assert_eq!(index.adjacent_closing_at(at), fresh.adjacent_closing_at(at));
            }
        }
        let (_, work) = measure_document_work(|| index.scopes_at(source.len() - 20));
        assert!(work.html_index_nodes_visited < 40, "{work:?}");
        assert!(work.html_scope_entries_visited < 4, "{work:?}");
    }

    #[test]
    fn insertion_edges_and_huge_text_do_not_reparse_untouched_prefixes() {
        for source_text in [
            "<p></p>".to_owned(),
            format!("<p><b>{}</b></p>", "x".repeat(2_000_000)),
        ] {
            let mut source = SourceSnapshot::new(source_text.into_bytes());
            let input = normalized(&source.bytes(), Encoding::Utf8);
            let mut index =
                HtmlScopeIndex::from_tokens(&input, Revision(1), &html::tokenize(&input.text));
            let at = input
                .text
                .find("</b>")
                .or_else(|| input.text.find("</p>"))
                .unwrap();
            for step in 0..25 {
                let at = at + step;
                let patch = SourcePatch::primary(at..at, b"y".to_vec());
                source = source.replace(at, at, b"y".to_vec()).unwrap();
                let (updated, work) = measure_document_work(|| {
                    index.updated(
                        &source,
                        Revision(step as u64 + 2),
                        Encoding::Utf8,
                        FileFormat::Unix,
                        &[patch],
                    )
                });
                index = updated.unwrap();
                assert_eq!(work.html_tokenized_bytes, 1);
                assert_eq!(work.source_decoded_bytes, 1);
                assert_eq!(
                    index.scopes_at(at).last().unwrap().tag.name,
                    if input.text.contains("<b>") { "b" } else { "p" }
                );
            }
            assert!(
                index.entries.len() < 10,
                "continued typing must coalesce adjacent text entries"
            );
        }
    }

    #[test]
    fn document_history_and_failed_edits_retain_matching_scope_snapshot() {
        let mut doc = Document::from_bytes(
            b"<p>a<b>BC</b>d</p><p>tail</p>".to_vec(),
            Encoding::Utf8,
            Format::Html,
        )
        .unwrap();
        let before = doc.projection().html_scope_index().unwrap().clone();
        doc.replace(2..3, "hello").unwrap();
        let current = doc.projection().html_scope_index().unwrap();
        assert_eq!(current.revision(), doc.revision());
        assert_oracle(current, &normalized(&doc.source_bytes(), doc.encoding()));
        assert!(doc.replace(usize::MAX..usize::MAX, "bad").is_err());
        assert_eq!(
            doc.projection().html_scope_index().unwrap().revision(),
            doc.revision()
        );
        assert!(doc.undo());
        assert!(doc
            .projection()
            .html_scope_index()
            .unwrap()
            .entries
            .shares_root_with(&before.entries));
        assert!(doc.redo());
        assert_oracle(
            doc.projection().html_scope_index().unwrap(),
            &normalized(&doc.source_bytes(), doc.encoding()),
        );
    }

    #[test]
    fn bounded_maintenance_accepts_only_lexically_converged_source_edits() {
        for text in [
            "<p><a href='outer'>a<a href='inner'>b</a>c</a>d</p>",
            "< x<p>a<b>b</b>c</p>",
            "<p>a<script>x</script>b</p>",
        ] {
            let source = SourceSnapshot::new(text.as_bytes().to_vec());
            let input = normalized(text.as_bytes(), Encoding::Utf8);
            let original =
                HtmlScopeIndex::from_tokens(&input, Revision(1), &html::tokenize(&input.text));
            for at in 0..=text.len() {
                for length in 0..=3.min(text.len() - at) {
                    for replacement in ["", "x", "b>", "<i>x</i>", "<a href='new'>x</a>", "</p>"] {
                        let patch =
                            SourcePatch::primary(at..at + length, replacement.as_bytes().to_vec());
                        let source = source
                            .replace(at, at + length, replacement.as_bytes().to_vec())
                            .unwrap();
                        if let Some(index) = original.updated(
                            &source,
                            Revision(2),
                            Encoding::Utf8,
                            FileFormat::Unix,
                            &[patch],
                        ) {
                            assert_oracle(&index, &normalized(&source.bytes(), Encoding::Utf8));
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn deeply_unclosed_scope_handles_drop_without_recursive_stack_growth() {
        let mut scope = None;
        for _ in 0..50_000 {
            scope = Some(Arc::new(HtmlScope {
                tag: Tag {
                    name: "b".to_owned(),
                    end: false,
                    attributes: Vec::new(),
                },
                opening: Arc::from("<b>"),
                parent: scope,
            }));
        }
        drop(scope);
    }

    #[test]
    fn deeply_unclosed_index_releases_retained_memory_without_stack_growth() {
        let source = "<b>".repeat(50_000);
        let input = normalized(source.as_bytes(), Encoding::Utf8);
        let index = HtmlScopeIndex::from_tokens(&input, Revision(1), &html::tokenize(&input.text));
        let mut retained = super::super::history_memory::RetainedMemory::default();
        let roots = retained.capture(|visitor| index.visit_retained_memory(visitor));
        assert!(retained.allocation_count() >= 50_000);
        retained.release(roots);
        assert_eq!(retained.allocation_count(), 0);
        assert_eq!(retained.bytes(), 0);
    }

    #[test]
    fn history_memory_tracks_shared_index_paths_and_releases_pruned_snapshots() {
        let source = SourceSnapshot::new(
            format!("<body>{}</body>", "<p><b>body</b></p>".repeat(10_000)).into_bytes(),
        );
        let input = normalized(&source.bytes(), Encoding::Utf8);
        let before = HtmlScopeIndex::from_tokens(&input, Revision(1), &html::tokenize(&input.text));
        let at = "<body>".len() + 5_000 * "<p><b>body</b></p>".len() + "<p><b>".len();
        let patch = SourcePatch::primary(at..at + 1, b"updated".to_vec());
        let changed_source = source.replace(at, at + 1, b"updated".to_vec()).unwrap();
        let after = before
            .updated(
                &changed_source,
                Revision(2),
                Encoding::Utf8,
                FileFormat::Unix,
                &[patch],
            )
            .unwrap();
        let mut retained = super::super::history_memory::RetainedMemory::default();
        let old_roots = retained.capture(|visitor| before.visit_retained_memory(visitor));
        let old_bytes = retained.bytes();
        let new_roots = retained.capture(|visitor| after.visit_retained_memory(visitor));
        assert!(
            retained.bytes() - old_bytes < 100_000,
            "one edit must not retain another full scope index"
        );
        retained.release(old_roots);
        let mut expected = super::super::history_memory::RetainedMemory::default();
        let _ = expected.capture(|visitor| after.visit_retained_memory(visitor));
        retained.assert_same_allocations(&expected);
        retained.release(new_roots);
        assert_eq!(retained.allocation_count(), 0);
        assert_eq!(retained.bytes(), 0);
    }
}
