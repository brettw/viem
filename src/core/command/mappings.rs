//! Portable key notation, mode-scoped mappings, and bounded input expansion.
use super::*;
use std::sync::Arc;

const NORMAL: u8 = 1;
const VISUAL: u8 = 2;
const OPERATOR: u8 = 4;
const INSERT: u8 = 8;
const COMMAND_LINE: u8 = 16;
const SELECT: u8 = 32;

#[derive(Clone, Debug, Default)]
pub(crate) struct KeyMappings(Arc<Vec<Mapping>>);
#[derive(Clone, Debug)]
struct Mapping {
    mode: u8,
    lhs: Vec<Key>,
    rhs: Vec<Key>,
    recursive: bool,
    select_as_visual: bool,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct MappingReplayPlan {
    pub events: Vec<(InputEvent, bool)>,
    pub restore_select: bool,
}

impl KeyMappings {
    /// None means this is not a supported mapping command. Each definition is
    /// parsed completely before replacing any mode's existing definition.
    pub(crate) fn execute(&mut self, input: &str) -> Option<Result<(), String>> {
        let input = input
            .trim_start()
            .strip_prefix(':')
            .unwrap_or(input.trim_start())
            .trim_start();
        let split = input.find(char::is_whitespace).unwrap_or(input.len());
        let name = &input[..split];
        let args = input[split..].trim_start();
        let (mode, operation) = match name {
            "map" | "noremap" | "unmap" | "mapclear" => (NORMAL | VISUAL | SELECT | OPERATOR, name),
            "map!" => (INSERT | COMMAND_LINE, "map"),
            "noremap!" => (INSERT | COMMAND_LINE, "noremap"),
            "unmap!" => (INSERT | COMMAND_LINE, "unmap"),
            "mapclear!" => (INSERT | COMMAND_LINE, "mapclear"),
            _ => {
                let (prefix, suffix) = name.split_at_checked(1)?;
                let mode = match prefix {
                    "n" => NORMAL,
                    "v" => VISUAL | SELECT,
                    "x" => VISUAL,
                    "s" => SELECT,
                    "o" => OPERATOR,
                    "i" => INSERT,
                    "c" => COMMAND_LINE,
                    _ => return None,
                };
                if !matches!(suffix, "map" | "noremap" | "unmap" | "mapclear") {
                    return None;
                }
                (mode, suffix)
            }
        };
        Some((|| {
            if operation == "mapclear" {
                if !args.trim().is_empty() {
                    return Err("mapclear takes no arguments".into());
                }
                Arc::make_mut(&mut self.0).retain(|mapping| mapping.mode & mode == 0);
                return Ok(());
            }
            let split = args.find(char::is_whitespace).unwrap_or(args.len());
            let lhs_text = &args[..split];
            if lhs_text.is_empty() {
                return Err("a mapping requires a left-hand key sequence".into());
            }
            let lhs = parse_key_notation(lhs_text)?;
            if lhs.is_empty() {
                return Err("a mapping's left-hand side cannot be empty".into());
            }
            let rhs_text = args[split..].trim_start_matches([' ', '\t']);
            if operation == "unmap" {
                if !rhs_text.is_empty() {
                    return Err("unmap takes one key sequence".into());
                }
                let old = self.0.len();
                Arc::make_mut(&mut self.0)
                    .retain(|mapping| mapping.mode & mode == 0 || mapping.lhs != lhs);
                if old == self.0.len() {
                    return Err("no such mapping".into());
                }
                return Ok(());
            }
            if rhs_text.is_empty() {
                return Err(
                    "a mapping requires a right-hand key sequence; use <Nop> for no action".into(),
                );
            }
            let rhs = parse_key_notation(rhs_text)?;
            for selected in [NORMAL, VISUAL, SELECT, OPERATOR, INSERT, COMMAND_LINE] {
                if selected & mode == 0 {
                    continue;
                }
                Arc::make_mut(&mut self.0)
                    .retain(|mapping| mapping.mode != selected || mapping.lhs != lhs);
                Arc::make_mut(&mut self.0).push(Mapping {
                    mode: selected,
                    lhs: lhs.clone(),
                    rhs: rhs.clone(),
                    recursive: operation == "map",
                    select_as_visual: mode & SELECT != 0 && mode & VISUAL != 0,
                });
            }
            Ok(())
        })())
    }
}

/// Vim's angle-bracket spelling for the keys represented by the portable input
/// vocabulary. Unknown bracket names report an error instead of silently
/// defining an unusable mapping. `<lt>` spells a literal opening bracket.
pub fn parse_key_notation(input: &str) -> Result<Vec<Key>, String> {
    let mut result = Vec::new();
    let mut rest = input;
    while !rest.is_empty() {
        let character = rest.chars().next().unwrap();
        if character != '<' {
            result.push(Key::Char(character));
            rest = &rest[character.len_utf8()..];
            continue;
        }
        let end = rest
            .find('>')
            .ok_or("unterminated key notation; use <lt> for a literal <")?;
        let name = &rest[1..end];
        rest = &rest[end + 1..];
        if name.eq_ignore_ascii_case("nop") {
            continue;
        }
        let lower = name.to_ascii_lowercase();
        let simple = match lower.as_str() {
            "lt" => Some(Key::Char('<')),
            "space" => Some(Key::Char(' ')),
            "bar" => Some(Key::Char('|')),
            "bslash" => Some(Key::Char('\\')),
            "esc" => Some(Key::Escape),
            "cr" | "enter" | "return" => Some(Key::Enter),
            "tab" => Some(Key::Tab),
            "s-tab" => Some(Key::BackTab),
            "s-cr" | "s-enter" => Some(Key::ShiftEnter),
            "bs" | "backspace" => Some(Key::Backspace),
            "del" | "delete" => Some(Key::Delete),
            "left" => Some(Key::Left),
            "right" => Some(Key::Right),
            "up" => Some(Key::Up),
            "down" => Some(Key::Down),
            "home" => Some(Key::Home),
            "end" => Some(Key::End),
            "pageup" => Some(Key::PageUp),
            "pagedown" => Some(Key::PageDown),
            "c-left" => Some(Key::WordLeft),
            "c-right" => Some(Key::WordRight),
            "c-home" => Some(Key::DocumentStart),
            "c-end" => Some(Key::DocumentEnd),
            _ => None,
        };
        if let Some(key) = simple {
            result.push(key);
            continue;
        }
        let mut modifiers = 0;
        let mut tail = lower.as_str();
        while tail.as_bytes().get(1) == Some(&b'-') {
            modifiers |= match tail.as_bytes()[0] {
                b's' => 1,
                b'c' => 2,
                b'a' | b'm' => 4,
                b'd' => 8,
                _ => break,
            };
            tail = &tail[2..];
        }
        if modifiers != 0 {
            let nav = match tail { "left" => Some(if modifiers & 6 != 0 { NavigationKey::WordLeft } else { NavigationKey::Left }), "right" => Some(if modifiers & 6 != 0 { NavigationKey::WordRight } else { NavigationKey::Right }), "up" => Some(NavigationKey::Up), "down" => Some(NavigationKey::Down), "home" => Some(if modifiers & 2 != 0 { NavigationKey::DocumentStart } else { NavigationKey::Home }), "end" => Some(if modifiers & 2 != 0 { NavigationKey::DocumentEnd } else { NavigationKey::End }), "pageup" => Some(NavigationKey::PageUp), "pagedown" => Some(NavigationKey::PageDown), _ => None };
            if let Some(key) = nav { result.push(canonical_mapping_key(Key::ModifiedNavigation { key, modifiers })); continue; }
        }
        if let Some(number) = tail
            .strip_prefix('f')
            .and_then(|number| number.parse::<u8>().ok())
            .filter(|number| (1..=35).contains(number))
        {
            result.push(Key::Function { number, modifiers });
            continue;
        }
        if modifiers == 2 && tail.chars().count() == 1 {
            result.push(canonical_mapping_key(Key::Ctrl(
                tail.chars().next().unwrap(),
            )));
            continue;
        }
        if modifiers == 1 && tail.chars().count() == 1 {
            result.push(Key::Char(tail.chars().next().unwrap().to_ascii_uppercase()));
            continue;
        }
        return Err(format!("unsupported key notation <{name}>"));
    }
    Ok(result)
}

fn canonical_mapping_key(key: Key) -> Key {
    match key {
        Key::ModifiedNavigation { key: key @ (NavigationKey::WordLeft | NavigationKey::WordRight | NavigationKey::DocumentStart | NavigationKey::DocumentEnd), modifiers: 2 | 4 | 8 } => key.key(),
        Key::Ctrl('i' | 'I') => Key::Tab,
        Key::Ctrl('m' | 'M') => Key::Enter,
        Key::Ctrl('[') => Key::Escape,
        Key::Ctrl(character) => Key::Ctrl(character.to_ascii_lowercase()),
        key => key,
    }
}

fn input_key(event: &InputEvent) -> Option<Key> {
    match event {
        InputEvent::Key(key) => Some(canonical_mapping_key(*key)),
        InputEvent::Text(text) if text.chars().count() == 1 => text.chars().next().map(Key::Char),
        _ => None,
    }
}

impl CommandInterpreter {
    fn mapping_mode(&self) -> Option<u8> {
        if self.substitute_confirmation.is_some()
            || self.mapping_suppressed
            || self.literal_input_pending()
            || self.register_pending
            || self.select_register_pending
            || self.command_line_register_pending()
            || self.insert_control_g_pending()
            || matches!(
                self.pending,
                Pending::ReplaceCharacter { .. }
                    | Pending::ReplaceVisual
                    | Pending::ReplaceVisualBlock
                    | Pending::Find { .. }
                    | Pending::OperatorFind { .. }
                    | Pending::SetMark
                    | Pending::JumpMark { .. }
                    | Pending::OperatorMark { .. }
                    | Pending::MacroRecord
                    | Pending::MacroPlay { .. }
            )
        {
            return None;
        }
        if self.is_native_selection() { return None; }
        Some(match self.mode {
            Mode::Normal if matches!(self.pending, Pending::Operator(_)) => OPERATOR,
            Mode::Normal => NORMAL,
            Mode::VisualCharacter | Mode::VisualLine | Mode::VisualBlock => if self.is_select_mode() { SELECT } else { VISUAL },
            Mode::Insert | Mode::Replace => INSERT,
            Mode::CommandLine => COMMAND_LINE,
        })
    }

