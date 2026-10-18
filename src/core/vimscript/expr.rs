//! Legacy expression evaluation. Like Vim, parsing and evaluation happen in
//! one pass over the command text; `evaluate == false` parses without side
//! effects to find where an expression ends (for skipped branches and `|`).
use super::error::{EvalResult, Fail, VimError};
use super::exec::{Runtime, ScriptHost};
use super::value::{
    compare_values, str2nr, Blob, CaseMode, CompareOp, Dict, Funcref, VStr, Value,
};

pub const MAX_NESTING: usize = 1000;

pub struct Eval<'r, 'h, 's> {
    pub rt: &'r mut Runtime,
    pub host: &'h mut dyn ScriptHost,
    pub src: &'s [u8],
    pub pos: usize,
    pub evaluate: bool,
    depth: usize,
}

pub fn is_name_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_'
}
pub fn is_name_char(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}
pub fn is_white(byte: u8) -> bool {
    byte == b' ' || byte == b'\t'
}
pub fn skip_white(src: &[u8], mut at: usize) -> usize {
    while at < src.len() && is_white(src[at]) {
        at += 1;
    }
    at
}

/// The result of parsing a callee expression without calling it, used by
/// `:call` to run user functions as resumable frames.
pub struct Callee {
    pub funcref: Funcref,
    pub args: Vec<Value>,
}

impl<'r, 'h, 's> Eval<'r, 'h, 's> {
    pub fn new(rt: &'r mut Runtime, host: &'h mut dyn ScriptHost, src: &'s [u8], pos: usize, evaluate: bool) -> Self {
        Self { rt, host, src, pos, evaluate, depth: 0 }
    }

