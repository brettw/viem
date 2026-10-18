//! Autocommand groups, definitions and Vim's file-pattern matching. Viem
//! accepts every Vim event name, but only the events listed in `FIRED` are
//! ever triggered; definitions for other events are retained silently.
use super::error::VimError;
use super::value::VStr;

/// Every Vim 9.2 autocommand event, in Vim's canonical spelling.
pub const EVENTS: &[&str] = &[
    "BufAdd", "BufDelete", "BufEnter", "BufFilePost", "BufFilePre", "BufHidden", "BufLeave",
    "BufNew", "BufNewFile", "BufReadCmd", "BufReadPost", "BufReadPre", "BufUnload",
    "BufWinEnter", "BufWinLeave", "BufWipeout", "BufWriteCmd", "BufWritePost", "BufWritePre",
    "CmdUndefined", "CmdlineChanged", "CmdlineEnter", "CmdlineLeave", "CmdlineLeavePre",
    "CmdwinEnter", "CmdwinLeave", "ColorScheme", "ColorSchemePre", "CompleteChanged",
    "CompleteDone", "CompleteDonePre", "CursorHold", "CursorHoldI", "CursorMoved",
    "CursorMovedC", "CursorMovedI", "DiffUpdated", "DirChanged", "DirChangedPre",
    "EncodingChanged", "ExitPre", "FileAppendCmd", "FileAppendPost", "FileAppendPre",
    "FileChangedRO", "FileChangedShell", "FileChangedShellPost", "FileReadCmd",
    "FileReadPost", "FileReadPre", "FileType", "FileWriteCmd", "FileWritePost", "FileWritePre",
    "FilterReadPost", "FilterReadPre", "FilterWritePost", "FilterWritePre", "FocusGained",
    "FocusLost", "FuncUndefined", "GUIEnter", "GUIFailed", "InsertChange", "InsertCharPre",
    "InsertEnter", "InsertLeave", "InsertLeavePre", "KeyInputPre", "MenuPopup", "ModeChanged",
    "OptionSet", "QuickFixCmdPost", "QuickFixCmdPre", "QuitPre", "RemoteReply", "SafeState",
    "SafeStateAgain", "SessionLoadPost", "SessionWritePost", "ShellCmdPost", "ShellFilterPost",
    "SigUSR1", "SourceCmd", "SourcePost", "SourcePre", "SpellFileMissing", "StdinReadPost",
    "StdinReadPre", "SwapExists", "Syntax", "TabClosed", "TabClosedPre", "TabEnter", "TabLeave",
    "TabNew", "TermChanged", "TermResponse", "TermResponseAll", "TerminalOpen",
    "TerminalWinOpen", "TextChanged", "TextChangedI", "TextChangedP", "TextChangedT",
    "TextYankPost", "User", "VimEnter", "VimLeave", "VimLeavePre", "VimResized", "VimResume",
    "VimSuspend", "WinClosed", "WinEnter", "WinLeave", "WinNew", "WinNewPre", "WinResized",
    "WinScrolled",
];

/// Alternative event names that select a canonical event.
const ALIASES: &[(&str, &str)] = &[
    ("BufRead", "BufReadPost"),
    ("BufWrite", "BufWritePre"),
    ("BufCreate", "BufAdd"),
    ("FileEncoding", "EncodingChanged"),
];

/// Events Viem triggers.
pub const FIRED: &[&str] = &[
    "BufEnter", "BufNewFile", "BufReadPost", "BufWinEnter", "BufWritePost", "BufWritePre",
    "ColorScheme", "ColorSchemePre", "FileType", "FocusGained", "FocusLost", "GUIEnter",
    "InsertEnter", "InsertLeave", "InsertLeavePre", "SourcePost", "SourcePre", "TextYankPost",
    "User", "VimEnter",
];

pub fn event_index(name: &str) -> Option<usize> {
    let canonical = ALIASES
        .iter()
        .find(|(alias, _)| alias.eq_ignore_ascii_case(name))
        .map_or(name, |(_, canonical)| canonical);
    EVENTS.iter().position(|event| event.eq_ignore_ascii_case(canonical))
}

