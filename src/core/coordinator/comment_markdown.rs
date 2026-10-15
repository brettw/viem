//! Simplified Markdown in Code comments. Recognition is synchronous and
//! bounded to the visible comments; code inside comments uses bounded
//! asynchronous syntax services, like Markdown Preview code blocks.
use super::*;
use crate::document::{
    comment_markdown::CommentMarkdown,
    syntax::{
        service::SyntaxService, SyntaxInputIdentity, SyntaxInputSnapshot, SyntaxRun,
        SyntaxStyleName,
    },
    FormattedTextTree,
};
use std::{collections::BTreeMap, ops::Range, sync::Arc};

/// Code regions highlighted at once; later regions keep plain code styling.
const MAX_CODE_SERVICES: usize = 32;
/// Lines recognized above each viewport, so short scrolls keep their styling.
const LEADING_LINES: usize = 20;

#[derive(Clone, Debug, PartialEq)]
struct RecognitionKey {
    revision: Revision,
    lines: Vec<Range<usize>>,
    comments: String,
    language: Option<String>,
}

/// The installed layer's spans per hard line, relative to the line start, in
/// cascade order. Carved code text uses an empty name.
type LineSignature = Vec<(Range<usize>, Arc<str>)>;

#[derive(Default)]
struct Installed {
    revision: Option<Revision>,
    sheet: Option<crate::document::StyleSheetRevision>,
    line_count: usize,
    lines: BTreeMap<usize, LineSignature>,
    empty: bool,
}

#[derive(Default)]
pub(super) struct CommentMarkdownSyntax {
    recognized: Option<(RecognitionKey, Arc<CommentMarkdown>)>,
    /// The recognition the services and `code_runs` were built from.
    services_for: Option<Arc<CommentMarkdown>>,
    services: Vec<CodeService>,
    code_runs: Vec<SyntaxRun>,
    next_generation: u64,
    installed: Installed,
}

impl CommentMarkdownSyntax {
    /// Discard code services so the next poll recreates them, for example
    /// with a replaced provider. The installed layer remains until then.
    pub(super) fn reset_services(&mut self) {
        for entry in &mut self.services {
            entry.service.cancel();
        }
        self.services.clear();
        self.services_for = None;
        self.code_runs.clear();
    }
}

struct CodeService {
    generation: u64,
    language: Option<String>,
    service: SyntaxService,
    anchor: Option<crate::document::TextAnchor>,
    input: SyntaxInputSnapshot,
    /// Index of the recognized code region this service highlights.
    region: usize,
}

impl<P: TextMeasurementProvider> Core<P> {
    /// Publish the comment Markdown layer for the current Code revision.
    /// Returns whether the presentation changed.
    pub(super) fn poll_comment_markdown(&mut self) -> bool {
        let enabled = self.document.format().is_code() && self.buffer_commands.comment_markdown.effective();
        if !enabled {
            for entry in &mut self.syntax.comments.services {
                entry.service.cancel();
            }
            let installed = std::mem::take(&mut self.syntax.comments.installed);
            self.syntax.comments = Default::default();
            if self.document.projection().has_comment_markdown() {
                // Keep the old signatures so their lines are relaid out.
                self.syntax.comments.installed = installed;
                return self.install_comment_markdown_layer(Vec::new(), Vec::new());
            }
            return false;
        }
        let revision = self.document.revision();
        let language = self.reflow_language();
        let mut options = self.buffer_commands.reflow_options.clone();
        options.set_language(language.as_deref());
        let key = RecognitionKey {
            revision,
            lines: self.visible_comment_lines(),
            comments: options.comments().to_owned(),
            language: language.clone(),
        };
        let recognized = match &self.syntax.comments.recognized {
            Some((cached, recognized)) if *cached == key => recognized.clone(),
            _ => {
                let mut recognized = CommentMarkdown::default();
                for lines in &key.lines {
                    let part = self.document.recognize_comment_markdown(&options, lines.clone());
                    recognized.roles.extend(part.roles);
                    for region in part.code {
                        if !recognized.code.iter().any(|known| known.segments == region.segments) {
                            recognized.code.push(region);
                        }
                    }
                }
                let recognized = Arc::new(recognized);
                self.syntax.comments.recognized = Some((key, recognized.clone()));
                recognized
            }
        };
        let same_recognition = self
            .syntax
            .comments
            .services_for
            .as_ref()
            .is_some_and(|previous| Arc::ptr_eq(previous, &recognized));
        let services_changed = if same_recognition {
            self.poll_comment_code_services()
        } else {
            self.rebuild_comment_code_services(&recognized, language);
            self.syntax.comments.services_for = Some(recognized.clone());
            true
        };
        if services_changed {
            self.syntax.comments.code_runs = self.comment_code_runs(&recognized);
        }
        let sheet = self.document.projection().style_sheet().revision;
        let installed = &self.syntax.comments.installed;
        let current = installed.revision == Some(revision)
            && installed.sheet == Some(sheet)
            && self.document.projection().has_comment_markdown() != installed.empty;
        if current && same_recognition && !services_changed {
            return false;
        }
        let mut runs: Vec<SyntaxRun> = recognized
            .roles
            .iter()
            .map(|span| SyntaxRun {
                range: span.range.clone(),
                name: SyntaxStyleName(span.role.style_name().into()),
                origin: "comment-markdown".into(),
                priority: 0,
            })
            .collect();
        runs.extend(self.syntax.comments.code_runs.iter().cloned());
        let carved: Vec<Range<usize>> = recognized
            .code
            .iter()
            .flat_map(|region| region.segments.iter().filter(|range| !range.is_empty()).cloned())
            .collect();
        if current && self.syntax.comments.installed.lines == self.layer_signatures(&carved, &runs) {
            return false;
        }
        self.install_comment_markdown_layer(carved, runs)
    }

