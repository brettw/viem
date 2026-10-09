//! Bounded visible Markdown code blocks share Code's asynchronous providers.
use super::*;
use crate::document::{
    code_style,
    syntax::{
        service::SyntaxService, SyntaxInputIdentity, SyntaxInputSnapshot, SyntaxRun,
        SyntaxStyleName,
    },
};
use std::collections::BTreeMap;

const MAX_VISIBLE_CODE_BLOCKS: usize = 32;
pub(super) struct MarkdownCodeSyntax {
    pub(super) services: BTreeMap<u64, MarkdownCodeService>,
    revision: Option<Revision>,
    sheet_revision: Option<crate::document::StyleSheetRevision>,
    document_sheet_revision: Option<crate::document::StyleSheetRevision>,
    language_change: Option<(DocumentId, Revision)>,
    #[cfg(test)]
    code_sheet: Option<std::sync::Arc<crate::document::StyleSheet>>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::syntax::{
        service::{SyntaxProvider, SyntaxRequest, SyntaxResult},
        Coverage,
    };
    use crate::layout::MockTextMeasurementProvider;
    use std::sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    };
    use std::time::{Duration, Instant};

    fn fixture_code_sheet(size: f32) -> Arc<crate::document::StyleSheet> {
        let mut sheet = code_style::default_sheet();
        let mut base = sheet.block_style(&sheet.base_paragraph).unwrap().clone();
        base.character.size = Some(crate::document::FontSize::Points(14.));
        sheet
            .apply_configuration_edit(
                &crate::document::StyleDefinitionEdit::UpdateBlock(base),
                crate::document::StyleSheetRevision(sheet.revision.0 + 1),
                false,
            )
            .unwrap();
        sheet
            .insert_character_style(
                crate::document::CharacterStyle {
                    id: StyleId::from("syntax:FixtureLargeCodeToken"),
                    based_on: None,
                    properties: crate::document::CharacterProperties {
                        size: Some(crate::document::FontSize::Points(size)),
                        ..Default::default()
                    },
                },
                crate::document::StyleDefinitionMetadata::generated("FixtureLargeCodeToken"),
            )
            .unwrap();
        Arc::new(sheet)
    }

    struct DelayedLeadingToken(Arc<AtomicBool>);
    impl SyntaxProvider for DelayedLeadingToken {
        fn analyze(&mut self, request: &SyntaxRequest, cancellation: &AtomicBool) -> SyntaxResult {
            let deadline = Instant::now() + Duration::from_secs(5);
            while !self.0.load(Ordering::Acquire)
                && !cancellation.load(Ordering::Acquire)
                && Instant::now() < deadline
            {
                std::thread::sleep(Duration::from_millis(1));
            }
            SyntaxResult {
                input: request.input.identity(),
                configuration: request.configuration.clone(),
                range: request.range.clone(),
                runs: vec![SyntaxRun {
                    range: 0..2,
                    name: SyntaxStyleName("FixtureLargeCodeToken".into()),
                    origin: "fixture".into(),
                    priority: 0,
                }],
                coverage: Coverage::Exact,
                diagnostics: Vec::new(),
                continuation: false,
            }
        }
    }

    struct GatedSmallTokens {
        released: Arc<AtomicUsize>,
        requests: Arc<Mutex<Vec<std::ops::Range<usize>>>>,
    }
    impl SyntaxProvider for GatedSmallTokens {
        fn analyze(&mut self, request: &SyntaxRequest, cancellation: &AtomicBool) -> SyntaxResult {
            let phase = {
                let mut requests = self.requests.lock().unwrap();
                requests.push(request.range.clone());
                requests.len()
            };
            let deadline = Instant::now() + Duration::from_secs(5);
            while self.released.load(Ordering::Acquire) < phase
                && !cancellation.load(Ordering::Acquire)
                && Instant::now() < deadline
            {
                std::thread::sleep(Duration::from_millis(1));
            }
            SyntaxResult {
                input: request.input.identity(),
                configuration: request.configuration.clone(),
                range: request.range.clone(),
                runs: vec![SyntaxRun {
                    range: request.range.clone(),
                    name: SyntaxStyleName("FixtureLargeCodeToken".into()),
                    origin: "fixture".into(),
                    priority: 0,
                }],
                coverage: Coverage::Exact,
                diagnostics: Vec::new(),
                continuation: false,
            }
        }
    }

    #[test]
    fn language_scroll_policy_waits_for_visible_coverage_exposed_by_smaller_tokens() {
        let source = format!("```rust\n{}```", "fn\n".repeat(50));
        let mut document =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Markdown)
                .unwrap();
        let mut defaults: serde_json::Value =
            serde_json::from_slice(&document.export_style_defaults().unwrap()).unwrap();
        let code = defaults["block_styles"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|style| style["id"] == "Code Block")
            .unwrap();
        code["character"]["size"] = 14.into();
        document
            .initialize_style_defaults(&serde_json::to_vec(&defaults).unwrap())
            .unwrap();
        let mut core = Core::new(document);
        core.syntax.markdown.code_sheet = Some(fixture_code_sheet(7.));
        let view = core.add_view(MockTextMeasurementProvider::new(), 500., 180.);
        let origin = (
            core.views[&view].layout.viewport_left(),
            core.views[&view].layout.viewport_top(),
        );
        let visible_lines = core
            .layout(view)
            .unwrap()
            .snapshot()
            .unwrap()
            .rows
            .iter()
            .filter(|row| row.y + row.height() > origin.1 && row.y < origin.1 + 180.)
            .count();
        let caret = (visible_lines + 1) * 3;
        core.handle(
            view,
            CoreEvent::GoToLine {
                document: core.document.id(),
                revision: core.document.revision(),
                line: visible_lines as u64 + 2,
            },
        )
        .unwrap();
        core.handle(
            view,
            CoreEvent::SetViewportOrigin {
                left: origin.0,
                top: Some(origin.1),
            },
        )
        .unwrap();
        assert_eq!(core.views[&view].commands.cursor(), caret);
        assert!(
            capture_caret_baseline_anchor(&core.document, &core.views[&view]).is_none(),
            "the caret starts below the unhighlighted viewport"
        );

        let released = Arc::new(AtomicUsize::new(0));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let (provider_release, provider_requests) = (released.clone(), requests.clone());
        core.set_syntax_provider_factory(Arc::new(move || {
            Box::new(GatedSmallTokens {
                released: provider_release.clone(),
                requests: provider_requests.clone(),
            })
        }));
        core.syntax.markdown.code_sheet = Some(fixture_code_sheet(7.));
        let identity = Some((core.document.id(), core.document.revision()));
        core.syntax.markdown.language_change = identity;
        core.poll_syntax();
        let deadline = Instant::now() + Duration::from_secs(5);
        while requests.lock().unwrap().is_empty() {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        let first = requests.lock().unwrap()[0].clone();
        assert!(
            first.end < caret,
            "the first actual request excludes the caret"
        );
        released.store(1, Ordering::Release);
        while requests.lock().unwrap().len() < 2 {
            core.poll_syntax();
            core.handle(
                view,
                CoreEvent::Resize {
                    width: 500.,
                    height: 180.,
                },
            )
            .unwrap();
            assert_eq!(core.views[&view].layout.viewport_top(), origin.1);
            assert_eq!(
                core.syntax.markdown.language_change, identity,
                "a font publication cannot finish before its newly visible coverage is requested"
            );
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        let second = requests.lock().unwrap()[1].clone();
        assert!(
            second.end > first.end && second.end > caret,
            "smaller rows expose uncaptured text before the newly visible caret"
        );
        let before = capture_caret_baseline_anchor(&core.document, &core.views[&view]).unwrap();
        let baseline_before =
            viewport::anchor_geometry(core.layout(view).unwrap().snapshot().unwrap(), before)
                .unwrap();
        let before_row =
            core.layout(view).unwrap().snapshot().unwrap().rows[baseline_before.row_index].baseline;
        released.store(usize::MAX, Ordering::Release);
        loop {
            core.poll_syntax();
            core.handle(
                view,
                CoreEvent::Resize {
                    width: 500.,
                    height: 180.,
                },
            )
            .unwrap();
            assert_eq!(core.views[&view].layout.viewport_left(), origin.0);
            assert_eq!(core.views[&view].layout.viewport_top(), origin.1);
            assert_eq!(core.views[&view].commands.cursor(), caret);
            if core.syntax.markdown.language_change.is_none() {
                break;
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        let after =
            viewport::anchor_geometry(core.layout(view).unwrap().snapshot().unwrap(), before)
                .unwrap();
        let after_row =
            core.layout(view).unwrap().snapshot().unwrap().rows[after.row_index].baseline;
        assert!(
            after_row < before_row - 1.,
            "the second capture changes real metrics before the caret"
        );
        assert_eq!(core.document.source_bytes(), source.as_bytes());
        assert!(!core.document.is_dirty());
    }

    #[test]
    fn language_scroll_policy_survives_missing_revision_layout_and_delayed_leading_font() {
        let source = format!(
            "{}```\n{}```\n\n{}",
            "before\n\n".repeat(2),
            "fn main() {}\n".repeat(12),
            "after\n\n".repeat(20)
        );
        let mut document =
            Document::from_bytes(source.into_bytes(), Encoding::Utf8, Format::Markdown).unwrap();
        let mut defaults: serde_json::Value =
            serde_json::from_slice(&document.export_style_defaults().unwrap()).unwrap();
        for style in defaults["block_styles"].as_array_mut().unwrap() {
            if style["id"] == "Base Paragraph" || style["id"] == "Code Block" {
                style["character"]["size"] = 14.into();
            }
        }
        document
            .initialize_style_defaults(&serde_json::to_vec(&defaults).unwrap())
            .unwrap();
        let at = document.text().find("fn main").unwrap();
        let line = document
            .projection()
            .text_tree()
            .hard_line_at_byte(at)
            .unwrap();
        let release = Arc::new(AtomicBool::new(false));
        let provider_release = release.clone();
        let mut core = Core::new(document);
        core.set_syntax_provider_factory(Arc::new(move || {
            Box::new(DelayedLeadingToken(provider_release.clone()))
        }));
        core.syntax.markdown.code_sheet = Some(fixture_code_sheet(40.));
        let view = core.add_view(MockTextMeasurementProvider::new(), 500., 180.);
        core.handle(
            view,
            CoreEvent::GoToLine {
                document: core.document.id(),
                revision: core.document.revision(),
                line: line as u64 + 1,
            },
        )
        .unwrap();
        let row = core
            .layout(view)
            .unwrap()
            .snapshot()
            .unwrap()
            .rows
            .iter()
            .find(|row| row.text_range.start == at)
            .unwrap();
        let top = row.y - 30.;
        core.handle(
            view,
            CoreEvent::SetViewportOrigin {
                left: 0.,
                top: Some(top),
            },
        )
        .unwrap();
        let origin = (
            core.views[&view].layout.viewport_left(),
            core.views[&view].layout.viewport_top(),
        );
        assert_eq!(core.views[&view].commands.cursor(), at);
        assert_eq!(
            crate::layout::DocumentLayoutStyles::character_at(
                core.document.projection(),
                at,
                false
            )
            .unwrap()
            .size,
            14.
        );

        // Exercise the first source-publication boundary explicitly: layouts
        // still name the old revision and no new language service exists yet.
        let prepared = core
            .document
            .prepare_code_block_language(
                core.document.id(),
                core.document.revision(),
                at,
                Some("rust"),
            )
            .unwrap();
        let map = prepared.text_position_map().clone();
        let commands = core
            .prepare_mapped_commands(&map, CommandInterpreter::capture_position_anchors)
            .unwrap();
        core.document.commit_model_transaction(prepared).unwrap();
        for (id, commands) in commands {
            core.views.get_mut(&id).unwrap().commands = commands;
        }
        let identity = Some((core.document.id(), core.document.revision()));
        core.syntax.markdown.language_change = identity;
        assert!(core.syntax.markdown.services.is_empty());
        assert_ne!(
            core.layout(view)
                .unwrap()
                .snapshot()
                .unwrap()
                .document_revision,
            core.document.revision()
        );
        core.finish_code_language_refresh();
        assert_eq!(
            core.syntax.markdown.language_change, identity,
            "empty work before current layout must not retire the language scroll policy"
        );
        core.refresh_views_after_native_change(view, &map).unwrap();
        core.restore_code_language_viewport(view, origin.0, origin.1)
            .unwrap();
        core.poll_syntax();
        assert_eq!(core.syntax.markdown.language_change, identity);

        release.store(true, Ordering::Release);
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            core.poll_syntax();
            core.handle(
                view,
                CoreEvent::Resize {
                    width: 500.,
                    height: 180.,
                },
            )
            .unwrap();
            assert_eq!(core.views[&view].layout.viewport_left(), origin.0);
            assert_eq!(core.views[&view].layout.viewport_top(), origin.1);
            assert_eq!(core.views[&view].commands.cursor(), at);
            let character = crate::layout::DocumentLayoutStyles::character_at(
                core.document.projection(),
                at,
                false,
            )
            .unwrap();
            if character.size == 40. && core.syntax.markdown.language_change.is_none() {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "the actual leading font capture must publish and retire the policy: size={} policy={:?} services={}",
                character.size, core.syntax.markdown.language_change, core.syntax.markdown.services.len()
            );
            std::thread::sleep(Duration::from_millis(1));
        }
        let visible = core
            .layout(view)
            .unwrap()
            .snapshot()
            .unwrap()
            .rows
            .iter()
            .find(|row| row.text_range.start == at)
            .unwrap();
        assert!(visible.baseline >= origin.1 && visible.baseline <= origin.1 + 180.);
        assert!(
            visible.height() >= 40.,
            "the accepted token changes real row metrics"
        );
        assert!(core.layout(view).unwrap().snapshot().unwrap().rows.len() < 100);
    }

    #[test]
    fn leading_code_token_keeps_syntax_font_metrics_over_base() {
        let source = b"```rust\nfn main() {}\n```";
        let mut document =
            Document::from_bytes(source.to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
        let mut code = code_style::default_sheet();
        let mut base = code.block_style(&code.base_paragraph).unwrap().clone();
        base.character.size = Some(crate::document::FontSize::Points(14.));
        code.apply_configuration_edit(
            &crate::document::StyleDefinitionEdit::UpdateBlock(base),
            crate::document::StyleSheetRevision(code.revision.0 + 1),
            false,
        )
        .unwrap();
        let mut keyword = code
            .character_style(code_style::resolve_name(&code, "Keyword").unwrap())
            .unwrap()
            .clone();
        keyword.properties.size = Some(crate::document::FontSize::Points(40.));
        let metadata = code.character_style_metadata(&keyword.id).unwrap().clone();
        code.insert_character_style(keyword, metadata).unwrap();
        let sheet = code_style::markdown_sheet(document.projection().style_sheet(), &code);
        document.install_markdown_code_presentation(
            sheet,
            &[
                SyntaxRun {
                    range: 0..document.text().len(),
                    name: SyntaxStyleName("ViemCodeBase".into()),
                    origin: "markdown-code".into(),
                    priority: -1,
                },
                SyntaxRun {
                    range: 0..2,
                    name: SyntaxStyleName("Keyword".into()),
                    origin: "fixture".into(),
                    priority: 0,
                },
            ],
        );
        let first =
            crate::layout::DocumentLayoutStyles::character_at(document.projection(), 0, false)
                .unwrap();
        let ordinary =
            crate::layout::DocumentLayoutStyles::character_at(document.projection(), 3, false)
                .unwrap();
        assert_eq!(
            first.size, 40.,
            "leading syntax font must override the block's base font"
        );
        assert_eq!(ordinary.size, 14.);
        assert_eq!(document.source_bytes(), source);
        assert!(!document.is_dirty());
    }
}
impl Default for MarkdownCodeSyntax {
    fn default() -> Self {
        Self {
            services: BTreeMap::new(),
            revision: None,
            sheet_revision: None,
            document_sheet_revision: None,
            language_change: None,
            #[cfg(test)]
            code_sheet: None,
        }
    }
}
pub(super) struct MarkdownCodeService {
    service: SyntaxService,
    anchors: Option<(crate::document::TextAnchor, crate::document::TextAnchor)>,
    input: Option<SyntaxInputSnapshot>,
}

impl<P: TextMeasurementProvider> Core<P> {
    pub fn set_markdown_code_language(
        &mut self,
        view: ViewId,
        document: DocumentId,
        revision: Revision,
        at: usize,
        language: Option<&str>,
    ) -> Result<CoreOutcome, CoreError> {
        self.views.get(&view).ok_or(CoreError::UnknownView(view))?;
        let prepared = self
            .document
            .prepare_code_block_language(document, revision, at, language)
            .map_err(command_model_transaction_error)?;
        if prepared.is_no_op() {
            return Ok(CoreOutcome {
                command: None,
                document_changed: false,
                position_map: None,
                layout_changed: false,
                composition_changes: Vec::new(),
            });
        }
        let viewport_origins: Vec<_> = self
            .views
            .iter()
            .map(|(id, view)| (*id, view.layout.viewport_left(), view.layout.viewport_top()))
            .collect();
        let map = prepared.text_position_map().clone();
        let before = self.views[&view]
            .commands
            .capture_history_restoration(&self.document)?;
        let mapped =
            self.prepare_mapped_commands(&map, CommandInterpreter::capture_position_anchors)?;
        self.finalize_open_edit_group(view)?;
        let prepared = self
            .document
            .rebind_prepared_after_group_close(prepared)
            .map_err(command_model_transaction_error)?;
        self.document
            .commit_model_transaction(prepared)
            .map_err(command_model_transaction_error)?;
        for (id, commands) in mapped {
            self.views.get_mut(&id).expect("prepared view").commands = commands;
        }
        let after = self.views[&view]
            .commands
            .capture_history_restoration(&self.document)?;
        self.document.attach_history_restoration(
            self.document.history_status().current.node,
            HistoryRestoration::new(before, after),
        )?;
        self.syntax.markdown.language_change = Some((self.document.id(), self.document.revision()));
        let composition_changes = self.refresh_views_after_native_change(view, &map)?;
        // The language label is viewport furniture, independent of the caret.
        // Source refresh can refine regional height estimates while restoring
        // text anchors. Retain each view's actual scroll origin for this label
        // action, and materialize its requested viewport without caret reveal.
        for (id, left, top) in viewport_origins {
            if let Err(error) = self.restore_code_language_viewport(id, left, top) {
                self.record_presentation_error(id, error);
            }
        }
        Ok(CoreOutcome {
            command: None,
            document_changed: true,
            position_map: Some(map),
            layout_changed: true,
            composition_changes,
        })
    }

    fn restore_code_language_viewport(
        &mut self,
        id: ViewId,
        left: f32,
        top: f32,
    ) -> Result<(), CoreError> {
        for _ in 0..3 {
            self.materialize_requested_viewport(id, left, top)?;
            let view = self.views.get_mut(&id).ok_or(CoreError::UnknownView(id))?;
            let snapshot = view.layout.snapshot().ok_or(LayoutError::NoRows)?;
            if snapshot.missing_viewport_edges(top, view.layout.height()) != (false, false) {
                continue;
            }
            // Absolute scroll requests normally retain their text location as
            // unknown heights become exact. A language edit instead retains
            // its numeric origin; apply it only within exact current coverage.
            view.layout.retain_viewport_origin(left, top)?;
            update_viewport_anchor(&self.document, view);
            return Ok(());
        }
        Err(LayoutError::OutsideMaterializedCoverage.into())
    }

    pub(super) fn poll_markdown_code_syntax(&mut self) -> bool {
        let full = self.syntax_input();
        let mut requests: BTreeMap<u64, (crate::document::Block, Vec<std::ops::Range<usize>>)> =
            BTreeMap::new();
        for view in self.views.values() {
            let visible = self.visible_syntax_range(&full, view);
            for block in self.document.projection().blocks_for_region(&visible) {
                if block.style.0 != "Code Block"
                    || block.code_language.is_none()
                    || block.range.is_empty()
                {
                    continue;
                }
                let request =
                    block.range.start.max(visible.start)..block.range.end.min(visible.end);
                if request.is_empty() {
                    continue;
                }
                if requests.len() >= MAX_VISIBLE_CODE_BLOCKS && !requests.contains_key(&block.id) {
                    continue;
                }
                requests
                    .entry(block.id)
                    .or_insert_with(|| (block, Vec::new()))
                    .1
                    .push(request);
            }
        }
        let revision = self.document.revision();
        let retain_scroll =
            self.syntax.markdown.language_change == Some((self.document.id(), revision));
        if !retain_scroll {
            self.syntax.markdown.language_change = None;
        }
        let code_sheet = code_style::snapshot();
        #[cfg(test)]
        let code_sheet = self
            .syntax
            .markdown
            .code_sheet
            .clone()
            .unwrap_or(code_sheet);
        let document_sheet_revision = self.document.projection().style_sheet().revision;
        let mut changed = self.syntax.markdown.revision != Some(revision)
            || self.syntax.markdown.sheet_revision != Some(code_sheet.revision)
            || self.syntax.markdown.document_sheet_revision != Some(document_sheet_revision)
            || self.syntax.markdown.services.keys().ne(requests.keys());
        let mut invalidated = Vec::new();
        for entry in self.syntax.markdown.services.values() {
            if let Some((start, end)) = entry.anchors {
                if let (Ok(start), Ok(end)) = (
                    self.document.resolve_text_anchor(start),
                    self.document.resolve_text_anchor(end),
                ) {
                    if let (Some(start), Some(end)) = (start.value(), end.value()) {
                        if start.offset() <= end.offset() {
                            invalidated.push(start.offset()..end.offset());
                        }
                    }
                }
            }
        }
        self.syntax
            .markdown
            .services
            .retain(|id, _| requests.contains_key(id));
        let mut runs = Vec::new();
        for (id, (block, regions)) in requests {
            let Ok(text) = full.text_tree().subrope(block.range.clone()) else {
                continue;
            };
            let mut input = SyntaxInputSnapshot::new(
                SyntaxInputIdentity {
                    document: self.document.id().0,
                    revision: revision.0,
                    generation: id,
                },
                text,
            );
            let entry =
                self.syntax
                    .markdown
                    .services
                    .entry(id)
                    .or_insert_with(|| MarkdownCodeService {
                        service: self.syntax.service.for_export(),
                        anchors: None,
                        input: None,
                    });
            let anchor = |at, association| {
                self.document.text_point(at).ok().and_then(|point| {
                    self.document
                        .text_anchor(
                            point,
                            association,
                            BoundaryAffinity::Downstream,
                            crate::document::DeletionRecovery::PreferFollowingThenPreceding,
                        )
                        .ok()
                })
            };
            entry.anchors = anchor(
                block.range.start,
                crate::document::Association::AfterInsertion,
            )
            .zip(anchor(
                block.range.end,
                crate::document::Association::BeforeInsertion,
            ));
            entry.service.set_language(block.code_language.clone());
            entry.service.set_filename(None);
            let mut map = None;
            let mut hull = None;
            if let Some(old) = &entry.input {
                match old.text_tree().changed_extent(input.text_tree()) {
                    None => {
                        // Isolated parsing depends only on complete body text and
                        // language/configuration. Exact persistent diff validates
                        // reuse after unrelated document edits or moved blocks.
                        input = old.clone();
                    }
                    Some((old_range, new_range)) => {
                        if let Ok(splice) =
                            crate::document::Splice::new(old_range.clone(), new_range.len())
                        {
                            map = PositionMap::for_text_snapshots(
                                self.document.id(),
                                Revision(old.identity().revision),
                                Revision(input.identity().revision),
                                old.text_tree(),
                                input.text_tree(),
                                vec![splice],
                            )
                            .ok();
                        }
                        if map.is_some() {
                            hull = Some((old_range, new_range));
                        }
                    }
                }
            }
            entry
                .service
                .rebase_input(input.clone(), map.as_ref(), hull);
            entry.input = Some(input.clone());
            changed |= entry.service.poll(input.identity());
            for region in regions {
                entry.service.request(
                    input.clone(),
                    region.start - block.range.start..region.end - block.range.start,
                );
            }
            runs.push(SyntaxRun {
                range: block.range.clone(),
                name: SyntaxStyleName("ViemCodeBase".into()),
                origin: "markdown-code".into(),
                priority: -1,
            });
            runs.extend(
                entry
                    .service
                    .runs(input.identity())
                    .into_iter()
                    .map(|mut run| {
                        run.range =
                            run.range.start + block.range.start..run.range.end + block.range.start;
                        run
                    }),
            );
            invalidated.push(block.range.clone());
        }
        if !changed {
            self.finish_code_language_refresh();
            return false;
        }
        let previous = self.document.projection().clone();
        let sheet =
            code_style::markdown_sheet(self.document.projection().style_sheet(), &code_sheet);
        let viewport_origins = retain_scroll.then(|| {
            self.views
                .iter()
                .map(|(id, view)| (*id, view.layout.viewport_left(), view.layout.viewport_top()))
                .collect::<Vec<_>>()
        });
        let anchors = if retain_scroll {
            BTreeMap::new()
        } else {
            self.caret_anchors_for_publication()
        };
        self.document
            .install_markdown_code_presentation(sheet.clone(), &runs);
        invalidated.sort_by_key(|range| (range.start, range.end));
        invalidated.dedup();
        let mut changed_metrics = false;
        for range in invalidated {
            let identical = self.syntax.markdown.revision == Some(revision)
                && match (
                    crate::layout::DocumentLayoutStyles::resolve_region(&previous, range.clone()),
                    crate::layout::DocumentLayoutStyles::resolve_region(
                        self.document.projection(),
                        range.clone(),
                    ),
                ) {
                    (Ok(old), Ok(new)) => {
                        old.shaping_runs == new.shaping_runs
                            && old.default_shaping_style == new.default_shaping_style
                            && old
                                .paragraphs
                                .iter()
                                .map(|paragraph| &paragraph.default_shaping_style)
                                .eq(new
                                    .paragraphs
                                    .iter()
                                    .map(|paragraph| &paragraph.default_shaping_style))
                    }
                    _ => false,
                };
            if !identical {
                changed_metrics = true;
                self.apply_code_presentation_change(
                    sheet.clone(),
                    crate::coordinator::syntax::metrics::MetricChange::Local(range),
                    anchors.clone(),
                );
            }
        }
        if !changed_metrics {
            self.apply_code_presentation_change(
                sheet,
                crate::coordinator::syntax::metrics::MetricChange::None,
                anchors,
            );
        }
        self.syntax.markdown.revision = Some(revision);
        self.syntax.markdown.sheet_revision = Some(code_sheet.revision);
        self.syntax.markdown.document_sheet_revision = Some(document_sheet_revision);
        if changed_metrics {
            if let Some(origins) = viewport_origins {
                for (id, left, top) in origins {
                    if let Err(error) = self.restore_code_language_viewport(id, left, top) {
                        self.record_presentation_error(id, error);
                    }
                }
            }
        }
        // New font metrics can expose text outside this poll's old viewport
        // requests. Let the next poll request that current visible coverage
        // before it decides whether the language refresh is complete.
        if !changed_metrics {
            self.finish_code_language_refresh();
        }
        true
    }

    fn finish_code_language_refresh(&mut self) {
        // A source refresh first polls with layouts from the old revision.
        // Their estimated regions can contain no code at all; an empty service
        // set at that point says nothing about the new visible syntax work.
        // Retire the scroll policy only after the new viewport is materialized.
        let current_viewports = self.views.values().all(|view| {
            current_snapshot_for_layout(
                &self.document,
                &view.layout,
                inspect_layout_provider(&view.engine),
            )
            .is_some_and(|snapshot| {
                snapshot.missing_viewport_edges(view.layout.viewport_top(), view.layout.height())
                    == (false, false)
            })
        });
        if current_viewports
            && self
                .syntax
                .markdown
                .services
                .values()
                .all(|entry| !entry.service.has_pending_work())
        {
            self.syntax.markdown.language_change = None;
        }
    }
}
