//! Neovim `lua-match?`: Lua 5.1 patterns as `string.find` evaluates them, an
//! unanchored search over bytes with C-locale character classes. Regular
//! patterns compile to the bounded byte DFA used by upstream `match?`; `%b`,
//! `%f`, and back-references need the budgeted backtracking matcher, a port of
//! Lua's `lstrlib.c` over the validated pattern bytes.
use super::TreeSitterError;
use regex_automata::dfa::dense;
use regex_automata::util::syntax;

/// `LUA_MAXCAPTURES`.
const MAX_CAPTURES: usize = 32;
/// Lua's `MAXCCALLS`; recursion is bounded by pattern structure, not input.
const MAX_DEPTH: usize = 200;
const CAP_UNFINISHED: isize = -1;
const CAP_POSITION: isize = -2;

pub(super) enum LuaPattern {
    Regular(Box<dense::DFA<Vec<u32>>>),
    Backtracking(Backtracking),
}

impl std::fmt::Debug for LuaPattern {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Regular(_) => f.write_str("LuaPattern::Regular"),
            Self::Backtracking(b) => write!(f, "LuaPattern::Backtracking({:?})", b.pattern),
        }
    }
}

impl LuaPattern {
    /// Malformed patterns are rejected with Lua's diagnostic wording.
    pub(super) fn compile(source: &str, size_limit: usize) -> Result<Self, String> {
        let pattern = source.as_bytes();
        let regular = validate(pattern)?;
        if !regular {
            return Ok(Self::Backtracking(Backtracking {
                pattern: pattern.to_vec(),
            }));
        }
        dense::Builder::new()
            .configure(
                dense::Config::new()
                    .dfa_size_limit(Some(size_limit))
                    .determinize_size_limit(Some(size_limit)),
            )
            .syntax(syntax::Config::new().unicode(false).utf8(false))
            .thompson(
                regex_automata::nfa::thompson::Config::new()
                    .utf8(false)
                    .nfa_size_limit(Some(size_limit)),
            )
            .build(&translate(pattern))
            .map(|dfa| Self::Regular(Box::new(dfa)))
            .map_err(|e| format!("bounded Lua pattern: {e}"))
    }
}

/// Returns whether the pattern is regular (no `%b`, `%f`, or back-reference).
fn validate(pattern: &[u8]) -> Result<bool, String> {
    let mut regular = true;
    let mut open = Vec::new();
    let mut closed = Vec::new();
    let mut position = Vec::new();
    let mut i = usize::from(pattern.first() == Some(&b'^'));
    while i < pattern.len() {
        match pattern[i] {
            b'(' => {
                if closed.len() >= MAX_CAPTURES {
                    return Err("too many captures".into());
                }
                if pattern.get(i + 1) == Some(&b')') {
                    closed.push(true);
                    position.push(true);
                    i += 2;
                } else {
                    open.push(closed.len());
                    closed.push(false);
                    position.push(false);
                    i += 1;
                }
            }
            b')' => {
                let level = open.pop().ok_or("invalid pattern capture")?;
                closed[level] = true;
                i += 1;
            }
            b'$' if i + 1 == pattern.len() => i += 1,
            b'%' if pattern.get(i + 1) == Some(&b'b') => {
                if i + 3 >= pattern.len() {
                    return Err("missing arguments to '%b'".into());
                }
                regular = false;
                i += 4;
            }
            b'%' if pattern.get(i + 1) == Some(&b'f') => {
                i += 2;
                if pattern.get(i) != Some(&b'[') {
                    return Err("missing '[' after '%f' in pattern".into());
                }
                i = class_end(pattern, i)?;
                regular = false;
            }
            b'%' if pattern.get(i + 1).is_some_and(u8::is_ascii_digit) => {
                let level = usize::from(pattern[i + 1] - b'0');
                if level == 0 || level > closed.len() || !closed[level - 1] || position[level - 1]
                {
                    return Err(format!("invalid capture index %{level}"));
                }
                regular = false;
                i += 2;
            }
            _ => {
                i = class_end(pattern, i)?;
                if matches!(pattern.get(i), Some(b'?' | b'*' | b'+' | b'-')) {
                    i += 1;
                }
            }
        }
    }
    if !open.is_empty() {
        return Err("unfinished capture".into());
    }
    Ok(regular)
}

