use super::vm::{AssertionKind, PositionKind, Special};

#[derive(Clone, Copy, Eq, PartialEq)]
enum Magic {
    Very,
    Normal,
    No,
    VeryNo,
}
impl Magic {
    fn special(self, c: char, escaped: bool) -> bool {
        let bare = match c {
            '^' | '$' => self != Self::VeryNo,
            '.' | '[' | '*' | '~' => matches!(self, Self::Very | Self::Normal),
            _ => self == Self::Very,
        };
        escaped != bare
    }
}

#[derive(Default)]
pub(super) struct Translation {
    pub regex: String,
    pub external: Vec<usize>,
    pub multiline: bool,
    pub starts: Vec<usize>,
    pub ends: Vec<usize>,
    pub case: Option<bool>,
    pub specials: Vec<Special>,
    pub groups: usize,
}

/// Parse Vim's magic grammar before passing the regular atoms to regex-syntax.
/// Synthetic named captures carry non-regular instructions into the VM compiler;
/// they never consume text or become user-visible capture registers.
pub(super) fn translate(source: &str, keyword: &super::VimKeyword) -> Result<Translation, String> {
    let chars: Vec<char> = source.chars().collect();
    let mut t = Translation::default();
    let mut i = 0;
    let mut magic = Magic::Normal;
    let mut branch_start = true;
    let mut initial_anchor = false;
    let mut atom = None;
    let mut branch_index = 0;
    let mut registers = Vec::new();
    let mut groups = Vec::new();
    while i < chars.len() {
        let start = t.regex.len();
        let c = chars[i];
        i += 1;
        let escaped = c == '\\';
        let c = if escaped {
            let c = *chars.get(i).ok_or("trailing Vim escape")?;
            i += 1;
            c
        } else {
            c
        };
        if escaped && matches!(c, 'm' | 'M' | 'v' | 'V' | 'c' | 'C') {
            match c {
                'm' => magic = Magic::Normal,
                'M' => magic = Magic::No,
                'v' => magic = Magic::Very,
                'V' => magic = Magic::VeryNo,
                'c' => t.case = Some(true),
                'C' => t.case = Some(false),
                _ => unreachable!(),
            }
            continue;
        }
        let operator = magic.special(c, escaped);
        if operator && matches!(c, '(' | ')' | '|') {
            initial_anchor = false;
            match c {
                '(' => {
                    t.groups += 1;
                    registers.push(t.groups);
                    t.regex.push_str(&format!("(?P<vimcap{}>", t.groups));
                    groups.push((start, branch_index));
                    branch_index = t.regex.len();
                    atom = None;
                    branch_start = true;
                }
                ')' => {
                    t.regex.push(')');
                    let (group_start, outer_branch) =
                        groups.pop().ok_or("unmatched Vim group close")?;
                    atom = Some(group_start);
                    branch_index = outer_branch;
                    branch_start = false;
                }
                '|' => {
                    t.regex.push('|');
                    branch_index = t.regex.len();
                    atom = None;
                    branch_start = true;
                }
                _ => unreachable!(),
            }
            continue;
        }
        let group_open = if chars.get(i) == Some(&'(') {
            1
        } else if chars.get(i) == Some(&'\\') && chars.get(i + 1) == Some(&'(') {
            2
        } else {
            0
        };
        if group_open != 0 && (operator && c == '%' || escaped && c == 'z') {
            initial_anchor = false;
            if c == 'z' {
                t.groups += 1;
                t.external.push(t.groups);
                t.regex.push_str(&format!("(?P<vimcap{}>", t.groups));
            } else {
                t.regex.push_str("(?:");
            }
            i += group_open;
            groups.push((start, branch_index));
            branch_index = t.regex.len();
            atom = None;
            branch_start = true;
            continue;
        }
        // A newline permits a following line-start anchor, but remains a real
        // preceding atom for repetition (notably Git commit trailer \n*).
        if operator && c == '*' && escaped && (atom.is_none() || initial_anchor) {
            return Err("Vim repetition has no preceding atom".into());
        }
        if operator
            && (matches!(c, '+' | '?' | '=' | '{') || c == '*' && atom.is_some() && !initial_anchor)
        {
            if atom.is_none() {
                return Err("Vim repetition has no preceding atom".into());
            }
            if c == '{' {
                let (spec, end) = super::repetition(&chars, i)?;
                t.regex.push_str(&spec);
                i = end;
            } else {
                t.regex.push(if c == '=' { '?' } else { c });
            }
            continue;
        }
        if operator && c == '@' {
            let begin = atom.ok_or("Vim assertion has no preceding atom")?;
            let number_start = i;
            while chars.get(i).is_some_and(char::is_ascii_digit) {
                i += 1;
            }
            let limit = if i == number_start {
                None
            } else {
                let n = chars[number_start..i]
                    .iter()
                    .collect::<String>()
                    .parse::<usize>()
                    .map_err(|_| "Vim lookbehind byte limit overflow")?;
                (n != 0).then_some(n)
            };
            let kind = match chars.get(i) {
                Some('=') => {
                    i += 1;
                    AssertionKind::Ahead(true)
                }
                Some('!') => {
                    i += 1;
                    AssertionKind::Ahead(false)
                }
                Some('>') => {
                    i += 1;
                    AssertionKind::Atomic
                }
                Some('<') if matches!(chars.get(i + 1), Some('=' | '!')) => {
                    i += 2;
                    AssertionKind::Behind {
                        positive: chars[i - 1] == '=',
                        limit,
                    }
                }
                _ => return Err("invalid Vim postfix assertion".into()),
            };
            if limit.is_some() && !matches!(kind, AssertionKind::Behind { .. }) {
                return Err("byte limits apply only to Vim lookbehind assertions".into());
            }
            let body = t.regex.split_off(begin);
            let id = t.specials.len();
            t.specials.push(Special::Assertion(kind));
            t.regex.push_str(&format!("(?P<vimspecial{id}>{body})"));
            continue;
        }
        if operator && c == '&' {
            initial_anchor = false;
            let body = t.regex.split_off(branch_index);
            let id = t.specials.len();
            t.specials
                .push(Special::Assertion(AssertionKind::Ahead(true)));
            t.regex.push_str(&format!("(?P<vimspecial{id}>{body})"));
            atom = None;
            branch_start = true;
            continue;
        }
        if operator && c == '[' {
            let after_open = i;
            match class(&chars, &mut i, false, keyword, &mut t.specials) {
                Ok((class, multiline)) => {
                    t.multiline |= multiline;
                    t.regex.push_str(&class);
                }
                Err(error) if error == "unterminated Vim character class" => {
                    // An unmatched opening bracket is a literal in Vim. The
                    // following characters still use the current magic mode.
                    i = after_open;
                    t.regex.push_str("\\[");
                }
                Err(error) => return Err(error),
            }
        } else if escaped && c == '_' {
            t.multiline = true;
            if chars.get(i) == Some(&'\\')
                && chars.get(i + 1).is_some_and(|c| {
                    character_class(*c).is_some() || matches!(c, '.' | '^' | '$' | '[')
                })
            {
                i += 1;
            }
            let c = *chars.get(i).ok_or("missing multiline Vim atom")?;
            i += 1;
            if c == '[' {
                t.regex
                    .push_str(&class(&chars, &mut i, true, keyword, &mut t.specials)?.0);
            } else if c == '.' {
                t.regex.push_str("(?s:.)");
            } else if matches!(c, '^' | '$') {
                t.regex.push(c);
            } else if matches!(c, 'k' | 'K') {
                t.regex
                    .push_str(&format!("(?-i:(?:{}|\\n))", keyword.class(c == 'K')));
            } else {
                let atom = character_class(c).ok_or("unsupported multiline Vim atom")?;
                t.regex.push_str(&format!("(?:{atom}|\\n)"));
            }
        } else if operator && c == '%' {
            match chars.get(i) {
                Some('[') => {
                    i += 1;
                    let mut count = 0;
                    while chars.get(i).is_some_and(|c| *c != ']') {
                        let mut child = abbreviation_atom(&chars, &mut i, magic, keyword)?;
                        // Child translations have local synthetic identities.
                        // Replace from highest to lowest so new identities
                        // cannot collide with a later replacement.
                        for id in (0..child.specials.len()).rev() {
                            child.regex = child.regex.replace(
                                &format!("<vimspecial{id}>"),
                                &format!("<vimspecial{}>", id + t.specials.len()),
                            );
                        }
                        t.specials.extend(child.specials);
                        t.regex.push_str("(?:");
                        t.regex.push_str(&child.regex);
                        t.multiline |= child.multiline;
                        count += 1;
                    }
                    if chars.get(i) != Some(&']') || count == 0 {
                        return Err("invalid Vim abbreviation".into());
                    }
                    i += 1;
                    for _ in 0..count {
                        t.regex.push_str(")?");
                    }
                }
                Some('^' | '$') => {
                    t.regex
                        .push_str(if chars[i] == '^' { "\\A" } else { "\\z" });
                    i += 1;
                }
                Some('d' | 'o' | 'x' | 'u' | 'U') => {
                    let kind = chars[i];
                    i += 1;
                    let scalar = character_code(&chars, &mut i, kind)?;
                    if matches!(scalar, '\0' | '\n') {
                        let id = t.specials.len();
                        t.specials.push(Special::ControlClass { negate: false });
                        t.regex.push_str(&format!("(?P<vimspecial{id}>\\x00)"));
                    } else {
                        t.regex.push_str(&regex::escape(&scalar.to_string()));
                    }
                }
                Some('#')
                    if chars.get(i + 1) == Some(&'=')
                        && matches!(chars.get(i + 2), Some('0' | '1' | '2')) =>
                {
                    // The engine-selection atom has no matching extent.
                    i += 3;
                    continue;
                }
                Some('0'..='9' | '<' | '>') => {
                    let comparison = match chars[i] {
                        '<' => {
                            i += 1;
                            std::cmp::Ordering::Less
                        }
                        '>' => {
                            i += 1;
                            std::cmp::Ordering::Greater
                        }
                        _ => std::cmp::Ordering::Equal,
                    };
                    let begin = i;
                    while chars.get(i).is_some_and(char::is_ascii_digit) {
                        i += 1;
                    }
                    if begin == i {
                        return Err(percent_atom_error(&chars, begin));
                    }
                    let value = chars[begin..i]
                        .iter()
                        .collect::<String>()
                        .parse::<usize>()
                        .map_err(|_| "Vim source-position number overflow")?;
                    let kind = match chars.get(i) {
                        Some('l') => PositionKind::Line,
                        Some('c') => PositionKind::ByteColumn,
                        Some('v') => PositionKind::VirtualColumn,
                        _ => return Err(percent_atom_error(&chars, begin)),
                    };
                    i += 1;
                    // Absolute line numbers change for every following line
                    // after an inserted/deleted line break. Column numbers
                    // need only the existing whole-hard-line dependencies.
                    t.multiline |= kind == PositionKind::Line;
                    let id = t.specials.len();
                    t.specials.push(Special::Position {
                        kind,
                        comparison,
                        value,
                    });
                    t.regex.push_str(&format!("(?P<vimspecial{id}>)"));
                }
                _ => return Err(percent_atom_error(&chars, i)),
            }
        } else if escaped && c == 'z' && matches!(chars.get(i), Some('s' | 'e')) {
            t.groups += 1;
            if chars[i] == 's' {
                t.starts.push(t.groups);
            } else {
                t.ends.push(t.groups);
            }
            t.regex.push_str(&format!("(?P<vimcap{}>)", t.groups));
            i += 1;
        } else if escaped && ('1'..='9').contains(&c) {
            let id = t.specials.len();
            t.specials
                .push(Special::Backreference((c as u8 - b'0') as usize));
            t.regex.push_str(&format!("(?P<vimspecial{id}>)"));
        } else if escaped && matches!(c, 't' | 'r' | 'n' | 'e' | 'b') {
            t.multiline |= c == 'n';
            t.regex.push_str(match c {
                'e' => "\\x1b",
                'b' => "\\x08",
                't' => "\\t",
                'r' => "\\r",
                _ => "\\n",
            });
        } else if operator && matches!(c, '<' | '>') {
            t.regex
                .push_str(if c == '<' { "\\b{start}" } else { "\\b{end}" });
        } else if escaped && matches!(c, 'k' | 'K') {
            t.regex
                .push_str(&format!("(?-i:{})", keyword.class(c == 'K')));
        } else if escaped && character_class(c).is_some() {
            t.regex.push_str(character_class(c).unwrap());
        } else if escaped
            && matches!(
                c,
                'g' | 'j' | 'q' | 'y' | 'B' | 'E' | 'G' | 'J' | 'N' | 'Q' | 'R' | 'T' | 'Y' | '0'
            )
        {
            // Vim reserves no atom for these escapes; e.g. css.vim spells
            // gradient as \\gradient. Pin the literal set instead of accepting
            // unsupported state-dependent atoms such as \\Z and \\z.
            t.regex.push(c);
        } else if escaped && c.is_alphanumeric() {
            return Err(format!("unsupported Vim escape: \\{c}"));
        } else if operator && c == '~' {
            return Err("substitution-dependent Vim atoms are outside native profile v1".into());
        } else {
            let end_branch = i == chars.len()
                || chars.get(i) == Some(&'\\')
                    && matches!(chars.get(i + 1), Some('|' | ')' | '&' | 'n'))
                || magic == Magic::Very && matches!(chars.get(i), Some('|' | ')'));
            if operator && (c == '.' || c == '^' && branch_start || c == '$' && end_branch) {
                t.regex.push(c);
            } else {
                t.regex.push_str(&regex::escape(&c.to_string()));
            }
            t.multiline |= c == '\n';
        }
        atom = Some(start);
        initial_anchor = operator && c == '^' && branch_start;
        branch_start = c == '\n' || escaped && c == 'n';
    }
    if !groups.is_empty() {
        return Err("unclosed Vim group".into());
    }
    for special in &mut t.specials {
        if let Special::Backreference(id) = special {
            *id = *registers
                .get(*id - 1)
                .ok_or("Vim backreference names an absent capture")?;
        }
    }
    Ok(t)
}

