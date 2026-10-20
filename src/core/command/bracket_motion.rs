//! `[` and `]` motions: enclosing unmatched delimiters, Vim's brace-based
//! method search and nroff/brace sections. Delimiters are matched like `%`:
//! by the first scalar of each logical grapheme, without quote heuristics.

use crate::document::HardLineSnapshot;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum BracketMotion {
    /// `[(`, `[{`, `])` and `]}`: the delimiter to find.
    Unmatched(char),
    /// `[m`, `]m`, `[M` and `]M`.
    Method { forward: bool, end: bool },
    /// `[[`, `]]`, `[]` and `][`; `end` looks for `}` instead of `{`.
    Section { forward: bool, end: bool },
}

impl BracketMotion {
    /// The motion named by the key after `[` or `]`.
    pub(super) fn from_keys(forward: bool, key: char) -> Option<Self> {
        Some(match (forward, key) {
            (false, '(' | '{') | (true, ')' | '}') => Self::Unmatched(key),
            (_, 'm' | 'M') => Self::Method { forward, end: key == 'M' },
            (_, '[' | ']') => Self::Section { forward, end: (key == '[') == forward },
            _ => return None,
        })
    }
}

/// A motion destination; sections reaching the last line include its final grapheme.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct BracketTarget {
    pub offset: usize,
    pub inclusive: bool,
}

/// Resolve `motion` from `origin`. `last_line` is the last addressable hard
/// line; `with_operator` selects Vim's operator variant of `]]`, which also
/// stops at `}` in the first column. `sections` is the 'sections' option.
pub(super) fn bracket_target(
    lines: &HardLineSnapshot,
    origin: usize,
    motion: BracketMotion,
    count: usize,
    last_line: usize,
    with_operator: bool,
    sections: &str,
) -> Option<BracketTarget> {
    let exclusive = |offset| BracketTarget { offset, inclusive: false };
    match motion {
        BracketMotion::Unmatched(delimiter) => unmatched_delimiter(lines, origin, delimiter, count).map(exclusive),
        BracketMotion::Method { forward, end } => method_boundary(lines, origin, forward, end, count).map(exclusive),
        BracketMotion::Section { forward, end } => {
            section(lines, origin, forward, if end { '}' } else { '{' }, count, last_line, with_operator, sections)
        }
    }
}

fn first_char(lines: &HardLineSnapshot, at: usize) -> Option<char> {
    if at >= lines.text_length() {
        return None;
    }
    std::str::from_utf8(lines.byte_chunk_at(at)).ok()?.chars().next()
}

fn step(lines: &HardLineSnapshot, at: usize, forward: bool) -> Option<usize> {
    if forward {
        lines.next_grapheme_boundary(at).filter(|next| *next < lines.text_length())
    } else {
        lines.previous_grapheme_boundary(at)
    }
}

/// Vim's `findmatchlimit()` for an explicit delimiter: the nearest `{`
/// before (or `}` after) `origin` that encloses it.
fn enclosing(lines: &HardLineSnapshot, origin: usize, forward: bool, opening: char, closing: char) -> Option<usize> {
    let (wanted, nested) = if forward { (closing, opening) } else { (opening, closing) };
    let mut depth = 0usize;
    let mut at = origin;
    loop {
        at = step(lines, at, forward)?;
        let character = first_char(lines, at)?;
        if character == nested {
            depth += 1;
        } else if character == wanted {
            if depth == 0 {
                return Some(at);
            }
            depth -= 1;
        }
    }
}

fn unmatched_delimiter(lines: &HardLineSnapshot, origin: usize, delimiter: char, count: usize) -> Option<usize> {
    let (opening, closing) = if matches!(delimiter, '(' | ')') { ('(', ')') } else { ('{', '}') };
    let forward = matches!(delimiter, ')' | '}');
    let mut found = None;
    let mut at = origin;
    // Vim keeps the outermost delimiter found when the count runs out of levels.
    for _ in 0..count.max(1) {
        let Some(next) = enclosing(lines, at, forward, opening, closing) else { break };
        found = Some(next);
        at = next;
    }
    found
}