/// Lua's `classEnd`: the index after one single-character class.
fn class_end(pattern: &[u8], mut p: usize) -> Result<usize, String> {
    let c = pattern[p];
    p += 1;
    match c {
        b'%' => {
            if p >= pattern.len() {
                return Err("malformed pattern (ends with '%')".into());
            }
            Ok(p + 1)
        }
        b'[' => {
            if pattern.get(p) == Some(&b'^') {
                p += 1;
            }
            loop {
                if p >= pattern.len() {
                    return Err("malformed pattern (missing ']')".into());
                }
                let c = pattern[p];
                p += 1;
                if c == b'%' && p < pattern.len() {
                    p += 1;
                }
                if pattern.get(p) == Some(&b']') {
                    return Ok(p + 1);
                }
            }
        }
        _ => Ok(p),
    }
}

/// C-locale `ctype` classes. A non-class letter after `%` matches itself.
fn match_class(c: u8, class: u8) -> bool {
    let result = match class.to_ascii_lowercase() {
        b'a' => c.is_ascii_alphabetic(),
        b'c' => c.is_ascii_control(),
        b'd' => c.is_ascii_digit(),
        b'g' => c.is_ascii_graphic(),
        b'l' => c.is_ascii_lowercase(),
        b'p' => c.is_ascii_punctuation(),
        b's' => matches!(c, b'\t' | b'\n' | 0x0b | 0x0c | b'\r' | b' '),
        b'u' => c.is_ascii_uppercase(),
        b'w' => c.is_ascii_alphanumeric(),
        b'x' => c.is_ascii_hexdigit(),
        b'z' => c == 0,
        _ => return class == c,
    };
    if class.is_ascii_uppercase() {
        !result
    } else {
        result
    }
}

/// Lua's `matchbracketclass`; `p` is the `[` and `ec` the closing `]`.
fn match_bracket_class(pattern: &[u8], c: u8, mut p: usize, ec: usize) -> bool {
    let mut sig = true;
    if pattern[p + 1] == b'^' {
        sig = false;
        p += 1;
    }
    loop {
        p += 1;
        if p >= ec {
            return !sig;
        }
        if pattern[p] == b'%' {
            p += 1;
            if match_class(c, pattern[p]) {
                return sig;
            }
        } else if pattern[p + 1] == b'-' && p + 2 < ec {
            p += 2;
            if pattern[p - 2] <= c && c <= pattern[p] {
                return sig;
            }
        } else if pattern[p] == c {
            return sig;
        }
    }
}

/// Lua's `singlematch` for the class in `p..ep`.
fn single_match(pattern: &[u8], c: u8, p: usize, ep: usize) -> bool {
    match pattern[p] {
        b'.' => true,
        b'%' => match_class(c, pattern[p + 1]),
        b'[' => match_bracket_class(pattern, c, p, ep - 1),
        literal => literal == c,
    }
}