fn percent_atom_error(chars: &[char], mut at: usize) -> String {
    let reason = match chars.get(at) {
        Some('#') if chars.get(at + 1) == Some(&'=') => {
            "invalid regular-expression engine selector"
        }
        Some('#') => "editor cursor-position assertion",
        Some('V') => "editor Visual-selection assertion",
        Some('\'') => "editor mark-position assertion",
        Some('C') => "composing-character atom",
        Some('0'..='9' | '<' | '>' | '.') => {
            if matches!(chars.get(at), Some('<' | '>')) {
                at += 1;
            }
            if chars.get(at) == Some(&'.') {
                "editor current-position assertion"
            } else {
                while chars.get(at).is_some_and(char::is_ascii_digit) {
                    at += 1;
                }
                match chars.get(at) {
                    Some('l') => "absolute source-line assertion",
                    Some('c') => "absolute source-byte-column assertion",
                    Some('v') => "absolute virtual-column assertion",
                    _ => "unknown position assertion",
                }
            }
        }
        _ => "unknown atom",
    };
    format!("unsupported Vim percent atom: {reason}")
}

fn abbreviation_atom(
    chars: &[char],
    i: &mut usize,
    magic: Magic,
    keyword: &super::VimKeyword,
) -> Result<Translation, String> {
    let begin = *i;
    let escaped = chars[*i] == '\\';
    *i += usize::from(escaped);
    let c = *chars.get(*i).ok_or("trailing Vim abbreviation escape")?;
    *i += 1;
    if c == '[' && magic.special(c, escaped) {
        class(chars, i, false, keyword, &mut Vec::new())?;
    } else if escaped && c == '_' {
        let c = *chars.get(*i).ok_or("missing multiline abbreviation atom")?;
        *i += 1;
        if c == '[' {
            class(chars, i, true, keyword, &mut Vec::new())?;
        }
    } else if c == '%' && magic.special(c, escaped) {
        let kind = *chars.get(*i).ok_or("missing percent abbreviation atom")?;
        *i += 1;
        if matches!(kind, 'd' | 'o' | 'x' | 'u' | 'U') {
            character_code(chars, i, kind)?;
        } else if !matches!(kind, '^' | '$') {
            return Err("nested or grouped Vim abbreviation atom is invalid".into());
        }
    }
    let mode = match magic {
        Magic::Normal => "\\m",
        Magic::Very => "\\v",
        Magic::No => "\\M",
        Magic::VeryNo => "\\V",
    };
    let source = format!("{mode}{}", chars[begin..*i].iter().collect::<String>());
    let translated = translate(&source, keyword)?;
    if translated.groups != 0
        || translated
            .specials
            .iter()
            .any(|s| !matches!(s, Special::ControlClass { .. }))
        || translated.case.is_some()
        || translated.regex.is_empty()
    {
        return Err("unsupported stateful Vim abbreviation atom".into());
    }
    Ok(translated)
}