    pub fn peek(&self) -> u8 {
        self.src.get(self.pos).copied().unwrap_or(0)
    }
    fn peek_at(&self, offset: usize) -> u8 {
        self.src.get(self.pos + offset).copied().unwrap_or(0)
    }
    pub fn at_end(&self) -> bool {
        self.pos >= self.src.len()
    }
    pub fn skip_white(&mut self) {
        self.pos = skip_white(self.src, self.pos);
    }
    fn rest(&self) -> &'s [u8] {
        &self.src[self.pos.min(self.src.len())..]
    }
    fn starts_with(&self, text: &[u8]) -> bool {
        self.rest().starts_with(text)
    }

    fn invalid(&self) -> Fail {
        Fail::Error(VimError::invalid_expression(self.rest()))
    }

    fn enter(&mut self) -> EvalResult<()> {
        self.depth += 1;
        if self.depth > MAX_NESTING {
            return Err(Fail::Error(VimError::new(1169, "Expression too recursive")));
        }
        self.rt.charge(1)?;
        Ok(())
    }

    /// Parse and evaluate one `expr1`, leaving `pos` after it (and after
    /// trailing white space).
    pub fn eval1(&mut self) -> EvalResult<Value> {
        self.enter()?;
        let result = self.eval1_inner();
        self.depth -= 1;
        result
    }

    fn eval1_inner(&mut self) -> EvalResult<Value> {
        let condition = self.eval2()?;
        self.skip_white();
        if self.peek() == b'?' && self.peek_at(1) == b'?' {
            // Falsy operator.
            self.pos += 2;
            self.skip_white();
            let truthy = self.evaluate && !condition.is_empty_value();
            let saved = self.evaluate;
            if truthy {
                self.evaluate = false;
            }
            let other = self.eval1()?;
            self.evaluate = saved;
            return Ok(if !self.evaluate || truthy { condition } else { other });
        }
        if self.peek() != b'?' {
            return Ok(condition);
        }
        self.pos += 1;
        self.skip_white();
        let truth = if self.evaluate { condition.to_bool()? } else { false };
        let saved = self.evaluate;
        self.evaluate = saved && truth;
        let then_value = self.eval1()?;
        self.evaluate = saved;
        self.skip_white();
        if self.peek() != b':' {
            return Err(Fail::Error(VimError::new(109, "Missing ':' after '?'")));
        }
        self.pos += 1;
        self.skip_white();
        self.evaluate = saved && !truth;
        let else_value = self.eval1()?;
        self.evaluate = saved;
        Ok(if !saved { Value::Number(0) } else if truth { then_value } else { else_value })
    }

    fn eval2(&mut self) -> EvalResult<Value> {
        let mut left = self.eval3()?;
        self.skip_white();
        if !(self.peek() == b'|' && self.peek_at(1) == b'|') {
            return Ok(left);
        }
        let saved = self.evaluate;
        let mut result = if saved { left.to_bool()? } else { false };
        while self.peek() == b'|' && self.peek_at(1) == b'|' {
            self.pos += 2;
            self.skip_white();
            self.evaluate = saved && !result;
            let right = self.eval3()?;
            if self.evaluate {
                result = right.to_bool()?;
            }
            self.skip_white();
        }
        self.evaluate = saved;
        left = Value::Number(result as i64);
        Ok(left)
    }

    fn eval3(&mut self) -> EvalResult<Value> {
        let left = self.eval4()?;
        self.skip_white();
        if !(self.peek() == b'&' && self.peek_at(1) == b'&') {
            return Ok(left);
        }
        let saved = self.evaluate;
        let mut result = if saved { left.to_bool()? } else { true };
        while self.peek() == b'&' && self.peek_at(1) == b'&' {
            self.pos += 2;
            self.skip_white();
            self.evaluate = saved && result;
            let right = self.eval4()?;
            if self.evaluate {
                result = right.to_bool()?;
            }
            self.skip_white();
        }
        self.evaluate = saved;
        Ok(Value::Number(result as i64))
    }

    /// Recognize a comparison operator at `pos`, returning it and its length.
    fn comparison_operator(&self) -> Option<(CompareOp, usize)> {
        let rest = self.rest();
        let (op, mut len) = match rest {
            [b'=', b'=', ..] => (CompareOp::Equal, 2),
            [b'=', b'~', ..] => (CompareOp::Match, 2),
            [b'!', b'=', ..] => (CompareOp::NotEqual, 2),
            [b'!', b'~', ..] => (CompareOp::NoMatch, 2),
            [b'>', b'=', ..] => (CompareOp::GreaterEqual, 2),
            [b'<', b'=', ..] => (CompareOp::LessEqual, 2),
            [b'>', b'>', ..] | [b'<', b'<', ..] => return None,
            [b'>', ..] => (CompareOp::Greater, 1),
            [b'<', ..] => (CompareOp::Less, 1),
            [b'i', b's', b'n', b'o', b't', ..] if !rest.get(5).is_some_and(|b| is_name_char(*b)) => (CompareOp::IsNot, 5),
            [b'i', b's', ..] if !rest.get(2).is_some_and(|b| is_name_char(*b)) => (CompareOp::Is, 2),
            [b'i', b's', b'n', b'o', b't', b'#' | b'?', ..] => (CompareOp::IsNot, 5),
            [b'i', b's', b'#' | b'?', ..] => (CompareOp::Is, 2),
            _ => return None,
        };
        if matches!(rest.get(len), Some(b'#' | b'?')) {
            len += 1;
        }
        Some((op, len))
    }

    fn eval4(&mut self) -> EvalResult<Value> {
        let left = self.eval5()?;
        self.skip_white();
        let Some((op, len)) = self.comparison_operator() else {
            return Ok(left);
        };
        let case = match self.src[self.pos + len - 1] {
            b'#' => CaseMode::Match,
            b'?' => CaseMode::Ignore,
            _ => CaseMode::Option,
        };
        self.pos += len;
        self.skip_white();
        let right = self.eval5()?;
        if !self.evaluate {
            return Ok(Value::Number(0));
        }
        let ignore_case = match case {
            CaseMode::Match => false,
            CaseMode::Ignore => true,
            CaseMode::Option => self.host.ignorecase(),
        };
        let fuel = &mut self.rt.fuel;
        let result = compare_values(&left, &right, op, ignore_case, &mut |text, pattern, ic| {
            super::regex::string_matches(text, pattern, ic, fuel)
        })?;
        Ok(Value::Number(result as i64))
    }

    fn eval5(&mut self) -> EvalResult<Value> {
        let mut left = self.eval6(false)?;
        loop {
            self.skip_white();
            let shift_left = self.starts_with(b"<<");
            let shift_right = self.starts_with(b">>");
            if !(shift_left || shift_right) {
                return Ok(left);
            }
            self.pos += 2;
            self.skip_white();
            let right = self.eval6(false)?;
            if self.evaluate {
                let (value, amount) = (left.to_number()?, right.to_number()?);
                if amount < 0 {
                    return Err(Fail::Error(VimError::new(1283, "Bitshift amount must be a positive number")));
                }
                left = Value::Number(if amount >= 64 {
                    0
                } else if shift_left {
                    ((value as u64) << amount) as i64
                } else {
                    ((value as u64) >> amount) as i64
                });
            }
        }
    }

    fn eval6(&mut self, _want_string: bool) -> EvalResult<Value> {
        let mut left = self.eval7(false)?;
        loop {
            self.skip_white();
            let op = match self.peek() {
                b'+' => "+",
                b'-' => "-",
                b'.' if self.peek_at(1) == b'.' => "..",
                b'.' => ".",
                _ => return Ok(left),
            };
            // "a.b" with a Dictionary was handled as a member in expr10.
            self.pos += op.len();
            if op == ".." && self.peek() == b'=' || op == "." && self.peek() == b'=' {
                // Assignment operator; not part of an expression.
                self.pos -= op.len();
                return Ok(left);
            }
            self.skip_white();
            let concat = op == "." || op == "..";
            let right = self.eval7(concat)?;
            if !self.evaluate {
                continue;
            }
            left = match op {
                "." | ".." => {
                    let mut bytes = left.to_bytes()?;
                    bytes.extend_from_slice(&right.to_bytes()?);
                    Value::String(bytes.into())
                }
                "+" => match (&left, &right) {
                    (Value::List(a), Value::List(b)) => {
                        let mut items = a.snapshot();
                        items.extend(b.snapshot());
                        Value::list(items)
                    }
                    (Value::Blob(a), Value::Blob(b)) => {
                        let mut bytes = a.bytes();
                        bytes.extend(b.bytes());
                        Value::Blob(Blob::new(bytes))
                    }
                    _ => arithmetic(&left, &right, b'+')?,
                },
                _ => arithmetic(&left, &right, b'-')?,
            };
        }
    }

    fn eval7(&mut self, want_string: bool) -> EvalResult<Value> {
        let mut left = self.eval9(want_string)?;
        loop {
            self.skip_white();
            let op = self.peek();
            if !matches!(op, b'*' | b'/' | b'%') {
                return Ok(left);
            }
            self.pos += 1;
            self.skip_white();
            let right = self.eval9(false)?;
            if self.evaluate {
                left = arithmetic(&left, &right, op)?;
            }
        }
    }

    fn eval9(&mut self, want_string: bool) -> EvalResult<Value> {
        self.skip_white();
        let start = self.pos;
        while matches!(self.peek(), b'!' | b'-' | b'+') {
            self.pos += 1;
            self.skip_white();
        }
        let unary = self.src[start..self.pos].iter().copied().filter(|b| !is_white(*b)).collect::<Vec<u8>>();
        let mut value = self.eval10(want_string)?;
        if !self.evaluate {
            return Ok(Value::Number(0));
        }
        for op in unary.iter().rev() {
            value = match op {
                b'!' => Value::Number(!value.to_bool()? as i64),
                b'-' => match value {
                    Value::Float(f) => Value::Float(-f),
                    other => Value::Number(other.to_number()?.wrapping_neg()),
                },
                _ => match value {
                    Value::Float(f) => Value::Float(f),
                    other => Value::Number(other.to_number()?),
                },
            };
        }
        Ok(value)
    }

    fn eval10(&mut self, want_string: bool) -> EvalResult<Value> {
        self.enter()?;
        let result = self.eval10_inner(want_string);
        self.depth -= 1;
        result
    }

    fn eval10_inner(&mut self, want_string: bool) -> EvalResult<Value> {
        let byte = self.peek();
        let value = match byte {
            b'0'..=b'9' => self.number(want_string)?,
            b'"' => self.double_quoted()?,
            b'\'' => self.single_quoted()?,
            b'$' if matches!(self.peek_at(1), b'"' | b'\'') => self.interpolated()?,
            b'$' => self.environment()?,
            b'[' => self.list_literal()?,
            b'#' if self.peek_at(1) == b'{' => {
                self.pos += 1;
                self.dict_literal(true)?
            }
            b'{' => {
                if let Some(lambda) = self.lambda()? {
                    lambda
                } else if self.is_curly_name()? {
                    return self.name_or_call();
                } else {
                    self.dict_literal(false)?
                }
            }
            b'&' => self.option_value()?,
            b'@' => self.register_value()?,
            b'(' => {
                self.pos += 1;
                self.skip_white();
                let value = self.eval1()?;
                self.skip_white();
                if self.peek() != b')' {
                    return Err(Fail::Error(VimError::new(110, "Missing ')'")));
                }
                self.pos += 1;
                value
            }
            _ => return self.name_or_call(),
        };
        self.subscripts(value, None)
    }

    // ----- literals --------------------------------------------------

    fn number(&mut self, want_string: bool) -> EvalResult<Value> {
        let rest = self.rest();
        // Blob literal.
        if rest.len() > 2 && rest[0] == b'0' && matches!(rest[1], b'z' | b'Z') {
            let mut at = 2;
            let mut bytes = Vec::new();
            loop {
                if at < rest.len() && rest[at] == b'.' && at + 1 < rest.len() && rest[at + 1].is_ascii_hexdigit() {
                    at += 1;
                    continue;
                }
                if at + 1 < rest.len() && rest[at].is_ascii_hexdigit() && rest[at + 1].is_ascii_hexdigit() {
                    let text = std::str::from_utf8(&rest[at..at + 2]).unwrap();
                    bytes.push(u8::from_str_radix(text, 16).unwrap());
                    at += 2;
                } else if at < rest.len() && rest[at].is_ascii_hexdigit() {
                    return Err(Fail::Error(VimError::new(973, "Blob literal should have an even number of hex characters")));
                } else {
                    break;
                }
            }
            self.pos += at;
            return Ok(Value::Blob(Blob::new(bytes)));
        }
        // Float: digits '.' digits [e[+-]digits], unless the right side of
        // a concatenation wants a String.
        if !want_string {
            let mut at = 0;
            while at < rest.len() && rest[at].is_ascii_digit() {
                at += 1;
            }
            if at < rest.len() && rest[at] == b'.' && rest.get(at + 1).is_some_and(|b| b.is_ascii_digit()) {
                let is_hex = rest.len() > 1 && rest[0] == b'0' && matches!(rest[1], b'x' | b'X' | b'b' | b'B');
                if !is_hex {
                    at += 1;
                    while at < rest.len() && rest[at].is_ascii_digit() {
                        at += 1;
                    }
                    if at < rest.len() && matches!(rest[at], b'e' | b'E') {
                        let mut exponent = at + 1;
                        if exponent < rest.len() && matches!(rest[exponent], b'+' | b'-') {
                            exponent += 1;
                        }
                        if exponent < rest.len() && rest[exponent].is_ascii_digit() {
                            while exponent < rest.len() && rest[exponent].is_ascii_digit() {
                                exponent += 1;
                            }
                            at = exponent;
                        }
                    }
                    let text = std::str::from_utf8(&rest[..at]).unwrap();
                    self.pos += at;
                    return Ok(Value::Float(text.parse().unwrap_or(0.0)));
                }
            }
        }
        let (value, used) = str2nr(rest, true);
        if used == 0 {
            return Err(self.invalid());
        }
        self.pos += used;
        if self.peek().is_ascii_alphanumeric() && !matches!(self.peek(), b'e' | b'E') {
            // e.g. "0x" followed by junk is fine in Vim; letters after a
            // decimal number are not.
        }
        Ok(Value::Number(value))
    }

    fn double_quoted(&mut self) -> EvalResult<Value> {
        let start = self.pos;
        self.pos += 1;
        let mut out = Vec::new();
        loop {
            let Some(&byte) = self.src.get(self.pos) else {
                return Err(Fail::Error(VimError::new(114, format!("Missing double quote: {}", String::from_utf8_lossy(&self.src[start..])))));
            };
            self.pos += 1;
            match byte {
                b'"' => break,
                b'\\' => {
                    let Some(&escape) = self.src.get(self.pos) else {
                        return Err(Fail::Error(VimError::new(114, format!("Missing double quote: {}", String::from_utf8_lossy(&self.src[start..])))));
                    };
                    self.pos += 1;
                    self.string_escape(escape, &mut out);
                }
                _ => out.push(byte),
            }
        }
        if !self.evaluate {
            return Ok(Value::Number(0));
        }
        // "\000" ends the string, like a NUL in Vim.
        if let Some(nul) = out.iter().position(|b| *b == 0) {
            out.truncate(nul);
        }
        Ok(Value::String(out.into()))
    }

    fn string_escape(&mut self, escape: u8, out: &mut Vec<u8>) {
        match escape {
            b'n' => out.push(b'\n'),
            b'r' => out.push(b'\r'),
            b't' => out.push(b'\t'),
            b'e' => out.push(0x1b),
            b'b' => out.push(0x08),
            b'f' => out.push(0x0c),
            b'x' | b'X' | b'u' | b'U' => {
                let max = match escape {
                    b'x' | b'X' => 2,
                    b'u' => 4,
                    _ => 8,
                };
                let digits: Vec<u8> = self.rest().iter().take(max).take_while(|b| b.is_ascii_hexdigit()).copied().collect();
                if digits.is_empty() {
                    out.push(escape);
                    return;
                }
                self.pos += digits.len();
                let value = u32::from_str_radix(std::str::from_utf8(&digits).unwrap(), 16).unwrap_or(0);
                if matches!(escape, b'x' | b'X') {
                    out.push(value as u8);
                } else if let Some(c) = char::from_u32(value) {
                    let mut buffer = [0; 4];
                    out.extend_from_slice(c.encode_utf8(&mut buffer).as_bytes());
                }
            }
            b'0'..=b'7' => {
                let mut value = (escape - b'0') as u32;
                for _ in 0..2 {
                    match self.peek() {
                        digit @ b'0'..=b'7' => {
                            value = value * 8 + (digit - b'0') as u32;
                            self.pos += 1;
                        }
                        _ => break,
                    }
                }
                out.push(value as u8);
            }
            b'<' => {
                // Special key notation: "\<C-W>", "\<CR>", "\<*C-w>".
                let rest = self.rest();
                let mut end = None;
                let mut at = 0;
                while at < rest.len() && at < 64 {
                    if rest[at] == b'>' && at > 0 {
                        end = Some(at);
                        break;
                    }
                    if rest[at] == b'\\' && rest.get(at + 1) == Some(&b'"') {
                        at += 2;
                        continue;
                    }
                    if rest[at] == b'"' || is_white(rest[at]) && !rest[..at].ends_with(b"-") {
                        break;
                    }
                    at += 1;
                }
                if let Some(end) = end {
                    let name = String::from_utf8_lossy(&rest[..end]).replace("\\\"", "\"");
                    if let Some(bytes) = super::keys::notation_bytes(&name) {
                        out.extend_from_slice(&bytes);
                        self.pos += end + 1;
                        return;
                    }
                }
                out.push(b'<');
            }
            other => out.push(other),
        }
    }

    fn single_quoted(&mut self) -> EvalResult<Value> {
        let start = self.pos;
        self.pos += 1;
        let mut out = Vec::new();
        loop {
            let Some(&byte) = self.src.get(self.pos) else {
                return Err(Fail::Error(VimError::new(115, format!("Missing single quote: {}", String::from_utf8_lossy(&self.src[start..])))));
            };
            self.pos += 1;
            if byte == b'\'' {
                if self.peek() == b'\'' {
                    self.pos += 1;
                    out.push(b'\'');
                    continue;
                }
                break;
            }
            out.push(byte);
        }
        Ok(if self.evaluate { Value::String(out.into()) } else { Value::Number(0) })
    }

    /// Heredoc interpolation has no surrounding quote delimiter, and quotes
    /// inside expressions retain their ordinary expression syntax.
    pub(crate) fn interpolate_heredoc(&mut self) -> EvalResult<Value> {
        let mut out = Vec::new();
        while self.pos < self.src.len() {
            let byte = self.src[self.pos];
            self.pos += 1;
            match byte {
                b'{' if self.peek() == b'{' => { self.pos += 1; out.push(b'{'); }
                b'}' if self.peek() == b'}' => { self.pos += 1; out.push(b'}'); }
                b'}' => return Err(Fail::Error(VimError::new(1278, "Stray '}' without a matching '{'"))),
                b'{' => {
                    self.skip_white();
                    let value = self.eval1()?;
                    self.skip_white();
                    if self.peek() != b'}' {
                        return Err(Fail::Error(VimError::new(1279, "Missing '}'")));
                    }
                    self.pos += 1;
                    out.extend(match &value {
                        Value::String(text) => text.as_bytes().to_vec(),
                        Value::List(_) | Value::Dict(_) | Value::Blob(_) | Value::Funcref(_) => value.string_bytes(),
                        other => other.to_bytes()?,
                    });
                }
                _ => out.push(byte),
            }
        }
        Ok(Value::String(out.into()))
    }

    fn interpolated(&mut self) -> EvalResult<Value> {
        let start = self.pos;
        self.pos += 1;
        let quote = self.peek();
        self.pos += 1;
        let mut out = Vec::new();
        loop {
            let Some(&byte) = self.src.get(self.pos) else {
                return Err(Fail::Error(if quote == b'"' {
                    VimError::new(114, format!("Missing double quote: {}", String::from_utf8_lossy(&self.src[start..])))
                } else {
                    VimError::new(115, format!("Missing single quote: {}", String::from_utf8_lossy(&self.src[start..])))
                }));
            };
            self.pos += 1;
            match byte {
                b'"' if quote == b'"' => break,
                b'\'' if quote == b'\'' => {
                    if self.peek() == b'\'' {
                        self.pos += 1;
                        out.push(b'\'');
                        continue;
                    }
                    break;
                }
                b'\\' if quote == b'"' => {
                    let escape = self.peek();
                    self.pos += 1;
                    if escape == b'{' || escape == b'}' {
                        out.push(escape);
                    } else {
                        self.string_escape(escape, &mut out);
                    }
                }
                b'{' if self.peek() == b'{' => {
                    self.pos += 1;
                    out.push(b'{');
                }
                b'}' if self.peek() == b'}' => {
                    self.pos += 1;
                    out.push(b'}');
                }
                b'}' => return Err(Fail::Error(VimError::new(1278, format!("Stray '}}' without a matching '{{': {}", String::from_utf8_lossy(&self.src[start..]))))),
                b'{' => {
                    self.skip_white();
                    let value = self.eval1()?;
                    self.skip_white();
                    if self.peek() != b'}' {
                        return Err(Fail::Error(VimError::new(1279, format!("Missing '}}': {}", String::from_utf8_lossy(&self.src[start..])))));
                    }
                    self.pos += 1;
                    if self.evaluate {
                        let text = match &value {
                            Value::String(text) => text.as_bytes().to_vec(),
                            Value::List(_) | Value::Dict(_) | Value::Blob(_) | Value::Funcref(_) => value.string_bytes(),
                            other => other.to_bytes()?,
                        };
                        out.extend(text);
                    }
                }
                _ => out.push(byte),
            }
        }
        Ok(if self.evaluate { Value::String(out.into()) } else { Value::Number(0) })
    }

    fn environment(&mut self) -> EvalResult<Value> {
        self.pos += 1;
        let start = self.pos;
        while is_name_char(self.peek()) {
            self.pos += 1;
        }
        if self.pos == start {
            return Err(self.invalid());
        }
        if !self.evaluate {
            return Ok(Value::Number(0));
        }
        let name = String::from_utf8_lossy(&self.src[start..self.pos]).into_owned();
        Ok(Value::String(self.rt.get_env(self.host, &name).into()))
    }

    fn list_literal(&mut self) -> EvalResult<Value> {
        self.pos += 1;
        let mut items = Vec::new();
        loop {
            self.skip_white();
            if self.peek() == b']' {
                self.pos += 1;
                break;
            }
            if self.at_end() {
                return Err(Fail::Error(VimError::new(697, format!("Missing end of List ']': {}", String::from_utf8_lossy(self.rest())))));
            }
            let item = self.eval1()?;
            if self.evaluate {
                items.push(item);
            }
            self.skip_white();
            match self.peek() {
                b',' => self.pos += 1,
                b']' => {
                    self.pos += 1;
                    break;
                }
                _ => return Err(Fail::Error(VimError::new(696, format!("Missing comma in List: {}", String::from_utf8_lossy(self.rest()))))),
            }
        }
        Ok(if self.evaluate { Value::list(items) } else { Value::Number(0) })
    }

    fn dict_literal(&mut self, literal_keys: bool) -> EvalResult<Value> {
        let start = self.pos;
        self.pos += 1;
        let dict = Dict::new();
        loop {
            self.skip_white();
            if self.peek() == b'}' {
                self.pos += 1;
                break;
            }
            if self.at_end() {
                return Err(Fail::Error(VimError::new(723, format!("Missing end of Dictionary '}}': {}", String::from_utf8_lossy(&self.src[start..])))));
            }
            let key = if literal_keys && (is_name_char(self.peek()) || self.peek() == b'-') {
                let key_start = self.pos;
                while is_name_char(self.peek()) || self.peek() == b'-' {
                    self.pos += 1;
                }
                VStr::from(&self.src[key_start..self.pos])
            } else {
                let key = self.eval1()?;
                if self.evaluate {
                    match &key {
                        Value::Float(_) => return Err(Fail::Error(VimError::new(806, "Using a Float as a String"))),
                        other => other.to_vstr()?,
                    }
                } else {
                    VStr::empty()
                }
            };
            self.skip_white();
            if self.peek() != b':' {
                return Err(Fail::Error(VimError::new(720, format!("Missing colon in Dictionary: {}", String::from_utf8_lossy(self.rest())))));
            }
            self.pos += 1;
            self.skip_white();
            let value = self.eval1()?;
            if self.evaluate {
                if dict.contains(key.as_bytes()) {
                    return Err(Fail::Error(VimError::new(721, format!("Duplicate key in Dictionary: \"{}\"", key.to_str_lossy()))));
                }
                dict.insert(key, value);
            }
            self.skip_white();
            match self.peek() {
                b',' => self.pos += 1,
                b'}' => {
                    self.pos += 1;
                    break;
                }
                _ => return Err(Fail::Error(VimError::new(722, format!("Missing comma in Dictionary: {}", String::from_utf8_lossy(self.rest()))))),
            }
        }
        Ok(if self.evaluate { Value::Dict(dict) } else { Value::Number(0) })
    }

    /// `{args -> expr}`; returns None (without moving) when this is not a lambda.
    fn lambda(&mut self) -> EvalResult<Option<Value>> {
        let start = self.pos;
        let mut at = self.pos + 1;
        let mut params = Vec::new();
        let mut varargs = false;
        loop {
            at = skip_white(self.src, at);
            if self.src[at..].starts_with(b"->") {
                at += 2;
                break;
            }
            if self.src[at..].starts_with(b"...") {
                varargs = true;
                at = skip_white(self.src, at + 3);
                if self.src[at..].starts_with(b"->") {
                    at += 2;
                    break;
                }
                return Ok(None);
            }
            let name_start = at;
            while at < self.src.len() && is_name_char(self.src[at]) {
                at += 1;
            }
            if at == name_start || !is_name_start(self.src[name_start]) {
                return Ok(None);
            }
            params.push(super::state::Param { name: VStr::from(&self.src[name_start..at]), default: None });
            at = skip_white(self.src, at);
            match self.src.get(at) {
                Some(b',') => at += 1,
                Some(b'-') if self.src.get(at + 1) == Some(&b'>') => {
                    at += 2;
                    break;
                }
                _ => return Ok(None),
            }
        }
        self.pos = at;
        self.skip_white();
        let body_start = self.pos;
        let saved = self.evaluate;
        self.evaluate = false;
        let parsed = self.eval1();
        self.evaluate = saved;
        parsed?;
        let body_end = self.pos;
        self.skip_white();
        if self.peek() != b'}' {
            self.pos = start;
            return Err(Fail::Error(VimError::new(451, format!("Expected }}: {}", String::from_utf8_lossy(&self.src[start..])))));
        }
        self.pos += 1;
        if !self.evaluate {
            return Ok(Some(Value::Number(0)));
        }
        let body = VStr::from(&self.src[body_start..body_end]);
        let name = self.rt.define_lambda(params, varargs, body);
        Ok(Some(Value::Funcref(Funcref::named(name))))
    }

    /// `{expr}` followed by `}` is a curly-braces name, not a Dictionary
    /// (Vim's eval_dict() check); `{}` is an empty Dictionary.
    fn is_curly_name(&mut self) -> EvalResult<bool> {
        let start = self.pos;
        let inner = skip_white(self.src, start + 1);
        if self.src.get(inner) == Some(&b'}') {
            return Ok(false);
        }
        let saved = self.evaluate;
        self.evaluate = false;
        self.pos = inner;
        let parsed = self.eval1();
        self.evaluate = saved;
        let end = skip_white(self.src, self.pos);
        self.pos = start;
        Ok(parsed.is_ok() && self.src.get(end) == Some(&b'}'))
    }

    fn option_value(&mut self) -> EvalResult<Value> {
        self.pos += 1;
        let scope = if self.starts_with(b"l:") {
            self.pos += 2;
            super::exec::OptionScope::Local
        } else if self.starts_with(b"g:") {
            self.pos += 2;
            super::exec::OptionScope::Global
        } else {
            super::exec::OptionScope::Both
        };
        let start = self.pos;
        if self.starts_with(b"t_") && self.src.len() >= self.pos + 4 {
            self.pos += 4;
        } else {
            while self.peek().is_ascii_alphanumeric() {
                self.pos += 1;
            }
        }
        if self.pos == start {
            return Err(Fail::Error(VimError::new(112, format!("Option name missing: {}", String::from_utf8_lossy(&self.src[start - 1..])))));
        }
        let name = String::from_utf8_lossy(&self.src[start..self.pos]).into_owned();
        if !self.evaluate {
            return Ok(Value::Number(0));
        }
        self.host.get_option(&name, scope).map_err(Fail::Error)
    }

    fn register_value(&mut self) -> EvalResult<Value> {
        self.pos += 1;
        let name = match self.peek() {
            0 => '"',
            byte => {
                let c = std::str::from_utf8(self.rest()).ok().and_then(|s| s.chars().next()).unwrap_or(byte as char);
                self.pos += c.len_utf8();
                c
            }
        };
        if !self.evaluate {
            return Ok(Value::Number(0));
        }
        let name = if name == '@' { '"' } else { name };
        self.host.get_register(name).map_err(Fail::Error)
    }

    // ----- names, variables and calls ----------------------------------

    /// Parse a variable or function name, expanding curly-brace parts.
    /// Returns the name and whether it was followed by `(`.
    pub fn parse_name(&mut self) -> EvalResult<Option<Vec<u8>>> {
        let start = self.pos;
        let mut name = Vec::new();
        // <SID>/<SNR> prefixes for functions.
        if self.starts_with(b"<SID>") || self.starts_with(b"<sid>") {
            self.pos += 5;
            name.extend_from_slice(format!("<SNR>{}_", self.rt.current_sid()).as_bytes());
        } else if self.starts_with(b"<SNR>") {
            self.pos += 5;
            name.extend_from_slice(b"<SNR>");
        }
        // Scope prefix.
        let rest = self.rest();
        if rest.len() >= 2 && rest[1] == b':' && matches!(rest[0], b'g' | b's' | b'l' | b'a' | b'v' | b'b' | b'w' | b't') && name.is_empty() {
            name.extend_from_slice(&rest[..2]);
            self.pos += 2;
            if rest[0] == b'a' && self.peek().is_ascii_digit() {
                while self.peek().is_ascii_digit() {
                    name.push(self.peek());
                    self.pos += 1;
                }
                return Ok(Some(name));
            }
        }
        loop {
            let byte = self.peek();
            if is_name_char(byte) || (byte == b'#' && !name.is_empty() && self.peek_at(1) != b'{') {
                name.push(byte);
                self.pos += 1;
            } else if byte == b'{' && !(name.is_empty() && self.pos != start) {
                self.curly(&mut name)?;
            } else {
                break;
            }
        }
        if name.is_empty() {
            self.pos = start;
            return Ok(None);
        }
        Ok(Some(name))
    }

    fn curly(&mut self, name: &mut Vec<u8>) -> EvalResult<()> {
        self.pos += 1;
        self.skip_white();
        let value = self.eval1()?;
        self.skip_white();
        if self.peek() != b'}' {
            return Err(self.invalid());
        }
        self.pos += 1;
        if self.evaluate {
            name.extend_from_slice(&value.to_bytes()?);
        } else {
            name.extend_from_slice(b"x");
        }
        Ok(())
    }

    fn name_or_call(&mut self) -> EvalResult<Value> {
        let start = self.pos;
        let Some(name) = self.parse_name()? else {
            if self.at_end() {
                return Err(Fail::Error(VimError::new(15, "Invalid expression: \"\"")));
            }
            return Err(self.invalid());
        };
        if self.peek() == b'(' {
            let args = self.call_arguments()?;
            if !self.evaluate {
                return self.subscripts_after(Value::Number(0), None, true);
            }
            let funcref = self.rt.resolve_function_name(self.host, &name)?;
            let value = self.rt.call_funcref(self.host, &funcref, args, None)?;
            return self.subscripts_after(value, None, true);
        }
        if !self.evaluate {
            return self.subscripts(Value::Number(0), None);
        }
        let value = match self.rt.get_variable(self.host, &name) {
            Ok(value) => value,
            Err(Fail::Error(error)) if error.code == 121 => {
                // A function name used as a value (legacy allows bare names
                // only via function()); report the undefined variable.
                let _ = start;
                return Err(Fail::Error(error));
            }
            Err(other) => return Err(other),
        };
        self.subscripts(value, None)
    }

    /// Parse `(arg, ...)` at `pos`.
    pub fn call_arguments(&mut self) -> EvalResult<Vec<Value>> {
        self.pos += 1;
        let mut args = Vec::new();
        loop {
            self.skip_white();
            if self.peek() == b')' {
                self.pos += 1;
                break;
            }
            if self.at_end() {
                return Err(Fail::Error(VimError::new(116, "Invalid arguments for function")));
            }
            let value = self.eval1()?;
            args.push(value);
            if args.len() > 20 {
                return Err(Fail::Error(VimError::new(740, "Too many arguments for function")));
            }
            self.skip_white();
            match self.peek() {
                b',' => self.pos += 1,
                b')' => {
                    self.pos += 1;
                    break;
                }
                _ => return Err(Fail::Error(VimError::new(116, "Invalid arguments for function"))),
            }
        }
        Ok(args)
    }

    /// Apply `[i]`, `[a:b]`, `.key`, `(args)` and `->method()` subscripts.
    /// `self_dict` is the Dictionary a Funcref member was taken from.
    pub fn subscripts(&mut self, value: Value, self_dict: Option<Dict>) -> EvalResult<Value> {
        self.subscripts_after(value, self_dict, false)
    }

    /// After a function call, white space may precede a `[` subscript, as
    /// in Vim where the call consumes trailing white space.
    fn subscripts_after(&mut self, mut value: Value, mut self_dict: Option<Dict>, mut after_call: bool) -> EvalResult<Value> {
        loop {
            if after_call {
                let next = skip_white(self.src, self.pos);
                if self.src.get(next) == Some(&b'[') {
                    self.pos = next;
                }
                after_call = false;
            }
            match self.peek() {
                b'[' => {
                    self_dict = None;
                    value = self.index(value)?;
                }
                b'.' if self.evaluate && matches!(value, Value::Dict(_)) && is_name_char(self.peek_at(1)) => {
                    self.pos += 1;
                    let start = self.pos;
                    while is_name_char(self.peek()) {
                        self.pos += 1;
                    }
                    let key = &self.src[start..self.pos];
                    let Value::Dict(dict) = &value else { unreachable!() };
                    match dict.get(key) {
                        Some(found) => {
                            self_dict = Some(dict.clone());
                            value = found;
                        }
                        None => return Err(Fail::Error(VimError::new(716, format!("Key not present in Dictionary: \"{}\"", String::from_utf8_lossy(key))))),
                    }
                }
                b'.' if !self.evaluate && is_name_char(self.peek_at(1)) && self.pos > 0 && !is_white(self.src[self.pos - 1]) => {
                    // When skipping we cannot know the type; Vim treats a
                    // dot directly followed by a name after a subscript or
                    // name as member access.
                    self.pos += 1;
                    while is_name_char(self.peek()) {
                        self.pos += 1;
                    }
                }
                b'(' => {
                    let args = self.call_arguments()?;
                    if self.evaluate {
                        let Value::Funcref(funcref) = &value else {
                            if let Value::String(name) = &value {
                                let funcref = self.rt.resolve_function_name(self.host, name.as_bytes())?;
                                value = self.rt.call_funcref(self.host, &funcref, args, self_dict.take())?;
                                continue;
                            }
                            return Err(Fail::Error(VimError::new(109, "Not a Funcref")));
                        };
                        let funcref = funcref.clone();
                        value = self.rt.call_funcref(self.host, &funcref, args, self_dict.take())?;
                    }
                    self_dict = None;
                    after_call = true;
                }
                b'-' if self.peek_at(1) == b'>' => {
                    self.pos += 2;
                    self.skip_white();
                    value = self.method_call(value)?;
                    self_dict = None;
                    after_call = true;
                }
                _ => return Ok(value),
            }
        }
    }

    fn method_call(&mut self, base: Value) -> EvalResult<Value> {
        if self.peek() == b'{' {
            // base->{lambda}(args)
            let Some(lambda) = self.lambda()? else { return Err(self.invalid()); };
            if self.peek() != b'(' {
                return Err(Fail::Error(VimError::new(107, "Missing parentheses: lambda")));
            }
            let mut args = vec![base];
            args.extend(self.call_arguments()?);
            if !self.evaluate {
                return Ok(Value::Number(0));
            }
            let Value::Funcref(funcref) = lambda else { unreachable!() };
            return self.rt.call_funcref(self.host, &funcref, args, None);
        }
        if self.peek() == b'(' {
            // base->(expr)(args)
            self.pos += 1;
            let callee = self.eval1()?;
            self.skip_white();
            if self.peek() != b')' {
                return Err(Fail::Error(VimError::new(110, "Missing ')'")));
            }
            self.pos += 1;
            if self.peek() != b'(' {
                return Err(Fail::Error(VimError::new(107, "Missing parentheses")));
            }
            let mut args = vec![base];
            args.extend(self.call_arguments()?);
            if !self.evaluate {
                return Ok(Value::Number(0));
            }
            let funcref = match callee {
                Value::Funcref(funcref) => funcref,
                Value::String(name) => self.rt.resolve_function_name(self.host, name.as_bytes())?,
                _ => return Err(Fail::Error(VimError::new(1275, "String or function required for ->(expr)"))),
            };
            return self.rt.call_funcref(self.host, &funcref, args, None);
        }
        let Some(name) = self.parse_name()? else {
            return Err(Fail::Error(VimError::new(260, "Missing name after ->")));
        };
        // base->some.name(args) / base->alist[idx](args)
        let mut callee: Option<(Value, Option<Dict>)> = None;
        while matches!(self.peek(), b'.' | b'[') {
            let (current, _) = match callee.take() {
                Some(found) => found,
                None => (if self.evaluate { self.rt.get_variable(self.host, &name)? } else { Value::Number(0) }, None),
            };
            if self.peek() == b'[' {
                callee = Some((self.index(current)?, None));
            } else {
                self.pos += 1;
                let start = self.pos;
                while is_name_char(self.peek()) {
                    self.pos += 1;
                }
                let key = self.src[start..self.pos].to_vec();
                if self.evaluate {
                    let Value::Dict(dict) = &current else { return Err(Fail::Error(VimError::new(715, "Dictionary required"))); };
                    let Some(found) = dict.get(&key) else {
                        return Err(Fail::Error(VimError::new(716, format!("Key not present in Dictionary: \"{}\"", String::from_utf8_lossy(&key)))));
                    };
                    callee = Some((found, Some(dict.clone())));
                } else {
                    callee = Some((Value::Number(0), None));
                }
            }
        }
        if self.peek() != b'(' {
            return Err(Fail::Error(VimError::new(107, format!("Missing parentheses: {}", String::from_utf8_lossy(&name)))));
        }
        let args = self.call_arguments()?;
        if !self.evaluate {
            return Ok(Value::Number(0));
        }
        match callee {
            Some((Value::Funcref(funcref), dict)) => {
                let mut all = vec![base];
                all.extend(args);
                self.rt.call_funcref(self.host, &funcref, all, dict)
            }
            Some((Value::String(callee_name), dict)) => {
                let funcref = self.rt.resolve_function_name(self.host, callee_name.as_bytes())?;
                let mut all = vec![base];
                all.extend(args);
                self.rt.call_funcref(self.host, &funcref, all, dict)
            }
            Some(_) => Err(Fail::Error(VimError::new(109, "Not a Funcref"))),
            None => {
                // A variable holding a Funcref takes precedence for scoped or
                // capitalized names, like a direct call.
                let funcref = self.rt.resolve_function_name(self.host, &name)?;
                let position = if funcref.args().is_empty() && funcref.dict().is_none() {
                    super::builtins::method_base_position(funcref.name().as_bytes())
                } else {
                    0
                };
                let mut all = args;
                let position = position.min(all.len());
                all.insert(position, base);
                self.rt.call_funcref(self.host, &funcref, all, None)
            }
        }
    }

    fn index(&mut self, value: Value) -> EvalResult<Value> {
        self.pos += 1;
        self.skip_white();
        let mut first = None;
        let mut range = false;
        if self.peek() == b':' {
            range = true;
        } else {
            first = Some(self.eval1()?);
            self.skip_white();
            if self.peek() == b':' {
                range = true;
            }
        }
        let mut second = None;
        if range {
            self.pos += 1;
            self.skip_white();
            if self.peek() != b']' {
                second = Some(self.eval1()?);
                self.skip_white();
            }
        }
        if self.peek() != b']' {
            return Err(Fail::Error(VimError::new(111, "Missing ']'")));
        }
        self.pos += 1;
        if !self.evaluate {
            return Ok(Value::Number(0));
        }
        index_value(&value, first.as_ref(), second.as_ref(), range)
    }

    /// For `:call`: parse a callee and its arguments without calling it.
    /// Returns None when the expression is something other than a plain
    /// call (the caller then evaluates it normally).
    pub fn parse_callee(&mut self) -> EvalResult<Option<Callee>> {
        let start = self.pos;
        let Some(name) = self.parse_name()? else {
            self.pos = start;
            return Ok(None);
        };
        let mut self_dict = None;
        let mut target: Option<Value> = None;
        // dict.member / dict[key] chains before the final call.
        while matches!(self.peek(), b'.' | b'[') {
            let current = match target.take() {
                Some(value) => value,
                None => match self.rt.get_variable(self.host, &name) {
                    Ok(value) => value,
                    Err(_) => {
                        self.pos = start;
                        return Ok(None);
                    }
                },
            };
            if self.peek() == b'[' {
                let Value::Dict(dict) = &current else {
                    target = Some(self.index(current)?);
                    self_dict = None;
                    continue;
                };
                let dict = dict.clone();
                target = Some(self.index(current)?);
                self_dict = Some(dict);
            } else {
                let Value::Dict(dict) = &current else {
                    self.pos = start;
                    return Ok(None);
                };
                self.pos += 1;
                let key_start = self.pos;
                while is_name_char(self.peek()) {
                    self.pos += 1;
                }
                let key = &self.src[key_start..self.pos];
                let Some(found) = dict.get(key) else {
                    return Err(Fail::Error(VimError::new(716, format!("Key not present in Dictionary: \"{}\"", String::from_utf8_lossy(key)))));
                };
                self_dict = Some(dict.clone());
                target = Some(found);
            }
        }
        if self.peek() != b'(' {
            self.pos = start;
            return Ok(None);
        }
        let funcref = match target {
            Some(Value::Funcref(funcref)) => funcref,
            Some(Value::String(callee)) => self.rt.resolve_function_name(self.host, callee.as_bytes())?,
            Some(_) => return Err(Fail::Error(VimError::new(109, "Not a Funcref"))),
            None => self.rt.resolve_function_name(self.host, &name)?,
        };
        let args = self.call_arguments()?;
        let funcref = match self_dict {
            Some(dict) if funcref.dict().is_none() || funcref.auto_bound() => {
                Funcref::partial(funcref.name().clone(), funcref.args().to_vec(), Some(dict), true)
            }
            _ => funcref,
        };
        Ok(Some(Callee { funcref, args }))
    }
}

