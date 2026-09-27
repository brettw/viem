//! Sort key policy is command grammar, while source movement belongs to the
//! document. Every key is computed against one immutable logical snapshot.
use super::ex::SortOptions;
use super::ex_execute::{ExExecuteError, ExExecutionContext, HardLineRange};
use super::search_regex::{CompiledRegex, RegexInput, RegexLimits, RegexWork};
use crate::document::{Document, Format};
use std::{cmp::Ordering, ops::Range};

#[derive(Eq, PartialEq)]
enum Key {
    Text(String),
    Number(Option<i64>),
}
impl Key {
    fn compare(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Self::Text(a), Self::Text(b)) => a.cmp(b),
            (Self::Number(a), Self::Number(b)) => a.cmp(b),
            _ => unreachable!("one sort has one key mode"),
        }
    }
}

pub(super) fn ordered_lines(
    document: &Document,
    context: &ExExecutionContext,
    lines: HardLineRange,
    options: &SortOptions,
    reverse: bool,
) -> Result<(Range<usize>, Vec<usize>), ExExecuteError> {
    if options
        .radix
        .is_some_and(|radix| !matches!(radix, 2 | 8 | 10 | 16))
    {
        return Err(ExExecuteError::InvalidSortArgument(
            "unsupported numeric radix".into(),
        ));
    }
    let snapshot = document.hard_line_snapshot();
    let mut source_lines = lines.start..lines.end + 1;
    // A physical final terminator stays a terminator, rather than becoming a
    // leading empty line. An actual empty paragraph in a rich format is content.
    if matches!(
        document.format(),
        Format::PlainText | Format::Code | Format::MarkdownSource
    ) && source_lines.end == snapshot.line_count()
        && source_lines.end > 1
        && snapshot
            .line(source_lines.end - 1)
            .unwrap()
            .content_range()
            .is_empty()
    {
        source_lines.end -= 1;
        source_lines.start = source_lines.start.min(source_lines.end);
    }
    let limits = RegexLimits::default();
    let regex = options
        .pattern
        .as_ref()
        .map(|pattern| {
            let pattern = if pattern.is_empty() {
                context
                    .last_search_pattern
                    .as_deref()
                    .ok_or(ExExecuteError::NoPreviousSearch)?
            } else {
                pattern.as_str()
            };
            // Vim sort's pattern obeys ignorecase but deliberately not smartcase.
            CompiledRegex::compile(pattern, context.search_options.ignorecase, limits)
                .map_err(ExExecuteError::from)
        })
        .transpose()?;
    let input = RegexInput::new(&snapshot);
    let mut work = RegexWork::new(limits);
    let mut records = Vec::with_capacity(source_lines.len());
    for index in source_lines.clone() {
        let range = snapshot.line(index).unwrap().content_range();
        let key_range = if let Some(regex) = &regex {
            match regex.find(&input, range.start, range.end, &mut work)? {
                Some(matched) if options.match_only => matched.range(),
                Some(matched) => matched.range().end..range.end,
                None => range.start..range.start,
            }
        } else {
            range.clone()
        };
        let text = snapshot
            .slice_utf8(key_range)
            .expect("validated sort key boundaries");
        let key = if let Some(radix) = options.radix {
            Key::Number(integer_key(&text, radix))
        } else {
            Key::Text(if options.ignore_case {
                text.to_lowercase()
            } else {
                text
            })
        };
        let identity = if options.unique {
            let whole = snapshot.slice_utf8(range).expect("hard-line boundaries");
            if options.ignore_case {
                whole.to_lowercase()
            } else {
                whole
            }
        } else {
            String::new()
        };
        records.push((index, key, identity));
    }
    records.sort_by(|a, b| a.1.compare(&b.1));
    // Vim reverses the sorted sequence, including equal-key runs.
    if reverse {
        records.reverse();
    }
    if options.unique {
        records.dedup_by(|a, b| a.2 == b.2);
    }
    Ok((
        source_lines,
        records.into_iter().map(|record| record.0).collect(),
    ))
}

fn integer_key(text: &str, radix: u32) -> Option<i64> {
    let bytes = text.as_bytes();
    // Octal deliberately scans to a decimal digit, as Vim does: a leading 8
    // or 9 is a present number with value zero, not an absent number.
    let start = bytes.iter().position(|byte| match radix {
        16 => byte.is_ascii_hexdigit(),
        2 => matches!(byte, b'0' | b'1'),
        _ => byte.is_ascii_digit(),
    })?;
    let negative = start > 0 && bytes[start - 1] == b'-';
    let mut at = start;
    if radix == 16 && bytes.get(at) == Some(&b'0') && matches!(bytes.get(at + 1), Some(b'x' | b'X'))
    {
        at += 2;
    } else if radix == 2
        && bytes.get(at) == Some(&b'0')
        && matches!(bytes.get(at + 1), Some(b'b' | b'B'))
    {
        at += 2;
    }
    let limit = if negative {
        i64::MAX as u64 + 1
    } else {
        i64::MAX as u64
    };
    let mut value = 0u64;
    while let Some(digit) = bytes
        .get(at)
        .and_then(|byte| (*byte as char).to_digit(radix))
    {
        value = value
            .saturating_mul(radix as u64)
            .saturating_add(digit as u64)
            .min(limit);
        at += 1;
    }
    Some(if negative {
        if value == i64::MAX as u64 + 1 {
            i64::MIN
        } else {
            -(value as i64)
        }
    } else {
        value as i64
    })
}