    /// Visible hard lines of every view, with leading context, merged.
    fn visible_comment_lines(&self) -> Vec<Range<usize>> {
        let input = self.syntax_input();
        let projection = self.document.projection();
        let mut ranges: Vec<Range<usize>> = self
            .views
            .values()
            .filter_map(|view| {
                let bytes = self.visible_syntax_range(&input, view);
                let first = projection.hard_line_at_offset(bytes.start)?;
                let last = projection.hard_line_at_offset(bytes.end).unwrap_or(first);
                Some(first.saturating_sub(LEADING_LINES)..last + 1)
            })
            .collect();
        ranges.sort_by_key(|range| range.start);
        let mut merged: Vec<Range<usize>> = Vec::new();
        for range in ranges {
            match merged.last_mut() {
                Some(last) if range.start <= last.end => last.end = last.end.max(range.end),
                _ => merged.push(range),
            }
        }
        merged
    }

    /// Collect completed coverage from the existing code services.
    fn poll_comment_code_services(&mut self) -> bool {
        let mut changed = false;
        for entry in &mut self.syntax.comments.services {
            changed |= entry.service.poll(entry.input.identity());
            entry.service.request(entry.input.clone(), 0..entry.input.byte_len());
        }
        changed
    }

    /// Match recognized code regions to retained services by their start
    /// anchor and language, rebasing changed code text so its retained colors
    /// follow the edit while new analysis is pending.
    fn rebuild_comment_code_services(&mut self, recognized: &CommentMarkdown, language: Option<String>) {
        let revision = self.document.revision();
        let previous = std::mem::take(&mut self.syntax.comments.services);
        let mut previous: Vec<(Option<usize>, Option<CodeService>)> = previous
            .into_iter()
            .map(|entry| {
                let at = entry
                    .anchor
                    .and_then(|anchor| self.document.resolve_text_anchor(anchor).ok())
                    .and_then(|outcome| outcome.value().map(|point| point.offset()));
                (at, Some(entry))
            })
            .collect();
        let mut services = Vec::new();
        for (region_index, region) in recognized.code.iter().enumerate().take(MAX_CODE_SERVICES) {
            let Some(start) = region.segments.first().map(|segment| segment.start) else {
                continue;
            };
            let Ok(text) = FormattedTextTree::try_from_text(region.text.as_str()) else {
                continue;
            };
            let region_language = region.language.clone().or_else(|| language.clone());
            let reused = previous.iter_mut().find_map(|(at, entry)| {
                (*at == Some(start) && entry.as_ref().is_some_and(|entry| entry.language == region_language))
                    .then(|| entry.take())
                    .flatten()
            });
            let generation = match &reused {
                Some(entry) => entry.generation,
                None => {
                    self.syntax.comments.next_generation += 1;
                    self.syntax.comments.next_generation
                }
            };
            let mut input = SyntaxInputSnapshot::new(
                SyntaxInputIdentity { document: self.document.id().0, revision: revision.0, generation },
                text,
            );
            let mut entry = match reused {
                Some(mut entry) => {
                    let old = entry.input.clone();
                    match old.text_tree().changed_extent(input.text_tree()) {
                        // Analysis depends only on the code text and language.
                        None => input = old,
                        Some((old_range, new_range)) => {
                            let map = crate::document::Splice::new(old_range.clone(), new_range.len())
                                .ok()
                                .and_then(|splice| {
                                    PositionMap::for_text_snapshots(
                                        self.document.id(),
                                        Revision(old.identity().revision),
                                        Revision(input.identity().revision),
                                        old.text_tree(),
                                        input.text_tree(),
                                        vec![splice],
                                    )
                                    .ok()
                                });
                            let hull = map.is_some().then_some((old_range, new_range));
                            entry.service.rebase_input(input.clone(), map.as_ref(), hull);
                        }
                    }
                    entry.input = input.clone();
                    entry
                }
                None => {
                    let mut service = self.syntax.service.for_export();
                    service.set_language(region_language.clone());
                    service.set_filename(None);
                    service.rebase_input(input.clone(), None, None);
                    CodeService {
                        generation,
                        language: region_language,
                        service,
                        anchor: None,
                        input: input.clone(),
                        region: region_index,
                    }
                }
            };
            entry.region = region_index;
            entry.service.poll(input.identity());
            entry.service.request(input.clone(), 0..input.byte_len());
            entry.anchor = self.document.text_point(start).ok().and_then(|point| {
                self.document
                    .text_anchor(
                        point,
                        crate::document::Association::BeforeInsertion,
                        BoundaryAffinity::Downstream,
                        crate::document::DeletionRecovery::PreferFollowingThenPreceding,
                    )
                    .ok()
            });
            services.push(entry);
        }
        for (_, entry) in previous {
            if let Some(mut entry) = entry {
                entry.service.cancel();
            }
        }
        self.syntax.comments.services = services;
    }

