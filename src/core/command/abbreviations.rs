//! Vim abbreviations (`:abbreviate` and its mode and `nore` forms), expanded
//! in Insert/Replace and command-line input when a non-keyword character,
//! <Esc>, <CR> or CTRL-] follows a typed abbreviation.
use super::mappings::parse_map_keys;
use super::*;
use std::sync::Arc;
use unicode_segmentation::UnicodeSegmentation;

const INSERT: u8 = 1;
const COMMAND_LINE: u8 = 2;

#[derive(Clone, Debug, Default)]
pub(crate) struct Abbreviations(Arc<Vec<Abbreviation>>);

#[derive(Clone, Debug)]
struct Abbreviation {
    mode: u8,
    lhs: String,
    rhs_text: String,
    recursive: bool,
    expr: bool,
    buffer: bool,
    sid: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Operation {
    Define { recursive: bool },
    Remove,
    Clear,
}

fn command_kind(name: &str) -> Option<(u8, Operation)> {
    let both = INSERT | COMMAND_LINE;
    Some(match name {
        "abbreviate" => (both, Operation::Define { recursive: true }),
        "noreabbrev" => (both, Operation::Define { recursive: false }),
        "iabbrev" => (INSERT, Operation::Define { recursive: true }),
        "inoreabbrev" => (INSERT, Operation::Define { recursive: false }),
        "cabbrev" => (COMMAND_LINE, Operation::Define { recursive: true }),
        "cnoreabbrev" => (COMMAND_LINE, Operation::Define { recursive: false }),
        "unabbreviate" => (both, Operation::Remove),
        "iunabbrev" => (INSERT, Operation::Remove),
        "cunabbrev" => (COMMAND_LINE, Operation::Remove),
        "abclear" => (both, Operation::Clear),
        "iabclear" => (INSERT, Operation::Clear),
        "cabclear" => (COMMAND_LINE, Operation::Clear),
        _ => return None,
    })
}

pub(crate) fn is_abbreviation_command(name: &str) -> bool {
    command_kind(name).is_some()
}

/// Vim's default 'iskeyword' (`@,48-57,_,192-255`) for Unicode text.
#[cfg(test)]
fn is_keyword(character: char) -> bool {
    character.is_alphanumeric() || character == '_'
}

/// Vim accepts three shapes: all keyword characters ("full-id"), keyword
/// last with only non-keyword characters before ("end-id"), or a
/// non-keyword last character ("non-id"); none contain white space.
fn valid_lhs(lhs: &str, is_keyword: impl Fn(char) -> bool) -> bool {
    let Some(last) = lhs.chars().last() else { return false };
    if lhs.chars().any(char::is_whitespace) {
        return false;
    }
    if !is_keyword(last) {
        return true;
    }
    let mut before = lhs.chars().rev().skip(1);
    lhs.chars().all(&is_keyword) || before.all(|character| !is_keyword(character))
}

fn mode_chars(mode: u8) -> &'static str {
    match mode {
        m if m == INSERT | COMMAND_LINE => "!",
        INSERT => "i",
        _ => "c",
    }
}

#[derive(Default)]
struct Attributes {
    buffer: bool,
    expr: bool,
}

fn parse_attributes(mut text: &str) -> (Attributes, &str) {
    let mut attributes = Attributes::default();
    loop {
        text = text.trim_start_matches([' ', '\t']);
        let lower = text.to_ascii_lowercase();
        if lower.starts_with("<buffer>") {
            attributes.buffer = true;
            text = &text[8..];
        } else if lower.starts_with("<expr>") {
            attributes.expr = true;
            text = &text[6..];
        } else if lower.starts_with("<silent>") || lower.starts_with("<script>") || lower.starts_with("<special>") {
            text = &text[text.find('>').expect("checked prefix") + 1..];
        } else {
            return (attributes, text);
        }
    }
}