/// A port of the `[m`/`]m`/`[M`/`]M` branch of Vim's `nv_bracket_block()`.
fn method_boundary(lines: &HardLineSnapshot, origin: usize, forward: bool, end: bool, count: usize) -> Option<usize> {
    let find = if forward { '}' } else { '{' };
    // Every enclosing brace level from the cursor outwards; `inner` is the
    // level just inside the outermost one, a method when inside a class.
    let (mut inner, mut outer) = (None, None);
    let mut at = origin;
    while let Some(found) = enclosing(lines, at, forward, '{', '}') {
        inner = outer;
        outer = Some(found);
        at = found;
    }
    // True for `[m` and `]M`, which want the brace they search for.
    let wants_found_brace = (find == '{') != end;
    let mut remaining = count.max(1);
    let mut cursor = origin;
    let mut result = None;
    let mut first_brace = outer;
    if let Some(method) = inner {
        result = Some(method);
        cursor = method;
        if wants_found_brace {
            remaining -= 1;
        }
    }
    while remaining > 0 {
        loop {
            let Some(next) = step(lines, cursor, forward) else {
                result?;
                remaining = 0;
                break;
            };
            cursor = next;
            let Some(brace @ ('{' | '}')) = first_char(lines, cursor) else { continue };
            if (brace == find && wants_found_brace) || (remaining == 1 && !wants_found_brace) {
                result = Some(cursor);
                first_brace = Some(cursor);
                remaining = 0;
            } else if first_brace.is_none() {
                // Started outside every block and just entered one.
                first_brace = Some(cursor);
                result = Some(cursor);
            } else if let Some(matched) = enclosing(lines, cursor, forward, '{', '}') {
                // The end of another method: continue from its other brace.
                cursor = matched;
                result = Some(matched);
            } else {
                return None;
            }
            break;
        }
        remaining = remaining.saturating_sub(1);
    }
    result
}

/// Vim's `findpar()` with `startPS()` for section motions.
#[allow(clippy::too_many_arguments)]
fn section(
    lines: &HardLineSnapshot,
    origin: usize,
    forward: bool,
    what: char,
    count: usize,
    last_line: usize,
    with_operator: bool,
    sections: &str,
) -> Option<BracketTarget> {
    let both = with_operator && forward && what == '{';
    let mut current = lines.line_at_offset(origin.min(lines.text_length())).ok()?.index().min(last_line);
    let mut remaining = count.max(1);
    while remaining > 0 {
        remaining -= 1;
        let mut nonblank_seen = false;
        let mut first = true;
        loop {
            let content = lines.line(current)?.content_range();
            if !content.is_empty() {
                nonblank_seen = true;
            }
            if !first && nonblank_seen && starts_section(lines, content.start, what, both, sections) {
                break;
            }
            let next = if forward { current.checked_add(1).filter(|next| *next <= last_line) } else { current.checked_sub(1) };
            match next {
                Some(next) => current = next,
                None if remaining > 0 => return None,
                None => break,
            }
            first = false;
        }
    }
    let start = lines.line(current)?.content_range().start;
    if both && first_char(lines, start) == Some('}') {
        // The operator form includes the line holding the `}`.
        return Some(match lines.line(current + 1).filter(|_| current < last_line) {
            Some(line) => BracketTarget { offset: line.content_range().start, inclusive: false },
            None => BracketTarget { offset: lines.text_length(), inclusive: false },
        });
    }
    if current == last_line && what != '}' && forward {
        let content = lines.line(current)?.content_range();
        if !content.is_empty() {
            let offset = lines.previous_grapheme_boundary(content.end).unwrap_or(content.start).max(content.start);
            return Some(BracketTarget { offset, inclusive: true });
        }
    }
    Some(BracketTarget { offset: start, inclusive: false })
}

fn starts_section(lines: &HardLineSnapshot, start: usize, what: char, both: bool, sections: &str) -> bool {
    let Some(first) = first_char(lines, start) else { return false };
    if first == what || first == '\u{c}' || (both && first == '}') {
        return true;
    }
    if first != '.' {
        return false;
    }
    let rest: Vec<char> = lines
        .line_at_offset(start)
        .ok()
        .and_then(|line| lines.slice_utf8(line.content_range()).ok())
        .map(|text| text.chars().skip(1).take(2).collect())
        .unwrap_or_default();
    in_macro(sections, rest.first().copied(), rest.get(1).copied())
}

