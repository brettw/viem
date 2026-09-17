//! Startup configuration is evaluated once before attaching a buffer's views.
use super::*;

impl<P: TextMeasurementProvider> Core<P> {
    pub fn initialize_startup(
        &mut self,
        text: &str,
    ) -> Vec<crate::command::startup::StartupDiagnostic> {
        if !self.views.is_empty() {
            return vec![crate::command::startup::StartupDiagnostic {
                line: 0,
                message: "startup configuration must be initialized before attaching views".into(),
            }];
        }
        let mut commands = CommandInterpreter::new();
        commands.install_buffer_state(&self.buffer_commands);
        if let Some(options) = &self.startup_view_options {
            commands.install_startup_view_options(options);
        } else {
            commands.set_layout_options(ViewLayout::new(1.0, 1.0).wrap());
            commands.set_visible_whitespace_defaults(
                self.whitespace_defaults.visible_whitespace.enabled,
                &self.whitespace_defaults.visible_whitespace.listchars,
            );
        }
        let diagnostics = commands.configure_startup(&mut self.document, text);
        self.buffer_commands = commands.export_buffer_state();
        self.startup_view_options = Some(commands.startup_view_options());
        diagnostics
    }

    pub fn has_pending_mapping(&self, view_id: ViewId) -> bool {
        self.views
            .get(&view_id)
            .is_some_and(|view| view.commands.has_pending_mapping())
    }

    pub fn flush_mapping_prefix(&mut self, view_id: ViewId) -> Result<CoreOutcome, CoreError> {
        self.handle(view_id, CoreEvent::FlushMappingPrefix)
    }
}
