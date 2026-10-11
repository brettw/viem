//! Buffer-owned Vim formatting options and literal comment definitions.
use super::syntax::vim::{VimPattern, VimRegexLimits};
use unicode_segmentation::UnicodeSegmentation;

pub const DEFAULT_FORMAT_OPTIONS: &str = "qj";
pub const DEFAULT_FORMAT_LIST_PATTERN: &str = r"^\s*\d\+[\]:.)}\t ]\s*";
pub const C_FAMILY_COMMENTS: &str = ":///,://!,://,s1:/*,mb:*,ex:*/";
pub const C_FAMILY_COMMENT_LANGUAGES: &[&str] = &[
    "c",
    "cpp",
    "objc",
    "rust",
    "swift",
    "c_sharp",
    "javascript",
    "typescript",
    "tsx",
    "go",
    "java",
    "php",
    "css",
];

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ReflowOptions {
    pub format_options: Option<String>,
    pub comments: Option<String>,
    pub format_list_pattern: Option<String>,
    pub(crate) language_comments: String,
    pub(crate) default_flags: Option<String>,
}
impl ReflowOptions {
    pub fn flags(&self) -> &str {
        self.format_options.as_deref().unwrap_or(
            self.default_flags
                .as_deref()
                .unwrap_or(DEFAULT_FORMAT_OPTIONS),
        )
    }
    pub fn has(&self, flag: char) -> bool {
        self.flags().contains(flag)
    }
    pub fn comments(&self) -> &str {
        self.comments.as_deref().unwrap_or(&self.language_comments)
    }
    pub fn list_pattern(&self) -> &str {
        self.format_list_pattern
            .as_deref()
            .unwrap_or(DEFAULT_FORMAT_LIST_PATTERN)
    }
    pub fn set_language(&mut self, language: Option<&str>) {
        self.language_comments =
            if language.is_some_and(|v| C_FAMILY_COMMENT_LANGUAGES.contains(&v)) {
                C_FAMILY_COMMENTS
            } else {
                ""
            }
            .into();
    }
    pub fn set_continuation_defaults(&mut self, enter: bool, open: bool) {
        let mut flags = DEFAULT_FORMAT_OPTIONS.to_owned();
        if enter {
            flags.push('r');
        }
        if open {
            flags.push('o');
        }
        self.default_flags = Some(flags);
    }
    pub fn set_flag(&mut self, flag: char, enabled: bool) {
        let mut value = self.flags().replace(flag, "");
        if enabled {
            value.push(flag);
        }
        self.format_options = Some(value);
    }
    pub(crate) fn profile(&self) -> CommentProfile {
        CommentProfile::parse(self.comments()).expect("validated comments option")
    }
    pub fn validate(&self) -> Result<(), String> {
        normalize_format_options(self.flags())?;
        CommentProfile::parse(self.comments())?;
        ListPattern::compile(self.list_pattern())?;
        Ok(())
    }
}
pub fn normalize_format_options(value: &str) -> Result<String, String> {
    let mut normalized = String::new();
    // Vim keeps the last occurrence of each flag, including its accepted comma.
    for flag in value.chars().rev() {
        if !"qjro/n2w1,".contains(flag) {
            return Err(format!("unsupported formatoptions flag: {flag}"));
        }
        if !normalized.contains(flag) {
            normalized.insert(0, flag);
        }
    }
    Ok(normalized)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommentLeader {
    pub text: String,
    pub blank: bool,
    pub first: bool,
    pub nested: bool,
    pub no_above: bool,
}
impl CommentLeader {
    pub fn matches(&self, text: &str) -> bool {
        text.strip_prefix(&self.text)
            .is_some_and(|tail| !self.blank || tail.is_empty() || tail.starts_with([' ', '\t']))
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BlockCommentSyntax {
    pub open: String,
    pub close: String,
    pub middle: String,
    pub middle_blank: bool,
    pub open_blank: bool,
    pub close_blank: bool,
    pub no_above: bool,
    pub automatic_end: bool,
    pub offset: i32,
    pub end_offset: i32,
}
impl BlockCommentSyntax {
    pub fn middle_matches(&self, text: &str) -> bool {
        text.strip_prefix(&self.middle).is_some_and(|tail| {
            !self.middle_blank || tail.is_empty() || tail.starts_with([' ', '\t'])
        })
    }
    pub fn open_matches(&self, text: &str) -> bool {
        text.strip_prefix(&self.open).is_some_and(|tail| {
            !self.open_blank || tail.is_empty() || tail.starts_with([' ', '\t'])
        })
    }
    pub fn find_close(&self, text: &str) -> Option<usize> {
        self.close_positions(text).next()
    }
    pub fn rfind_close(&self, text: &str) -> Option<usize> {
        self.close_positions(text).last()
    }
    fn close_positions<'a>(&'a self, text: &'a str) -> impl Iterator<Item = usize> + 'a {
        text.match_indices(&self.close).filter_map(|(at, _)| {
            let after = &text[at + self.close.len()..];
            (!self.close_blank || after.is_empty() || after.starts_with([' ', '\t'])).then_some(at)
        })
    }
    pub fn middle_prefix(&self, indent: &str, tabstop: usize) -> String {
        let prefix = if self.offset < 0 {
            let width = super::indentation::end_column(0, indent, tabstop);
            " ".repeat(width.saturating_sub(self.offset.unsigned_abs() as usize))
        } else {
            format!("{indent}{}", " ".repeat(self.offset as usize))
        };
        format!("{prefix}{} ", self.middle)
    }
    pub fn body_middle_prefix(&self, indent: &str, inner: &str, tabstop: usize) -> String {
        let mut prefix = self.middle_prefix(indent, tabstop);
        prefix.pop(); // body alignment supplies the gap, which may be empty
        let original = super::indentation::end_column(0, indent, tabstop);
        let original = super::indentation::end_column(original, &self.open, tabstop);
        let original = super::indentation::end_column(original, inner, tabstop);
        let middle = super::indentation::end_column(0, &prefix, tabstop);
        prefix.push_str(&" ".repeat(original.saturating_sub(middle)));
        prefix
    }
}
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CommentProfile {
    pub line_leaders: Vec<CommentLeader>,
    pub blocks: Vec<BlockCommentSyntax>,
}
impl CommentProfile {
    pub fn parse(value: &str) -> Result<Self, String> {
        if value.len() > 16 * 1024 {
            return Err("comments exceeds option byte limit".into());
        }
        let mut result = Self::default();
        if value.is_empty() {
            return Ok(result);
        }
        let mut parts = Vec::new();
        let mut current = String::new();
        let mut escaped = false;
        for ch in value.chars() {
            if escaped {
                current.push(ch);
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == ',' {
                parts.push(std::mem::take(&mut current));
            } else {
                current.push(ch);
            }
        }
        if escaped {
            return Err("unterminated comments escape".into());
        }
        parts.push(current);
        let parse = |part: &str| -> Result<(String, String), String> {
            let (flags, text) = part.split_once(':').ok_or("comments part needs ':'")?;
            if text.is_empty() || text.contains(['\r', '\n', '\0']) {
                return Err("empty or multiline comment leader".into());
            }
            if flags.chars().any(|c| !"nbfsmelrOx-0123456789".contains(c)) {
                return Err("invalid comments flag".into());
            }
            Ok((flags.into(), text.into()))
        };
        let offset = |flags: &str, right: i32| -> Result<i32, String> {
            let number: String = flags
                .chars()
                .filter(|c| *c == '-' || c.is_ascii_digit())
                .collect();
            if number.is_empty() {
                return Ok(if flags.contains('r') { right } else { 0 });
            }
            let value = number
                .parse::<i32>()
                .map_err(|_| "invalid comments offset")?;
            if value.unsigned_abs() > 1024 {
                return Err("comments offset exceeds 1024".into());
            }
            Ok(value)
        };
        let mut index = 0;
        while index < parts.len() {
            let (flags, text) = parse(&parts[index])?;
            if flags.contains('s') {
                let (middle_flags, middle) =
                    parse(parts.get(index + 1).ok_or("missing middle comment")?)?;
                let (end_flags, close) = parse(parts.get(index + 2).ok_or("missing end comment")?)?;
                if !middle_flags.contains('m') || !end_flags.contains('e') {
                    return Err("three-piece comment needs start,middle,end".into());
                }
                result.blocks.push(BlockCommentSyntax {
                    offset: offset(
                        &flags,
                        text.chars().count() as i32 - middle.chars().count() as i32,
                    )?,
                    end_offset: offset(
                        &end_flags,
                        close.chars().count() as i32 - middle.chars().count() as i32,
                    )?,
                    open: text,
                    middle,
                    close,
                    middle_blank: middle_flags.contains('b'),
                    open_blank: flags.contains('b'),
                    close_blank: end_flags.contains('b'),
                    no_above: flags.contains('O'),
                    automatic_end: end_flags.contains('x'),
                });
                index += 3;
            } else {
                if flags.contains(['m', 'e']) {
                    return Err("middle/end comment requires preceding start".into());
                }
                result.line_leaders.push(CommentLeader {
                    text,
                    blank: flags.contains('b'),
                    first: flags.contains('f'),
                    nested: flags.contains('n'),
                    no_above: flags.contains('O'),
                });
                index += 1;
            }
        }
        Ok(result)
    }
    /// Prefix consumed by a repeating or first-only leader, including nested leaders.
    pub fn line_prefix<'a>(&'a self, text: &str) -> Option<(usize, &'a CommentLeader)> {
        let first = self
            .line_leaders
            .iter()
            .find(|leader| leader.matches(text))?;
        let mut end = first.text.len();
        if first.nested {
            loop {
                let gap = text[end..].len() - text[end..].trim_start_matches([' ', '\t']).len();
                let Some(next) = self
                    .line_leaders
                    .iter()
                    .find(|leader| leader.nested && leader.matches(&text[end + gap..]))
                else {
                    break;
                };
                end += gap + next.text.len();
            }
        }
        Some((end, first))
    }
    /// Locate a line leader outside quoted strings and closed inline blocks.
    /// This local lexical check needs no syntax worker or preceding-document scan.
    pub fn trailing_line_leader(&self, text: &str) -> Option<(usize, &CommentLeader)> {
        let mut quote = None;
        let mut escaped = false;
        let mut at = 0;
        while at < text.len() {
            let rest = &text[at..];
            let ch = rest.chars().next()?;
            if let Some(delimiter) = quote {
                if escaped {
                    escaped = false;
                } else if ch == '\\' {
                    escaped = true;
                } else if ch == delimiter {
                    quote = None;
                }
            } else {
                if let Some(leader) = self.line_leaders.iter().find(|leader| leader.matches(rest)) {
                    return Some((at, leader));
                }
                if let Some(block) = self.blocks.iter().find(|block| block.open_matches(rest)) {
                    let end = block.find_close(&rest[block.open.len()..])?;
                    at += block.open.len() + end + block.close.len();
                    continue;
                }
                if matches!(ch, '\'' | '"' | '`') {
                    quote = Some(ch);
                }
            }
            at += ch.len_utf8();
        }
        None
    }
    /// Strip a repeated leader only when the preceding line belongs to the same comment.
    pub fn joined_prefix_len(&self, previous: &str, next: &str, in_block: bool) -> usize {
        let indent = next.len() - next.trim_start_matches([' ', '\t']).len();
        let rest = &next[indent..];
        if let Some((end, leader)) = self.line_prefix(rest) {
            if !leader.first && self.trailing_line_leader(previous).is_some() {
                return indent + end + rest[end..].len()
                    - rest[end..].trim_start_matches([' ', '\t']).len();
            }
        }
        for block in &self.blocks {
            if block.find_close(previous).is_none()
                && (block.open_matches(previous.trim_start_matches([' ', '\t']))
                    || (in_block && block.middle_matches(previous.trim_start_matches([' ', '\t']))))
                && block.middle_matches(rest)
                && !rest.starts_with(&block.close)
            {
                let end = indent + block.middle.len();
                return end + next[end..].len() - next[end..].trim_start_matches([' ', '\t']).len();
            }
        }
        indent
    }
}

/// The existing bounded Vim regexp engine gives formatlistpat its own dialect,
/// including magic switches, alternation, and \ze, without invoking Vimscript.
#[derive(Clone, Debug)]
pub struct ListPattern(VimPattern);
impl ListPattern {
    pub fn compile(value: &str) -> Result<Self, String> {
        VimPattern::compile(value, false, VimRegexLimits::default()).map(Self)
    }
    pub fn prefix_len(&self, text: &str, fuel: &mut usize) -> Result<Option<usize>, String> {
        let found = self
            .0
            .find_text_with_control(text, 0, fuel, &mut || false)?
            .filter(|found| found.start == 0 && found.end > 0 && found.end < text.len());
        if let Some(found) = &found {
            if !text.grapheme_indices(true).any(|(at, _)| at == found.end) {
                return Err("formatlistpat ends inside a grapheme".into());
            }
        }
        Ok(found.map(|found| found.end))
    }
}