/// One expansion: delete `erase` graphemes before the cursor, then input
/// `keys` (remapped when `recursive`).
pub(crate) struct Expansion {
    pub erase: usize,
    pub keys: Vec<Key>,
    pub recursive: bool,
}

impl Abbreviations {
    pub(crate) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    fn has_mode(&self, mode: u8) -> bool {
        self.0.iter().any(|abbreviation| abbreviation.mode & mode != 0)
    }

    /// Execute an abbreviation command by full name. `Ok(Some(text))` is a
    /// listing to show.
    pub(crate) fn execute_command(&mut self, name: &str, argument: &str, sid: usize, is_keyword: impl Fn(char) -> bool) -> Result<Option<String>, String> {
        let (mode, operation) = command_kind(name).ok_or_else(|| format!("E492: Not an editor command: {name}"))?;
        let (attributes, rest) = parse_attributes(argument);
        let rest = rest.trim_start_matches([' ', '\t']);
        if operation == Operation::Clear {
            if !rest.is_empty() { return Err(format!("E474: Invalid argument: {rest}")); }
            Arc::make_mut(&mut self.0).retain(|abbreviation| {
                abbreviation.mode & mode == 0 || abbreviation.buffer != attributes.buffer
            });
            return Ok(None);
        }
        let (lhs, rhs) = match rest.find([' ', '\t']) {
            Some(at) => (&rest[..at], rest[at..].trim_start_matches([' ', '\t'])),
            None => (rest, ""),
        };
        if operation == Operation::Remove {
            if lhs.is_empty() {
                return Err("E471: Argument required".into());
            }
            let before = self.0.len();
            // Vim also removes by right-hand side when no left side matches.
            Arc::make_mut(&mut self.0).retain(|abbreviation| abbreviation.mode & mode == 0 || abbreviation.buffer != attributes.buffer || abbreviation.lhs != lhs);
            if self.0.len() == before {
                Arc::make_mut(&mut self.0).retain(|abbreviation| abbreviation.mode & mode == 0 || abbreviation.buffer != attributes.buffer || abbreviation.rhs_text != lhs);
            }
            if self.0.len() == before {
                return Err("E24: No such abbreviation".into());
            }
            return Ok(None);
        }
        if lhs.is_empty() || rhs.is_empty() {
            return Ok(Some(self.listing(mode, (!lhs.is_empty()).then_some(lhs), attributes.buffer)));
        }
        if !valid_lhs(lhs, &is_keyword) {
            return Err(format!("E474: Invalid argument: {lhs}"));
        }
        let Operation::Define { recursive } = operation else { unreachable!("handled above") };
        let abbreviations = Arc::make_mut(&mut self.0);
        for selected in [INSERT, COMMAND_LINE].into_iter().filter(|selected| selected & mode != 0) {
            abbreviations.retain(|abbreviation| abbreviation.mode != selected || abbreviation.buffer != attributes.buffer || abbreviation.lhs != lhs);
            abbreviations.push(Abbreviation {
                mode: selected,
                lhs: lhs.to_owned(),
                rhs_text: rhs.to_owned(),
                recursive,
                expr: attributes.expr,
                buffer: attributes.buffer,
                sid,
            });
        }
        Ok(None)
    }

