use super::{Expr, Value, MAX_BINDINGS, MAX_DEPTH, MAX_EXPRESSION_BYTES};

#[derive(Clone, Debug, PartialEq, Eq)]
enum Token {
    Name(String),
    String(String),
    Number(i64),
    Symbol(String),
    Member(String),
    End,
}
pub(super) struct Parser {
    tokens: Vec<Token>,
    at: usize,
}
impl Parser {
    pub(super) fn parse_many(source: &str) -> Result<Vec<Expr>, String> {
        if source.len() > MAX_EXPRESSION_BYTES {
            return Err("syntax setup expression byte budget exceeded".into());
        }
        let mut parser = Self {
            tokens: lex(source, false)?,
            at: 0,
        };
        let mut values = Vec::new();
        while parser.peek() != &Token::End {
            if values.len() >= MAX_BINDINGS {
                return Err("syntax setup execute expression budget exceeded".into());
            }
            values.push(parser.expression(0, 0)?);
        }
        Ok(values)
    }
    pub(super) fn parse(source: &str) -> Result<Expr, String> {
        if source.len() > MAX_EXPRESSION_BYTES {
            return Err("syntax setup expression byte budget exceeded".into());
        }
        let mut parser = Self {
            tokens: lex(source, true)?,
            at: 0,
        };
        let expression = parser.expression(0, 0)?;
        if parser.peek() != &Token::End {
            return Err("unsupported trailing setup expression".into());
        }
        Ok(expression)
    }
    fn peek(&self) -> &Token {
        self.tokens.get(self.at).unwrap_or(&Token::End)
    }
    fn consume(&mut self, symbol: &str) -> bool {
        if self.peek() == &Token::Symbol(symbol.into()) {
            self.at += 1;
            true
        } else {
            false
        }
    }
    fn expect(&mut self, symbol: &str) -> Result<(), String> {
        if self.consume(symbol) {
            Ok(())
        } else {
            Err(format!("expected setup token: {symbol}"))
        }
    }
    fn expression(&mut self, minimum: u8, depth: usize) -> Result<Expr, String> {
        if depth >= MAX_DEPTH {
            return Err("syntax setup parse depth budget exceeded".into());
        }
        let mut left = if self.consume("!") {
            Expr::Not(Box::new(self.expression(7, depth + 1)?))
        } else if self.consume("-") {
            Expr::Negate(Box::new(self.expression(7, depth + 1)?))
        } else if self.consume("+") {
            self.expression(7, depth + 1)?
        } else if self.consume("(") {
            let e = self.expression(0, depth + 1)?;
            self.expect(")")?;
            e
        } else if self.consume("[") {
            Expr::List(self.sequence("]", depth + 1)?)
        } else if self.consume("{") {
            let mut values = Vec::new();
            if !self.consume("}") {
                loop {
                    if values.len() >= MAX_BINDINGS {
                        return Err("syntax setup dictionary budget exceeded".into());
                    }
                    let key = self.expression(0, depth + 1)?;
                    self.expect(":")?;
                    let value = self.expression(0, depth + 1)?;
                    values.push((key, value));
                    if self.consume("}") {
                        break;
                    }
                    self.expect(",")?;
                    if self.consume("}") {
                        break;
                    }
                }
            }
            Expr::Dictionary(values)
        } else {
            let token = self.peek().clone();
            self.at += 1;
            match token {
                Token::String(s) => Expr::Value(Value::Text(s)),
                Token::Number(n) => Expr::Value(Value::Number(n)),
                Token::Name(name) if self.consume("(") => {
                    Expr::Call(name, self.sequence(")", depth + 1)?)
                }
                Token::Name(name) => Expr::Variable(name),
                _ => return Err("unsupported setup expression atom".into()),
            }
        };
        loop {
            if minimum <= 8 {
                if let Token::Member(name) = self.peek().clone() {
                    self.at += 1;
                    left = Expr::Member(Box::new(left), name);
                    continue;
                }
            }
            if minimum <= 8 && self.consume("[") {
                let start = if self.peek() == &Token::Symbol(":".into()) {
                    None
                } else {
                    Some(Box::new(self.expression(0, depth + 1)?))
                };
                if self.consume(":") {
                    let end = if self.peek() == &Token::Symbol("]".into()) {
                        None
                    } else {
                        Some(Box::new(self.expression(0, depth + 1)?))
                    };
                    self.expect("]")?;
                    left = Expr::Slice(Box::new(left), start, end);
                } else {
                    self.expect("]")?;
                    left = Expr::Index(
                        Box::new(left),
                        start.ok_or("setup index requires an expression")?,
                    );
                }
                continue;
            }
            if minimum <= 8 && self.consume("->") {
                let Token::Name(name) = self.peek().clone() else {
                    return Err("setup method name required".into());
                };
                self.at += 1;
                self.expect("(")?;
                let mut values = self.sequence(")", depth + 1)?;
                let position = usize::from(name == "printf" && !values.is_empty());
                values.insert(position, left);
                left = Expr::Call(name, values);
                continue;
            }
            let Token::Symbol(operator) = self.peek().clone() else {
                break;
            };
            let precedence = match operator.as_str() {
                "?" => 1,
                "||" => 2,
                "&&" => 3,
                "==" | "!=" | "==#" | "!=#" | "==?" | "!=?" | "<" | "<=" | ">" | ">=" | "=~#"
                | "!~#" | "=~?" | "!~?" | "=~" | "!~" => 4,
                ".." | "." | "+" | "-" => 5,
                "*" | "/" | "%" => 6,
                _ => break,
            };
            if precedence < minimum {
                break;
            }
            self.at += 1;
            if operator == "?" {
                let yes = self.expression(0, depth + 1)?;
                self.expect(":")?;
                let no = self.expression(precedence, depth + 1)?;
                left = Expr::Conditional(Box::new(left), Box::new(yes), Box::new(no));
            } else {
                let right = self.expression(precedence + 1, depth + 1)?;
                left = Expr::Binary(operator, Box::new(left), Box::new(right));
            }
        }
        Ok(left)
    }
    fn sequence(&mut self, end: &str, depth: usize) -> Result<Vec<Expr>, String> {
        let mut values = vec![];
        if self.consume(end) {
            return Ok(values);
        }
        loop {
            if values.len() >= 1024 {
                return Err("syntax setup list budget exceeded".into());
            }
            values.push(self.expression(0, depth)?);
            if self.consume(end) {
                break;
            }
            self.expect(",")?;
            if self.consume(end) {
                break;
            }
        }
        Ok(values)
    }
}
fn lex(source: &str, trailing_comment: bool) -> Result<Vec<Token>, String> {
    let mut chars = source.char_indices().peekable();
    let mut tokens = vec![];
    while let Some((at, ch)) = chars.next() {
        if ch.is_whitespace() {
            continue;
        }
        if tokens.len() >= 4096 {
            return Err("syntax setup token budget exceeded".into());
        }
        if trailing_comment && ch == '"' && tokens.last().is_some_and(|token| {
            matches!(token, Token::Name(_) | Token::Number(_) | Token::String(_) | Token::Member(_))
                || matches!(token, Token::Symbol(symbol) if matches!(symbol.as_str(), ")" | "]" | "}"))
        }) {
            break;
        }
        if ch == '.'
            && at > 0
            && !source.as_bytes()[at - 1].is_ascii_whitespace()
            && chars
                .peek()
                .is_some_and(|(_, c)| c.is_ascii_alphabetic() || *c == '_')
        {
            let mut end = at + 1;
            while chars
                .peek()
                .is_some_and(|(_, c)| c.is_ascii_alphanumeric() || *c == '_')
            {
                end = chars.next().unwrap().0 + 1;
            }
            if !chars.peek().is_some_and(|(_, c)| matches!(c, ':' | '(')) {
                tokens.push(Token::Member(source[at + 1..end].into()));
                continue;
            }
            // A scoped name after the dot is a concatenation operand.
            while chars
                .peek()
                .is_some_and(|(_, c)| c.is_ascii_alphanumeric() || matches!(c, '_' | ':' | '#'))
            {
                end = chars.next().unwrap().0 + 1;
            }
            tokens.push(Token::Symbol(".".into()));
            tokens.push(Token::Name(source[at + 1..end].into()));
            continue;
        }
        if ch == '\'' || ch == '"' {
            let mut value = Vec::new();
            let mut closed = false;
            while let Some((_, c)) = chars.next() {
                if c == ch {
                    if ch == '\'' && chars.peek().is_some_and(|(_, c)| *c == '\'') {
                        chars.next();
                        value.push(b'\'');
                        continue;
                    }
                    closed = true;
                    break;
                }
                if c == '\\' && ch == '"' {
                    let (_, next) = chars.next().ok_or("unterminated setup string escape")?;
                    if matches!(next, 'x' | 'X' | 'u' | 'U' | '0'..='7') {
                        let (radix, maximum) = match next {
                            'x' | 'X' => (16, 2),
                            'u' => (16, 4),
                            'U' => (16, 8),
                            _ => (8, 3),
                        };
                        let mut count = usize::from(radix == 8);
                        let mut number = if radix == 8 {
                            next.to_digit(8).unwrap()
                        } else {
                            0
                        };
                        while count < maximum
                            && chars.peek().is_some_and(|(_, c)| c.is_digit(radix))
                        {
                            number = number
                                .checked_mul(radix)
                                .and_then(|number| {
                                    number.checked_add(
                                        chars.next().unwrap().1.to_digit(radix).unwrap(),
                                    )
                                })
                                .ok_or("invalid setup string numeric escape")?;
                            count += 1;
                        }
                        if count == 0 {
                            value.push(next as u8);
                        } else if matches!(next, 'u' | 'U') {
                            let character =
                                char::from_u32(number).ok_or("invalid setup Unicode escape")?;
                            value.extend_from_slice(character.encode_utf8(&mut [0; 4]).as_bytes());
                        } else {
                            value.push(number as u8);
                        }
                        continue;
                    }
                    let escaped = match next {
                        'n' => '\n',
                        'r' => '\r',
                        't' => '\t',
                        'b' => '\u{8}',
                        'e' => '\u{1b}',
                        'f' => '\u{c}',
                        '\\' => '\\',
                        '"' => '"',
                        '<' => return Err("setup editor key escapes are unsupported".into()),
                        _ => next,
                    };
                    value.extend_from_slice(escaped.encode_utf8(&mut [0; 4]).as_bytes());
                } else {
                    value.extend_from_slice(c.encode_utf8(&mut [0; 4]).as_bytes());
                }
            }
            if !closed {
                return Err("unterminated setup string".into());
            }
            if let Some(zero) = value.iter().position(|byte| *byte == 0) {
                value.truncate(zero);
            }
            tokens
                .push(Token::String(String::from_utf8(value).map_err(|_| {
                    "setup string contains unsupported non-UTF-8 bytes"
                })?));
            continue;
        }
        if ch.is_ascii_digit() {
            let mut end = at + ch.len_utf8();
            let radix = if ch == '0' {
                match chars.peek().map(|(_, ch)| ch) {
                    Some('x' | 'X') => {
                        end = chars.next().unwrap().0 + 1;
                        16
                    }
                    Some('b' | 'B') => {
                        end = chars.next().unwrap().0 + 1;
                        2
                    }
                    Some('o' | 'O') => {
                        end = chars.next().unwrap().0 + 1;
                        8
                    }
                    Some('0'..='7') => 8,
                    _ => 10,
                }
            } else {
                10
            };
            let start = if end > at + 1 { end } else { at };
            while chars.peek().is_some_and(|(_, c)| c.is_digit(radix)) {
                let (at, c) = chars.next().unwrap();
                end = at + c.len_utf8();
            }
            if source[end..].starts_with('.')
                && source
                    .as_bytes()
                    .get(end + 1)
                    .is_some_and(u8::is_ascii_digit)
            {
                return Err("syntax setup floating-point values are unsupported".into());
            }
            tokens.push(Token::Number(
                i64::from_str_radix(&source[start..end], radix)
                    .map_err(|_| "invalid setup number")?,
            ));
            continue;
        }
        if ch.is_ascii_alphabetic()
            || ch == '_'
            || ch == '&' && chars.peek().is_some_and(|(_, c)| c.is_ascii_alphabetic())
        {
            let mut end = at + ch.len_utf8();
            while chars
                .peek()
                .is_some_and(|(_, c)| c.is_ascii_alphanumeric() || matches!(c, '_' | ':' | '#'))
            {
                let (at, c) = chars.next().unwrap();
                end = at + c.len_utf8();
            }
            tokens.push(Token::Name(source[at..end].into()));
            continue;
        }
        let operator = [
            "=~#", "!~#", "==#", "!=#", "=~?", "!~?", "==?", "!=?", "||", "&&", "==", "!=", ">=",
            "<=", "=~", "!~", "..", "->",
        ]
        .into_iter()
        .find(|s| source[at..].starts_with(s));
        if let Some(operator) = operator {
            for _ in 1..operator.len() {
                chars.next();
            }
            tokens.push(Token::Symbol(operator.into()));
        } else if "!?():,.[]<>{}+-*/%".contains(ch) {
            tokens.push(Token::Symbol(ch.to_string()));
        } else {
            return Err(format!("unsupported syntax setup token: {ch}"));
        }
    }
    tokens.push(Token::End);
    Ok(tokens)
}