/// Regular patterns only. Every class becomes an explicit byte class, so no
/// regex metacharacter or Unicode interpretation leaks through.
fn translate(pattern: &[u8]) -> String {
    let mut regex = String::new();
    let mut i = 0;
    if pattern.first() == Some(&b'^') {
        regex.push_str(r"\A");
        i = 1;
    }
    while i < pattern.len() {
        match pattern[i] {
            b'(' if pattern.get(i + 1) == Some(&b')') => i += 2,
            b'(' => {
                regex.push_str("(?:");
                i += 1;
            }
            b')' => {
                regex.push(')');
                i += 1;
            }
            b'$' if i + 1 == pattern.len() => {
                regex.push_str(r"\z");
                i += 1;
            }
            _ => {
                let ep = class_end(pattern, i).expect("validated Lua pattern");
                regex.push('[');
                let mut any = false;
                let mut byte = 0usize;
                while byte < 256 {
                    if !single_match(pattern, byte as u8, i, ep) {
                        byte += 1;
                        continue;
                    }
                    let start = byte;
                    while byte + 1 < 256 && single_match(pattern, (byte + 1) as u8, i, ep) {
                        byte += 1;
                    }
                    regex.push_str(&format!(r"\x{start:02X}-\x{byte:02X}"));
                    any = true;
                    byte += 1;
                }
                if !any {
                    // An empty class matches nothing.
                    regex.push_str(r"a&&b");
                }
                regex.push(']');
                i = ep;
                match pattern.get(i) {
                    Some(b'*' | b'-') => {
                        regex.push('*');
                        i += 1;
                    }
                    Some(b'+') => {
                        regex.push('+');
                        i += 1;
                    }
                    Some(b'?') => {
                        regex.push('?');
                        i += 1;
                    }
                    _ => {}
                }
            }
        }
    }
    regex
}

pub(super) struct Backtracking {
    pattern: Vec<u8>,
}

impl Backtracking {
    /// `string.find(text, pattern) ~= nil`. Each matcher step is charged.
    pub(super) fn find(
        &self,
        text: &[u8],
        charge: &mut dyn FnMut(usize) -> Result<(), TreeSitterError>,
    ) -> Result<bool, TreeSitterError> {
        let anchor = self.pattern.first() == Some(&b'^');
        let mut state = State {
            src: text,
            pattern: &self.pattern,
            level: 0,
            capture: [(0, CAP_UNFINISHED); MAX_CAPTURES],
            depth: 0,
            charge,
        };
        for start in 0..=text.len() {
            state.level = 0;
            if state.do_match(start, usize::from(anchor))?.is_some() {
                return Ok(true);
            }
            if anchor {
                break;
            }
        }
        Ok(false)
    }
}

struct State<'a, 'c> {
    src: &'a [u8],
    pattern: &'a [u8],
    level: usize,
    capture: [(usize, isize); MAX_CAPTURES],
    depth: usize,
    charge: &'c mut dyn FnMut(usize) -> Result<(), TreeSitterError>,
}

type Step = Result<Option<usize>, TreeSitterError>;

impl State<'_, '_> {
    fn do_match(&mut self, s: usize, p: usize) -> Step {
        if self.depth >= MAX_DEPTH {
            return Err(TreeSitterError::Limit("Lua pattern too complex"));
        }
        self.depth += 1;
        let result = self.match_at(s, p);
        self.depth -= 1;
        result
    }