    fn listing(&self, mode: u8, prefix: Option<&str>, buffer_only: bool) -> String {
        let mut groups: Vec<(u8, &Abbreviation)> = Vec::new();
        for abbreviation in self.0.iter().filter(|abbreviation| {
            abbreviation.mode & mode != 0 && (!buffer_only || abbreviation.buffer) && prefix.is_none_or(|prefix| abbreviation.lhs.starts_with(prefix))
        }) {
            match groups.iter_mut().find(|(_, first)| {
                first.lhs == abbreviation.lhs
                    && first.rhs_text == abbreviation.rhs_text
                    && first.recursive == abbreviation.recursive
                    && first.expr == abbreviation.expr
                    && first.buffer == abbreviation.buffer
            }) {
                Some((modes, _)) => *modes |= abbreviation.mode,
                None => groups.push((abbreviation.mode, abbreviation)),
            }
        }
        if groups.is_empty() {
            return "No abbreviation found".into();
        }
        groups
            .iter()
            .map(|(modes, abbreviation)| {
                let mut line = format!("{:<3}{}", mode_chars(*modes), abbreviation.lhs);
                while line.chars().count() < 15 {
                    line.push(' ');
                }
                if !line.ends_with(' ') {
                    line.push(' ');
                }
                line.push(if abbreviation.recursive { ' ' } else { '*' });
                line.push(if abbreviation.buffer { '@' } else { ' ' });
                line.push_str(&abbreviation.rhs_text);
                line
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub(crate) fn script_query(&self, function: &str, args: &[crate::vimscript::Value]) -> crate::vimscript::Value {
        use crate::vimscript::{Dict, Value};
        let text = |index: usize| args.get(index).and_then(|value| value.to_text().ok()).unwrap_or_default();
        let mode = match text(1).as_str() { "c" => COMMAND_LINE, "i" => INSERT, _ => INSERT | COMMAND_LINE };
        let lhs = text(0);
        let found = self.0.iter().filter(|entry| entry.mode & mode != 0 && match function {
            "hasmapto" => entry.rhs_text.contains(&lhs),
            "mapcheck" => entry.lhs.starts_with(&lhs) || lhs.starts_with(&entry.lhs),
            _ => entry.lhs == lhs,
        }).max_by_key(|entry| entry.buffer);
        if function == "hasmapto" { return Value::Number(found.is_some() as i64); }
        if function == "maparg" && args.get(3).and_then(|value| value.to_number().ok()).unwrap_or(0) != 0 {
            let Some(entry) = found else { return Value::Dict(Dict::new()); };
            return Value::Dict(Dict::from_pairs([
                ("lhs".into(), Value::str(&entry.lhs)), ("rhs".into(), Value::str(&entry.rhs_text)),
                ("expr".into(), Value::Number(entry.expr as i64)), ("buffer".into(), Value::Number(entry.buffer as i64)),
                ("noremap".into(), Value::Number(!entry.recursive as i64)), ("abbr".into(), Value::Number(1)),
                ("sid".into(), Value::Number(entry.sid as i64)), ("mode".into(), Value::str(mode_chars(entry.mode))),
            ]));
        }
        Value::String(found.map(|entry| entry.rhs_text.clone()).unwrap_or_default().into())
    }

    /// The abbreviation `before` (text before the cursor) ends with, for
    /// `mode`. `typed` is the text inserted in this Insert session: an
    /// abbreviation must have been typed there. The longest match wins.
    fn find(&self, mode: u8, before: &str, typed: Option<&str>, is_keyword: impl Fn(char) -> bool) -> Option<&Abbreviation> {
        self.0
            .iter()
            .filter(|abbreviation| {
                if abbreviation.mode & mode == 0 || !before.ends_with(&abbreviation.lhs) {
                    return false;
                }
                if typed.is_some_and(|typed| !typed.ends_with(&abbreviation.lhs)) {
                    return false;
                }
                let previous = if typed == Some(abbreviation.lhs.as_str()) { None }
                    else { before[..before.len() - abbreviation.lhs.len()].chars().last() };
                if abbreviation.lhs.chars().all(&is_keyword) {
                    if abbreviation.lhs.chars().count() == 1 {
                        previous.is_none_or(char::is_whitespace)
                    } else {
                        previous.is_none_or(|character| !is_keyword(character))
                    }
                } else if abbreviation.lhs.chars().last().is_some_and(&is_keyword) {
                    previous.is_none_or(|character| is_keyword(character) || character.is_whitespace())
                } else {
                    previous.is_none_or(char::is_whitespace)
                }
            })
            .max_by_key(|abbreviation| (abbreviation.buffer, abbreviation.lhs.len()))
    }
}

/// Whether `event` triggers abbreviation expansion.
fn triggers(event: &InputEvent, is_keyword: impl Fn(char) -> bool) -> bool {
    match event {
        InputEvent::Key(Key::Char(character)) => !is_keyword(*character),
        InputEvent::Key(Key::Enter | Key::Escape | Key::Tab | Key::Ctrl(']')) => true,
        InputEvent::Text(text) => text.chars().next().is_some_and(|character| !is_keyword(character)),
        _ => false,
    }
}

impl CommandInterpreter {
    fn abbreviation_mode(&self) -> Option<u8> {
        match self.mode {
            Mode::Insert | Mode::Replace => Some(INSERT),
            Mode::CommandLine => Some(COMMAND_LINE),
            _ => None,
        }
    }

    /// Cheap check for `resolve`: the typed fast path must not bypass a
    /// possible expansion.
    pub(super) fn abbreviation_applies(&self, event: &InputEvent) -> bool {
        !self.abbreviations().is_empty()
            && !self.expanding_abbreviation
            && self.abbreviation_mode().is_some_and(|mode| self.abbreviations().has_mode(mode))
            && triggers(event, |character| self.vim_options.is_keyword(character))
    }

    fn abbreviations(&self) -> &Abbreviations {
        &self.mappings.1
    }

    /// Execute an abbreviation command; a listing becomes a message.
    pub(crate) fn execute_abbreviation_command(&mut self, name: &str, argument: &str, sid: usize) -> CommandOutput {
        match self.mappings.1.execute_command(name, argument, sid, |character| self.vim_options.is_keyword(character)) {
            Ok(None) => CommandOutput::complete(),
            Ok(Some(listing)) => {
                let mut outcome = ExOutcome::default();
                outcome.frontend_requests.push(ExFrontendRequest::Info(ex_execute::ExInfoRequest::Message(listing)));
                CommandOutput { ex_outcome: Some(outcome), ..CommandOutput::complete() }
            }
            Err(message) => CommandOutput { status: CommandStatus::Error(message), ..CommandOutput::complete() },
        }
    }

    fn abbreviation_expansion(&mut self, document: &mut Document, event: &InputEvent) -> Option<Expansion> {
        if self.mapping_suppressed || self.literal_input_pending() || !self.abbreviation_applies(event) {
            return None;
        }
        let mode = self.abbreviation_mode()?;
        let (before, typed) = if mode == COMMAND_LINE {
            let buffer = &self.command_line_state.as_ref()?.buffer;
            (buffer.input[..buffer.cursor.min(buffer.input.len())].to_owned(), None)
        } else {
            let cursor = self.cursor.min(document.projection().text_tree().byte_len());
            let line = document.hard_line_at_offset(cursor)?;
            let start = document.hard_line_snapshot().line(line)?.content_range().start;
            let before = document.projection().text_tree().slice(start..cursor).ok()?.to_string();
            let typed = self.insert_session.as_ref().map(|session| session.last_inserted.text.clone()).unwrap_or_default();
            (before, Some(typed))
        };
        let abbreviation = self.abbreviations().find(mode, &before, typed.as_deref(), |character| self.vim_options.is_keyword(character))?.clone();
        let keys = if abbreviation.expr {
            let saved = self.script_count.replace(self.count.unwrap_or(0));
            let mut runtime = crate::vimscript::Runtime::new(self.script.clone());
            let mut host = super::scripting::InterpreterHost::new(self, document);
            host.textlock = true;
            let value = runtime.eval_expression(&mut host, abbreviation.rhs_text.as_bytes(), abbreviation.sid);
            drop(host);
            self.script_count = saved;
            let bytes = value.ok()?.to_bytes().ok()?;
            super::mappings::script_keys(&bytes)
        } else {
            parse_map_keys(&abbreviation.rhs_text)
        };
        Some(Expansion { erase: abbreviation.lhs.graphemes(true).count(), keys, recursive: abbreviation.recursive })
    }

    /// Expand an abbreviation before `event` is processed. Returns the
    /// expansion's output; the caller then processes `event` itself, except
    /// CTRL-] which only expands.
    pub(super) fn handle_abbreviation(
        &mut self,
        document: &mut Document,
        event: &InputEvent,
    ) -> Result<Option<(CommandOutput, bool)>, DocumentError> {
        let Some(expansion) = self.abbreviation_expansion(document, event) else { return Ok(None) };
        let mut output = CommandOutput::complete();
        let checkpoint = self.clone();
        let document_checkpoint = document.begin_command_checkpoint();
        self.expanding_abbreviation = true;
        let planned = self.set_plan_compound_replay(false);
        let result = (|| {
            for _ in 0..expansion.erase {
                let old = self.set_mapping_suppressed(true);
                let next = self.handle(document, InputEvent::Key(Key::Backspace));
                self.set_mapping_suppressed(old);
                let next = next?;
                let stop = command_status_stops_compound(&next.status);
                output.merge(next);
                if stop { return Ok(()); }
            }
            for key in expansion.keys {
                let old = self.set_mapping_suppressed(!expansion.recursive);
                let next = self.handle(document, InputEvent::Key(key));
                self.set_mapping_suppressed(old);
                let next = next?;
                let stop = command_status_stops_compound(&next.status);
                output.merge(next);
                if stop { return Ok(()); }
            }
            Ok(())
        })();
        self.expanding_abbreviation = false;
        self.set_plan_compound_replay(planned);
        if result.is_err() || command_status_stops_compound(&output.status) {
            document.rollback_command_checkpoint(document_checkpoint);
            *self = checkpoint;
            result?;
            output.document_changed = false;
            return Ok(Some((output, true)));
        }
        document.commit_command_checkpoint(document_checkpoint);
        let consumed = matches!(event, InputEvent::Key(Key::Ctrl(']')));
        Ok(Some((output, consumed)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn left_sides_follow_vims_three_shapes() {
        for valid in ["teh", "#i", "#$i", "a.", ";;", "foo_bar"] {
            assert!(valid_lhs(valid, is_keyword), "{valid}");
        }
        for invalid in ["a.b", "a b", "", "x#y"] {
            assert!(!valid_lhs(invalid, is_keyword), "{invalid}");
        }
    }

    #[test]
    fn matches_need_a_boundary_and_typed_text() {
        let mut abbreviations = Abbreviations::default();
        abbreviations.execute_command("iabbrev", "teh the", 0, is_keyword).unwrap();
        abbreviations.execute_command("iabbrev", "#i #include", 0, is_keyword).unwrap();
        assert_eq!(abbreviations.find(INSERT, "say teh", Some("teh"), is_keyword).unwrap().rhs_text, "the");
        assert!(abbreviations.find(INSERT, "steh", Some("steh"), is_keyword).is_none());
        assert!(abbreviations.find(INSERT, "say teh", Some("eh"), is_keyword).is_none());
        assert!(abbreviations.find(INSERT, "x#i", Some("x#i"), is_keyword).is_some());
        assert!(abbreviations.find(INSERT, ">#i", Some(">#i"), is_keyword).is_none());
        assert_eq!(abbreviations.find(INSERT, " #i", Some("#i"), is_keyword).unwrap().rhs_text, "#include");
        assert!(abbreviations.find(COMMAND_LINE, "teh", None, is_keyword).is_none());
        assert_eq!(abbreviations.listing(INSERT, None, false), "i  teh           the\ni  #i            #include");
        assert_eq!(abbreviations.execute_command("iunabbrev", "nosuch", 0, is_keyword), Err("E24: No such abbreviation".into()));
    }
}