    /// The services' current runs, mapped from code text to the document.
    fn comment_code_runs(&self, recognized: &CommentMarkdown) -> Vec<SyntaxRun> {
        let mut runs = Vec::new();
        for entry in &self.syntax.comments.services {
            let Some(region) = recognized.code.get(entry.region) else { continue };
            for run in entry.service.runs(entry.input.identity()) {
                for range in region.document_ranges(run.range.clone()) {
                    runs.push(SyntaxRun { range, ..run.clone() });
                }
            }
        }
        runs
    }

    /// Per-line signatures of a layer, relative to each line's start.
    fn layer_signatures(&self, carved: &[Range<usize>], runs: &[SyntaxRun]) -> BTreeMap<usize, LineSignature> {
        let projection = self.document.projection();
        let snapshot = self.document.hard_line_snapshot();
        let mut lines: BTreeMap<usize, LineSignature> = BTreeMap::new();
        let mut push = |range: &Range<usize>, name: Arc<str>| {
            let Some(line) = projection.hard_line_at_offset(range.start) else { return };
            let Some(start) = snapshot.line(line).map(|info| info.content_range().start) else { return };
            lines.entry(line).or_default().push((range.start - start..range.end - start, name));
        };
        let carve: Arc<str> = Arc::from("");
        for range in carved {
            push(range, carve.clone());
        }
        for run in runs {
            push(&run.range, run.name.0.clone());
        }
        lines
    }

    /// Install the layer and relayout only the lines whose comment Markdown
    /// changed, aligned by unchanged leading and trailing lines.
    fn install_comment_markdown_layer(&mut self, carved: Vec<Range<usize>>, runs: Vec<SyntaxRun>) -> bool {
        let revision = self.document.revision();
        let empty = carved.is_empty() && runs.is_empty();
        if empty && !self.document.projection().has_comment_markdown() && self.syntax.comments.installed.lines.is_empty() {
            // Nothing was or will be presented, as in most Code without
            // comment Markdown: record the revision without any relayout.
            self.syntax.comments.installed = Installed {
                revision: Some(revision),
                sheet: Some(self.document.projection().style_sheet().revision),
                line_count: self.document.projection().presentation_line_count(),
                lines: BTreeMap::new(),
                empty: true,
            };
            return false;
        }
        let caret_anchors = self.caret_anchors_for_publication();
        let signatures = self.layer_signatures(&carved, &runs);
        if !self.document.install_comment_markdown_presentation(carved, &runs) {
            return false;
        }
        let line_count = self.document.projection().presentation_line_count();
        let previous = std::mem::take(&mut self.syntax.comments.installed);
        let changed = changed_line_hull(&previous.lines, previous.line_count, &signatures, line_count);
        self.syntax.comments.installed = Installed {
            revision: Some(revision),
            sheet: Some(self.document.projection().style_sheet().revision),
            line_count,
            lines: signatures,
            empty,
        };
        // Identical per-line content needs no invalidation: the syntax
        // publication preceding a new revision already invalidated paint.
        if let Some(lines) = changed {
            let snapshot = self.document.hard_line_snapshot();
            let start = snapshot.line(lines.start).map_or(0, |info| info.content_range().start);
            let end = snapshot
                .line(lines.end.saturating_sub(1).max(lines.start))
                .map_or(start, |info| info.content_range().end);
            let sheet = self.syntax.sheet.clone();
            let change = crate::coordinator::syntax::metrics::MetricChange::Local(start..end);
            self.apply_code_presentation_change(sheet, change, caret_anchors);
        }
        true
    }
}