pub fn event_name(index: usize) -> &'static str {
    EVENTS[index]
}

pub fn is_fired(index: usize) -> bool {
    FIRED.contains(&EVENTS[index])
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AutocmdPattern {
    /// An expanded file pattern (for FileType/User/etc. the matched name).
    Pattern(String),
    /// `<buffer>`: this buffer, which is the only one a Viem script state serves.
    Buffer,
}

#[derive(Clone, Debug)]
pub struct Autocmd {
    pub group: usize,
    pub event: usize,
    pub pattern: AutocmdPattern,
    pub once: bool,
    pub nested: bool,
    pub command: VStr,
    pub sid: usize,
    /// Monotonic identity so a running autocommand removed by itself is not
    /// executed again, and `++once` removes exactly this definition.
    pub id: u64,
}

#[derive(Debug, Default)]
pub struct AutocmdTable {
    /// User group names; group number N refers to `groups[N - 1]`, 0 is the
    /// default group.
    groups: Vec<Option<String>>,
    current: usize,
    entries: Vec<Autocmd>,
    next_id: u64,
}

impl AutocmdTable {
    pub fn current_group(&self) -> usize {
        self.current
    }
    pub fn group_name(&self, group: usize) -> Option<&str> {
        if group == 0 {
            return None;
        }
        self.groups.get(group - 1).and_then(|name| name.as_deref())
    }
    pub fn find_group(&self, name: &str) -> Option<usize> {
        self.groups.iter().position(|group| group.as_deref() == Some(name)).map(|index| index + 1)
    }
    /// `:augroup {name}`; "END" (any case) returns to the default group.
    pub fn select_group(&mut self, name: &str) {
        if name.eq_ignore_ascii_case("end") {
            self.current = 0;
            return;
        }
        self.current = match self.find_group(name) {
            Some(group) => group,
            None => {
                self.groups.push(Some(name.to_string()));
                self.groups.len()
            }
        };
    }
    /// `:augroup! {name}`.
    pub fn delete_group(&mut self, name: &str) -> Result<bool, VimError> {
        let Some(group) = self.find_group(name) else {
            return Err(VimError::new(367, format!("No such group: \"{name}\"")));
        };
        if self.current == group {
            return Err(VimError::new(936, "Cannot delete the current group"));
        }
        let in_use = self.entries.iter().any(|entry| entry.group == group);
        self.entries.retain(|entry| entry.group != group);
        self.groups[group - 1] = None;
        Ok(in_use)
    }
    pub fn add(&mut self, mut autocmd: Autocmd) {
        self.next_id += 1;
        autocmd.id = self.next_id;
        self.entries.push(autocmd);
    }
    /// Remove definitions selected by `:autocmd!`. `None` matches anything.
    pub fn remove(&mut self, group: usize, event: Option<usize>, pattern: Option<&AutocmdPattern>) {
        self.entries.retain(|entry| {
            !(entry.group == group
                && event.is_none_or(|event| entry.event == event)
                && pattern.is_none_or(|pattern| &entry.pattern == pattern))
        });
    }
    pub fn remove_id(&mut self, id: u64) {
        self.entries.retain(|entry| entry.id != id);
    }
    pub fn contains_id(&self, id: u64) -> bool {
        self.entries.iter().any(|entry| entry.id == id)
    }
    pub fn entries(&self) -> &[Autocmd] {
        &self.entries
    }
    /// Definitions for `event` whose pattern matches `name` (a file name, a
    /// file type, a color scheme name, ...).
    pub fn matching(&self, event: usize, group: Option<usize>, name: &str, file: bool) -> Vec<Autocmd> {
        self.entries
            .iter()
            .filter(|entry| entry.event == event && group.is_none_or(|group| entry.group == group))
            .filter(|entry| match &entry.pattern {
                AutocmdPattern::Buffer => true,
                AutocmdPattern::Pattern(pattern) => pattern_matches(pattern, name, file),
            })
            .cloned()
            .collect()
    }
    pub fn has_event(&self, event: usize) -> bool {
        self.entries.iter().any(|entry| entry.event == event)
    }
}

/// Match a comma-separated autocommand pattern list. For file names a
/// pattern without '/' matches only the tail. File names match ignoring
/// case on macOS and Windows, like Vim's default 'fileignorecase'.
pub fn pattern_matches(patterns: &str, name: &str, file: bool) -> bool {
    if patterns.len() > 65_536 || name.len() > 65_536 { return false; }
    split_patterns(patterns).iter().any(|pattern| {
        let ignore_case = file && cfg!(any(target_os = "macos", target_os = "windows"));
        let name = if file { name.replace('\\', "/") } else { name.to_string() };
        if file && !pattern.contains('/') {
            let tail = name.rsplit('/').next().unwrap_or(&name);
            glob_match(pattern, tail, ignore_case)
        } else {
            glob_match(pattern, &name, ignore_case)
        }
    })
}

/// Split at commas that are not escaped and not inside braces.
fn split_patterns(patterns: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut current = String::new();
    let mut depth = 0usize;
    let mut chars = patterns.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                current.push(c);
                if let Some(next) = chars.next() {
                    current.push(next);
                }
            }
            '{' => {
                depth += 1;
                current.push(c);
            }
            '}' => {
                depth = depth.saturating_sub(1);
                current.push(c);
            }
            ',' if depth == 0 => result.push(std::mem::take(&mut current)),
            _ => current.push(c),
        }
    }
    result.push(current);
    result.retain(|pattern| !pattern.is_empty());
    result
}