    fn match_at(&mut self, mut s: usize, mut p: usize) -> Step {
        let (src, pattern) = (self.src, self.pattern);
        loop {
            (self.charge)(1)?;
            if p == pattern.len() {
                return Ok(Some(s));
            }
            match pattern[p] {
                b'(' if pattern.get(p + 1) == Some(&b')') => {
                    return self.start_capture(s, p + 2, CAP_POSITION)
                }
                b'(' => return self.start_capture(s, p + 1, CAP_UNFINISHED),
                b')' => return self.end_capture(s, p + 1),
                b'$' if p + 1 == pattern.len() => {
                    return Ok((s == src.len()).then_some(s));
                }
                b'%' if pattern[p + 1] == b'b' => match self.match_balance(s, p + 2)? {
                    Some(next) => {
                        s = next;
                        p += 4;
                        continue;
                    }
                    None => return Ok(None),
                },
                b'%' if pattern[p + 1] == b'f' => {
                    p += 2;
                    let ep = class_end(pattern, p).expect("validated Lua pattern");
                    let previous = if s == 0 { 0 } else { src[s - 1] };
                    let current = src.get(s).copied().unwrap_or(0);
                    if match_bracket_class(pattern, previous, p, ep - 1)
                        || !match_bracket_class(pattern, current, p, ep - 1)
                    {
                        return Ok(None);
                    }
                    p = ep;
                    continue;
                }
                b'%' if pattern[p + 1].is_ascii_digit() => {
                    match self.match_capture(s, usize::from(pattern[p + 1] - b'1'))? {
                        Some(next) => {
                            s = next;
                            p += 2;
                            continue;
                        }
                        None => return Ok(None),
                    }
                }
                _ => {}
            }
            let ep = class_end(pattern, p).expect("validated Lua pattern");
            let matched = s < src.len() && single_match(pattern, src[s], p, ep);
            match pattern.get(ep) {
                Some(b'?') => {
                    if matched {
                        if let Some(end) = self.do_match(s + 1, ep + 1)? {
                            return Ok(Some(end));
                        }
                    }
                    p = ep + 1;
                }
                Some(b'*') => return self.max_expand(s, p, ep),
                Some(b'+') => {
                    return if matched {
                        self.max_expand(s + 1, p, ep)
                    } else {
                        Ok(None)
                    }
                }
                Some(b'-') => return self.min_expand(s, p, ep),
                _ => {
                    if !matched {
                        return Ok(None);
                    }
                    s += 1;
                    p = ep;
                }
            }
        }
    }

    fn max_expand(&mut self, s: usize, p: usize, ep: usize) -> Step {
        let mut count = 0;
        while s + count < self.src.len() && single_match(self.pattern, self.src[s + count], p, ep)
        {
            (self.charge)(1)?;
            count += 1;
        }
        loop {
            if let Some(end) = self.do_match(s + count, ep + 1)? {
                return Ok(Some(end));
            }
            if count == 0 {
                return Ok(None);
            }
            count -= 1;
        }
    }

    fn min_expand(&mut self, mut s: usize, p: usize, ep: usize) -> Step {
        loop {
            if let Some(end) = self.do_match(s, ep + 1)? {
                return Ok(Some(end));
            }
            if s < self.src.len() && single_match(self.pattern, self.src[s], p, ep) {
                s += 1;
            } else {
                return Ok(None);
            }
        }
    }

    fn start_capture(&mut self, s: usize, p: usize, what: isize) -> Step {
        self.capture[self.level] = (s, what);
        self.level += 1;
        let result = self.do_match(s, p)?;
        if result.is_none() {
            self.level -= 1;
        }
        Ok(result)
    }

    fn end_capture(&mut self, s: usize, p: usize) -> Step {
        let level = (0..self.level)
            .rev()
            .find(|&l| self.capture[l].1 == CAP_UNFINISHED)
            .expect("validated Lua pattern");
        self.capture[level].1 = (s - self.capture[level].0) as isize;
        let result = self.do_match(s, p)?;
        if result.is_none() {
            self.capture[level].1 = CAP_UNFINISHED;
        }
        Ok(result)
    }

    fn match_balance(&mut self, s: usize, p: usize) -> Step {
        let (open, close) = (self.pattern[p], self.pattern[p + 1]);
        if self.src.get(s) != Some(&open) {
            return Ok(None);
        }
        let mut depth = 1usize;
        for at in s + 1..self.src.len() {
            (self.charge)(1)?;
            let c = self.src[at];
            if c == close {
                depth -= 1;
                if depth == 0 {
                    return Ok(Some(at + 1));
                }
            } else if c == open {
                depth += 1;
            }
        }
        Ok(None)
    }

