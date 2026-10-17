//! Color scheme mode. `:highlight` and 'background' update an import's
//! highlight table; setup control flow, expressions, functions and limits are
//! the syntax loader's own. Unsupported statements are skipped and reported
//! instead of rejecting the scheme, but exhausted budgets and cancellation
//! still end evaluation.
use super::super::colorscheme::{VimBackground, VimHighlights};
use super::*;

pub(super) struct ColorSession {
    highlights: VimHighlights,
    warnings: Vec<VimDiagnostic>,
}

const MAX_WARNINGS: usize = 256;

/// Evaluate a color scheme as gVim loads it with `:colorscheme`, starting from
/// Vim's default state: 'background' light and `:syntax on`.
pub(crate) fn colorscheme(
    name: &str,
    source: &[u8],
    limits: VimLoadLimits,
) -> Result<(VimHighlights, Vec<VimDiagnostic>), Vec<VimDiagnostic>> {
    if source.len() > limits.source_bytes {
        return Err(vec![VimDiagnostic::new(name, 0, "color scheme exceeds the source byte budget")]);
    }
    let text = commands::decode_source(source).map_err(|error| vec![VimDiagnostic::new(name, 0, &error)])?;
    let mut loader = Loader::new(limits, None);
    loader.setup.begin_colorscheme(VimBackground::Light.as_str());
    loader.colors = Some(ColorSession { highlights: VimHighlights::startup(), warnings: Vec::new() });
    loader.source(name, &text);
    if !loader.errors.is_empty() {
        return Err(loader.errors);
    }
    let session = loader.colors.take().expect("color session");
    Ok((session.highlights, session.warnings))
}

impl Loader<'_> {
    /// Commands with color scheme meaning. `None` defers to the setup loader.
    pub(super) fn colorscheme_command(&mut self, cmd: &str, rest: &str, forced: bool) -> Option<Result<(), String>> {
        let bare = cmd.trim_end_matches('!');
        Some(match commands::command_name(cmd) {
            "highlight" => {
                let session = self.colors.as_mut()?;
                match session.highlights.command(rest, forced) {
                    // `:highlight clear` also forgets the scheme's name.
                    Ok(true) => self.setup.unlet("g:colors_name", true),
                    Ok(false) => Ok(()),
                    Err(error) => Err(error),
                }
            }
            "set" | "setlocal" => self.colorscheme_options(rest),
            "syntax" => match rest.split_whitespace().collect::<Vec<_>>().as_slice() {
                ["reset"] => {
                    self.colors.as_mut()?.highlights.syntax_reset();
                    Ok(())
                }
                ["on" | "enable"] => Ok(()),
                _ => Err(format!("unsupported color scheme syntax command: {rest}")),
            },
            "let" => {
                let (target, expression) = rest.split_once('=')?;
                let target = target.trim();
                let option = target.strip_prefix('&')?;
                let option = option.strip_prefix("l:").or_else(|| option.strip_prefix("g:")).unwrap_or(option);
                if !matches!(option, "background" | "bg") {
                    // Editor and terminal options do not change GUI colors.
                    return Some(Ok(()));
                }
                self.setup
                    .evaluate(expression.trim())
                    .and_then(|value| value.text())
                    .and_then(|value| self.colorscheme_background(&value))
            }
            _ if matches!(bare, "sil" | "sile" | "silen" | "silent") => self.command(rest),
            _ if matches!(bare, "colo" | "colorscheme" | "colors" | "colorsc" | "colorsch" | "colorsche" | "colorschem") => {
                Err("a color scheme that loads another color scheme is imported without it".into())
            }
            _ => return None,
        })
    }

    fn colorscheme_options(&mut self, source: &str) -> Result<(), String> {
        for option in source.split_whitespace().take_while(|word| !word.starts_with('"')) {
            let Some(rest) = option.strip_prefix("background").or_else(|| option.strip_prefix("bg")) else {
                // Other options are editor or terminal state, not GUI colors.
                continue;
            };
            match rest {
                "&" | "&vim" | "&vi" => self.colorscheme_background("light")?,
                "" | "?" => {}
                _ => match rest.strip_prefix('=').or_else(|| rest.strip_prefix(':')) {
                    Some(value) => self.colorscheme_background(value)?,
                    None => return Err(format!("E518: Unknown option: {option}")),
                },
            }
        }
        Ok(())
    }

    fn colorscheme_background(&mut self, value: &str) -> Result<(), String> {
        let background = match value {
            "light" => VimBackground::Light,
            "dark" => VimBackground::Dark,
            _ => return Err(format!("E474: Invalid argument: background={value}")),
        };
        let named = self.setup.evaluate("exists('g:colors_name')")?.truth()?;
        self.colors
            .as_mut()
            .ok_or("color scheme mode is inactive")?
            .highlights
            .set_background(background, named);
        self.setup.set_colorscheme_background(background.as_str());
        Ok(())
    }

    /// Move ordinary statement failures to warnings. Returns false when an
    /// exhausted budget or cancellation must still end evaluation.
    pub(super) fn demote_colorscheme_errors(&mut self, from: usize) -> bool {
        let Some(session) = self.colors.as_mut() else {
            return false;
        };
        let mut fatal = Vec::new();
        for error in self.errors.drain(from..) {
            if error.message.contains("budget") || error.message.contains("cancelled") {
                fatal.push(error);
            } else if session.warnings.len() < MAX_WARNINGS {
                session.warnings.push(error);
            }
        }
        let recovered = fatal.is_empty();
        self.errors.extend(fatal);
        recovered
    }
}
