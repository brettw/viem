//! Normal and Visual number changes using Vim's default `bin,hex` number formats.

use super::*;

#[derive(Debug)]
struct NumberToken {
    range: Range<usize>,
    digits: Range<usize>,
    radix: u32,
    negative: bool,
}

fn byte(snapshot: &HardLineSnapshot, at: usize) -> Option<u8> {
    snapshot.byte_chunk_at(at).first().copied()
}

fn radix_at(snapshot: &HardLineSnapshot, at: usize, end: usize) -> u32 {
    if at + 2 >= end || byte(snapshot, at) != Some(b'0') { return 10; }
    match (byte(snapshot, at + 1), byte(snapshot, at + 2)) {
        (Some(b'x' | b'X'), Some(digit)) if digit.is_ascii_hexdigit() => 16,
        (Some(b'b' | b'B'), Some(b'0' | b'1')) => 2,
        _ => 10,
    }
}

fn token_at(snapshot: &HardLineSnapshot, line: &Range<usize>, at: usize) -> Option<NumberToken> {
    let current = byte(snapshot, at)?;
    let mut start = at;
    if current.is_ascii_hexdigit() {
        let mut hex_start = at;
        while hex_start > line.start && byte(snapshot, hex_start - 1).is_some_and(|c| c.is_ascii_hexdigit()) {
            hex_start -= 1;
        }
        if hex_start >= line.start + 2 && radix_at(snapshot, hex_start - 2, line.end) == 16 {
            start = hex_start - 2;
        } else if current.is_ascii_digit() {
            while start > line.start && byte(snapshot, start - 1).is_some_and(|c| c.is_ascii_digit()) { start -= 1; }
            if start >= line.start + 2 && radix_at(snapshot, start - 2, line.end) == 2 { start -= 2; }
        } else if matches!(current, b'b' | b'B') && at > line.start
            && radix_at(snapshot, at - 1, line.end) == 2 {
            start -= 1;
        } else { return None; }
    } else if matches!(current, b'x' | b'X') && at > line.start
        && radix_at(snapshot, at - 1, line.end) == 16 {
        start -= 1;
    } else { return None; }
    let radix = radix_at(snapshot, start, line.end);
    let digits_start = start + if radix == 10 { 0 } else { 2 };
    let mut end = digits_start;
    while end < line.end && byte(snapshot, end).is_some_and(|c| (c as char).is_digit(radix)) { end += 1; }
    if end == digits_start { return None; }
    let negative = radix == 10 && start > line.start && byte(snapshot, start - 1) == Some(b'-');
    if negative { start -= 1; }
    Some(NumberToken { range: start..end, digits: digits_start..end, radix, negative })
}

fn find_number(snapshot: &HardLineSnapshot, line: Range<usize>, cursor: usize) -> Option<NumberToken> {
    if cursor >= line.end { return None; }
    if let Some(token) = token_at(snapshot, &line, cursor) { return Some(token); }
    let mut at = cursor;
    while at < line.end {
        let chunk = snapshot.byte_chunk_at(at);
        let available = chunk.len().min(line.end - at);
        if available == 0 { return None; }
        if let Some(relative) = chunk[..available].iter().position(u8::is_ascii_digit) {
            return token_at(snapshot, &line, at + relative);
        }
        at += available;
    }
    None
}

fn replacement(snapshot: &HardLineSnapshot, token: &NumberToken, count: usize, subtract: bool) -> Result<String, DocumentError> {
    let digits = snapshot.slice_utf8(token.digits.clone()).map_err(DocumentError::FormattedTextStorage)?;
    let parsed = u64::from_str_radix(&digits, token.radix);
    let magnitude = parsed.as_ref().copied().unwrap_or(u64::MAX);
    let count = count as u64;
    let mut negative = token.negative;
    let magnitude = if parsed.is_err() { magnitude }
        else if token.radix != 10 {
            if subtract { magnitude.wrapping_sub(count) } else { magnitude.wrapping_add(count) }
        } else {
            let bound = i128::from(u64::MAX);
            let mut value = if negative { -i128::from(magnitude) } else { i128::from(magnitude) };
            value += if subtract { -i128::from(count) } else { i128::from(count) };
            // Vim's decimal domain includes both signs of the full unsigned
            // magnitude. Crossing either end wraps to the opposite sign.
            if value > bound { value -= bound * 2 + 1; }
            if value < -bound { value += bound * 2 + 1; }
            negative = value < 0;
            value.unsigned_abs() as u64
        };
    let prefix = snapshot.slice_utf8(token.range.start..token.digits.start)
        .map_err(DocumentError::FormattedTextStorage)?;
    let width = if token.radix != 10 || digits.starts_with('0') { digits.len() } else { 0 };
    let number = match token.radix {
        2 => format!("{magnitude:0width$b}"),
        16 => {
            let upper = digits.bytes().rev().find(u8::is_ascii_alphabetic)
                .map_or(prefix.ends_with('X'), |letter| letter.is_ascii_uppercase());
            if upper { format!("{magnitude:0width$X}") } else { format!("{magnitude:0width$x}") }
        }
        _ => format!("{magnitude:0width$}"),
    };
    Ok(if token.radix != 10 { format!("{prefix}{number}") }
        else if negative { format!("-{number}") } else { number })
}

fn first_digit(snapshot: &HardLineSnapshot, region: &Range<usize>) -> Option<usize> {
    let mut at = region.start;
    while at < region.end {
        let chunk = snapshot.byte_chunk_at(at);
        let available = chunk.len().min(region.end - at);
        if available == 0 { return None; }
        if let Some(relative) = chunk[..available].iter().position(u8::is_ascii_digit) {
            return Some(at + relative);
        }
        at += available;
    }
    None
}