fn character_class(c: char) -> Option<&'static str> {
    Some(match c {
        's' => "[ \\t]",
        'S' => "[^ \\t\\n]",
        'd' => "[0-9]",
        'D' => "[^0-9\\n]",
        'w' => "[A-Za-z0-9_]",
        'W' => "[^A-Za-z0-9_\\n]",
        'h' => "[A-Za-z_]",
        'H' => "[^A-Za-z_\\n]",
        'a' => "[A-Za-z]",
        'A' => "[^A-Za-z\\n]",
        'l' => "(?-i:[a-z])",
        'L' => "(?-i:[^a-z\\n])",
        'u' => "(?-i:[A-Z])",
        'U' => "(?-i:[^A-Z\\n])",
        'x' => "[A-Fa-f0-9]",
        'X' => "[^A-Fa-f0-9\\n]",
        'o' => "[0-7]",
        'O' => "[^0-7\\n]",
        'k' | 'i' => "[_0-9\\p{L}\\x{c0}-\\x{ff}]",
        'K' | 'I' => "[_\\p{L}\\x{c0}-\\x{ff}]",
        // A fixed portable filename environment follows Vim's Unix defaults.
        // UTF-8 filename classes include every scalar from U+00A0 onward.
        'f' => "[A-Za-z0-9/._+,#$%~=\\-\\x{a0}-\\x{10ffff}]",
        'F' => "[A-Za-z/._+,#$%~=\\-\\x{a0}-\\x{10ffff}]",
        // Vim's UTF-8 printable environment, checked against every valid scalar
        // in MacVim 9.1.1887. Surrogates are already absent from UTF-8 inputs.
        'p' => "[\\x20-\\x7e\\x{a0}-\\x{10ffff}&&[^\\x{70f}\\x{180b}-\\x{180e}\\x{200b}-\\x{200f}\\x{202a}-\\x{202e}\\x{2060}-\\x{206f}\\x{feff}\\x{fff9}-\\x{fffb}\\x{fffe}-\\x{ffff}]]",
        'P' => "[\\x20-\\x7e\\x{a0}-\\x{10ffff}&&[^0-9\\x{70f}\\x{180b}-\\x{180e}\\x{200b}-\\x{200f}\\x{202a}-\\x{202e}\\x{2060}-\\x{206f}\\x{feff}\\x{fff9}-\\x{fffb}\\x{fffe}-\\x{ffff}]]",
        _ => return None,
    })
}

