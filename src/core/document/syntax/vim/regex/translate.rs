use super::vm::{AssertionKind, Special};

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
pub(super) fn translate(source: &str) -> Result<Translation, String> {
    let chars: Vec<char> = source.chars().collect();
    let mut t = Translation::default();
    let mut i = 0;
    let mut very = false;
    let mut branch_start = true;
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
        if escaped && matches!(c, 'm' | 'v' | 'c' | 'C') {
            match c {
                'm' => very = false,
                'v' => very = true,
                'c' => t.case = Some(true),
                'C' => t.case = Some(false),
                _ => unreachable!(),
            }
            continue;
        }
        if escaped && matches!(c, 'M' | 'V') {
            return Err("nomagic modes are outside native profile v1".into());
        }
        let operator = escaped != very;
        if operator && matches!(c, '(' | ')' | '|') {
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
        if operator && c == '%' && chars.get(i) == Some(&'(')
            || escaped && c == 'z' && chars.get(i) == Some(&'(')
        {
            if c == 'z' {
                t.groups += 1;
                t.external.push(t.groups);
                t.regex.push_str(&format!("(?P<vimcap{}>", t.groups));
            } else {
                t.regex.push_str("(?:");
            }
            i += 1;
            groups.push((start, branch_index));
            branch_index = t.regex.len();
            atom = None;
            branch_start = true;
            continue;
        }
        if operator && matches!(c, '+' | '?' | '=' | '{') || !escaped && c == '*' && !branch_start {
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
            let body = t.regex.split_off(branch_index);
            let id = t.specials.len();
            t.specials
                .push(Special::Assertion(AssertionKind::Ahead(true)));
            t.regex.push_str(&format!("(?P<vimspecial{id}>{body})"));
            atom = None;
            branch_start = true;
            continue;
        }
        if !escaped && c == '[' {
            let (class, multiline) = class(&chars, &mut i, false)?;
            t.multiline |= multiline;
            t.regex.push_str(&class);
        } else if escaped && c == '_' {
            t.multiline = true;
            let c = *chars.get(i).ok_or("missing multiline Vim atom")?;
            i += 1;
            if c == '[' {
                t.regex.push_str(&class(&chars, &mut i, true)?.0);
            } else if c == '.' {
                t.regex.push_str("(?s:.)");
            } else if matches!(c, '^' | '$') {
                t.regex.push(c);
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
                        let c = chars[i];
                        if c == '\\' || c == '[' {
                            return Err(
                                "non-literal Vim abbreviation atom is outside native profile"
                                    .into(),
                            );
                        }
                        t.regex.push_str("(?:");
                        t.regex.push_str(&regex::escape(&c.to_string()));
                        count += 1;
                        i += 1;
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
                Some('#')
                    if chars.get(i + 1) == Some(&'=')
                        && matches!(chars.get(i + 2), Some('0' | '1' | '2')) =>
                {
                    // The engine-selection atom has no matching extent.
                    i += 3;
                    continue;
                }
                _ => return Err("unsupported Vim percent atom".into()),
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
        } else if escaped && character_class(c).is_some() {
            t.regex.push_str(character_class(c).unwrap());
        } else if escaped && c.is_alphanumeric() {
            return Err(format!("unsupported Vim escape: \\{c}"));
        } else if !escaped && c == '~' {
            return Err("substitution-dependent Vim atoms are outside native profile v1".into());
        } else {
            let end_branch = i == chars.len()
                || chars.get(i) == Some(&'\\')
                    && matches!(chars.get(i + 1), Some('|' | ')' | '&' | 'n'))
                || very && matches!(chars.get(i), Some('|' | ')'));
            if !escaped && (c == '.' || c == '^' && branch_start || c == '$' && end_branch) {
                t.regex.push(c);
            } else {
                t.regex.push_str(&regex::escape(&c.to_string()));
            }
            t.multiline |= c == '\n';
        }
        atom = Some(start);
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
        _ => return None,
    })
}

fn class(chars: &[char], i: &mut usize, mut newline: bool) -> Result<(String, bool), String> {
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
    while let Some(&c) = chars.get(*i) {
        *i += 1;
        if c == ']' && !first {
            out.push(']');
            // POSIX space/control classes contain LF as a character, but Vim
            // requires an explicit \n or \_ atom to include the line boundary.
            out = format!("[{out}&&[^\\n]]");
            return Ok((
                if newline {
                    format!("(?:{out}|\\n)")
                } else {
                    out
                },
                newline,
            ));
        }
        if c == '[' && chars.get(*i) == Some(&':') {
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
            if matches!(name.as_str(), "return" | "tab" | "escape" | "backspace") {
                out.truncate(out.len() - 2);
                out.push_str(match name.as_str() {
                    "return" => "\\r",
                    "tab" => "\\t",
                    "escape" => "\\x1b",
                    _ => "\\x08",
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
                    out.push('\\');
                    out.push(next);
                }
                'e' => out.push_str("\\x1b"),
                'b' => out.push_str("\\x08"),
                '\\' | ']' | '^' | '-' | '[' => {
                    out.push('\\');
                    out.push(next);
                }
                _ => return Err(format!("unsupported Vim class escape: \\{next}")),
            }
        } else {
            if c == '-' && chars.get(*i) == Some(&'-') && !first {
                return Err(
                    "ambiguous double-dash character class is outside native profile v1".into(),
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
