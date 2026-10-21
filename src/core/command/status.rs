//! View-local command status; no frontend reconstructs pending Vim grammar.
use super::*;
impl CommandInterpreter {
    pub fn recording_register(&self) -> Option<char> {
        self.recording.as_ref().map(|(name, _)| *name)
    }
    /// Bounded presentation only. It is never parsed to execute or replay input.
    pub fn pending_command_text(&self) -> String {
        if !self.mapping_pending.is_empty() {
            return bounded(&mappings::render_key_notation(&self.mapping_pending));
        }
        if self.mode == Mode::CommandLine || !self.command_input_pending() {
            return String::new();
        }
        bounded(&self.status_input)
    }
    pub fn active_count(&self) -> Option<usize> {
        let operator = match self.pending {
            Pending::Operator(op)
            | Pending::OperatorFind { operator: op, .. }
            | Pending::OperatorBracket { operator: op, .. }
            | Pending::OperatorMark { operator: op, .. }
            | Pending::TextObject { operator: op, .. } => Some(op),
            _ => None,
        };
        if let Some(op) = operator {
            return op.operator_count.checked_mul(op.motion_count.unwrap_or(1));
        }
        self.count.or(match self.pending {
            Pending::G {
                count,
                count_explicit: true,
                ..
            }
            | Pending::Z {
                count,
                count_explicit: true,
            }
            | Pending::Find { count, .. }
            | Pending::ReplaceCharacter { count, .. }
            | Pending::Bracket { count, .. }
            | Pending::MacroPlay { count }
            | Pending::VisualTextObject { count, .. } => Some(count),
            Pending::Window { count } => count,
            _ => None,
        })
    }
    pub(super) fn record_status_input(&mut self, event: &InputEvent) {
        if !self.command_input_pending() {
            self.status_input.clear();
        }
        if !matches!(
            self.mode,
            Mode::Normal | Mode::VisualCharacter | Mode::VisualLine | Mode::VisualBlock
        ) {
            return;
        }
        let text = match event {
            InputEvent::Key(key) => std::borrow::Cow::Owned(mappings::render_key_notation(&[*key])),
            InputEvent::Text(text) => std::borrow::Cow::Borrowed(text.as_str()),
        };
        // Keep the prefix bounded even if an input producer sends an enormous string.
        for character in text.chars() {
            if self.status_input.len() + character.len_utf8() > 128 {
                break;
            }
            self.status_input.push(character);
        }
    }
}
fn bounded(text: &str) -> String {
    if text.len() <= 32 {
        return text.into();
    }
    let mut end = 29;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &text[..end])
}