fn character_code(chars: &[char], i: &mut usize, kind: char) -> Result<char, String> {
    let (radix, maximum) = match kind {
        'd' => (10, usize::MAX),
        'o' => (
            8,
            if chars.get(*i).is_some_and(|c| *c > '3') {
                2
            } else {
                3
            },
        ),
        'x' => (16, 2),
        'u' => (16, 4),
        'U' => (16, 8),
        _ => unreachable!(),
    };
    let begin = *i;
    let mut value = 0u32;
    while *i - begin < maximum {
        let Some(digit) = chars.get(*i).and_then(|c| c.to_digit(radix)) else {
            break;
        };
        value = value
            .checked_mul(radix)
            .and_then(|v| v.checked_add(digit))
            .ok_or("Vim character code overflow")?;
        *i += 1;
    }
    if begin == *i {
        return Err("Vim character code requires digits".into());
    }
    char::from_u32(value).ok_or_else(|| "Vim character code is not a Unicode scalar".into())
}

fn class(
    chars: &[char],
    i: &mut usize,
    mut newline: bool,
    keyword: &super::VimKeyword,
    specials: &mut Vec<Special>,
) -> Result<(String, bool), String> {
    let mut out = String::from("[");
    let negate = chars.get(*i) == Some(&'^');
    if negate {
        out.push('^');
        *i += 1;
    }
    // In Vim negated collections exclude a line break unless prefixed by \_.
    if negate {
        out.push_str("\\n");
    }
    let mut first = true;
    let mut controls = false;
    while let Some(&c) = chars.get(*i) {
        *i += 1;
        if c == ']' && !first {
            out.push(']');
            // POSIX space/control classes contain LF as a character, but Vim
            // requires an explicit \n or \_ atom to include the line boundary.
            if controls {
                // Vim represents source NUL as LF internally. Numeric ranges
                // keep both endpoints (0 and 10 alias NUL), and complement is
                // applied after that aliasing. String predicates instead see
                // LF directly. Keep the class intact for the context-aware VM.
                if negate {
                    out.replace_range(..4, "["); // strip the synthetic [^\\n
                }
                let id = specials.len();
                specials.push(Special::ControlClass { negate });
                out = format!("(?P<vimspecial{id}>{out})");
            } else {
                out = format!("[{out}&&[^\\n]]");
            }
            return Ok((
                if newline {
                    format!("(?:{out}|\\n)")
                } else {
                    out
                },
                newline,
            ));
        }
        if c == '['
            && chars.get(*i) == Some(&'.')
            && chars.get(*i + 2) == Some(&'.')
            && chars.get(*i + 3) == Some(&']')
        {
            let scalar = *chars.get(*i + 1).ok_or("missing Vim collation element")?;
            out.push_str(&format!("\\x{{{:x}}}", scalar as u32));
            *i += 4;
        } else if c == '['
            && chars.get(*i) == Some(&'=')
            && chars.get(*i + 2) == Some(&'=')
            && chars.get(*i + 3) == Some(&']')
        {
            return Err("Vim equivalence classes are outside the native profile".into());
        } else if c == '[' && valid_posix_class(chars, *i) {
            out.push_str("[:");
            *i += 1;
            let start = *i;
            while chars.get(*i).is_some_and(|c| *c != ':') {
                *i += 1;
            }
            if chars.get(*i + 1) != Some(&']') {
                return Err("invalid Vim POSIX class".into());
            }
            let name: String = chars[start..*i].iter().collect();
            if matches!(
                name.as_str(),
                "return"
                    | "tab"
                    | "escape"
                    | "backspace"
                    | "ident"
                    | "keyword"
                    | "fname"
                    | "lower"
                    | "upper"
                    | "print"
            ) {
                out.truncate(out.len() - 2);
                let keyword_class = keyword.class(false);
                out.push_str(match name.as_str() {
                    "return" => "\\r",
                    "tab" => "\\t",
                    "escape" => "\\x1b",
                    "backspace" => "\\x08",
                    // Unlike \\i, Vim's POSIX ident class is restricted to bytes.
                    "ident" => "A-Za-z0-9_\\x{c0}-\\x{d6}\\x{d8}-\\x{f6}\\x{f8}-\\x{ff}",
                    "keyword" => &keyword_class,
                    "fname" => character_class('f').unwrap(),
                    "print" => character_class('p').unwrap(),
                    "lower" => "\\p{Lowercase}",
                    "upper" => "\\p{Uppercase}",
                    _ => unreachable!(),
                });
            } else {
                out.push_str(&name);
                out.push_str(":]");
            }
            *i += 2;
        } else if c == '\\' {
            let next = *chars.get(*i).ok_or("unterminated Vim class escape")?;
            *i += 1;
            match next {
                't' | 'r' | 'n' => {
                    newline |= next == 'n';
                    if next != 'n' {
                        out.push('\\');
                        out.push(next);
                    } else {
                        // Keep an otherwise empty collection syntactically
                        // valid; the actual hard-line branch is added below.
                        out.push_str("[a&&[^a]]");
                    }
                }
                'e' => out.push_str("\\x1b"),
                'b' => out.push_str("\\x08"),
                '\\' | ']' | '^' | '-' | '[' => {
                    out.push('\\');
                    out.push(next);
                }
                'd' | 'o' | 'x' | 'u' | 'U'
                    if chars.get(*i).is_some_and(|c| {
                        c.is_digit(if next == 'd' {
                            10
                        } else if next == 'o' {
                            8
                        } else {
                            16
                        })
                    }) =>
                {
                    let scalar = character_code(chars, i, next)?;
                    controls |= scalar <= '\n';
                    out.push_str(&format!("\\x{{{:x}}}", scalar as u32));
                }
                _ => {
                    // Other collection escapes include BOTH the backslash and
                    // the following character; they are not ordinary atoms.
                    out.push_str("\\\\");
                    out.push_str(&format!("\\x{{{:x}}}", next as u32));
                }
            }
        } else {
            if c == '-' && chars.get(*i) == Some(&'-') && !first {
                return Err(
                    "ambiguous double-dash character class is outside the native profile".into(),
                );
            }
            if matches!(c, '[' | ']' | '&' | '~' | '^')
                || c == '-' && (first || chars.get(*i) == Some(&']'))
            {
                out.push('\\');
            }
            out.push(c);
        }
        first = false;
    }
    Err("unterminated Vim character class".into())
}

fn valid_posix_class(chars: &[char], at: usize) -> bool {
    if chars.get(at) != Some(&':') {
        return false;
    }
    // Unknown/incomplete POSIX spellings retain a literal '[' in Vim. All
    // supported names are short, so this recognition has bounded local work.
    let mut end = at + 1;
    while end - at <= 10 && chars.get(end).is_some_and(char::is_ascii_alphabetic) {
        end += 1;
    }
    if chars.get(end) != Some(&':') || chars.get(end + 1) != Some(&']') {
        return false;
    }
    matches!(
        chars[at + 1..end].iter().collect::<String>().as_str(),
        "alnum"
            | "alpha"
            | "blank"
            | "cntrl"
            | "digit"
            | "graph"
            | "lower"
            | "print"
            | "punct"
            | "space"
            | "upper"
            | "xdigit"
            | "return"
            | "tab"
            | "escape"
            | "backspace"
            | "ident"
            | "keyword"
            | "fname"
    )
}
