//! Minimal source edits inside Markdown's passive HTML blocks. Entity expansion
//! and visible-run ownership use the same provenance resolver as other edits;
//! there is no standalone HTML editing document or HTML-mode transaction.
use super::*;

pub(super) fn literal_html(text: &str, encoding: super::super::Encoding) -> String {
    let mut output = String::new();
    for ch in text.chars() {
        match ch {
            '&' => output.push_str("&amp;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            ' ' => output.push_str("&#32;"),
            '\t' => output.push_str("&#9;"),
            '\n' => output.push_str("<br>"),
            '\r' => output.push_str("&#13;"),
            ch if encoding.encode_fragment(&ch.to_string()).is_err() => {
                output.push_str(&format!("&#{};", ch as u32));
            }
            ch => output.push(ch),
        }
    }
    output
}

impl Document {
    pub(super) fn prepare_markdown_html_text_edits(
        &self,
        edits: &[TextEdit],
    ) -> Result<Option<PreparedModelTransaction>, ModelTransactionError> {
        if self.format() != Format::Markdown
            || !edits.iter().any(|edit| {
                self.projection()
                    .blocks_for_region(&edit.range)
                    .iter()
                    .any(|block| block.markdown_html)
            })
        {
            return Ok(None);
        }
        // Comments and literal reference syntax retain their exact spelling.
        // Their ordinary Markdown edit path already knows that policy.
        if edits.iter().all(|edit| self.projection().style_spans_for_region(&edit.range).iter().any(|span| {
            span.range.start <= edit.range.start && edit.range.end <= span.range.end
                && matches!(&span.application, StyleApplication::Automatic(id) if matches!(id.0.as_str(), "Comment" | "Markdown reference"))
        })) { return Ok(None); }
        let decoded = self.encoding().decode(&self.source_bytes())?;
        let input = normalize(&decoded, self.file_format());
        let syntax = super::super::markdown_syntax::Blocks::parse(&input.text).to_source(&input);
        let Some(first) = edits.first() else {
            return Ok(None);
        };
        let at =
            super::super::source_edit::insertion_point(self.projection(), first.range.start, None)
                .ok_or(DocumentError::AmbiguousProjection)?;
        let Some(block) = syntax.blocks.iter().find(|block| {
            matches!(block.role, super::super::markdown_syntax::BlockRole::Html)
                && block.range.start <= at
                && at <= block.range.end
        }) else {
            return Ok(None);
        };
        let contributors = self
            .projection()
            .provenance_contained_in_source(&block.range);
        let Some(start) = contributors.iter().map(|span| span.formatted.start).min() else {
            return Ok(None);
        };
        let Some(end) = contributors
            .iter()
            .filter(|span| {
                !input.endings.iter().any(|ending| {
                    ending.source == span.source && ending.source.end == block.range.end
                })
            })
            .map(|span| span.formatted.end)
            .max()
        else {
            return Ok(None);
        };
        let contained = edits
            .iter()
            .all(|edit| start <= edit.range.start && edit.range.end <= end);
        let mut contributor_patches = None;
        if contained {
            let patches = (|| -> Result<Vec<SourcePatch>, DocumentError> {
                let mut groups: Vec<(Range<usize>, Vec<&TextEdit>)> = Vec::new();
                for edit in edits {
                    let range =
                        super::super::source_edit::complete_contributors(self.projection(), edit)?
                            .range;
                    if let Some((previous, members)) = groups
                        .last_mut()
                        .filter(|(previous, _)| range.start < previous.end)
                    {
                        previous.end = previous.end.max(range.end);
                        members.push(edit);
                    } else {
                        groups.push((range, vec![edit]));
                    }
                }
                let mut patches = Vec::new();
                for (range, members) in groups {
                    let mut replacement = self.text()[range.clone()].to_owned();
                    for edit in members.into_iter().rev() {
                        replacement.replace_range(
                            edit.range.start - range.start..edit.range.end - range.start,
                            &edit.replacement,
                        );
                    }
                    let expanded = TextEdit::new(range, replacement);
                    let runs = if expanded.range.is_empty() {
                        let at = super::super::source_edit::insertion_point(
                            self.projection(),
                            expanded.range.start,
                            None,
                        )
                        .ok_or(DocumentError::AmbiguousProjection)?;
                        vec![at..at]
                    } else {
                        super::super::source_edit::visible_runs(self.projection(), &expanded.range)?
                            .into_iter()
                            .map(|run| run.source)
                            .collect()
                    };
                    let replacement = self
                        .encoding()
                        .encode_fragment(&literal_html(&expanded.replacement, self.encoding()))?;
                    for (index, range) in runs.into_iter().enumerate() {
                        patches.push(SourcePatch::primary(
                            range,
                            if index == 0 {
                                replacement.clone()
                            } else {
                                Vec::new()
                            },
                        ));
                    }
                }
                Ok(patches)
            })();
            if let Ok(patches) = patches {
                if let Ok(prepared) =
                    self.prepare_text_edits_with_patches(edits.to_vec(), Some(patches.clone()))
                {
                    return Ok(Some(prepared));
                }
                if edits.len() == 1
                    && edits[0].replacement.is_empty()
                    && self
                        .projection()
                        .blocks_for_region(&edits[0].range)
                        .iter()
                        .any(|block| block.markdown_html && block.range == edits[0].range)
                {
                    if let Some(first) = patches
                        .iter()
                        .position(|patch| patch.replacement.is_empty() && !patch.range.is_empty())
                    {
                        for syntax in ["<span></span>", "<p></p>"] {
                            let mut retained = patches.clone();
                            retained[first].replacement =
                                self.encoding().encode_fragment(syntax)?;
                            if let Ok(prepared) =
                                self.prepare_text_edits_with_patches(edits.to_vec(), Some(retained))
                            {
                                return Ok(Some(prepared));
                            }
                        }
                    }
                }
                contributor_patches = Some(patches);
            }
        }
        // Crossing out of a passive block keeps the existing Markdown policy:
        // expose that block as equivalent Markdown before joining its neighbor.
        // This conversion is never used for a contained edit.
        if !contained {
            let source = self
                .state()
                .source
                .bytes_in(block.range.clone())
                .ok_or(DocumentError::AmbiguousProjection)?;
            let scratch = Document::from_bytes_with_file_format(
                source,
                self.encoding(),
                Format::Markdown,
                self.file_format(),
            )?;
            if scratch.text() != &self.text()[start..end] {
                return Ok(None);
            }
            let converted = super::super::markdown_serialization::markdown_document(scratch.projection())
                .replace('\n', self.file_format().spelling());
            let mut replacement = self.encoding().encode_fragment(&converted)?;
            let ending = self
                .encoding()
                .encode_fragment(self.file_format().spelling())?;
            if input
                .endings
                .iter()
                .any(|ending| ending.source.end == block.range.end)
                && !replacement.ends_with(&ending)
            {
                replacement.extend(ending);
            }
            if replacement == scratch.source_bytes() {
                return Ok(None);
            }
            return self
                .prepare_markdown_supporting_patches(
                    vec![SourcePatch::primary(block.range.clone(), replacement)],
                    |doc| doc.prepare_text_edits(edits.to_vec()),
                )
                .map(Some);
        }
        // Add only the selected paragraph boundary's structural patches to
        // the ordinary visible contributors, then verify the actual edit once.
        // Enclosing attributes and unrelated syntax never enter a serializer.
        let Some(mut patches) = contributor_patches else {
            return Ok(None);
        };
        let Some(boundaries) = self.passive_html_boundary_patches(edits, &input)? else {
            return Ok(None);
        };
        patches.extend(boundaries);
        patches.sort_by_key(|patch| (patch.range.start, patch.range.end));
        let mut combined: Vec<SourcePatch> = Vec::new();
        for patch in patches {
            if let Some(previous) = combined.last_mut().filter(|previous| {
                patch.range.start < previous.range.end || patch.range == previous.range
            }) {
                if !previous.replacement.is_empty() && !patch.replacement.is_empty() {
                    return Ok(None);
                }
                previous.range.end = previous.range.end.max(patch.range.end);
                if previous.replacement.is_empty() {
                    previous.replacement = patch.replacement;
                }
            } else {
                combined.push(patch);
            }
        }
        self.prepare_text_edits_with_patches(edits.to_vec(), Some(combined))
            .map(Some)
    }

    fn passive_html_boundary_patches(
        &self,
        edits: &[TextEdit],
        input: &super::super::line_endings::NormalizedText,
    ) -> Result<Option<Vec<SourcePatch>>, ModelTransactionError> {
        use super::super::html::{self, TokenKind};
        let converter = super::super::rich_text::Builder::new(input, self.revision());
        let tokens = html::tokenize(&input.text);
        let mut active = Vec::new();
        let mut hidden: Vec<&str> = Vec::new();
        for token in &tokens {
            if let TokenKind::Tag(tag) = &token.kind {
                if tag.end {
                    if let Some(index) = hidden.iter().rposition(|name| *name == tag.name) {
                        hidden.truncate(index);
                        continue;
                    }
                } else if html::hidden(&tag.name) {
                    hidden.push(&tag.name);
                    continue;
                }
            }
            if hidden.is_empty() {
                active.push((token, converter.source_range(token.range.clone())));
            }
        }
        let mut paired = BTreeMap::new();
        let mut stack = Vec::new();
        for (index, (token, _)) in active.iter().enumerate() {
            let TokenKind::Tag(tag) = &token.kind else {
                continue;
            };
            if tag.end {
                if let Some(at) = stack.iter().rposition(|&open: &usize| matches!(&active[open].0.kind, TokenKind::Tag(open) if open.name == tag.name)) {
                    let opening = stack[at];
                    paired.insert(active[opening].0.range.start, index);
                    paired.insert(token.range.start, opening);
                    stack.truncate(at);
                }
            } else if !html::void(&tag.name) {
                stack.push(index);
            }
        }
        struct Merge {
            first_open: usize,
            final_close: usize,
            owner: String,
            closing_wrappers: Vec<Range<usize>>,
            opening_wrappers: Vec<Range<usize>>,
        }
        // Key each connected merge by its current rightmost paragraph opener.
        // A later adjacent boundary extends it before any retained closer or
        // neutral wrapper insertion is emitted.
        let mut merges: BTreeMap<usize, Merge> = BTreeMap::new();
        let mut patches = Vec::new();
        for edit in edits {
            for (offset, character) in self.text()[edit.range.clone()].char_indices() {
                if character != '\n' {
                    continue;
                }
                let at = edit.range.start + offset;
                let preceding = super::super::source_edit::insertion_point(
                    self.projection(),
                    at,
                    Some(BoundaryAffinity::Upstream),
                );
                let following = super::super::source_edit::insertion_point(
                    self.projection(),
                    at + 1,
                    Some(BoundaryAffinity::Downstream),
                );
                let (Some(left), Some(right)) = (preceding, following) else {
                    continue;
                };
                if left >= right {
                    continue;
                }
                let boundary = active
                    .iter()
                    .filter(|(token, source)| {
                        left <= source.start
                            && source.end <= right
                            && matches!(&token.kind, TokenKind::Tag(tag) if html::block(&tag.name))
                    })
                    .collect::<Vec<_>>();
                let Some((closing, close_source)) = boundary.first().copied() else {
                    continue;
                };
                let Some((opening, open_source)) = boundary.last().copied() else {
                    continue;
                };
                if boundary.len() < 2 {
                    continue;
                }
                let (TokenKind::Tag(close), TokenKind::Tag(open)) = (&closing.kind, &opening.kind)
                else {
                    unreachable!()
                };
                if !close.end
                    || open.end
                    || !(html::heading_or_paragraph(&close.name) || close.name == "div")
                    || !(html::heading_or_paragraph(&open.name) || open.name == "div")
                {
                    continue;
                }
                // A neutral div may end or begin at the joined boundary. Move
                // its exact token around the merged paragraph rather than
                // dropping its attributes or rewriting the surrounding block.
                let mut closing_wrappers = Vec::new();
                let mut opening_wrappers = Vec::new();
                let mut supported = true;
                for (token, source) in &boundary[1..boundary.len() - 1] {
                    let TokenKind::Tag(tag) = &token.kind else {
                        unreachable!()
                    };
                    if tag.name != "div" || tag.end && !opening_wrappers.is_empty() {
                        supported = false;
                        break;
                    }
                    if tag.end {
                        closing_wrappers.push(source.clone());
                    } else {
                        opening_wrappers.push(source.clone());
                    }
                }
                if !supported {
                    continue;
                }
                let Some(&closing_index) = paired.get(&opening.range.start) else {
                    continue;
                };
                let Some(&opening_index) = paired.get(&closing.range.start) else {
                    continue;
                };
                let left_owner = active[opening_index].0.range.start;
                let mut merged = merges.remove(&left_owner).unwrap_or(Merge {
                    first_open: opening_index,
                    final_close: closing_index,
                    owner: close.name.clone(),
                    closing_wrappers: Vec::new(),
                    opening_wrappers: Vec::new(),
                });
                merged.final_close = closing_index;
                merged.closing_wrappers.extend(closing_wrappers);
                merged.opening_wrappers.extend(opening_wrappers);
                merges.insert(opening.range.start, merged);
                patches.push(SourcePatch::primary(close_source.clone(), Vec::new()));
                patches.push(SourcePatch::primary(open_source.clone(), Vec::new()));
                // Whitespace suppressed by the old paragraph boundary must not
                // become a new visible space when that boundary disappears.
                for (token, source) in &active {
                    if !matches!(token.kind, TokenKind::Text) {
                        continue;
                    }
                    let range = source.start.max(left)..source.end.min(right);
                    if range.start >= range.end {
                        continue;
                    }
                    let bytes = self
                        .state()
                        .source
                        .bytes_in(range.clone())
                        .ok_or(DocumentError::AmbiguousProjection)?;
                    let text = self.encoding().decode_region(&bytes, range.start)?.text;
                    if text
                        .bytes()
                        .all(|byte| matches!(byte, b' ' | b'\t' | b'\r' | b'\n'))
                    {
                        patches.push(SourcePatch::primary(range, Vec::new()));
                    }
                }
            }
        }
        for merged in merges.into_values() {
            let (right_close, right_close_source) = &active[merged.final_close];
            let TokenKind::Tag(right_tag) = &right_close.kind else {
                unreachable!()
            };
            if merged.owner != right_tag.name {
                let raw = &input.text[right_close.range.clone()];
                let name_at = raw
                    .find(|character: char| character.is_ascii_alphabetic())
                    .ok_or(DocumentError::AmbiguousProjection)?;
                let spelling = format!(
                    "{}{}{}",
                    &raw[..name_at],
                    merged.owner,
                    &raw[name_at + right_tag.name.len()..]
                );
                patches.push(SourcePatch::primary(
                    right_close_source.clone(),
                    self.encoding().encode_fragment(&spelling)?,
                ));
            }
            for (wrappers, insertion) in [
                (merged.closing_wrappers, right_close_source.end),
                (merged.opening_wrappers, active[merged.first_open].1.start),
            ] {
                if wrappers.is_empty() {
                    continue;
                }
                let mut bytes = Vec::new();
                for source in wrappers {
                    bytes.extend(
                        self.state()
                            .source
                            .bytes_in(source.clone())
                            .ok_or(DocumentError::AmbiguousProjection)?,
                    );
                    patches.push(SourcePatch::primary(source, Vec::new()));
                }
                patches.push(SourcePatch::primary(insertion..insertion, bytes));
            }
        }
        patches.sort_by_key(|patch| (patch.range.start, patch.range.end));
        patches.dedup_by(|a, b| a.range == b.range && a.replacement == b.replacement);
        Ok((!patches.is_empty()).then_some(patches))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Encoding;

    #[test]
    fn passive_html_text_edits_keep_tags_and_entity_boundaries() {
        for source in ["<p><b>A&amp;B</b> tail</p>", "<p>A&#x1f600;B</p>"] {
            let mut document =
                Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown)
                    .unwrap();
            let original = document.text().to_owned();
            let end = original[1..].chars().next().unwrap().len_utf8() + 1;
            document
                .apply_edits(vec![TextEdit::new(1..end, " <& ")])
                .unwrap();
            let mut expected = original;
            expected.replace_range(1..end, " <& ");
            assert_eq!(document.text(), expected);
            let saved = String::from_utf8(document.source_bytes()).unwrap();
            assert!(saved.starts_with("<p>"));
            assert!(saved.ends_with("</p>"));
            assert!(saved.contains(" &lt;&amp; "), "{saved}");
            let reopened =
                Document::from_bytes(saved.into_bytes(), Encoding::Utf8, Format::Markdown).unwrap();
            assert_eq!(reopened.text(), expected);
            assert!(document.undo());
            assert_eq!(document.source_bytes(), source.as_bytes());
            assert!(document.redo());
            assert_eq!(document.text(), expected);
        }
    }

    #[test]
    fn passive_html_batches_rewrite_shared_entities_once() {
        let source = "<p><b>&fjlig;</b></p>";
        let mut document =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown)
                .unwrap();
        document
            .apply_edits(vec![TextEdit::new(0..1, "F"), TextEdit::new(1..2, "J")])
            .unwrap();
        assert_eq!(document.text(), "FJ");
        assert_eq!(document.source_bytes(), b"<p><b>FJ</b></p>");
        assert!(document.undo());
        assert_eq!(document.source_bytes(), source.as_bytes());
    }

    #[test]
    fn passive_html_structural_deletion_keeps_unrelated_source() {
        for encoding in [Encoding::Utf8, Encoding::Utf16Le, Encoding::Utf16Be] {
            for ending in ["\n", "\r\n"] {
                let source = format!("<div data-keep='yes'><p>A</p>{ending}<p>B</p><span data-secret='v'><!--keep--></span><script type='metadata'>opaque</script></div>");
                let mut original = encoding.bom_bytes().to_vec();
                original.extend(encoding.encode_fragment(&source).unwrap());
                let mut document = Document::from_bytes_with_file_format(
                    original.clone(),
                    encoding,
                    Format::Markdown,
                    if ending == "\r\n" {
                        FileFormat::Dos
                    } else {
                        FileFormat::Unix
                    },
                )
                .unwrap();
                let mut expected = document.text().to_owned();
                assert!(expected.starts_with("A\nB"), "{expected:?}");
                expected.replace_range(1..2, "");
                document.apply_edits(vec![TextEdit::new(1..2, "")]).unwrap();
                assert_eq!(document.text(), expected);
                let saved = document.source_bytes();
                assert_eq!(encoding.decode(&saved).unwrap().text,
                    "<div data-keep='yes'><p>AB</p><span data-secret='v'><!--keep--></span><script type='metadata'>opaque</script></div>");
                let reopened = Document::from_bytes_with_file_format(
                    saved.clone(),
                    encoding,
                    Format::Markdown,
                    document.file_format(),
                )
                .unwrap();
                assert_eq!(reopened.text(), expected);
                assert!(document.undo());
                assert_eq!(document.source_bytes(), original);
                assert!(document.redo());
                assert_eq!(document.source_bytes(), saved);
            }
        }
    }
    #[test]
    fn passive_html_cross_block_deletion_keeps_existing_markdown_policy() {
        let source = "<div>A</div>\n\nB";
        let mut document =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown)
                .unwrap();
        assert_eq!(document.text(), "A\nB");
        document.apply_edits(vec![TextEdit::new(1..2, "")]).unwrap();
        assert_eq!(document.text(), "AB");
        let saved = document.source_bytes();
        assert_eq!(
            Document::from_bytes(saved.clone(), Encoding::Utf8, Format::Markdown)
                .unwrap()
                .text(),
            "AB"
        );
        assert!(document.undo());
        assert_eq!(document.source_bytes(), source.as_bytes());
        assert!(document.redo());
        assert_eq!(document.source_bytes(), saved);
    }

    #[test]
    fn passive_html_nested_wrapper_boundary_remains_editable() {
        for (source, expected_source) in [
            (
                "<div data-keep='yes'>A</div><div>B</div>",
                "<div data-keep='yes'>AB</div>",
            ),
            (
                "<div data-keep='yes'><p>A</p></div><p>B</p>",
                "<div data-keep='yes'><p>AB</p></div>",
            ),
            (
                "<p>A</p><div data-keep='yes'><p>B</p></div>",
                "<div data-keep='yes'><p>AB</p></div>",
            ),
            (
                "<div data-keep='yes'><h1>A</h1><p>B</p></div>",
                "<div data-keep='yes'><h1>AB</h1></div>",
            ),
        ] {
            let mut document =
                Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown)
                    .unwrap();
            assert_eq!(document.text(), "A\nB");
            let style = document.projection().blocks()[0].style.clone();
            document.apply_edits(vec![TextEdit::new(1..2, "")]).unwrap();
            assert_eq!(document.text(), "AB");
            assert_eq!(document.projection().blocks()[0].style, style);
            assert_eq!(document.source_bytes(), expected_source.as_bytes());
            assert_eq!(
                Document::from_bytes(document.source_bytes(), Encoding::Utf8, Format::Markdown)
                    .unwrap()
                    .text(),
                "AB"
            );
            assert!(document.undo());
            assert_eq!(document.source_bytes(), source.as_bytes());
        }
    }
    #[test]
    fn passive_html_connected_merges_keep_first_owner_and_final_closer() {
        for (source, expected_source) in [
            ("<h1>A</h1><p>B</p><p>C</p>", "<h1>AC</h1>"),
            ("<h1>A</h1><p>B</p><h1>C</h1>", "<h1>AC</h1>"),
            (
                "<div data-keep='yes'><h1>A</h1></div><p>B</p><p>C</p>",
                "<div data-keep='yes'><h1>AC</h1></div>",
            ),
            (
                "<h1>A</h1><p>B</p><div data-keep='yes'><p>C</p></div>",
                "<div data-keep='yes'><h1>AC</h1></div>",
            ),
            ("<h1>A</h1><p></p><p>C</p>", "<h1>AC</h1>"),
        ] {
            let mut document =
                Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown)
                    .unwrap();
            let end = document.text().rfind('C').unwrap();
            let style = document.projection().blocks()[0].style.clone();
            document
                .apply_edits(vec![TextEdit::new(1..end, "")])
                .unwrap();
            assert_eq!(document.text(), "AC", "{source}");
            assert_eq!(document.projection().blocks()[0].style, style);
            assert_eq!(document.source_bytes(), expected_source.as_bytes());
            assert_eq!(
                Document::from_bytes(document.source_bytes(), Encoding::Utf8, Format::Markdown)
                    .unwrap()
                    .text(),
                "AC"
            );
            assert!(document.undo());
            assert_eq!(document.source_bytes(), source.as_bytes());
            assert!(document.redo());
            assert_eq!(document.source_bytes(), expected_source.as_bytes());
        }
    }

    #[test]
    fn passive_html_emptied_implicit_paragraph_retains_unselected_boundary() {
        let source = "<div data-keep='yes'>\nx\n</div>\n\nTail";
        let mut document =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown)
                .unwrap();
        assert_eq!(document.text(), "x\nTail");
        document.apply_edits(vec![TextEdit::new(0..1, "")]).unwrap();
        assert_eq!(document.text(), "\nTail");
        let saved = document.source_bytes();
        assert!(std::str::from_utf8(&saved)
            .unwrap()
            .contains("data-keep='yes'"));
        assert_eq!(
            Document::from_bytes(saved, Encoding::Utf8, Format::Markdown)
                .unwrap()
                .text(),
            "\nTail"
        );
        assert!(document.undo());
        assert_eq!(document.source_bytes(), source.as_bytes());
    }
}
