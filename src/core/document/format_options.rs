//! Buffer-owned Vim formatting options and literal comment definitions.
use super::syntax::vim::{VimPattern, VimRegexLimits};
use unicode_segmentation::UnicodeSegmentation;

#[path = "comment_profiles.rs"]
mod language_profiles;
pub use language_profiles::default_comments;

pub const DEFAULT_FORMAT_OPTIONS: &str = "qj";
pub const DEFAULT_FORMAT_LIST_PATTERN: &str = r"^\s*\d\+[\]:.)}\t ]\s*";
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ReflowOptions {
    pub format_options: Option<String>,
    pub comments: Option<String>,
    pub format_list_pattern: Option<String>,
    pub(crate) language_comments: String,
    pub(crate) language: Option<String>,
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
        self.language = language.map(super::syntax::detection::canonical_language);
        self.language_comments = self.language.as_deref().map(default_comments).unwrap_or("").into();
    }
    /// Selected canonical language, independent of an explicit comments override.
    pub fn language(&self) -> Option<&str> {
        self.language.as_deref()
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
    pub middle_no_above: bool,
    pub end_no_above: bool,
    /// Authored `f` after `s`; Vim's earlier `s` takes precedence for a block.
    pub first: bool,
    pub automatic_end: bool,
    pub offset: i32,
    pub end_offset: i32,
}
impl BlockCommentSyntax {
    pub fn middle_is_whitespace(&self) -> bool {
        self.middle.chars().all(|ch| matches!(ch, ' ' | '\t'))
    }
    /// End of a decoration in the original line, before indentation has been stripped.
    /// Whitespace middles belong to the indentation and must not be consumed twice.
    pub fn middle_body_start(&self, text: &str) -> Option<usize> {
        let indent = text.len() - text.trim_start_matches([' ', '\t']).len();
        if self.middle_is_whitespace() {
            return text[..indent].ends_with(&self.middle).then_some(indent);
        }
        self.middle_matches(&text[indent..]).then_some(indent + self.middle.len())
    }
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
        if self.middle_is_whitespace() {
            format!("{prefix}{}", self.middle)
        } else {
            format!("{prefix}{} ", self.middle)
        }
    }
    pub fn body_middle_prefix(&self, indent: &str, inner: &str, tabstop: usize) -> String {
        let mut prefix = self.middle_prefix(indent, tabstop);
        if !self.middle_is_whitespace() {
            prefix.pop(); // body alignment supplies the gap, which may be empty
        }
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
    order: Vec<CommentPart>,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CommentPart { Line(usize), Block(usize) }

/// First configured matching start, retaining ordering between line and block rules.
#[derive(Clone, Copy, Debug)]
pub enum CommentStart<'a> {
    Line { end: usize, leader: &'a CommentLeader },
    Block { index: usize, syntax: &'a BlockCommentSyntax },
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
            let Some(start) = flags.find(|c: char| c == '-' || c.is_ascii_digit()) else {
                if flags.contains('l') && flags.contains('r') {
                    return Err("conflicting comments alignment flags".into());
                }
                return Ok(if flags.contains('r') { right } else { 0 });
            };
            let end = flags[start..].find(|c: char| c != '-' && !c.is_ascii_digit())
                .map_or(flags.len(), |end| start + end);
            if flags[end..].contains(|c: char| c == '-' || c.is_ascii_digit()) {
                return Err("comments offset must be one number".into());
            }
            let number = &flags[start..end];
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
                // Vim visits flags in order: `s1fl` uses block semantics, while
                // `fs1` uses first-only semantics before it reaches the `s`.
                // The latter hybrid is deliberately outside this profile model.
                if flags.find('f').is_some_and(|first| first < flags.find('s').unwrap()) {
                    return Err("first-only flag before block start is unsupported".into());
                }
                if flags.chars().any(|c| !"sbflrO-0123456789".contains(c))
                    || middle_flags.chars().any(|c| !"mbO".contains(c))
                    || end_flags.chars().any(|c| !"ebxlrO-0123456789".contains(c))
                {
                    return Err("unsupported comments flags for three-piece part".into());
                }
                result.order.push(CommentPart::Block(result.blocks.len()));
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
                    middle_no_above: middle_flags.contains('O'),
                    end_no_above: end_flags.contains('O'),
                    first: flags.contains('f'),
                    automatic_end: end_flags.contains('x'),
                });
                index += 3;
            } else {
                if flags.contains(['m', 'e']) {
                    return Err("middle/end comment requires preceding start".into());
                }
                if flags.chars().any(|c| !"nbfO".contains(c)) {
                    return Err("unsupported comments flags for line leader".into());
                }
                if flags.contains('n') && flags.contains('f') {
                    return Err("first-only nested comment leaders are unsupported".into());
                }
                result.order.push(CommentPart::Line(result.line_leaders.len()));
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
        match self.match_start(text)? {
            CommentStart::Line { end, leader } => Some((end, leader)),
            CommentStart::Block { .. } => None,
        }
    }
    pub fn match_start(&self, text: &str) -> Option<CommentStart<'_>> {
        self.order.iter().find_map(|part| match *part {
            CommentPart::Line(index) => {
                let leader = &self.line_leaders[index];
                leader.matches(text).then(|| CommentStart::Line {
                    end: self.repeated_line_end(text, leader), leader,
                })
            }
            CommentPart::Block(index) => {
                let syntax = &self.blocks[index];
                syntax.open_matches(text).then_some(CommentStart::Block { index, syntax })
            }
        })
    }
    fn repeated_line_end(&self, text: &str, first: &CommentLeader) -> usize {
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
        end
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordered_matching_does_not_promote_line_leaders_over_blocks() {
        let block_first = CommentProfile::parse("s1:#{,mb:#,ex:#},:#").unwrap();
        assert!(matches!(block_first.match_start("#{ body"), Some(CommentStart::Block { index: 0, .. })));
        assert!(block_first.line_prefix("#{ body").is_none());
        assert_eq!(block_first.line_prefix("# body").unwrap().0, 1);

        let line_first = CommentProfile::parse(":#,s1:#{,mb:#,ex:#}").unwrap();
        assert!(matches!(line_first.match_start("#{ body"), Some(CommentStart::Line { end: 1, .. })));
    }

    #[test]
    fn ordered_documentation_and_nested_leaders_keep_their_identity() {
        let profile = CommentProfile::parse(":///,://!,://,n:>,n:)").unwrap();
        assert_eq!(profile.line_prefix("/// docs").unwrap().1.text, "///");
        assert_eq!(profile.line_prefix("//! inner").unwrap().1.text, "//!");
        assert_eq!(profile.line_prefix("> ) > text").unwrap().0, 5);
    }

    #[test]
    fn whitespace_middles_keep_their_exact_width_and_body_boundary() {
        let profile = CommentProfile::parse("s:<!--,m:    ,e:-->").unwrap();
        let block = &profile.blocks[0];
        assert!(block.middle_is_whitespace());
        assert_eq!(block.middle_prefix("  ", 4), "      ");
        assert_eq!(block.middle_body_start("      body"), Some(6));
        assert_eq!(block.middle_body_start("  body"), None);
        assert_eq!(block.body_middle_prefix("  ", " ", 4), "       ");
        let starred = &CommentProfile::parse("s1:/*,mb:*,ex:*/").unwrap().blocks[0];
        assert_eq!(starred.middle_body_start(" * body"), Some(2));
        assert_eq!(starred.middle_body_start(" *pointer"), None);
    }

    #[test]
    fn flags_are_retained_or_rejected_instead_of_silently_reinterpreted() {
        let profile = CommentProfile::parse("s1flO:{-,mbO:-,exO:-}").unwrap();
        let block = &profile.blocks[0];
        assert!(block.first && block.no_above && block.middle_no_above && block.end_no_above);
        assert_eq!(block.offset, 1);
        assert_eq!(block.middle_prefix("", 4), " - ");
        for value in [
            "fs1:{-,mb:-,ex:-}", "ns1:/*,m:*,e:*/", "s:/*,mn:*,e:*/",
            "s:/*,m:*,ef:*/", "x:#", "r:#", "2:#", "nf:#",
            "s1r2:/*,m:*,e:*/", "s--1:/*,m:*,e:*/", "s1025:/*,m:*,e:*/",
            "s:/*,e:*/", "s:/*,mf:*,e:*/", "slr:/*,m:*,e:*/",
        ] {
            assert!(CommentProfile::parse(value).is_err(), "unexpectedly accepted {value}");
        }
        let aligned = CommentProfile::parse("sr1:/*,m:*,ex-2:*/").unwrap();
        assert_eq!(aligned.blocks[0].offset, 1); // numeric alignment wins in Vim
        assert_eq!(aligned.blocks[0].end_offset, -2);
    }
}
