//! Color-scheme declarations hosted by the shared Vim script runtime.
use super::super::colorscheme::{VimBackground, VimHighlights};
use super::*;
use crate::vimscript::{EvalResult, Fail, HostCommand, Runtime, VimError};

pub(super) struct ColorSession {
    pub(super) highlights: VimHighlights,
    warnings: Vec<VimDiagnostic>,
}
const MAX_WARNINGS: usize = 256;

pub(crate) fn colorscheme(
    name: &str,
    source: &[u8],
    limits: VimLoadLimits,
) -> Result<(VimHighlights, Vec<VimDiagnostic>), Vec<VimDiagnostic>> {
    if source.len() > limits.source_bytes {
        return Err(vec![VimDiagnostic::new(
            name,
            0,
            "color scheme exceeds the source byte budget",
        )]);
    }
    let text =
        commands::decode_source(source).map_err(|e| vec![VimDiagnostic::new(name, 0, &e)])?;
    let mut loader = Loader::new(limits, None);
    loader.colors = Some(ColorSession {
        highlights: VimHighlights::startup(),
        warnings: Vec::new(),
    });
    loader.source(name, &text);
    let mut session = loader.colors.take().unwrap();
    let mut fatal = Vec::new();
    for error in loader.errors {
        if error.message.contains("budget") || error.message.contains("cancelled") {
            fatal.push(error);
        } else if session.warnings.len() < MAX_WARNINGS {
            session.warnings.push(error);
        }
    }
    if !fatal.is_empty() {
        return Err(fatal);
    }
    Ok((session.highlights, session.warnings))
}
impl Loader<'_> {
    pub(super) fn color_command(
        &mut self,
        rt: &mut Runtime,
        command: &HostCommand,
    ) -> EvalResult<()> {
        let rest = command.argument.trim();
        let result = match command.name {
            Some("highlight") => self
                .colors
                .as_mut()
                .unwrap()
                .highlights
                .command(rest, command.bang)
                .map(|cleared| {
                    if cleared {
                        rt.state.globals().remove(b"colors_name");
                    }
                }),
            Some("syntax") => match rest {
                "reset" => {
                    self.colors.as_mut().unwrap().highlights.syntax_reset();
                    Ok(())
                }
                "on" | "enable" => Ok(()),
                _ => Err(format!("unsupported color scheme syntax command: {rest}")),
            },
            Some("set" | "setlocal" | "setglobal") => return self.script_set(rt, rest),
            _ => Err(format!(
                "unsupported color scheme command: {}",
                command.text
            )),
        };
        result.map_err(|e| Fail::Error(VimError::plain(e)))
    }
    pub(super) fn color_background(&mut self, rt: &Runtime, value: &str) -> Result<(), VimError> {
        let background = match value {
            "light" => VimBackground::Light,
            "dark" => VimBackground::Dark,
            _ => {
                return Err(VimError::new(
                    474,
                    format!("Invalid argument: background={value}"),
                ))
            }
        };
        self.colors
            .as_mut()
            .unwrap()
            .highlights
            .set_background(background, rt.state.globals().contains(b"colors_name"));
        Ok(())
    }
}
