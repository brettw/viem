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
    analysis: Option<&'r super::compiler::Context>,
    constant: bool,
}

pub fn is_name_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_'
}
pub fn is_name_char(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}
pub fn is_white(byte: u8) -> bool {
    byte == b' ' || byte == b'\t' || byte == b'\n'
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
        Self { rt, host, src, pos, evaluate, depth: 0, analysis: None, constant: false }
    }

    pub(crate) fn analyze(rt: &'r mut Runtime, host: &'h mut dyn ScriptHost, src: &'s [u8], pos: usize, context: &'r super::compiler::Context) -> Self {
        Self {rt, host, src, pos, evaluate: true, depth: 0, analysis: Some(context), constant: false}
    }
    pub(crate) fn constant(rt: &'r mut Runtime, host: &'h mut dyn ScriptHost, src: &'s [u8]) -> Self {
        let mut eval = Self::new(rt, host, src, 0, true); eval.constant = true; eval
    }
    fn operator_space(&self, length: usize) -> EvalResult<()> {
        if self.rt.vim9() && (self.pos == 0 || !is_white(self.src[self.pos - 1]) || !self.src.get(self.pos + length).is_some_and(|b|is_white(*b))) {
            return Err(VimError::new(1004, "White space required before and after operator").into());
        }
        Ok(())
    }
    fn comma(&mut self) -> EvalResult<()> {
        if self.rt.vim9() && !self.src.get(self.pos + 1).is_some_and(|b|is_white(*b) || matches!(b, b')' | b']' | b'}' | b'>')) {
            return Err(VimError::new(1069, "White space required after comma").into());
        }
        self.pos += 1; Ok(())
    }
    fn analyzing(&self) -> bool {self.analysis.is_some() && self.evaluate}
    fn variable(&mut self, name: &[u8]) -> EvalResult<Value> {
        if self.constant && !matches!(name, b"true" | b"false" | b"null" | b"v:true" | b"v:false" | b"v:null" | b"v:none") {return Err(self.invalid());}
        if self.analyzing() {
            if let Some(path) = self.rt.state.import(self.rt.current_sid(), name) {return super::vim9::module(self.rt, self.host, &path);}
        }
        if let Some(context) = self.analysis {context.lookup(self.rt, name)} else {self.rt.get_variable(self.host, name)}
    }
    fn reference(&mut self, name: &[u8]) -> EvalResult<Funcref> {
        if self.constant {return if matches!(name, b"has" | b"exists_compiled") {Ok(Funcref::named(name))} else {Err(self.invalid())};}
        if let Some(context) = self.analysis {
            let reference = Funcref::named(self.rt.canonical_function_name(name));
            Ok(match context.lookup(self.rt, name) {
                Ok(Value::Funcref(reference)) => reference,
                Ok(value) => reference.with_type(super::types::Type::infer(&value)),
                Err(_) => reference,
            })
        } else {self.rt.resolve_function_name(self.host, name)}
    }
    fn invoke(&mut self, funcref: &Funcref, args: Vec<Value>, dict: Option<Dict>) -> EvalResult<Value> {
        if self.constant {
            let name = funcref.name().as_bytes();
            if !matches!(name, b"has" | b"exists_compiled") {return Err(self.invalid());}
            if name == b"exists_compiled" && !args.first().is_some_and(|v|matches!(v, Value::String(s) if s.as_bytes().iter().all(|b|b.is_ascii_alphanumeric() || b"_:*!&+$#".contains(b)))) {return Err(self.invalid());}
            return super::builtins::call(self.rt, self.host, &String::from_utf8_lossy(name), args).ok_or_else(||self.invalid())?;
        }
        if let Some(context) = self.analysis {
            if let Some(function) = funcref.specialization().or_else(||self.rt.state.function(funcref.name().as_bytes())) {context.depend_on(function);}
            if let Some(ty) = funcref.declared_type() {super::compiler::invoke(self.rt, &ty, &args)} else {super::compiler::call(self.rt, context, funcref.name().as_bytes(), &args)}
        } else {self.rt.call_funcref(self.host, funcref, args, dict)}
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
            self.operator_space(2)?;
            self.pos += 2;
            self.skip_white();
            let truthy = self.evaluate && !condition.is_empty_value();
            let saved = self.evaluate;
            if truthy && !self.analyzing() {
                self.evaluate = false;
            }
            let other = self.eval1()?;
            self.evaluate = saved;
            if self.analyzing() {let a = super::types::Type::infer(&condition); let b = super::types::Type::infer(&other); return Ok(Value::type_value(if a == b {a} else {super::types::Type::Any}));}
            return Ok(if !self.evaluate || truthy { condition } else { other });
        }
        if self.peek() != b'?' {
            return Ok(condition);
        }
        self.operator_space(1)?;
        self.pos += 1;
        self.skip_white();
        let truth = if self.analyzing() {super::types::Type::Bool.require(&super::types::Type::infer(&condition))?; true} else if self.evaluate { if self.rt.vim9() {super::vim9::truth(&condition)?} else {condition.to_bool()?} } else { false };
        let saved = self.evaluate;
        self.evaluate = saved && truth;
        let then_value = self.eval1()?;
        self.evaluate = saved;
        self.skip_white();
        if self.peek() != b':' {
            return Err(Fail::Error(VimError::new(109, "Missing ':' after '?'")));
        }
        self.operator_space(1)?;
        self.pos += 1;
        self.skip_white();
        self.evaluate = saved && (!truth || self.analysis.is_some());
        let else_value = self.eval1()?;
        self.evaluate = saved;
        if self.analyzing() {let a = super::types::Type::infer(&then_value); let b = super::types::Type::infer(&else_value); return Ok(Value::type_value(if a == b {a} else {super::types::Type::Any}));}
        Ok(if !saved { Value::Number(0) } else if truth { then_value } else { else_value })
    }

    fn eval2(&mut self) -> EvalResult<Value> {
        let mut left = self.eval3()?;
        self.skip_white();
        if !(self.peek() == b'|' && self.peek_at(1) == b'|') {
            return Ok(left);
        }
        let saved = self.evaluate;
        let mut result = if self.analyzing() {super::types::Type::Bool.require(&super::types::Type::infer(&left))?; false} else if saved { if self.rt.vim9() {super::vim9::truth(&left)?} else {left.to_bool()?} } else { false };
        while self.peek() == b'|' && self.peek_at(1) == b'|' {
            self.operator_space(2)?;
            self.pos += 2;
            self.skip_white();
            self.evaluate = saved && (!result || self.analysis.is_some());
            let right = self.eval3()?;
            if self.evaluate {
                result = if self.analyzing() {super::types::Type::Bool.require(&super::types::Type::infer(&right))?; false} else if self.rt.vim9() {super::vim9::truth(&right)?} else {right.to_bool()?};
            }
            self.skip_white();
        }
        self.evaluate = saved;
        left = if self.rt.vim9() {Value::Bool(result)} else {Value::Number(result as i64)};
        Ok(left)
    }

    fn eval3(&mut self) -> EvalResult<Value> {
        let left = self.eval4()?;
        self.skip_white();
        if !(self.peek() == b'&' && self.peek_at(1) == b'&') {
            return Ok(left);
        }
        let saved = self.evaluate;
        let mut result = if self.analyzing() {super::types::Type::Bool.require(&super::types::Type::infer(&left))?; true} else if saved { if self.rt.vim9() {super::vim9::truth(&left)?} else {left.to_bool()?} } else { true };
        while self.peek() == b'&' && self.peek_at(1) == b'&' {
            self.operator_space(2)?;
            self.pos += 2;
            self.skip_white();
            self.evaluate = saved && (result || self.analysis.is_some());
            let right = self.eval4()?;
            if self.evaluate {
                result = if self.analyzing() {super::types::Type::Bool.require(&super::types::Type::infer(&right))?; false} else if self.rt.vim9() {super::vim9::truth(&right)?} else {right.to_bool()?};
            }
            self.skip_white();
        }
        self.evaluate = saved;
        Ok(if self.rt.vim9() {Value::Bool(result)} else {Value::Number(result as i64)})
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
            _ if self.rt.vim9() => CaseMode::Match,
            _ => CaseMode::Option,
        };
        self.operator_space(len)?;
        self.pos += len;
        self.skip_white();
        let right = self.eval5()?;
        if !self.evaluate {
            return Ok(Value::Number(0));
        }
        if self.analyzing() {return Ok(Value::type_value(super::types::Type::Bool));}
        if self.rt.vim9() && (matches!(left, Value::Null) || matches!(right, Value::Null)) && matches!(op, CompareOp::Equal | CompareOp::NotEqual) {
            let equal = left.is_null() && right.is_null(); return Ok(Value::Bool(if op == CompareOp::Equal {equal} else {!equal}));
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
        Ok(if self.rt.vim9() {Value::Bool(result)} else {Value::Number(result as i64)})
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
            self.operator_space(2)?;
            self.pos += 2;
            self.skip_white();
            let right = self.eval6(false)?;
            if self.evaluate {
                if self.analyzing() {super::types::Type::Number.require(&super::types::Type::infer(&left))?; super::types::Type::Number.require(&super::types::Type::infer(&right))?; left = Value::type_value(super::types::Type::Number); continue;}
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
                b'.' if !self.rt.vim9() => ".",
                _ => return Ok(left),
            };
            // "a.b" with a Dictionary was handled as a member in expr10.
            self.operator_space(op.len())?;
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
            if self.analyzing() {left = super::compiler::binary(&left, &right, op)?; continue;}
            left = match op {
                "." | ".." => {
                    let bytes_for = |value: &Value| -> Result<Vec<u8>, VimError> {
                        if self.rt.vim9() && matches!(value, Value::Bool(_) | Value::None | Value::Null) {Ok(value.display_bytes(true, false))} else {value.to_bytes()}
                    };
                    let mut bytes = bytes_for(&left)?;
                    bytes.extend_from_slice(&bytes_for(&right)?);
                    Value::String(bytes.into())
                }
                "+" => match (&left, &right) {
                    (Value::Tuple(a), Value::Tuple(b)) => {let mut items = a.as_ref().clone(); items.extend(b.iter().cloned()); Value::Tuple(std::sync::Arc::new(items))}
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
                    _ => {if self.rt.vim9() {super::compiler::binary(&left, &right, "+")?;} arithmetic(&left, &right, b'+')?},
                },
                _ => {if self.rt.vim9() {super::compiler::binary(&left, &right, "-")?;} arithmetic(&left, &right, b'-')?},
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
            self.operator_space(1)?;
            self.pos += 1;
            self.skip_white();
            let right = self.eval9(false)?;
            if self.evaluate {
                left = if self.analyzing() {super::compiler::binary(&left, &right, &(op as char).to_string())?} else {if self.rt.vim9() {super::compiler::binary(&left, &right, &(op as char).to_string())?;} arithmetic(&left, &right, op)?};
            }
        }
    }

    fn eval9(&mut self, want_string: bool) -> EvalResult<Value> {
        self.skip_white();
        let start = self.pos;
        while matches!(self.peek(), b'!' | b'-' | b'+') {
            if self.rt.vim9() && matches!(self.peek(), b'+' | b'-') && self.peek() == self.peek_at(1) {return Err(self.invalid());}
            self.pos += 1;
            self.skip_white();
        }
        let unary = self.src[start..self.pos].iter().copied().filter(|b| !is_white(*b)).collect::<Vec<u8>>();
        let mut value = self.eval10(want_string)?;
        if !self.evaluate {
            return Ok(Value::Number(0));
        }
        for op in unary.iter().rev() {
            if self.analyzing() {value = if *op == b'!' {Value::type_value(super::types::Type::Bool)} else {super::compiler::binary(&Value::Number(0), &value, "+")?}; continue;}
            value = match op {
                b'!' => if self.rt.vim9() {Value::Bool(value.is_empty_value())} else {Value::Number(!value.to_bool()? as i64)},
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

    fn read_type(&mut self) -> EvalResult<super::types::Type> {
        let mut at = self.pos;
        let result = self.read_type_at(&mut at);
        self.pos = at;
        result
    }

    fn read_type_at(&mut self, at: &mut usize) -> EvalResult<super::types::Type> {
        if self.evaluate && !self.constant {super::types::parse_active(self.src, at, self.rt, self.host)}
        else {super::types::parse_in(self.src, at, self.rt).map_err(Into::into)}
    }

    fn eval10_inner(&mut self, want_string: bool) -> EvalResult<Value> {
        let byte = self.peek();
        let value = match byte {
            b'<' if self.rt.vim9() => {
                self.pos += 1;
                let ty = self.read_type()?;
                self.skip_white(); if self.peek() != b'>' {return Err(VimError::new(1009, "Missing > after type cast").into());}
                self.pos += 1; self.skip_white(); let value = self.eval9(want_string)?;
                if self.analyzing() {Value::type_value(ty)} else if self.evaluate {ty.check(value)?} else {value}
            }
            b'0'..=b'9' => self.number(want_string)?,
            b'"' => self.double_quoted()?,
            b'\'' => self.single_quoted()?,
            b'$' if matches!(self.peek_at(1), b'"' | b'\'') => self.interpolated()?,
            b'$' => self.environment()?,
            b'[' => self.list_literal()?,
            b'#' if self.peek_at(1) == b'{' => {
                if self.rt.vim9() {return Err(VimError::new(1097, "Line not recognized: legacy Dictionary literal").into());}
                self.pos += 1;
                self.dict_literal(true)?
            }
            b'{' if self.rt.vim9() => self.dict_literal(true)?,
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
                if self.rt.vim9() {
                    if let Some(value) = self.vim9_lambda()? {return self.subscripts(value, None);}
                }
                self.pos += 1;
                self.skip_white();
                if self.rt.vim9() && self.peek() == b')' {self.pos += 1; return self.subscripts(Value::Tuple(std::sync::Arc::new(vec![])), None);}
                let mut value = self.eval1()?;
                self.skip_white();
                if self.rt.vim9() && self.peek() == b',' {
                    let mut items = vec![value];
                    while self.peek() == b',' {self.comma()?; self.skip_white(); if self.peek() == b')' {break;} items.push(self.eval1()?); self.skip_white();}
                    value = Value::Tuple(std::sync::Arc::new(items));
                }
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
        let (value, used) = str2nr(rest, !self.rt.vim9());
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
                            Value::Type(_) if self.analyzing() => vec![],
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
        if self.analyzing() {return Ok(Value::type_value(super::types::Type::String));}
        if self.constant {return Err(self.invalid());}
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
                b',' => self.comma()?,
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
                        Value::Type(_) if self.analyzing() => format!("<key{}>", dict.len()).into(),
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
            if self.rt.vim9() && !self.src.get(self.pos + 1).is_some_and(|b|is_white(*b)) {return Err(VimError::new(1069, "White space required after colon").into());}
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
                b',' => self.comma()?,
                b'}' => {
                    self.pos += 1;
                    break;
                }
                _ => return Err(Fail::Error(VimError::new(722, format!("Missing comma in Dictionary: {}", String::from_utf8_lossy(self.rest()))))),
            }
        }
        Ok(if self.evaluate { Value::Dict(dict) } else { Value::Number(0) })
    }

    fn vim9_lambda(&mut self) -> EvalResult<Option<Value>> {
        use super::types::Type;
        let start = self.pos;
        let Some(close) = delimited_end(self.src, start, b'(', b')') else {return Ok(None);};
        let mut at = skip_white(self.src, close);
        let mut result_type = Type::Any;
        let infer_result = self.src.get(at) != Some(&b':');
        let result_at = at + 1;
        // Identify the lambda before resolving types or importing modules.
        // A grouping expression can be followed by a ternary's colon.
        if !infer_result {
            at += 1;
            if super::types::parse(self.src, &mut at).is_err() {return Ok(None);}
            at = skip_white(self.src, at);
        }
        if !self.src.get(at..).is_some_and(|s|s.starts_with(b"=>")) {return Ok(None);}
        let body_at = skip_white(self.src, at + 2);
        if !infer_result {let mut at = result_at; result_type = self.read_type_at(&mut at)?;}
        let mut params = vec![]; let mut types = vec![]; let mut rest = None; at = start + 1;
        while at < close - 1 {
            at = skip_white(self.src, at);
            if at == close - 1 {break;}
            if rest.is_some() {return Err(VimError::new(1069, "Variable arguments must be last").into());}
            let variadic = self.src[at..].starts_with(b"...");
            if variadic {at += 3;}
            let begin = at;
            while self.src.get(at).is_some_and(|b|is_name_char(*b)) {at += 1;}
            if begin == at {return Err(self.invalid());}
            let name = VStr::from(&self.src[begin..at]);
            if name.as_bytes() != b"_" && params.iter().any(|p: &super::state::Param|p.name == name) {return Err(VimError::new(853, "Duplicate argument name").into());}
            let ty = if self.src.get(at) == Some(&b':') {at += 1; self.read_type_at(&mut at)?} else {Type::Any};
            if variadic {
                if !matches!(ty, Type::List(_)) {return Err(VimError::new(1012, "Variable arguments require a List type").into());}
                rest = Some((name, ty));
            } else {params.push(super::state::Param {name, default: None}); types.push(ty);}
            at = skip_white(self.src, at);
            if self.src.get(at) == Some(&b'=') {return Err(VimError::new(1172, "Cannot use default values in a lambda").into());}
            if self.src.get(at) == Some(&b',') {at += 1;}
            else if at != close - 1 {return Err(self.invalid());}
        }
        self.pos = body_at;
        let body = if self.peek() == b'{' {
            let end = delimited_end(self.src, self.pos, b'{', b'}').ok_or_else(||self.invalid())?;
            self.pos = end;
            super::state::FunctionBody::Lines(super::source::split_command_lines(&self.src[body_at + 1..end - 1], 1).into())
        } else {
            let saved = self.evaluate; self.evaluate = false;
            let parsed = self.eval1(); self.evaluate = saved; parsed?;
            super::state::FunctionBody::Lambda(VStr::from(&self.src[body_at..self.pos]))
        };
        if !self.evaluate {return Ok(Some(Value::Number(0)));}
        if self.constant {return Err(self.invalid());}
        let mut function = super::state::UserFunction {
            name: VStr::empty(), params, varargs: rest.is_some(), body,
            flags: super::state::FunctionFlags {abort: true, closure: true, ..Default::default()},
            sid: self.rt.current_sid(), def_lnum: 0, outer: self.rt.capture_scope(),
            signature: Some(super::vim9::Signature {generics: vec![], lexical_types: self.analysis.map_or_else(Default::default, |c|c.variables.clone()), bindings: self.rt.current_frame().and_then(|f|f.function.as_ref()).and_then(|f|f.signature.as_ref()).map_or_else(Default::default, |s|s.bindings.clone()), checked: Default::default(), parameters: types, rest, result: result_type, owner: self.analysis.and_then(|c|c.owner.clone()).or_else(||self.rt.current_class())}),
        };
        super::compiler::analyze_lambda(self.rt, self.host, &mut function, self.analysis.cloned().unwrap_or_default(), infer_result)?;
        if self.analysis.is_some() {
            return Ok(Some(Value::type_value(super::compiler::signature(&function))));
        }
        if self.constant {return Err(self.invalid());}
        let name = self.rt.state.next_lambda_name();
        let mut function = function; function.name = name.clone();
        let ty = super::compiler::signature(&function);
        self.rt.state.define_function(function)?;
        Ok(Some(Value::Funcref(Funcref::named(name).with_type(ty))))
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
        if self.constant {return Err(self.invalid());}
        let name = self.rt.define_lambda(params, varargs, body)?;
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
        if self.analyzing() {return Ok(Value::type_value(super::types::Type::Any));}
        if self.constant {return Err(self.invalid());}
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
        if self.analyzing() {return Ok(Value::type_value(super::types::Type::String));}
        if self.constant {return Err(self.invalid());}
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
        if self.rt.vim9() {return Err(VimError::new(1079, "Cannot use curly braces in a name").into());}
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
        // Legacy Vim permits whitespace between a named function and `(`.
        // Keep it untouched otherwise: adjacent expressions can be arguments
        // of :echo and :execute.
        if self.rt.vim9() && self.peek() == b'<' && self.pos > 0 && !is_white(self.src[self.pos - 1]) {
            let types = self.type_arguments()?;
            let call = self.peek() == b'(';
            let args = if call {self.call_arguments()?} else {vec![]};
            if !self.evaluate {return self.subscripts_after(Value::Number(0), None, call);}
            let reference = self.reference(&name)?;
            let reference = super::vim9::specialize(self.rt, &reference, types)?;
            let value = if call {self.invoke(&reference, args, None)?} else {Value::Funcref(reference)};
            return self.subscripts_after(value, None, call);
        }
        let after_name = skip_white(self.src, self.pos);
        if self.src.get(after_name) == Some(&b'(') {
            if self.rt.vim9() && after_name != self.pos {return Err(VimError::new(1068, "No white space allowed before (").into());}
            self.pos = after_name;
        }
        if self.peek() == b'(' {
            let args = self.call_arguments()?;
            if !self.evaluate {
                return self.subscripts_after(Value::Number(0), None, true);
            }
            if let Some(context) = self.analysis {let value = super::compiler::call(self.rt, context, &name, &args)?; return self.subscripts_after(value, None, true);}
            let funcref = self.reference(&name)?;
            let value = self.invoke(&funcref, args, None)?;
            return self.subscripts_after(value, None, true);
        }
        if !self.evaluate {
            return self.subscripts(Value::Number(0), None);
        }
        if self.analysis.is_some() {let value = self.variable(&name)?; return self.subscripts(value, None);}
        let value = match self.variable(&name) {
            Ok(value) => value,
            Err(Fail::Error(error)) if error.code == 121 => {
                // A function name used as a value (legacy allows bare names
                // only via function()); report the undefined variable.
                let _ = start;
                return Err(Fail::Error(error));
            }
            Err(other) => return Err(other),
        };
        if matches!(value, Value::Module(_)) && self.peek() != b'.' {return Err(VimError::new(1060, "Expected dot after import name").into());}
        self.subscripts(value, None)
    }

    pub(crate) fn type_arguments(&mut self) -> EvalResult<Vec<super::types::Type>> {
        self.pos += 1; let mut types = vec![];
        if self.src.get(skip_white(self.src, self.pos)) == Some(&b'>') {return Err(VimError::new(1555, "Empty type argument list").into());}
        loop {
            types.push(self.read_type()?); self.skip_white();
            if self.peek() == b'>' {self.pos += 1; break;}
            if self.peek() != b',' {return Err(VimError::new(1556, "Missing > after type arguments").into());} self.pos += 1;
        }
        Ok(types)
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
                b',' => self.comma()?,
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
                b'<' if self.rt.vim9() && self.pos > 0 && !is_white(self.src[self.pos - 1]) => {
                    let types = self.type_arguments()?;
                    if self.evaluate {
                        let Value::Funcref(reference) = &value else {return Err(VimError::new(1558, "Not a generic function").into());};
                        value = Value::Funcref(super::vim9::specialize(self.rt, reference, types)?);
                    }
                }
                b'.' if self.analyzing() && is_name_char(self.peek_at(1)) => {
                    self.pos += 1; let start = self.pos; while is_name_char(self.peek()) {self.pos += 1;}
                    value = self.analysis.unwrap().member(self.rt, &value, &self.src[start..self.pos])?;
                }
                b'[' => {
                    self_dict = None;
                    value = self.index(value)?;
                }
                b'.' if self.evaluate && matches!(value, Value::Class(_) | Value::Object(_)) && is_name_char(self.peek_at(1)) => {
                    self.pos += 1; let start = self.pos;
                    while is_name_char(self.peek()) {self.pos += 1;}
                    value = super::objects::member(self.rt, &value, &String::from_utf8_lossy(&self.src[start..self.pos]))?;
                    self_dict = None;
                }
                b'.' if self.evaluate && matches!(value, Value::Dict(_) | Value::Module(_)) && is_name_char(self.peek_at(1)) => {
                    self.pos += 1;
                    let start = self.pos;
                    while is_name_char(self.peek()) {
                        self.pos += 1;
                    }
                    let key = &self.src[start..self.pos];
                    let (Value::Dict(dict) | Value::Module(dict)) = &value else { unreachable!() };
                    dict.exported_member(key)?;
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
                    if self.analyzing() {
                        value = if let Value::Funcref(reference) = &value {self.invoke(reference, args, None)?}
                            else {super::compiler::invoke(self.rt, &super::types::Type::infer(&value), &args)?};
                    } else if self.evaluate {
                        let Value::Funcref(funcref) = &value else {
                            if let Value::String(name) = &value {
                                let funcref = self.reference(name.as_bytes())?;
                                value = self.invoke(&funcref, args, self_dict.take())?;
                                continue;
                            }
                            return Err(Fail::Error(VimError::new(109, "Not a Funcref")));
                        };
                        let funcref = funcref.clone();
                        value = self.invoke(&funcref, args, self_dict.take())?;
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
            return self.invoke(&funcref, args, None);
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
                Value::String(name) => self.reference(name.as_bytes())?,
                _ => return Err(Fail::Error(VimError::new(1275, "String or function required for ->(expr)"))),
            };
            return self.invoke(&funcref, args, None);
        }
        let Some(name) = self.parse_name()? else {
            return Err(Fail::Error(VimError::new(260, "Missing name after ->")));
        };
        // base->some.name(args) / base->alist[idx](args)
        let mut callee: Option<(Value, Option<Dict>)> = None;
        while matches!(self.peek(), b'.' | b'[') {
            let (current, _) = match callee.take() {
                Some(found) => found,
                None => (if self.evaluate { self.variable(&name)? } else { Value::Number(0) }, None),
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
                self.invoke(&funcref, all, dict)
            }
            Some((Value::String(callee_name), dict)) => {
                let funcref = self.reference(callee_name.as_bytes())?;
                let mut all = vec![base];
                all.extend(args);
                self.invoke(&funcref, all, dict)
            }
            Some(_) => Err(Fail::Error(VimError::new(109, "Not a Funcref"))),
            None => {
                // A variable holding a Funcref takes precedence for scoped or
                // capitalized names, like a direct call.
                let funcref = self.reference(&name)?;
                let position = if funcref.args().is_empty() && funcref.dict().is_none() {
                    super::builtins::method_base_position(funcref.name().as_bytes())?
                } else {
                    0
                };
                let mut all = args;
                let position = position.min(all.len());
                all.insert(position, base);
                self.invoke(&funcref, all, None)
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
            if self.rt.vim9() && (!self.src.get(self.pos.wrapping_sub(1)).is_some_and(|b|is_white(*b) || *b == b'[')
                || !self.src.get(self.pos + 1).is_some_and(|b|is_white(*b) || *b == b']')) {return Err(VimError::new(1004, "White space required before and after ':'").into());}
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
        if self.analyzing() {
            use super::types::Type;
            let ty = Type::infer(&value);
            let item = match ty {
                Type::List(t) => if range {Type::List(t)} else {*t},
                Type::Dict(t) => *t,
                Type::String => Type::String, Type::Blob => if range {Type::Blob} else {Type::Number},
                Type::Tuple(items, rest) => if range {Type::Tuple(items, rest)} else {
                    first.as_ref().and_then(|v|v.to_number().ok()).and_then(|n|items.get(if n < 0 {(items.len() as i64 + n) as usize} else {n as usize})).cloned().or(rest.map(|t|*t)).unwrap_or(Type::Any)
                },
                Type::Any => Type::Any, _ => return Err(VimError::new(689, "Cannot index this type").into()),
            }; return Ok(Value::type_value(item));
        }
        if self.rt.vim9() {
            if let Value::String(text) = &value {
                return super::vim9::string_index(text, first.as_ref(), second.as_ref(), range);
            }
        }
        if let Value::Tuple(items) = &value {
            let result = index_value(&Value::list(items.as_ref().clone()), first.as_ref(), second.as_ref(), range)?;
            return Ok(if let Value::List(items) = result {Value::Tuple(std::sync::Arc::new(items.snapshot()))} else {result});
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
                None => match self.variable(&name) {
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
                self.pos += 1;
                let key_start = self.pos;
                while is_name_char(self.peek()) {
                    self.pos += 1;
                }
                let key = &self.src[key_start..self.pos];
                let found = match &current {
                    Value::Dict(dict) | Value::Module(dict) => {
                        if matches!(current, Value::Module(_)) {dict.exported_member(key)?; self_dict = None;}
                        else {self_dict = Some(dict.clone());}
                        dict.get(key).ok_or_else(||VimError::new(716, format!("Key not present in Dictionary: \"{}\"", String::from_utf8_lossy(key))))?
                    }
                    Value::Class(_) | Value::Object(_) => {
                        self_dict = None;
                        super::objects::member(self.rt, &current, &String::from_utf8_lossy(key))?
                    }
                    _ => {self.pos = start; return Ok(None);}
                };
                target = Some(found);
            }
        }
        let specialization = if self.rt.vim9() && self.peek() == b'<' {Some(self.type_arguments()?)} else {None};
        self.skip_white();
        if self.peek() != b'(' {
            self.pos = start;
            return Ok(None);
        }
        let funcref = match target {
            Some(Value::Funcref(funcref)) => funcref,
            Some(Value::String(callee)) => self.reference(callee.as_bytes())?,
            Some(_) => return Err(Fail::Error(VimError::new(109, "Not a Funcref"))),
            None => self.reference(&name)?,
        };
        let funcref = if let Some(types) = specialization {super::vim9::specialize(self.rt, &funcref, types)?} else {funcref};
        let args = self.call_arguments()?;
        let funcref = match self_dict {
            Some(dict) if funcref.dict().is_none() || funcref.auto_bound() => {
                funcref.bind(funcref.args().to_vec(), Some(dict), true)
            }
            _ => funcref,
        };
        Ok(Some(Callee { funcref, args }))
    }
}

/// Find a balanced delimiter while preserving quoted strings as opaque bytes.
fn delimited_end(src: &[u8], start: usize, open: u8, close: u8) -> Option<usize> {
    let mut depth = 1; let mut at = start + 1; let mut quote = None;
    while at < src.len() {
        let byte = src[at];
        if let Some(delimiter) = quote {
            if byte == b'\\' && delimiter == b'"' {at += 2; continue;}
            if byte == delimiter {
                if delimiter == b'\'' && src.get(at + 1) == Some(&delimiter) {at += 2; continue;}
                quote = None;
            }
        } else if matches!(byte, b'\'' | b'"') {quote = Some(byte);}
        else if byte == open {depth += 1;}
        else if byte == close {depth -= 1; if depth == 0 {return Some(at + 1);}}
        at += 1;
    }
    None
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
