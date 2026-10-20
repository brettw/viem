//! Messages for Normal commands that report on the buffer without changing it:
//! `ga`, `g8` and `CTRL-G`. Their spelling follows gVim 9.2 built without
//! digraphs, with a buffer-wide 'ruler' because the native status always
//! shows the cursor position.

use std::fmt::Write;

/// `ga`: the decimal, hexadecimal and octal value of each scalar in the
/// grapheme under the cursor, or `NUL` at an empty line.
pub(super) fn character_report(grapheme: &str) -> String {
    if is_line_end(grapheme) {
        return "NUL".to_owned();
    }
    let mut report = String::new();
    let mut scalars = grapheme.chars().peekable();
    if let Some(&first) = scalars.peek().filter(|first| u32::from(**first) < 0x80) {
        scalars.next();
        let value = u32::from(first);
        let shown = match value {
            0..=0x1f => format!("^{}", char::from_u32(value + 0x40).unwrap_or('?')),
            0x7f => "^?".to_owned(),
            _ => first.to_string(),
        };
        let _ = write!(report, "<{shown}>  {value},  Hex {value:02x},  Octal {value:03o}");
    }
    for scalar in scalars {
        let value = u32::from(scalar);
        if !report.is_empty() {
            report.push(' ');
        }
        report.push('<');
        report.push(scalar);
        let _ = if value < 0x10000 {
            write!(report, "> {value}, Hex {value:04x}, Octal {value:o}")
        } else {
            write!(report, "> {value}, Hex {value:08x}, Octal {value:o}")
        };
    }
    report
}

/// `g8`: the UTF-8 bytes of the grapheme under the cursor, with `+` between
/// its scalars, or `NUL` at an empty line.
pub(super) fn utf8_report(grapheme: &str) -> String {
    if is_line_end(grapheme) {
        return "NUL".to_owned();
    }
    let mut report = String::new();
    for (index, scalar) in grapheme.chars().enumerate() {
        if index > 0 {
            report.push_str("+ ");
        }
        let mut encoded = [0; 4];
        for byte in scalar.encode_utf8(&mut encoded).bytes() {
            let _ = write!(report, "{byte:02x} ");
        }
    }
    report
}

/// The buffer facts reported by `CTRL-G`.
pub(super) struct FileReport<'a> {
    pub name: Option<&'a str>,
    pub modified: bool,
    pub read_only: bool,
    /// The cursor's one-based physical line.
    pub line: usize,
    pub line_count: usize,
    pub empty: bool,
}

/// `CTRL-G`: the file name, its state and the cursor's place in the buffer.
pub(super) fn file_report(report: &FileReport<'_>) -> String {
    let mut message = format!("\"{}\"", report.name.unwrap_or("[No Name]"));
    message.push_str(if report.modified { " [Modified]" } else { " " });
    if report.read_only {
        message.push_str("[readonly]");
    }
    if report.modified || report.read_only {
        message.push(' ');
    }
    if report.empty {
        message.push_str("--No lines in buffer--");
        return message;
    }
    let count = report.line_count.max(1);
    let line = report.line.clamp(1, count);
    // Vim divides first beyond a million lines to avoid overflow; usize
    // arithmetic cannot overflow here, but the rounding must match.
    let percent = if line > 1_000_000 { line / (count / 100).max(1) } else { line * 100 / count };
    let noun = if count == 1 { "line" } else { "lines" };
    let _ = write!(message, "{count} {noun} --{percent}%--");
    message
}

fn is_line_end(grapheme: &str) -> bool {
    grapheme.is_empty() || grapheme.chars().all(|scalar| matches!(scalar, '\n' | '\r'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn character_report_matches_vim_spelling() {
        assert_eq!(character_report("a"), "<a>  97,  Hex 61,  Octal 141");
        assert_eq!(character_report("\t"), "<^I>  9,  Hex 09,  Octal 011");
        assert_eq!(character_report("\u{7f}"), "<^?>  127,  Hex 7f,  Octal 177");
        assert_eq!(character_report("\0"), "<^@>  0,  Hex 00,  Octal 000");
        assert_eq!(character_report("é"), "<é> 233, Hex 00e9, Octal 351");
        assert_eq!(character_report("e\u{301}"), "<e>  101,  Hex 65,  Octal 145 <\u{301}> 769, Hex 0301, Octal 1401");
        assert_eq!(character_report("😀"), "<😀> 128512, Hex 0001f600, Octal 373000");
        assert_eq!(character_report(""), "NUL");
        assert_eq!(character_report("\n"), "NUL");
    }

    #[test]
    fn utf8_report_separates_composing_scalars() {
        assert_eq!(utf8_report("a"), "61 ");
        assert_eq!(utf8_report("é"), "c3 a9 ");
        assert_eq!(utf8_report("e\u{301}"), "65 + cc 81 ");
        assert_eq!(utf8_report(""), "NUL");
    }

    #[test]
    fn file_report_matches_vim_ruler_form() {
        let mut report = FileReport { name: Some("a.txt"), modified: false, read_only: false, line: 1, line_count: 3, empty: false };
        assert_eq!(file_report(&report), "\"a.txt\" 3 lines --33%--");
        report.modified = true;
        assert_eq!(file_report(&report), "\"a.txt\" [Modified] 3 lines --33%--");
        report.read_only = true;
        assert_eq!(file_report(&report), "\"a.txt\" [Modified][readonly] 3 lines --33%--");
        report.modified = false;
        assert_eq!(file_report(&report), "\"a.txt\" [readonly] 3 lines --33%--");
        let single = FileReport { name: None, modified: false, read_only: false, line: 1, line_count: 1, empty: false };
        assert_eq!(file_report(&single), "\"[No Name]\" 1 line --100%--");
        let empty = FileReport { empty: true, ..single };
        assert_eq!(file_report(&empty), "\"[No Name]\" --No lines in buffer--");
    }
}