#[derive(Debug)]
enum GlobToken {
    Literal(char),
    Any,
    One,
    Class(bool, Vec<(char, char)>),
    Alternatives(Vec<Vec<GlobToken>>),
}

fn parse_glob(chars: &[char], at: &mut usize, in_braces: bool) -> Vec<Vec<GlobToken>> {
    let mut alternatives = vec![Vec::new()];
    while *at < chars.len() {
        let c = chars[*at];
        *at += 1;
        let tokens = alternatives.last_mut().unwrap();
        match c {
            '*' => tokens.push(GlobToken::Any),
            '?' => tokens.push(GlobToken::One),
            '\\' if *at < chars.len() => {
                tokens.push(GlobToken::Literal(chars[*at]));
                *at += 1;
            }
            '{' => {
                let inner = parse_glob(chars, at, true);
                tokens.push(GlobToken::Alternatives(inner));
            }
            '}' if in_braces => return alternatives,
            ',' if in_braces => alternatives.push(Vec::new()),
            '[' => {
                let start = *at;
                let negated = chars.get(*at) == Some(&'^') || chars.get(*at) == Some(&'!');
                if negated {
                    *at += 1;
                }
                let mut ranges = Vec::new();
                let mut first = true;
                let mut closed = false;
                while *at < chars.len() {
                    let item = chars[*at];
                    if item == ']' && !first {
                        *at += 1;
                        closed = true;
                        break;
                    }
                    first = false;
                    let item = if item == '\\' && *at + 1 < chars.len() {
                        *at += 1;
                        chars[*at]
                    } else {
                        item
                    };
                    *at += 1;
                    if chars.get(*at) == Some(&'-') && chars.get(*at + 1).is_some_and(|c| *c != ']') {
                        ranges.push((item, chars[*at + 1]));
                        *at += 2;
                    } else {
                        ranges.push((item, item));
                    }
                }
                if closed {
                    tokens.push(GlobToken::Class(negated, ranges));
                } else {
                    *at = start;
                    tokens.push(GlobToken::Literal('['));
                }
            }
            other => tokens.push(GlobToken::Literal(other)),
        }
    }
    alternatives
}

fn chars_equal(a: char, b: char, ignore_case: bool) -> bool {
    a == b || (ignore_case && a.to_lowercase().eq(b.to_lowercase()))
}