/// `expr[i]` and `expr[a : b]` for String, List, Dictionary and Blob.
pub fn index_value(value: &Value, first: Option<&Value>, second: Option<&Value>, range: bool) -> EvalResult<Value> {
    match value {
        Value::Dict(dict) => {
            if range {
                return Err(Fail::Error(VimError::new(719, "Cannot slice a Dictionary")));
            }
            let key = first.map(|key| key.to_vstr()).transpose()?.unwrap_or_default();
            dict.get(key.as_bytes()).ok_or_else(|| {
                Fail::Error(VimError::new(716, format!("Key not present in Dictionary: \"{}\"", key.to_str_lossy())))
            })
        }
        Value::List(list) => {
            let items = list.snapshot();
            let len = items.len() as i64;
            if range {
                let mut start = first.map(Value::to_number).transpose()?.unwrap_or(0);
                let mut end = second.map(Value::to_number).transpose()?.unwrap_or(len - 1);
                if start < 0 {
                    start += len;
                    if start < 0 {
                        return Ok(Value::list(Vec::new()));
                    }
                }
                if end < 0 {
                    end += len;
                }
                if end >= len {
                    end = len - 1;
                }
                if start > end || start >= len {
                    return Ok(Value::list(Vec::new()));
                }
                return Ok(Value::list(items[start as usize..=end as usize].to_vec()));
            }
            let mut index = first.map(Value::to_number).transpose()?.unwrap_or(0);
            if index < 0 {
                index += len;
            }
            if index < 0 || index >= len {
                return Err(Fail::Error(VimError::new(684, format!("List index out of range: {}", first.map(|f| f.to_number().unwrap_or(0)).unwrap_or(0)))));
            }
            Ok(items[index as usize].clone())
        }
        Value::Blob(blob) => {
            let bytes = blob.bytes();
            let len = bytes.len() as i64;
            if range {
                let mut start = first.map(Value::to_number).transpose()?.unwrap_or(0);
                let mut end = second.map(Value::to_number).transpose()?.unwrap_or(len - 1);
                if start < 0 {
                    start = (len + start).max(0);
                }
                if end < 0 {
                    end += len;
                }
                if end >= len {
                    end = len - 1;
                }
                if start > end || start >= len {
                    return Ok(Value::Blob(Blob::new(Vec::new())));
                }
                return Ok(Value::Blob(Blob::new(bytes[start as usize..=end as usize].to_vec())));
            }
            let mut index = first.map(Value::to_number).transpose()?.unwrap_or(0);
            if index < 0 {
                index += len;
            }
            if index < 0 || index >= len {
                return Err(Fail::Error(VimError::new(979, format!("Blob index out of range: {index}"))));
            }
            Ok(Value::Number(bytes[index as usize] as i64))
        }
        Value::Funcref(_) => Err(Fail::Error(VimError::new(695, "Cannot index a Funcref"))),
        Value::Float(_) => Err(Fail::Error(VimError::new(806, "Using a Float as a String"))),
        other => {
            let text = other.to_vstr()?;
            let bytes = text.as_bytes();
            let len = bytes.len() as i64;
            if range {
                let mut start = first.map(Value::to_number).transpose()?.unwrap_or(0);
                let mut end = second.map(Value::to_number).transpose()?.unwrap_or(len - 1);
                if start < 0 {
                    start = (len + start).max(0);
                }
                if end < 0 {
                    end += len;
                }
                if end >= len {
                    end = len - 1;
                }
                if start > end || start >= len {
                    return Ok(Value::str(""));
                }
                return Ok(Value::String(bytes[start as usize..=end as usize].to_vec().into()));
            }
            let index = first.map(Value::to_number).transpose()?.unwrap_or(0);
            if index < 0 || index >= len {
                return Ok(Value::str(""));
            }
            Ok(Value::String(vec![bytes[index as usize]].into()))
        }
    }
}