    fn match_capture(&mut self, s: usize, level: usize) -> Step {
        let (start, len) = self.capture[level];
        let len = len as usize;
        (self.charge)(len.max(1))?;
        Ok((self.src.len() - s >= len && self.src[start..start + len] == self.src[s..s + len])
            .then_some(s + len))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Both engines, so the DFA translation is checked against the port of
    /// Lua's own matcher on every regular pattern.
    fn find(pattern: &str, text: &str) -> bool {
        let backtracking = Backtracking {
            pattern: pattern.as_bytes().to_vec(),
        };
        let mut charge = |_| Ok(());
        let expected = backtracking.find(text.as_bytes(), &mut charge).unwrap();
        if let LuaPattern::Regular(dfa) = LuaPattern::compile(pattern, 1 << 20).unwrap() {
            use regex_automata::dfa::Automaton;
            let found = dfa
                .try_search_fwd(&regex_automata::Input::new(text.as_bytes()))
                .unwrap()
                .is_some();
            assert_eq!(found, expected, "DFA disagrees for {pattern:?} on {text:?}");
        }
        expected
    }

    #[test]
    fn nvim_c_and_cpp_patterns() {
        assert!(find("^[A-Z][A-Z0-9_]+$", "MAX_SIZE"));
        assert!(!find("^[A-Z][A-Z0-9_]+$", "Max"));
        assert!(!find("^[A-Z][A-Z0-9_]+$", "A"));
        assert!(find("^/[*][*][^*].*[*]/$", "/** doc */"));
        assert!(!find("^/[*][*][^*].*[*]/$", "/*** rule */"));
        assert!(find("^__builtin_", "__builtin_expect"));
        assert!(find("^m_.*$", "m_count"));
        assert!(find("^[%u]", "Foo"));
        assert!(!find("^%u", "foo"));
        assert!(find("/[*/][!*/]<?[^a-zA-Z]", "/// brief"));
        assert!(!find("/[*/][!*/]<?[^a-zA-Z]", "// plain"));
    }

    #[test]
    fn classes_sets_and_quantifiers() {
        assert!(find("%d+", "abc123"));
        assert!(!find("^%d+$", "12a"));
        assert!(find("^%a%w*$", "x9"));
        assert!(find("%s", "a\tb"));
        assert!(find("^%S+$", "ab"));
        assert!(find("^[%]]$", "]"));
        assert!(find("^[]]$", "]"));
        assert!(find("^[^]]$", "a"));
        assert!(find("^[a-c-]+$", "b-a"));
        assert!(find("^a-b$", "aaab"));
        assert!(find("^ab?c$", "ac"));
        assert!(find("^%.$", "."));
        assert!(!find("^%.$", "x"));
        assert!(find("a$b", "a$b"), "a non-final $ is literal");
        assert!(find("x^", "x^"), "a non-initial ^ is literal");
        assert!(find("^*$", "*"), "a leading quantifier is literal");
        assert!(find("^(a(b))()$", "ab"));
        // Lua patterns are byte-oriented: . consumes one byte of é.
        assert!(find("^..$", "é"));
        assert!(!find("^.$", "é"));
        assert!(!find("^%a$", "é"));
    }

    #[test]
    fn non_regular_features() {
        assert!(find("^%b()$", "(a(b)c)"));
        assert!(!find("^%b()$", "(a(b)c"));
        assert!(find("%f[%w]word", "a word"));
        assert!(!find("%f[%w]word", "aword"));
        assert!(find("^(%a+)=%1$", "abc=abc"));
        assert!(!find("^(%a+)=%1$", "abc=abd"));
    }

    #[test]
    fn malformed_patterns_are_rejected() {
        for pattern in ["[a", "a%", "(a", "a)", "%b(", "%fa", "%1", "(a%1)", "()%1"] {
            assert!(LuaPattern::compile(pattern, 1 << 20).is_err(), "{pattern:?}");
        }
    }

    #[test]
    fn backtracking_is_budgeted() {
        let pattern = Backtracking {
            pattern: b"a*a*a*b".to_vec(),
        };
        let mut steps = 0;
        let mut charge = |n| {
            steps += n;
            if steps > 10_000 {
                Err(TreeSitterError::Limit("predicate steps"))
            } else {
                Ok(())
            }
        };
        let text = "a".repeat(5_000);
        assert!(pattern.find(text.as_bytes(), &mut charge).is_err());
    }
}