/// The number's edit, widened to whole graphemes so that a digit is never
/// split from an authored accent.
fn number_edit(snapshot: &HardLineSnapshot, token: &NumberToken, count: usize, subtract: bool) -> Result<TextEdit, DocumentError> {
    let mut range = token.range.clone();
    if !snapshot.is_grapheme_boundary(range.start) {
        range.start = snapshot.previous_grapheme_boundary(range.start).ok_or(DocumentError::AmbiguousProjection)?;
    }
    if !snapshot.is_grapheme_boundary(range.end) {
        range.end = snapshot.next_grapheme_boundary(range.end).ok_or(DocumentError::AmbiguousProjection)?;
    }
    let prefix = snapshot.slice_utf8(range.start..token.range.start).map_err(DocumentError::FormattedTextStorage)?;
    let suffix = snapshot.slice_utf8(token.range.end..range.end).map_err(DocumentError::FormattedTextStorage)?;
    let replacement = format!("{prefix}{}{suffix}", replacement(snapshot, token, count, subtract)?);
    Ok(TextEdit::new(range, replacement))
}

/// Vim's Visual `op_addsub()`: in each region, the first number that starts
/// inside it, read no further than the region's end. A `-` counts only when
/// it is selected too. The progressive `g` form adds [count] more for every
/// changed region.
fn visual_number_edits(
    snapshot: &HardLineSnapshot,
    regions: &[Range<usize>],
    count: usize,
    subtract: bool,
    progressive: bool,
) -> Result<Vec<TextEdit>, DocumentError> {
    let mut amount = count;
    let mut edits = Vec::new();
    for region in regions {
        let Some(token) = first_digit(snapshot, region).and_then(|digit| token_at(snapshot, region, digit)) else { continue };
        edits.push(number_edit(snapshot, &token, amount, subtract)?);
        if progressive {
            amount = amount.saturating_add(count);
        }
    }
    Ok(edits)
}

/// The parts of `range` on each hard line, as Vim's per-line Visual regions.
pub(super) fn line_regions(snapshot: &HardLineSnapshot, range: Range<usize>) -> Vec<Range<usize>> {
    let mut regions = Vec::new();
    let Ok(first) = snapshot.line_at_offset(range.start.min(snapshot.text_length())) else { return regions };
    let mut index = first.index();
    while let Some(line) = snapshot.line(index) {
        let content = line.content_range();
        if index > first.index() && content.start >= range.end { break; }
        let start = content.start.max(range.start);
        let end = content.end.min(range.end);
        if start < end { regions.push(start..end); }
        index += 1;
    }
    regions
}

impl CommandInterpreter {
    /// Apply Visual number changes to `regions` as one transaction and put
    /// the cursor at `start`, where the selection began.
    pub(super) fn change_region_numbers(
        &mut self,
        document: &mut Document,
        regions: &[Range<usize>],
        start: usize,
        count: usize,
        subtract: bool,
        progressive: bool,
    ) -> Result<bool, DocumentError> {
        let snapshot = document.hard_line_snapshot();
        let edits = visual_number_edits(&snapshot, regions, count, subtract, progressive)?;
        let before = document.revision();
        let cursor = if edits.is_empty() { start } else {
            commit_model_with_cursor(document, ModelRequest::ApplyTextEdits {
                document: document.id(), revision: document.revision(), edits,
            }, start, Association::BeforeInsertion)?
        };
        self.cursor = normalize_normal_cursor_document(document, &document.hard_line_snapshot(), cursor);
        Ok(before != document.revision())
    }

    pub(super) fn change_number(&mut self, document: &mut Document, count: usize, subtract: bool) -> Result<CommandOutput, DocumentError> {
        self.requested_register = None;
        let snapshot = document.hard_line_snapshot();
        let line = snapshot.line_at_offset(self.cursor).map_err(|_| DocumentError::AmbiguousProjection)?.content_range();
        let Some(token) = find_number(&snapshot, line, self.cursor) else { return Ok(CommandOutput::complete()); };
        let edit = number_edit(&snapshot, &token, count, subtract)?;
        let start = edit.range.start;
        let before = document.revision();
        let end = commit_model_with_cursor(document, ModelRequest::ApplyTextEdits {
            document: document.id(), revision: document.revision(),
            edits: vec![edit],
        }, start, Association::AfterInsertion)?;
        let cursor = document.hard_line_snapshot().previous_grapheme_boundary(end).unwrap_or(end);
        let moved = self.cursor != cursor;
        self.cursor = cursor;
        let changed = before != document.revision();
        if changed && !self.replaying { self.last_repeat = Some(RepeatAction::NumberChange { count, subtract }); }
        Ok(CommandOutput { document_changed: changed, cursor_moved: moved || changed, ..CommandOutput::complete() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn number_change_at_a_large_line_tail_reads_only_its_numeric_context() {
        let source = format!("{} 009", "z".repeat(1_000_000));
        let mut document = Document::new(source.clone());
        let mut commands = CommandInterpreter::new();
        commands.cursor = source.len() - 2;
        let (result, work) = crate::document::measure_document_work(|| commands.change_number(&mut document, 1, false));
        assert!(result.unwrap().document_changed);
        assert_eq!(commands.cursor, source.len() - 1);
        assert_eq!(document.hard_line_snapshot().slice_utf8(source.len() - 3..source.len()).unwrap(), "010");
        assert_eq!(work.source_full_materializations, 0, "{work:?}");
        assert!(work.source_range_materialized_bytes < 1024, "{work:?}");
        assert_eq!(work.formatted_full_materialized_bytes, 0, "{work:?}");
    }
}