/// The new revision's lines whose signatures differ from the previous layer,
/// after skipping lines equal by position from the start and from the end.
fn changed_line_hull(
    old: &BTreeMap<usize, LineSignature>,
    old_count: usize,
    new: &BTreeMap<usize, LineSignature>,
    new_count: usize,
) -> Option<Range<usize>> {
    let empty = Vec::new();
    let old_at = |line: usize| old.get(&line).unwrap_or(&empty);
    let new_at = |line: usize| new.get(&line).unwrap_or(&empty);
    let interesting = |map: &BTreeMap<usize, LineSignature>| {
        map.keys().next().copied().zip(map.keys().next_back().copied())
    };
    let (first, last) = match (interesting(old), interesting(new)) {
        (None, None) => return None,
        (Some(a), None) | (None, Some(a)) => a,
        (Some(a), Some(b)) => (a.0.min(b.0), a.1.max(b.1)),
    };
    // Leading lines keep their indices; trailing lines keep their distance
    // from the end. Signatures only exist between `first` and `last`.
    let mut start = first;
    while start <= last && start < new_count && start < old_count && old_at(start) == new_at(start) {
        start += 1;
    }
    if start > last {
        return None;
    }
    let delta = new_count as isize - old_count as isize;
    let mut end = (last as isize + delta.max(0)) as usize + 1;
    end = end.min(new_count);
    while end > start {
        let new_line = end - 1;
        let old_line = new_line as isize - delta;
        if old_line < 0 || old_at(old_line as usize) != new_at(new_line) {
            break;
        }
        end -= 1;
    }
    (start < end).then_some(start..end).or_else(|| Some(start..(start + 1).min(new_count.max(1))))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::syntax::{
        service::{SyntaxProvider, SyntaxRequest, SyntaxResult},
        Coverage,
    };
    use crate::document::{Encoding, ResolvedCharacterStyle};
    use crate::layout::{DocumentLayoutStyles, MockTextMeasurementProvider};
    use std::sync::atomic::AtomicBool;
    use std::time::{Duration, Instant};

    /// Comments from `//` to the line end; `let` elsewhere is a keyword.
    struct Fixture;
    impl SyntaxProvider for Fixture {
        fn analyze(&mut self, request: &SyntaxRequest, _: &AtomicBool) -> SyntaxResult {
            let text = request.input.slice(0..request.input.byte_len()).unwrap();
            let mut runs = Vec::new();
            let mut comments = Vec::new();
            let mut at = 0;
            for line in text.split_inclusive('\n') {
                if let Some(found) = line.find("//") {
                    comments.push(at + found..at + line.trim_end_matches('\n').len());
                }
                at += line.len();
            }
            let run = |range, name: &str| SyntaxRun {
                range,
                name: SyntaxStyleName(name.into()),
                origin: "fixture".into(),
                priority: 0,
            };
            for (found, _) in text.match_indices("let") {
                if !comments.iter().any(|range| range.contains(&found)) {
                    runs.push(run(found..found + 3, "Keyword"));
                }
            }
            runs.extend(comments.into_iter().map(|range| run(range, "Comment")));
            runs.sort_by_key(|run| run.range.start);
            SyntaxResult {
                input: request.input.identity(),
                configuration: request.configuration.clone(),
                range: request.range.clone(),
                runs,
                coverage: Coverage::Exact,
                diagnostics: Vec::new(),
                continuation: false,
            }
        }
    }

    const SOURCE: &str = "// Use **bold\n// text** and `let x`.\nlet y = 1;\n// ```python\n// let z\n// ```\n";

    fn core() -> Core<MockTextMeasurementProvider> {
        let mut core = Core::<MockTextMeasurementProvider>::new(
            Document::from_bytes(SOURCE.as_bytes().to_vec(), Encoding::Utf8, Format::Code).unwrap(),
        );
        core.initialize_code_detection("fixture.rs", false).unwrap();
        core.set_syntax_provider_factory(Arc::new(|| Box::new(Fixture)));
        core.add_view(MockTextMeasurementProvider::new(), 400., 300.);
        core
    }

    fn style(core: &Core<MockTextMeasurementProvider>, needle: &str, nth: usize) -> ResolvedCharacterStyle {
        let at = SOURCE.match_indices(needle).nth(nth).unwrap().0;
        DocumentLayoutStyles::character_at(core.document.projection(), at, false).unwrap()
    }

    fn settle(core: &mut Core<MockTextMeasurementProvider>, done: impl Fn(&Core<MockTextMeasurementProvider>) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            core.poll_syntax();
            if done(core) {
                return;
            }
            assert!(Instant::now() < deadline, "syntax did not settle");
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn comment_markdown_layers_roles_over_comment_style_and_highlights_code() {
        let mut core = core();
        assert!(core.comment_markdown().effective(), "on by default");
        core.set_comment_markdown(false);
        let keyword = |core: &Core<MockTextMeasurementProvider>| style(core, "let", 1);
        settle(&mut core, |core| style(core, "Use", 0).foreground != keyword(core).foreground);
        let comment = style(&core, "Use", 0);
        assert!(!style(&core, "bold", 0).bold, "disabled");

        core.set_comment_markdown(true);
        settle(&mut core, |core| style(core, "let", 0).foreground == keyword(core).foreground
            && style(core, "let", 2).foreground == keyword(core).foreground);
        // Strong keeps the comment's own appearance and adds bold, across the leader.
        for (needle, nth) in [("bold", 0), ("text", 0), ("**", 0), ("**", 1)] {
            let resolved = style(&core, needle, nth);
            assert!(resolved.bold, "{needle}");
            assert_eq!(resolved.foreground, comment.foreground, "{needle}");
        }
        assert!(!style(&core, "// text", 0).bold, "leaders stay comment text");
        // Code is carved out of the comment and uses the code's highlighting.
        assert!(style(&core, " x`", 0).foreground_is_default);
        assert_eq!(style(&core, "`let", 0).foreground, comment.foreground);
        assert_eq!(style(&core, "```", 0).foreground, comment.foreground);
        assert_eq!(style(&core, "python", 0).foreground, comment.foreground);
        let languages: Vec<_> = core.syntax.comments.services.iter().map(|entry| entry.language.clone()).collect();
        assert!(languages.contains(&Some("rust".into())), "{languages:?}");
        assert!(languages.contains(&Some("python".into())), "{languages:?}");

        core.set_comment_markdown(false);
        settle(&mut core, |core| !style(core, "bold", 0).bold);
        assert_eq!(style(&core, "let", 0).foreground, comment.foreground);
        assert!(core.syntax.comments.services.is_empty());
    }

    #[test]
    fn edits_reinstall_the_layer_for_the_new_revision() {
        let mut core = core();
        core.set_comment_markdown(true);
        settle(&mut core, |core| style(core, "bold", 0).bold);
        let at = SOURCE.find("bold").unwrap();
        core.document.replace(at..at, "very ").unwrap();
        core.poll_syntax();
        let resolved = DocumentLayoutStyles::character_at(core.document.projection(), at, false).unwrap();
        assert!(resolved.bold, "recognition is synchronous for the new revision");
        let closer = SOURCE.find("** and").unwrap() + "very ".len();
        core.document.replace(closer..closer + 2, "").unwrap();
        core.poll_syntax();
        let resolved = DocumentLayoutStyles::character_at(core.document.projection(), at, false).unwrap();
        assert!(!resolved.bold, "an unclosed delimiter is not strong");
    }

    fn signature(entries: &[(usize, &str)]) -> BTreeMap<usize, LineSignature> {
        entries.iter().map(|(line, name)| (*line, vec![(0..1, Arc::from(*name))])).collect()
    }

    #[test]
    fn unchanged_layers_need_no_relayout() {
        let layer = signature(&[(3, "a"), (5, "b")]);
        assert_eq!(changed_line_hull(&layer, 10, &layer, 10), None);
        assert_eq!(changed_line_hull(&BTreeMap::new(), 10, &BTreeMap::new(), 10), None);
    }

    #[test]
    fn changes_are_bounded_by_matching_prefix_and_suffix() {
        let old = signature(&[(3, "a"), (5, "b"), (8, "c")]);
        let new = signature(&[(3, "a"), (5, "x"), (8, "c")]);
        assert_eq!(changed_line_hull(&old, 10, &new, 10), Some(5..6));
        // A line inserted after the change shifts `c` without relayout; the
        // ambiguous inserted line is included.
        let new = signature(&[(3, "a"), (5, "x"), (9, "c")]);
        assert_eq!(changed_line_hull(&old, 10, &new, 11), Some(5..7));
        // A layer that disappears relayouts its former lines.
        assert_eq!(changed_line_hull(&old, 10, &BTreeMap::new(), 10), Some(3..9));
    }
}