    pub(crate) fn mapping_applies(&self, event: &InputEvent) -> bool {
        let Some(mode) = self.mapping_mode() else {
            return false;
        };
        let Some(key) = input_key(event) else {
            return false;
        };
        if self.recording.is_some()
            && self.mode == Mode::Normal
            && self.pending == Pending::None
            && self.mapping_pending.is_empty()
            && key == Key::Char('q')
        {
            return false;
        }
        !self.mapping_pending.is_empty()
            || self
                .mappings
                .0
                .iter()
                .any(|mapping| mapping.mode == mode && mapping.lhs.first() == Some(&key))
    }

    pub(crate) fn has_pending_mapping(&self) -> bool {
        !self.mapping_pending.is_empty()
    }
    pub(crate) fn set_mapping_suppressed(&mut self, suppressed: bool) -> bool {
        std::mem::replace(&mut self.mapping_suppressed, suppressed)
    }

    pub(crate) fn flush_mapping(&mut self) -> Option<MappingReplayPlan> {
        if self.mapping_pending.is_empty() {
            return None;
        }
        let mode = self.mapping_mode()?;
        Some(
            self.resolve_mapping(mode, true)
                .expect("flushing always releases the prefix"),
        )
    }

    fn resolve_mapping(&mut self, mode: u8, flush: bool) -> Option<MappingReplayPlan> {
        let pending = &self.mapping_pending;
        if !flush
            && self.mappings.0.iter().any(|mapping| {
                mapping.mode == mode
                    && mapping.lhs.len() > pending.len()
                    && mapping.lhs.starts_with(pending)
            })
        {
            return None;
        }
        let selected = self
            .mappings
            .0
            .iter()
            .filter(|mapping| mapping.mode == mode && pending.starts_with(&mapping.lhs))
            .max_by_key(|mapping| mapping.lhs.len());
        let restore_select = self.is_select_mode() && selected.is_some_and(|mapping| mapping.select_as_visual);
        let mut events = Vec::new();
        let consumed = if let Some(mapping) = selected {
            // Vim does not remap a RHS's initial complete copy of its own LHS.
            let self_prefix = mapping.rhs.starts_with(&mapping.lhs);
            events.extend(mapping.rhs.iter().enumerate().map(|(index, key)| {
                (
                    InputEvent::Key(*key),
                    mapping.recursive && !(self_prefix && index < mapping.lhs.len()),
                )
            }));
            mapping.lhs.len()
        } else {
            events.push((InputEvent::Key(pending[0]), false));
            1
        };
        events.extend(
            pending[consumed..]
                .iter()
                .map(|key| (InputEvent::Key(*key), true)),
        );
        self.mapping_pending.clear();
        if restore_select { self.selection_behavior = SelectionBehavior::Visual; }
        Some(MappingReplayPlan { events, restore_select })
    }