/// Vim's `inmacro()`: two-character nroff macro names, where a space in the
/// option also matches a space or the end of the line.
fn in_macro(option: &str, first: Option<char>, second: Option<char>) -> bool {
    let names: Vec<char> = option.chars().collect();
    names.chunks(2).any(|name| {
        let (a, b) = (name[0], name.get(1).copied());
        let first_matches = Some(a) == first || (a == ' ' && matches!(first, None | Some(' ')));
        let second_matches = b == second
            || (matches!(b, None | Some(' ')) && (first.is_none() || matches!(second, None | Some(' '))));
        first_matches && second_matches
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;

    fn target(text: &str, origin: usize, forward: bool, key: char, count: usize) -> Option<usize> {
        let document = Document::new(text);
        let lines = document.hard_line_snapshot();
        let motion = BracketMotion::from_keys(forward, key).unwrap();
        bracket_target(&lines, origin, motion, count, document.addressable_hard_line_count() - 1, false, "SHNHH HUnhsh")
            .map(|target| target.offset)
    }

    #[test]
    fn unmatched_delimiters_skip_nested_pairs_and_keep_the_last_level() {
        let text = "f(a, (b), g(c))";
        let at = text.find('c').unwrap();
        assert_eq!(target(text, at, false, '(', 1), Some(11));
        assert_eq!(target(text, at, false, '(', 2), Some(1));
        assert_eq!(target(text, at, false, '(', 9), Some(1));
        assert_eq!(target(text, at, true, ')', 1), Some(13));
        assert_eq!(target(text, at, true, ')', 2), Some(14));
        assert_eq!(target(text, 0, false, '(', 1), None);
        assert_eq!(target(text, 1, false, '(', 1), None, "the delimiter under the cursor is not enclosing");
    }

    #[test]
    fn methods_follow_vim_brace_search() {
        let text = "class A {\n  void f() {\n    x;\n  }\n\n  void g() {\n    y;\n  }\n}\n";
        let f = text.find("f() {").unwrap() + 4;
        let f_end = text.find("  }").unwrap() + 2;
        let g = text.find("g() {").unwrap() + 4;
        let g_end = text.rfind("  }").unwrap() + 2;
        let x = text.find('x').unwrap();
        let y = text.find('y').unwrap();
        let between = text.find("\n\n").unwrap() + 1;
        assert_eq!(target(text, y, false, 'm', 1), Some(g));
        assert_eq!(target(text, y, false, 'm', 2), Some(f));
        assert_eq!(target(text, x, true, 'm', 1), Some(g));
        assert_eq!(target(text, between, false, 'm', 1), Some(f));
        assert_eq!(target(text, between, true, 'm', 1), Some(g));
        assert_eq!(target(text, x, true, 'M', 1), Some(f_end));
        assert_eq!(target(text, y, false, 'M', 1), Some(f_end));
        assert_eq!(target(text, between, true, 'M', 1), Some(g_end));
        assert_eq!(target(text, 0, true, 'm', 1), Some(8), "outside the class, ]m finds its start");
    }

    #[test]
    fn sections_stop_at_first_column_braces_form_feeds_and_macros() {
        let text = "intro\n{\none\n}\n\u{c}\n.SH NAME\nlast";
        let line = |index: usize| text.split('\n').take(index).map(|line| line.len() + 1).sum::<usize>();
        assert_eq!(target(text, 0, true, ']', 1), Some(line(1)));
        assert_eq!(target(text, 0, true, ']', 2), Some(line(4)));
        assert_eq!(target(text, 0, true, ']', 3), Some(line(5)));
        assert_eq!(target(text, 0, true, ']', 4), Some(text.len() - 1), "the last line ends inclusively");
        assert_eq!(target(text, 0, true, ']', 5), None);
        assert_eq!(target(text, line(6), false, '[', 1), Some(line(5)));
        assert_eq!(target(text, 0, true, '[', 1), Some(line(3)), "][ finds a closing brace");
        assert_eq!(target(text, line(6), false, ']', 1), Some(line(5)), "[] also stops at section macros");
        assert_eq!(target(text, line(4), false, ']', 1), Some(line(3)), "[] finds a closing brace backward");
        assert!(in_macro("SHNHH HUnhsh", Some('H'), None));
        assert!(!in_macro("SHNHH HUnhsh", Some('X'), Some('Y')));
    }
}