/// Legacy `+ - * / %` on Numbers and Floats.
pub fn arithmetic(left: &Value, right: &Value, op: u8) -> EvalResult<Value> {
    let float = matches!(left, Value::Float(_)) || matches!(right, Value::Float(_));
    if float {
        if op == b'%' {
            return Err(Fail::Error(VimError::new(804, "Cannot use '%' with Float")));
        }
        let (a, b) = (
            match left { Value::String(_) => left.to_number()? as f64, _ => left.to_float()? },
            match right { Value::String(_) => right.to_number()? as f64, _ => right.to_float()? },
        );
        return Ok(Value::Float(match op {
            b'+' => a + b,
            b'-' => a - b,
            b'*' => a * b,
            _ => a / b,
        }));
    }
    let (a, b) = (left.to_number()?, right.to_number()?);
    Ok(Value::Number(match op {
        b'+' => a.wrapping_add(b),
        b'-' => a.wrapping_sub(b),
        b'*' => a.wrapping_mul(b),
        b'/' => {
            if b == 0 {
                if a == 0 {
                    i64::MIN
                } else if a > 0 {
                    i64::MAX
                } else {
                    -i64::MAX
                }
            } else {
                a.wrapping_div(b)
            }
        }
        _ => {
            if b == 0 {
                0
            } else {
                a.wrapping_rem(b)
            }
        }
    }))
}