fn match_tokens(tokens: &[GlobToken], text: &[char], ignore_case: bool, budget: &mut usize) -> Vec<usize> {
    // Returns every possible end position, which keeps alternatives simple.
    let mut positions = vec![0usize];
    for token in tokens {
        let mut next = Vec::new();
        for &position in &positions {
            if *budget == 0 {
                return Vec::new();
            }
            *budget -= 1;
            match token {
                GlobToken::Literal(c) => {
                    if text.get(position).is_some_and(|t| chars_equal(*t, *c, ignore_case)) {
                        next.push(position + 1);
                    }
                }
                GlobToken::One => {
                    if position < text.len() {
                        next.push(position + 1);
                    }
                }
                GlobToken::Any => {
                    let count = text.len() - position + 1;
                    if count > *budget { *budget = 0; return Vec::new(); }
                    *budget -= count;
                    next.extend(position..=text.len());
                }
                GlobToken::Class(negated, ranges) => {
                    if ranges.len() > *budget { *budget = 0; return Vec::new(); }
                    *budget -= ranges.len();
                    if let Some(t) = text.get(position) {
                        let hit = ranges.iter().any(|(low, high)| {
                            (low..=high).contains(&t)
                                || (ignore_case
                                    && t.to_lowercase().chain(t.to_uppercase()).any(|c| (*low..=*high).contains(&c)))
                        });
                        if hit != *negated {
                            next.push(position + 1);
                        }
                    }
                }
                GlobToken::Alternatives(alternatives) => {
                    for alternative in alternatives {
                        if *budget == 0 { return Vec::new(); }
                        *budget -= 1;
                        for end in match_tokens(alternative, &text[position..], ignore_case, budget) {
                            next.push(position + end);
                        }
                    }
                }
            }
        }
        next.sort_unstable();
        next.dedup();
        positions = next;
        if positions.is_empty() {
            break;
        }
    }
    positions
}

pub fn glob_match(pattern: &str, text: &str, ignore_case: bool) -> bool {
    if pattern.len() > 65_536 || text.len() > 65_536 { return false; }
    let mut depth = 0usize;
    let mut escaped = false;
    for character in pattern.chars() {
        if escaped { escaped = false; continue; }
        match character {
            '\\' => escaped = true,
            '{' => { depth += 1; if depth > 128 { return false; } }
            '}' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    let pattern: Vec<char> = pattern.chars().collect();
    let text: Vec<char> = text.chars().collect();
    let mut at = 0;
    let alternatives = parse_glob(&pattern, &mut at, false);
    let mut budget = 100_000usize;
    alternatives.iter().any(|tokens| match_tokens(tokens, &text, ignore_case, &mut budget).contains(&text.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_patterns_follow_vim() {
        assert!(pattern_matches("*.txt", "/home/user/notes.txt", true));
        assert!(!pattern_matches("*.txt", "/home/user/notes.md", true));
        assert!(pattern_matches("*.{md,markdown}", "/x/a.markdown", true));
        assert!(pattern_matches("*/doc/*.txt", "/tmp/doc/xx.txt", true));
        assert!(!pattern_matches("/doc/*.txt", "/tmp/doc/xx.txt", true));
        assert!(pattern_matches("*.c,*.h", "/src/a.h", true));
        assert!(pattern_matches("python", "python", false));
        assert!(pattern_matches("*", "anything", false));
        assert!(pattern_matches("[ch]", "c", false));
        assert!(!pattern_matches("[^ch]", "c", false));
        assert!(pattern_matches("Make\\,file", "Make,file", false));
    }

    #[test]
    fn glob_work_and_recursion_are_bounded() {
        assert!(!glob_match(&format!("{}x{}", "{".repeat(129), "}".repeat(129)), "x", false));
        assert!(!glob_match("*", &"x".repeat(65_537), false));
        assert!(!glob_match(&"*".repeat(100), &"x".repeat(16_384), false));
        assert!(glob_match("{a,{b,c}}*", "b.txt", false));
    }

    #[test]
    fn events_resolve_aliases_and_case() {
        assert_eq!(event_index("bufread"), event_index("BufReadPost"));
        assert_eq!(event_index("BUFWRITE"), event_index("BufWritePre"));
        assert!(event_index("NoSuchEvent").is_none());
    }
}
