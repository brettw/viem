//! Vim regular expressions on strings for `=~`, `match()`, `substitute()`,
//! `split()` and friends. Expressions always use 'magic' with an empty
//! 'cpoptions', as in Vim, through the same bounded matcher as Vim syntax.
use super::error::VimError;
use crate::document::syntax::vim::{VimPattern, VimRegexLimits};
use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;

const CACHE_LIMIT: usize = 128;

thread_local! {
    static CACHE: std::cell::RefCell<HashMap<(Vec<u8>, bool), Arc<VimPattern>>> = std::cell::RefCell::new(HashMap::new());
}

pub fn compile(pattern: &[u8], ignore_case: bool) -> Result<Arc<VimPattern>, VimError> {
    let key = (pattern.to_vec(), ignore_case);
    if let Some(found) = CACHE.with(|cache| cache.borrow().get(&key).cloned()) {
        return Ok(found);
    }
    let source = String::from_utf8_lossy(pattern);
    let compiled = VimPattern::compile(&source, ignore_case, VimRegexLimits::default())
        .map_err(|error| VimError::new(383, format!("Invalid search string: {source} ({error})")))?;
    let compiled = Arc::new(compiled);
    CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if cache.len() >= CACHE_LIMIT {
            cache.clear();
        }
        cache.insert(key, compiled.clone());
    });
    Ok(compiled)
}

/// A match with byte offsets; `groups[0]` is the whole match and
/// `groups[1..=9]` are `\1`..`\9`.
#[derive(Clone, Debug)]
pub struct Match {
    pub start: usize,
    pub end: usize,
    pub groups: Vec<Option<Range<usize>>>,
}

pub fn find_at(pattern: &VimPattern, text: &str, start: usize, fuel: &mut u64) -> Result<Option<Match>, VimError> {
    if start > text.len() {
        return Ok(None);
    }
    let mut start = start;
    while !text.is_char_boundary(start) {
        start += 1;
    }
    let mut budget = (*fuel).min(usize::MAX as u64) as usize;
    let before = budget;
    let found = pattern
        .find_text_with_control(text, start, &mut budget, &mut || false)
        .map_err(|error| {
            if error.contains("budget") || error.contains("cancelled") {
                VimError::plain("pattern matching work budget exhausted")
            } else {
                VimError::plain(error)
            }
        })?;
    *fuel = fuel.saturating_sub((before - budget) as u64);
    Ok(found.map(|found| {
        let mut groups = vec![Some(found.start..found.end)];
        for index in pattern.register_captures().iter().take(9) {
            groups.push(found.captures.get(*index).cloned().flatten());
        }
        while groups.len() < 10 {
            groups.push(None);
        }
        Match { start: found.start, end: found.end, groups }
    }))
}

pub fn string_matches(text: &[u8], pattern: &[u8], ignore_case: bool, fuel: &mut u64) -> Result<bool, VimError> {
    let compiled = compile(pattern, ignore_case)?;
    let text = String::from_utf8_lossy(text);
    Ok(find_at(&compiled, &text, 0, fuel)?.is_some())
}

/// Expand a `substitute()` replacement: `&`/`\0`, `\1`..`\9`, `~` is literal
/// in expressions; `\n` inserts NL, `\r` CR, `\t` Tab, `\u\U\l\L\e\E` case.
pub fn expand_replacement(replacement: &str, text: &str, found: &Match) -> String {
    let group = |index: usize| -> &str {
        found.groups.get(index).cloned().flatten().map_or("", |range| &text[range])
    };
    #[derive(Clone, Copy, PartialEq)]
    enum Case {
        None,
        UpperOne,
        LowerOne,
        Upper,
        Lower,
    }
    let mut out = String::new();
    let mut one = Case::None;
    let mut all = Case::None;
    let push = |out: &mut String, piece: &str, one: &mut Case, all: Case| {
        for c in piece.chars() {
            let c: String = match (*one, all) {
                (Case::UpperOne, _) => {
                    *one = Case::None;
                    c.to_uppercase().collect()
                }
                (Case::LowerOne, _) => {
                    *one = Case::None;
                    c.to_lowercase().collect()
                }
                (_, Case::Upper) => c.to_uppercase().collect(),
                (_, Case::Lower) => c.to_lowercase().collect(),
                _ => c.to_string(),
            };
            out.push_str(&c);
        }
    };
    let mut chars = replacement.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '&' => push(&mut out, group(0), &mut one, all),
            '\\' => match chars.next() {
                Some(d @ '0'..='9') => push(&mut out, group(d as usize - '0' as usize), &mut one, all),
                Some('n') => out.push('\0'),
                Some('r') => out.push('\r'),
                Some('t') => out.push('\t'),
                Some('u') => one = Case::UpperOne,
                Some('l') => one = Case::LowerOne,
                Some('U') => all = Case::Upper,
                Some('L') => all = Case::Lower,
                Some('e') | Some('E') => {
                    all = Case::None;
                    one = Case::None;
                }
                Some('&') => push(&mut out, "&", &mut one, all),
                Some(other) => push(&mut out, &other.to_string(), &mut one, all),
                None => out.push('\\'),
            },
            other => push(&mut out, &other.to_string(), &mut one, all),
        }
    }
    // In substitute() "\n" inserts a NL character; Vim stores NUL internally
    // for buffer lines only.
    out.replace('\0', "\n")
}
