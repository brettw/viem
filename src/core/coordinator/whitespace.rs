//! Connect buffer editing columns and view presentation without changing source.
use super::*;
use crate::document::IndentationOptions;
use crate::layout::{LayoutRect, WhitespaceMarker, WhitespacePresentationOptions};

impl<P: TextMeasurementProvider> Core<P> {
    /// The native menu and Ex `list` share one view-local override.
    pub fn set_visible_whitespace(&mut self, id: ViewId, enabled: bool) -> Result<(), CoreError> {
        self.views
            .get_mut(&id)
            .ok_or(CoreError::UnknownView(id))?
            .commands
            .set_visible_whitespace(enabled);
        self.materialize_immediate_viewport(id, ImmediateLayoutIntent::PreserveViewport)?;
        self.rematerialize_active_composition(id, false)
    }
    pub fn set_indentation_defaults(
        &mut self,
        options: IndentationOptions,
    ) -> Result<(), CoreError> {
        options.validate().map_err(|_| LayoutError::InvalidStyle)?;
        if self.buffer_commands.indentation.defaults() == options {
            return Ok(());
        }
        self.buffer_commands.indentation.set_default(options);
        for view in self.views.values_mut() {
            view.commands
                .set_indentation_defaults(options)
                .map_err(|_| LayoutError::InvalidStyle)?;
        }
        self.refresh_whitespace_views()
    }

    pub fn set_whitespace_presentation_defaults(
        &mut self,
        options: WhitespacePresentationOptions,
    ) -> Result<(), CoreError> {
        options.validate().map_err(|_| LayoutError::InvalidStyle)?;
        if self.whitespace_defaults == options {
            return Ok(());
        }
        self.whitespace_defaults = options;
        self.refresh_whitespace_views()
    }

    fn refresh_whitespace_views(&mut self) -> Result<(), CoreError> {
        for id in self.views.keys().copied().collect::<Vec<_>>() {
            // Defaults have been validated and published. A transient provider
            // failure belongs to this view's presentation, and must not stop
            // other views from receiving the same committed settings.
            let result = self
                .materialize_immediate_viewport(id, ImmediateLayoutIntent::PreserveViewport)
                .and_then(|_| self.rematerialize_active_composition(id, false));
            if let Err(error) = result {
                self.record_presentation_error(id, error);
            }
        }
        Ok(())
    }

    pub(super) fn synchronize_whitespace(&mut self, id: ViewId) -> Result<(), CoreError> {
        let view = self.views.get_mut(&id).ok_or(CoreError::UnknownView(id))?;
        view.commands.set_visible_whitespace_defaults(
            self.whitespace_defaults.visible_whitespace.enabled,
            &self.whitespace_defaults.visible_whitespace.listchars,
        );
        let mut options = self.whitespace_defaults.clone();
        options.visible_whitespace.enabled = view.commands.visible_whitespace().enabled();
        options.visible_whitespace.listchars =
            view.commands.visible_whitespace().listchars().into();
        let before = view.layout.configuration_generation();
        view.layout
            .set_whitespace_presentation(
                options,
                self.document.format(),
                view.commands.indentation_options().tabstop,
            )
            .map_err(|_| LayoutError::InvalidStyle)?;
        if before != view.layout.configuration_generation() {
            cancel_active_layout_work(view);
        }
        Ok(())
    }

