//! Vim's JSON and JavaScript value grammars, with bounded nesting and output.
use super::{
    builtins::{MAX_LIST_ITEMS, MAX_STRING_BYTES},
    error::{EvalResult, Fail, VimError},
    value::{Dict, VStr, Value},
};

pub(super) fn decode(source: &str, js: bool) -> EvalResult<Value> {
    if source.len() > MAX_STRING_BYTES {
        return Err(Fail::Interrupt("JSON input too large".into()));
    }
    let mut parser = Parser {
        source,
        at: 0,
        js,
        items: 0,
    };
    parser.space();
    if parser.at == source.len() {
        return Ok(Value::None);
    }
    let value = parser.value(0)?;
    parser.space();
    if parser.at != source.len() {
        return Err(VimError::new(488, "Trailing characters").into());
    }
    Ok(value)
}

struct Parser<'a> {
    source: &'a str,
    at: usize,
    js: bool,
    items: usize,
}
impl Parser<'_> {
    fn error(&self) -> Fail {
        VimError::new(
            491,
            format!("JSON decode error at '{}'", &self.source[self.at..]),
        )
        .into()
    }
    fn space(&mut self) {
        while self.peek().is_some_and(|b| b.is_ascii_whitespace()) {
            self.at += 1;
        }
    }
    fn peek(&self) -> Option<u8> {
        self.source.as_bytes().get(self.at).copied()
    }
    fn eat(&mut self, b: u8) -> bool {
        self.space();
        if self.peek() == Some(b) {
            self.at += 1;
            true
        } else {
            false
        }
    }
    fn value(&mut self, depth: usize) -> EvalResult<Value> {
        self.space();
        self.items += 1;
        if depth > 100 || self.items > MAX_LIST_ITEMS {
            return Err(Fail::Interrupt(
                "JSON nesting or item limit exceeded".into(),
            ));
        }
        match self.peek() {
            Some(b'"') => self.string().map(Value::String),
            Some(b'\'') if self.js => self.string().map(Value::String),
            Some(b'[') => {
                self.at += 1;
                let mut items = Vec::new();
                while !self.eat(b']') {
                    if self.js && self.eat(b',') {
                        items.push(Value::None);
                        self.items += 1;
                        if self.items > MAX_LIST_ITEMS {
                            return Err(Fail::Interrupt("JSON item limit exceeded".into()));
                        }
                        continue;
                    }
                    items.push(self.value(depth + 1)?);
                    if self.eat(b']') {
                        break;
                    }
                    if !self.eat(b',') {
                        return Err(self.error());
                    }
                }
                Ok(Value::list(items))
            }
            Some(b'{') => {
                self.at += 1;
                let mut items = std::collections::BTreeMap::new();
                while !self.eat(b'}') {
                    self.space();
                    let key =
                        if self.peek() == Some(b'"') || self.js && self.peek() == Some(b'\'') {
                            self.string()?
                        } else if self.js {
                            let start = self.at;
                            while self.peek().is_some_and(|b| {
                                b.is_ascii_alphanumeric() || b == b'_' || b == b'$'
                            }) {
                                self.at += 1;
                            }
                            if start == self.at {
                                return Err(self.error());
                            }
                            VStr::from(&self.source[start..self.at])
                        } else {
                            return Err(self.error());
                        };
                    if !self.eat(b':') {
                        return Err(self.error());
                    }
                    let value = self.value(depth + 1)?;
                    if items.insert(key, value).is_some() {
                        return Err(VimError::new(938, "Duplicate key in JSON").into());
                    }
                    if self.eat(b'}') {
                        break;
                    }
                    if !self.eat(b',') {
                        return Err(self.error());
                    }
                }
                Ok(Value::Dict(Dict::from_pairs(items)))
            }
            _ => {
                for (token, value) in [
                    ("true", Value::Bool(true)),
                    ("false", Value::Bool(false)),
                    ("null", Value::Null),
                    ("NaN", Value::Float(f64::NAN)),
                    ("Infinity", Value::Float(f64::INFINITY)),
                    ("-Infinity", Value::Float(f64::NEG_INFINITY)),
                ] {
                    if self.source[self.at..].starts_with(token) {
                        self.at += token.len();
                        return Ok(value);
                    }
                }
                let start = self.at;
                while self.peek().is_some_and(|b| {
                    b.is_ascii_digit() || matches!(b, b'-' | b'+' | b'.' | b'e' | b'E')
                }) {
                    self.at += 1;
                }
                let token = &self.source[start..self.at];
                // serde checks the number grammar; integer -0 remains a Number in Vim.
                if token == "-0" {
                    return Ok(Value::Number(0));
                }
                let normalized = if !token.contains(['.', 'e', 'E']) {
                    token.parse::<i64>().ok().map(|n| n.to_string())
                } else {
                    None
                };
                let number: serde_json::Number =
                    serde_json::from_str(normalized.as_deref().unwrap_or(token))
                        .map_err(|_| self.error())?;
                Ok(number
                    .as_i64()
                    .map(Value::Number)
                    .unwrap_or_else(|| Value::Float(number.as_f64().unwrap())))
            }
        }
    }
    fn string(&mut self) -> EvalResult<VStr> {
        let quote = self.peek().unwrap();
        self.at += 1;
        let mut out = String::new();
        loop {
            let Some(c) = self.source[self.at..].chars().next() else {
                return Err(self.error());
            };
            self.at += c.len_utf8();
            if c as u32 == quote as u32 {
                return Ok(out.into());
            }
            if c != '\\' {
                out.push(c);
                continue;
            }
            let Some(escape) = self.peek() else {
                return Err(self.error());
            };
            self.at += 1;
            match escape {
                b'b' => out.push('\u{8}'),
                b'f' => out.push('\u{c}'),
                b'n' => out.push('\n'),
                b'r' => out.push('\r'),
                b't' => out.push('\t'),
                b'u' => {
                    let high = self.hex4()?;
                    let scalar = if (0xd800..=0xdbff).contains(&high)
                        && self.source[self.at..].starts_with("\\u")
                    {
                        self.at += 2;
                        let low = self.hex4()?;
                        if !(0xdc00..=0xdfff).contains(&low) {
                            return Err(self.error());
                        }
                        0x10000 + ((high - 0xd800) << 10) + low - 0xdc00
                    } else {
                        high
                    };
                    out.push(char::from_u32(scalar).ok_or_else(|| self.error())?);
                }
                b if b.is_ascii() => out.push(b as char),
                _ => {
                    self.at -= 1;
                    let c = self.source[self.at..].chars().next().unwrap();
                    self.at += c.len_utf8();
                    out.push(c);
                }
            }
        }
    }
    fn hex4(&mut self) -> EvalResult<u32> {
        let end = self.at + 4;
        let bytes = self
            .source
            .as_bytes()
            .get(self.at..end)
            .ok_or_else(|| self.error())?;
        if !bytes.iter().all(u8::is_ascii_hexdigit) {
            return Err(self.error());
        }
        let value = u32::from_str_radix(std::str::from_utf8(bytes).unwrap(), 16).unwrap();
        self.at = end;
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn excessive_nesting_stops_before_stack_exhaustion() {
        let source = format!("{}0{}", "[".repeat(101), "]".repeat(101));
        assert!(matches!(decode(&source, false), Err(Fail::Interrupt(_))));
    }
    #[test]
    fn elisions_obey_the_container_budget() {
        let mut parser = Parser {
            source: "[,]",
            at: 0,
            js: true,
            items: MAX_LIST_ITEMS - 1,
        };
        assert!(matches!(parser.value(0), Err(Fail::Interrupt(_))));
    }
}