    pub(super) fn handle_mapping(
        &mut self,
        document: &mut Document,
        event: &InputEvent,
    ) -> Result<Option<CommandOutput>, DocumentError> {
        if !self.mapping_applies(event) {
            return Ok(None);
        }
        let key = input_key(event).expect("mapping input has a key");
        if matches!(key, Key::Escape | Key::Ctrl('c')) && !self.mapping_pending.is_empty() {
            self.mapping_pending.clear();
            return Ok(None);
        }
        self.record_event(event);
        let mode = self.mapping_mode().unwrap();
        self.mapping_pending.push(key);
        let Some(plan) = self.resolve_mapping(mode, false) else {
            return Ok(Some(CommandOutput::pending()));
        };
        if self.plan_compound_replay {
            self.pending_replay = Some(ReplayPlan::Mapping(plan));
            return Ok(Some(CommandOutput::complete()));
        }
        if !self.begin_replay_frame() {
            return Ok(Some(CommandOutput {
                status: CommandStatus::Error("mapping recursion limit reached".into()),
                ..CommandOutput::complete()
            }));
        }
        let old_depth = document.edit_group_depth();
        document.begin_edit_group();
        let result = (|| {
            let mut output = CommandOutput::complete();
            for (event, remap) in plan.events {
                let old = self.set_mapping_suppressed(!remap);
                let next = self.handle(document, event);
                self.set_mapping_suppressed(old);
                let next = next?;
                let stop = command_status_stops_compound(&next.status);
                output.merge(next);
                if stop {
                    break;
                }
            }
            Ok(Some(output))
        })();
        document.restore_edit_group_depth(old_depth + 1);
        document.end_edit_group();
        self.end_replay_frame();
        if plan.restore_select { self.finish_select_mapping(); }
        result
    }
}

pub(crate) fn empty_mapping_output() -> CommandOutput {
    CommandOutput::complete()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coordinator::{Core, CoreEvent, ViewId};
    use crate::layout::MockTextMeasurementProvider;

    fn setup(source: &str, startup: &str) -> (Core<MockTextMeasurementProvider>, ViewId) {
        let mut core = Core::new(Document::new(source));
        assert_eq!(core.initialize_startup(startup), vec![]);
        let view = core.add_view(MockTextMeasurementProvider::new(), 400., 200.);
        (core, view)
    }
    fn keys(
        core: &mut Core<MockTextMeasurementProvider>,
        view: ViewId,
        text: &str,
    ) -> CommandOutput {
        let mut result = CommandOutput::complete();
        for key in parse_key_notation(text).unwrap() {
            result = core
                .handle_with_layout(view, CoreEvent::Input(InputEvent::Key(key)))
                .unwrap()
                .command
                .unwrap();
            assert!(
                !command_status_stops_compound(&result.status),
                "{text}: {:?}",
                result.status
            );
        }
        result
    }
    #[test]
    fn requested_yank_mapping_preserves_count_register_and_hard_line_extent() {
        let (mut core, view) = setup("alpha beta\ngamma delta\nlast", "map Y y$\nnmap a d");
        keys(&mut core, view, "l\"a2Y");
        assert_eq!(
            core.command_state(view)
                .unwrap()
                .register('a')
                .unwrap()
                .text,
            "lpha beta\ngamma delta"
        );
        assert_eq!(core.document().text(), "alpha beta\ngamma delta\nlast");
    }
    #[test]
    fn function_mapping_prefills_prompt_and_explicit_return_executes_split() {
        let (mut core, view) = setup("abc", "map <C-F2> :sp\nmap <S-C-F3> :sp<CR>");
        keys(&mut core, view, "<C-F2>");
        assert_eq!(core.command_state(view).unwrap().mode(), Mode::CommandLine);
        assert_eq!(
            core.command_state(view).unwrap().command_line().unwrap(),
            "sp"
        );
        keys(&mut core, view, "<Esc>");
        let output = core
            .handle_with_layout(
                view,
                CoreEvent::Input(InputEvent::Key(Key::Function {
                    number: 3,
                    modifiers: 3,
                })),
            )
            .unwrap()
            .command
            .unwrap();
        assert_eq!(output.status, CommandStatus::Complete);
        assert!(output.ex_outcome.unwrap().frontend_requests.len() > 0);
    }
    #[test]
    fn modes_recursive_nonrecursive_operator_and_literal_operands() {
        let (mut core, view) = setup(
            "abc def ghi",
            "map Q w\nnmap Z Q\nnnoremap X Q\nomap K e\nnmap b l",
        );
        keys(&mut core, view, "Z");
        assert_eq!(core.command_state(view).unwrap().cursor(), 4);
        keys(&mut core, view, "0fb");
        assert_eq!(core.command_state(view).unwrap().cursor(), 1);
        keys(&mut core, view, "0dK");
        assert_eq!(core.document().text(), " def ghi");
        keys(&mut core, view, "u");
        assert_eq!(core.document().text(), "abc def ghi");
        let output = core
            .handle_with_layout(view, CoreEvent::Input(InputEvent::key('X')))
            .unwrap()
            .command
            .unwrap();
        assert!(matches!(output.status, CommandStatus::Unsupported(_)));
    }
    #[test]
    fn mapped_edit_is_one_undo_group_and_dot_keeps_last_semantic_edit() {
        let (mut core, view) = setup("abcdef", "nnoremap Q xx");
        keys(&mut core, view, "Q");
        assert_eq!(core.document().text(), "cdef");
        keys(&mut core, view, "u");
        assert_eq!(core.document().text(), "abcdef");
        keys(&mut core, view, "<C-r>.");
        assert_eq!(core.document().text(), "def");
    }
    #[test]
    fn multi_key_mismatch_and_timeout_choose_longest_complete_prefix() {
        let (mut core, view) = setup("abc def ghi", "nmap g l\nnmap gg w\nnmap jk w");
        keys(&mut core, view, "g");
        assert!(core.has_pending_mapping(view));
        core.flush_mapping_prefix(view).unwrap();
        assert_eq!(core.command_state(view).unwrap().cursor(), 1);
        keys(&mut core, view, "gg");
        assert_eq!(core.command_state(view).unwrap().cursor(), 4);
        keys(&mut core, view, "gl");
        assert_eq!(core.command_state(view).unwrap().cursor(), 6);
        keys(&mut core, view, "j<Esc>");
        assert!(!core.has_pending_mapping(view));
    }
    #[test]
    fn mappings_apply_across_views_and_interactive_unmap_is_shared() {
        let (mut core, a) = setup("abc def", "map Y y$");
        let b = core.add_view(MockTextMeasurementProvider::new(), 400., 200.);
        keys(&mut core, b, "lY");
        assert_eq!(
            core.command_state(b).unwrap().register('"').unwrap().text,
            "bc def"
        );
        keys(&mut core, a, ":unmap Y<CR>");
        keys(&mut core, b, "Y");
        assert_eq!(
            core.command_state(b).unwrap().register('"').unwrap().kind,
            RegisterKind::Linewise
        );
    }
    #[test]
    fn recursive_cycles_terminate_and_self_prefix_keeps_vim_behavior() {
        let (mut core, view) = setup("abc def", "map Q Z\nmap Z Q\nmap Y Y");
        let output = core
            .handle_with_layout(view, CoreEvent::Input(InputEvent::key('Q')))
            .unwrap()
            .command
            .unwrap();
        assert!(matches!(output.status, CommandStatus::Error(_)));
        keys(&mut core, view, "Y");
        assert_eq!(
            core.command_state(view)
                .unwrap()
                .register('"')
                .unwrap()
                .kind,
            RegisterKind::Linewise
        );
    }
    #[test]
    fn insert_mapping_and_literal_function_quotation() {
        let (mut core, view) = setup("abc", "imap jj <Esc>\nimap <C-F2> word");
        keys(&mut core, view, "i<C-F2>jj");
        assert_eq!(core.document().text(), "wordabc");
        assert_eq!(core.command_state(view).unwrap().mode(), Mode::Normal);
        keys(&mut core, view, "i<C-v><C-F2><Esc>");
        assert!(core.document().text().contains("<C-F2>"));
    }
    #[test]
    fn startup_reports_line_errors_continues_and_cannot_change_source() {
        let mut core: Core<MockTextMeasurementProvider> = Core::new(Document::new("abc\ndef"));
        let source = core.document().source_bytes();
        let revision = core.document().revision();
        let diagnostics = core.initialize_startup("\" comment\nset nowrap ignorecase\nset ff=dos\nnormal dd\nmap <Unknown> x\nmap Y y$\nset tw=72");
        assert_eq!(
            diagnostics.iter().map(|d| d.line).collect::<Vec<_>>(),
            [3, 4, 5]
        );
        assert_eq!(core.document().source_bytes(), source);
        assert_eq!(core.document().revision(), revision);
        let a = core.add_view(MockTextMeasurementProvider::new(), 100., 100.);
        let b = core.add_view(MockTextMeasurementProvider::new(), 100., 100.);
        assert!(!core.layout(a).unwrap().wrap());
        assert!(!core.layout(b).unwrap().wrap());
        assert_eq!(core.initialize_startup("map Y dd").len(), 1);
        keys(&mut core, a, "Y");
        assert_eq!(core.document().source_bytes(), source);
    }
    #[test]
    fn headless_mapping_and_key_notation_parser() {
        let mut document = Document::new("abc def");
        let mut commands = CommandInterpreter::new();
        assert!(commands
            .configure_startup(&mut document, "map Y y$")
            .is_empty());
        commands
            .handle(&mut document, InputEvent::key('Y'))
            .unwrap();
        assert_eq!(commands.register('"').unwrap().text, "abc def");
        assert_eq!(
            parse_key_notation("<D-M-C-S-F35><lt><Space><Nop>").unwrap(),
            [
                Key::Function {
                    number: 35,
                    modifiers: 15
                },
                Key::Char('<'),
                Key::Char(' ')
            ]
        );
        assert!(parse_key_notation("<F36>").is_err());
    }
    #[test]
    fn visual_mapping_and_macro_recording_share_semantic_replay() {
        let (mut core, view) = setup("abcdef", "nmap Q x\nvmap Q e");
        keys(&mut core, view, "qaQq");
        assert_eq!(
            core.command_state(view)
                .unwrap()
                .register('a')
                .unwrap()
                .text,
            "Q"
        );
        keys(&mut core, view, "@a");
        assert_eq!(core.document().text(), "cdef");
        keys(&mut core, view, "vQd");
        assert_eq!(core.document().text(), "");
        keys(&mut core, view, "u");
        assert_eq!(core.document().text(), "cdef");
    }
    #[test]
    fn normal_bang_bypasses_mappings_and_plain_normal_expands_them() {
        let (mut core, view) = setup("abc def ghi", "nmap l w");
        keys(&mut core, view, ":normal l<CR>");
        assert_eq!(core.command_state(view).unwrap().cursor(), 4);
        keys(&mut core, view, ":normal! l<CR>");
        assert_eq!(core.command_state(view).unwrap().cursor(), 1);
    }
    #[test]
    fn startup_protects_file_policies_case_insensitively_and_atomically() {
        let (mut core, _) = setup("abc\ndef", "");
        let mut fresh: Core<MockTextMeasurementProvider> = Core::new(Document::new("abc\ndef"));
        let before = fresh.document().source_bytes();
        let diagnostics = fresh.initialize_startup("set FF=dos\nset FILEFORMAT&\nset tw=19 ff=dos\nset FFS=mac\nset fileformats=dos\nset tw=31");
        assert_eq!(
            diagnostics.iter().map(|item| item.line).collect::<Vec<_>>(),
            [1, 2, 3, 4, 5]
        );
        assert_eq!(fresh.document().source_bytes(), before);
        assert_eq!(fresh.text_width().effective(), 31);
        assert_eq!(core.initialize_startup("set FF=dos").len(), 1);
    }
    #[test]
    fn insert_mapping_keeps_the_entire_typing_session_in_one_undo_group() {
        let (mut core, view) = setup("", "imap <F2> MID\nimap jj <Esc>");
        keys(&mut core, view, "ibefore<F2>afterjj");
        assert_eq!(core.document().text(), "beforeMIDafter");
        keys(&mut core, view, "u");
        assert_eq!(core.document().text(), "");
    }

    #[test]
    fn control_aliases_match_the_same_physical_keys() {
        let (mut core, view) = setup("abc def", "nmap <C-M> l\nnmap <C-I> l\nnmap <C-[> l");
        keys(&mut core, view, "<CR><Tab><Esc>");
        assert_eq!(core.command_state(view).unwrap().cursor(), 3);
        for key in [Key::Ctrl('m'), Key::Ctrl('I'), Key::Ctrl('[')] {
            core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(key)))
                .unwrap();
        }
        assert_eq!(core.command_state(view).unwrap().cursor(), 6);
    }
    #[test]
    fn supplied_layout_mapping_requires_coordinator_and_validates_snapshot_first() {
        use crate::layout::{LayoutEngine, ViewLayout};
        let mut document = Document::new("abc def");
        let mut commands = CommandInterpreter::new();
        assert!(commands
            .configure_startup(&mut document, "map Q gj")
            .is_empty());
        let mut engine = LayoutEngine::new(MockTextMeasurementProvider::new());
        let mut view = ViewLayout::new(40., 200.);
        engine.relayout(&document, &mut view).unwrap();
        let snapshot = view.snapshot().unwrap().clone();
        let mut context =
            LayoutCommandContext::new(&snapshot, true, Viewport::new(0., 200.).unwrap());
        let output = commands
            .handle_with_layout(&mut document, InputEvent::key('Q'), &mut context)
            .unwrap();
        assert!(matches!(output.status, CommandStatus::Unsupported(_)));
        let mut other = Document::new("other");
        let output = commands
            .handle_with_layout(&mut other, InputEvent::key('Q'), &mut context)
            .unwrap();
        assert_eq!(
            output.status,
            CommandStatus::Error("stale layout context".into())
        );
    }
}