    /// Markers refer to the same source/composition and immutable layout that
    /// supplies text, caret and selection geometry to the frontend.
    pub fn whitespace_markers(&self, id: ViewId) -> Result<Vec<WhitespaceMarker>, CoreError> {
        let view = self.views.get(&id).ok_or(CoreError::UnknownView(id))?;
        let layout = view.composition_layout.as_ref().unwrap_or(&view.layout);
        let snapshot = layout.snapshot().ok_or(LayoutError::NoRows)?;
        let viewport = LayoutRect {
            x: layout.viewport_left(),
            y: layout.viewport_top(),
            width: layout.width(),
            height: layout.height(),
        };
        if let Some(composition) = &view.composition {
            let overlay = composition.overlay(&self.document)?;
            let tree = overlay
                .layout_text_tree()
                .map_err(DocumentError::FormattedTextStorage)?;
            Ok(layout.whitespace_markers(&tree, snapshot, viewport))
        } else {
            Ok(layout.whitespace_markers(
                self.document.projection().text_tree(),
                snapshot,
                viewport,
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::{CommandStatus, Key};
    use crate::layout::MockTextMeasurementProvider;

    fn ex(core: &mut Core<MockTextMeasurementProvider>, id: ViewId, text: &str) -> CommandStatus {
        for c in text.chars() {
            core.handle(id, CoreEvent::Input(InputEvent::key(c)))
                .unwrap();
        }
        core.handle(id, CoreEvent::Input(InputEvent::Key(Key::Enter)))
            .unwrap()
            .command
            .unwrap()
            .status
    }

    #[test]
    fn list_overrides_are_view_local_and_return_to_live_defaults() {
        let mut core = Core::new(Document::new("\tword  "));
        let a = core.add_view(MockTextMeasurementProvider::new(), 200.0, 100.0);
        let b = core.add_view(MockTextMeasurementProvider::new(), 200.0, 100.0);
        assert!(!core.whitespace_markers(a).unwrap().is_empty());
        let source = core.document.source_bytes();
        let revision = core.document.revision();
        let history = core.document.history_status();
        assert_eq!(ex(&mut core, a, ":set nolist"), CommandStatus::Complete);
        assert!(core.whitespace_markers(a).unwrap().is_empty());
        assert!(!core.whitespace_markers(b).unwrap().is_empty());
        let mut options = WhitespacePresentationOptions::default();
        options.visible_whitespace.listchars = "trail:!".into();
        core.set_whitespace_presentation_defaults(options).unwrap();
        assert!(core.whitespace_markers(a).unwrap().is_empty());
        assert_eq!(ex(&mut core, a, ":set list<"), CommandStatus::Complete);
        assert!(core
            .whitespace_markers(a)
            .unwrap()
            .iter()
            .any(|m| m.text == "!"));
        assert_eq!(
            ex(&mut core, a, ":set lcs=trail:?"),
            CommandStatus::Complete
        );
        assert!(core
            .whitespace_markers(a)
            .unwrap()
            .iter()
            .any(|m| m.text == "?"));
        assert!(!core
            .whitespace_markers(b)
            .unwrap()
            .iter()
            .any(|m| m.text == "?"));
        assert_eq!(core.document.source_bytes(), source);
        assert_eq!(core.document.revision(), revision);
        assert_eq!(core.document.history_status(), history);
    }

    #[test]
    fn invalid_ex_batch_and_invalid_defaults_leave_options_and_geometry_untouched() {
        let mut core = Core::new(Document::new(" a  "));
        let id = core.add_view(MockTextMeasurementProvider::new(), 200.0, 100.0);
        let before = core.layout(id).unwrap().configuration_generation();
        assert!(!matches!(
            ex(&mut core, id, ":set nolist lcs=tab:x"),
            CommandStatus::Complete
        ));
        assert!(core.views[&id].commands.visible_whitespace().enabled());
        let mut options = WhitespacePresentationOptions::default();
        options.visible_whitespace.style.size = Some(-1.0);
        assert!(core.set_whitespace_presentation_defaults(options).is_err());
        assert_eq!(core.layout(id).unwrap().configuration_generation(), before);
    }

    #[test]
    fn a_provider_failure_does_not_skip_later_views_after_valid_settings_publish() {
        let mut core = Core::new(Document::new("\tword  "));
        let a = core.add_view(MockTextMeasurementProvider::new(), 200.0, 100.0);
        let b = core.add_view(MockTextMeasurementProvider::new(), 200.0, 100.0);
        core.views
            .get_mut(&a)
            .unwrap()
            .engine
            .provider_mut()
            .set_metrics_generation(crate::layout::MetricsGeneration(99));
        core.views
            .get_mut(&a)
            .unwrap()
            .engine
            .provider_mut()
            .fail_next_batch("temporary font service failure");
        core.set_indentation_defaults(IndentationOptions {
            tabstop: 8,
            ..Default::default()
        })
        .unwrap();
        assert_eq!(core.layout(a).unwrap().whitespace_tabstop(), 8);
        assert_eq!(core.layout(b).unwrap().whitespace_tabstop(), 8);
        assert!(core.layout(a).unwrap().last_error().is_some());
        assert!(core.layout(b).unwrap().last_error().is_none());
        core.materialize_immediate_viewport(a, ImmediateLayoutIntent::PreserveViewport)
            .unwrap();
        assert!(core.layout(a).unwrap().last_error().is_none());
    }

    #[test]
    fn tabstop_is_shared_by_buffer_editing_reflow_and_every_view_layout() {
        let mut core = Core::new(Document::new("\ta"));
        let a = core.add_view(MockTextMeasurementProvider::new(), 200.0, 100.0);
        let b = core.add_view(MockTextMeasurementProvider::new(), 200.0, 100.0);
        assert_eq!(ex(&mut core, a, ":set ts=8 sw=0"), CommandStatus::Complete);
        assert_eq!(core.layout(a).unwrap().whitespace_tabstop(), 8);
        assert_eq!(core.layout(b).unwrap().whitespace_tabstop(), 8);
        core.set_indentation_defaults(IndentationOptions {
            tabstop: 4,
            softtabstop: 4,
            ..Default::default()
        })
        .unwrap();
        assert_eq!(
            core.views[&a].commands.indentation_options().shift_width(),
            8
        );
        assert_eq!(core.views[&b].commands.indentation_options().softtabstop, 4);
        assert_eq!(ex(&mut core, b, ":set ts<"), CommandStatus::Complete);
        assert_eq!(core.layout(b).unwrap().whitespace_tabstop(), 4);
    }
    fn keys(core: &mut Core<MockTextMeasurementProvider>, id: ViewId, text: &str) {
        for character in text.chars() {
            event(core, id, InputEvent::key(character));
        }
    }
    fn event(core: &mut Core<MockTextMeasurementProvider>, id: ViewId, input: InputEvent) {
        let outcome = core.handle(id, CoreEvent::Input(input.clone())).unwrap();
        assert!(
            !outcome
                .command
                .is_some_and(|command| matches!(command.status, CommandStatus::Error(_))),
            "{input:?}"
        );
    }

    #[test]
    fn real_dispatch_continues_comments_closes_star_and_undoes_atomically() {
        let document = Document::from_bytes(
            b"/*".to_vec(),
            crate::document::Encoding::Utf8,
            crate::document::Format::Code,
        )
        .unwrap();
        let mut core = Core::new(document);
        let id = core.add_view(MockTextMeasurementProvider::new(), 400.0, 150.0);
        core.set_code_language(
            crate::document::syntax::detection::LanguageSelection::Language("cpp".into()),
        );
        keys(&mut core, id, "A");
        event(&mut core, id, InputEvent::Key(Key::Enter));
        assert_eq!(core.document.text(), "/*\n * ");
        event(&mut core, id, InputEvent::text("/"));
        assert_eq!(core.document.text(), "/*\n */");
        event(&mut core, id, InputEvent::Key(Key::Escape));
        keys(&mut core, id, "u");
        assert_eq!(core.document.text(), "/*");
    }

    #[test]
    fn real_dispatch_counted_open_and_dot_use_destination_indentation() {
        let mut core = Core::new(Document::new("  first\n    last"));
        let id = core.add_view(MockTextMeasurementProvider::new(), 400.0, 150.0);
        keys(&mut core, id, "2o");
        event(&mut core, id, InputEvent::text("new"));
        event(&mut core, id, InputEvent::Key(Key::Escape));
        assert_eq!(core.document.text(), "  first\n  new\n  new\n    last");
        keys(&mut core, id, "G.");
        assert_eq!(
            core.document.text(),
            "  first\n  new\n  new\n    last\n    new\n    new"
        );
        keys(&mut core, id, "u");
        assert_eq!(core.document.text(), "  first\n  new\n  new\n    last");
    }

    #[test]
    fn real_dispatch_tab_backspace_and_empty_autoindent_cleanup() {
        let mut core = Core::new(Document::new("  word"));
        let id = core.add_view(MockTextMeasurementProvider::new(), 400.0, 150.0);
        keys(&mut core, id, "I");
        event(&mut core, id, InputEvent::Key(Key::Tab));
        assert_eq!(core.document.text(), "    word");
        event(&mut core, id, InputEvent::Key(Key::Backspace));
        assert_eq!(core.document.text(), "  word");
        event(&mut core, id, InputEvent::Key(Key::Escape));
        keys(&mut core, id, "A");
        event(&mut core, id, InputEvent::Key(Key::Enter));
        assert_eq!(core.document.text(), "  word\n  ");
        event(&mut core, id, InputEvent::Key(Key::Escape));
        assert_eq!(core.document.text(), "  word\n");
        keys(&mut core, id, "u");
        assert_eq!(core.document.text(), "  word");
    }
    #[test]
    fn wrapped_open_above_does_not_insert_indentation_into_preceding_text() {
        let original = "  one two three four five six seven eight";
        let mut core = Core::new(Document::new(original));
        let id = core.add_view(MockTextMeasurementProvider::new(), 50.0, 400.0);
        assert_eq!(ex(&mut core, id, ":set wrap"), CommandStatus::Complete);
        let snapshot = core.layout(id).unwrap().snapshot().unwrap();
        assert!(snapshot.rows.len() > 2);
        let split = snapshot.rows[1].text_range.start;
        keys(&mut core, id, "jO");
        assert_eq!(
            core.document.text(),
            format!("{}\n{}", &original[..split], &original[split..])
        );
        event(&mut core, id, InputEvent::Key(Key::Escape));
        keys(&mut core, id, "u");
        assert_eq!(core.document.text(), original);
    }
}
