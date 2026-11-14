//! Lexical boundaries of C# interpolated strings. The complete literal,
//! including its expressions, is protected from comment formatting.
use super::MAX_NESTING;

#[derive(Clone, Debug, Eq, PartialEq)]
enum Frame {
    String { quote: u8, verbatim: bool, interpolated: bool },
    Expression { braces: usize, parens: usize, brackets: usize },
    Format { verbatim: bool },
    Comment,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct Interpolation(Vec<Frame>);

impl Interpolation {
    pub(super) fn start(text: &str) -> Option<(Self, usize)> {
        let (verbatim, width) = if text.starts_with("$@\"") || text.starts_with("@$\"") {
            (true, 3)
        } else if text.starts_with("$\"") && !text.starts_with("$\"\"\"") {
            (false, 2)
        } else { return None };
        Some((Self(vec![Frame::String { quote: b'"', verbatim, interpolated: true }]), width))
    }

    /// Consume a hard-line fragment, retaining a bounded stack across lines.
    /// Return the end of the literal, or None when it continues on another line.
    pub(super) fn scan(&mut self, text: &str, mut at: usize) -> Result<Option<usize>, ()> {
        let bytes = text.as_bytes();
        while at < bytes.len() {
            if self.0.len() >= MAX_NESTING { return Err(()) }
            let rest = &text[at..];
            match self.0.last_mut().ok_or(())? {
                Frame::Comment => {
                    if rest.starts_with("*/") { self.0.pop(); at += 2; }
                    else { at += rest.chars().next().unwrap().len_utf8(); }
                }
                Frame::String { quote, verbatim, interpolated } => {
                    if !*verbatim && bytes[at] == b'\\' {
                        at += 1;
                        at += text.get(at..).and_then(|tail| tail.chars().next()).ok_or(())?.len_utf8();
                    } else if bytes[at] == *quote {
                        if *verbatim && bytes.get(at + 1) == Some(quote) { at += 2; }
                        else { self.0.pop(); at += 1; }
                    } else if *interpolated && bytes[at] == b'{' {
                        if bytes.get(at + 1) == Some(&b'{') { at += 2; }
                        else {
                            self.0.push(Frame::Expression { braces: 0, parens: 0, brackets: 0 });
                            at += 1;
                        }
                    } else if *interpolated && bytes[at] == b'}' {
                        if bytes.get(at + 1) != Some(&b'}') { return Err(()) }
                        at += 2;
                    } else { at += rest.chars().next().unwrap().len_utf8(); }
                }
                Frame::Format { verbatim } => {
                    if bytes[at] == b'}' { self.0.pop(); at += 1; }
                    else if !*verbatim && bytes[at] == b'\\' {
                        at += 1;
                        at += text.get(at..).and_then(|tail| tail.chars().next()).ok_or(())?.len_utf8();
                    } else if *verbatim && rest.starts_with("\"\"") { at += 2; }
                    else if bytes[at] == b'"' { return Err(()) }
                    else { at += rest.chars().next().unwrap().len_utf8(); }
                }
                Frame::Expression { braces, parens, brackets } => {
                    if rest.starts_with("//") { return Ok(None) }
                    if rest.starts_with("/*") { self.0.push(Frame::Comment); at += 2; continue }
                    if let Some((nested, width)) = Self::start(rest) {
                        self.0.extend(nested.0); at += width; continue
                    }
                    // Raw strings have distinct delimiter rules. Never guess
                    // their endpoint or expose their contents as comments.
                    if rest.starts_with("\"\"\"") || rest.starts_with('$') && rest.trim_start_matches('$').starts_with("\"\"\"") {
                        return Err(())
                    }
                    if rest.starts_with("@\"") {
                        self.0.push(Frame::String { quote: b'"', verbatim: true, interpolated: false });
                        at += 2; continue
                    }
                    if matches!(bytes[at], b'"' | b'\'') {
                        self.0.push(Frame::String { quote: bytes[at], verbatim: false, interpolated: false });
                        at += 1; continue
                    }
                    match bytes[at] {
                        b'{' => { *braces += 1; if *braces >= MAX_NESTING { return Err(()) } }
                        b'}' if *braces == 0 => { self.0.pop(); }
                        b'}' => *braces -= 1,
                        b'(' => *parens += 1,
                        b')' => *parens = parens.checked_sub(1).ok_or(())?,
                        b'[' => *brackets += 1,
                        b']' => *brackets = brackets.checked_sub(1).ok_or(())?,
                        b':' if *braces == 0 && *parens == 0 && *brackets == 0 => {
                            if rest.starts_with("::") { at += 2; continue }
                            let verbatim = matches!(self.0.get(self.0.len() - 2), Some(Frame::String { verbatim: true, .. }));
                            *self.0.last_mut().unwrap() = Frame::Format { verbatim };
                        }
                        _ => {}
                    }
                    at += rest.chars().next().unwrap().len_utf8();
                }
            }
            if self.0.is_empty() { return Ok(Some(at)) }
        }
        if matches!(self.0.last(), Some(Frame::String { verbatim: false, .. } | Frame::Format { verbatim: false })) {
            return Err(())
        }
        Ok(None)
    }
}
