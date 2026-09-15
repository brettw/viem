//! The closed set of deterministic collection/string helpers allowed at load.
use super::{
    equal, BTreeMap, Expr, Parser, Setup, Value, VimPattern, VimRegexLimits, MAX_BINDINGS,
    MAX_VALUE_BYTES,
};

impl Setup<'_> {
    pub(super) fn collection_call(
        &mut self,
        name: &str,
        inputs: &[Expr],
        arguments: &BTreeMap<String, Value>,
        depth: usize,
    ) -> Result<Value, String> {
        let values = self.eval_values(inputs, arguments, depth)?;
        let result = match (name, values.as_slice()) {
            ("extend", [Value::List(left), Value::List(right)]) => {
                if left.len().saturating_add(right.len()) > MAX_BINDINGS {
                    return Err("syntax setup list item budget exceeded".into());
                }
                Value::List(left.iter().chain(right).cloned().collect())
            }
            ("extend", [Value::Dictionary(left), Value::Dictionary(right)]) => {
                let mut values = left.clone();
                values.extend(right.clone());
                Value::Dictionary(values)
            }
            ("extend", [Value::Dictionary(left), Value::Dictionary(right), Value::Text(mode)])
                if matches!(mode.as_str(), "keep" | "force" | "error") =>
            {
                let mut values = left.clone();
                for (key, value) in right {
                    if mode == "error" && values.contains_key(key) {
                        return Err("setup extend has a duplicate dictionary key".into());
                    }
                    if mode != "keep" || !values.contains_key(key) {
                        values.insert(key.clone(), value.clone());
                    }
                }
                Value::Dictionary(values)
            }
            ("add", [Value::List(items), value]) => {
                if items.len() >= MAX_BINDINGS {
                    return Err("syntax setup list item budget exceeded".into());
                }
                let mut items = items.clone();
                items.push(value.clone());
                Value::List(items)
            }
            ("insert", [Value::List(items), value]) => {
                let mut items = items.clone();
                items.insert(0, value.clone());
                Value::List(items)
            }
            ("map" | "filter", [Value::List(items), Value::Text(source)]) => {
                let expression = Parser::parse(source)?;
                let mut values = Vec::new();
                let mut bytes = 0usize;
                for (index, item) in items.iter().enumerate() {
                    self.charge(source.len())?;
                    let mut locals = arguments.clone();
                    locals.insert("v:key".into(), Value::Number(index as i64));
                    locals.insert("v:val".into(), item.clone());
                    let value = self.eval(&expression, &locals, depth + 1)?;
                    let selected = if name == "map" {
                        Some(value)
                    } else if value.truth()? {
                        Some(item.clone())
                    } else {
                        None
                    };
                    if let Some(value) = selected {
                        bytes = bytes.saturating_add(value.bytes());
                        values.push(value);
                    }
                    if bytes > MAX_VALUE_BYTES {
                        return Err("syntax setup mapped value byte budget exceeded".into());
                    }
                }
                Value::List(values)
            }
            _ => {
                return Err(format!(
                    "unsupported syntax collection function arguments: {name}"
                ))
            }
        };
        if result.bytes() > MAX_VALUE_BYTES {
            return Err("syntax setup collection byte budget exceeded".into());
        }
        if let Some(Expr::Variable(target)) = inputs.first() {
            if target.starts_with("a:")
                || target.starts_with("l:")
                || arguments.contains_key(&format!("l:{target}"))
            {
                return Err(
                    "mutating a function-local or argument collection is unsupported".into(),
                );
            }
            self.require_unshared_collection(target)?;
            self.assign_value(target, result.clone())?;
        } else if matches!(inputs.first(), Some(Expr::Index(..) | Expr::Member(..))) {
            return Err("mutation of nested setup collection references is unsupported".into());
        }
        self.charge(result.bytes())?;
        Ok(result)
    }
    pub(super) fn binary(
        &mut self,
        operator: &str,
        left: Value,
        right: Value,
    ) -> Result<Value, String> {
        let value = match operator {
            "&&" | "||" => Value::Number(i64::from(right.truth()?)),
            ".." | "." => Value::Text(format!("{}{}", left.text()?, right.text()?)),
            "+" if matches!((&left, &right), (Value::List(_), Value::List(_))) => {
                let (Value::List(mut left), Value::List(right)) = (left, right) else {
                    unreachable!()
                };
                left.extend(right);
                Value::List(left)
            }
            "+" | "-" | "*" | "/" | "%" => {
                let left = left.number()?;
                let right = right.number()?;
                let result = match operator {
                    "+" => left.checked_add(right),
                    "-" => left.checked_sub(right),
                    "*" => left.checked_mul(right),
                    "/" => left.checked_div(right),
                    _ => left.checked_rem(right),
                };
                Value::Number(result.ok_or("syntax setup arithmetic overflow or zero divisor")?)
            }
            "==" | "==#" => Value::Number(i64::from(equal(&left, &right)?)),
            "!=" | "!=#" => Value::Number(i64::from(!equal(&left, &right)?)),
            "==?" | "!=?" => {
                let same = match (&left, &right) {
                    (Value::Text(left), Value::Text(right)) => {
                        left.to_lowercase() == right.to_lowercase()
                    }
                    _ => equal(&left, &right)?,
                };
                Value::Number(i64::from(same ^ operator.starts_with('!')))
            }
            ">" | ">=" | "<" | "<=" => {
                let order = match (&left, &right) {
                    (Value::Text(left), Value::Text(right)) => left.cmp(right),
                    _ => left.number()?.cmp(&right.number()?),
                };
                Value::Number(i64::from(match operator {
                    ">" => order.is_gt(),
                    ">=" => order.is_ge(),
                    "<" => order.is_lt(),
                    _ => order.is_le(),
                }))
            }
            "=~#" | "!~#" | "=~?" | "!~?" | "=~" | "!~" => {
                let pattern = VimPattern::compile(
                    &right.text()?,
                    operator.ends_with('?'),
                    VimRegexLimits::default(),
                )?;
                let cancel = self.cancel;
                let found = pattern.is_match_text_with_control(
                    &left.text()?,
                    &mut self.fuel,
                    &mut || cancel.is_some_and(|c| c.load(super::Ordering::Relaxed)),
                )?;
                Value::Number(i64::from(found ^ operator.starts_with('!')))
            }
            _ => return Err("unsupported setup binary operator".into()),
        };
        if value.bytes() > MAX_VALUE_BYTES {
            return Err("syntax setup value byte budget exceeded".into());
        }
        Ok(value)
    }

    pub(super) fn lookup(&self, collection: &Value, key: &Value) -> Result<Option<Value>, String> {
        match (collection, key) {
            (Value::Scope(scope), Value::Text(key)) => {
                let name = format!("{scope}{key}");
                Ok(if scope == "s:" {
                    self.script.borrow().variables.get(&name).cloned()
                } else {
                    self.variables.get(&name).cloned()
                })
            }
            (Value::Dictionary(items), Value::Text(key)) => Ok(items.get(key).cloned()),
            (Value::List(items), Value::Number(index)) => {
                let index = if *index < 0 {
                    items.len() as i64 + index
                } else {
                    *index
                };
                Ok(usize::try_from(index)
                    .ok()
                    .and_then(|index| items.get(index))
                    .cloned())
            }
            // Vim's legacy string indexing is byte based. Invalid UTF-8 fragments
            // cannot enter this compiler's text representation.
            (Value::Text(text), Value::Number(index)) => {
                let Ok(index) = usize::try_from(*index) else {
                    return Ok(None);
                };
                if index >= text.len() {
                    return Ok(None);
                }
                let byte = text.as_bytes()[index];
                if !byte.is_ascii() {
                    return Err("setup byte indexing would split a Unicode character".into());
                }
                Ok(Some(Value::Text(char::from(byte).to_string())))
            }
            _ => Err("unsupported syntax setup index types".into()),
        }
    }

    pub(super) fn builtin(
        &mut self,
        name: &str,
        values: Vec<Value>,
        depth: usize,
    ) -> Result<Value, String> {
        let out = match (name, values.as_slice()) {
            ("and" | "or" | "xor", [left, right]) => {
                let left = left.number()?;
                let right = right.number()?;
                Value::Number(match name {
                    "and" => left & right,
                    "or" => left | right,
                    _ => left ^ right,
                })
            }
            ("invert", [value]) => Value::Number(!value.number()?),
            ("copy" | "deepcopy", [value]) => value.clone(),
            ("string", [value]) => Value::Text(literal(value)?),
            ("get", [collection, key]) => self.lookup(collection, key)?.unwrap_or(Value::Number(0)),
            ("get", [collection, key, default]) => self
                .lookup(collection, key)?
                .unwrap_or_else(|| default.clone()),
            ("getline", [Value::Number(line)]) => {
                if *line <= 0 {
                    return Ok(Value::Text(String::new()));
                }
                if !(1..=32).contains(line) {
                    return Err("setup getline is confined to the first 32 supplied lines".into());
                }
                Value::Text(
                    self.prefix
                        .split('\n')
                        .nth(*line as usize - 1)
                        .unwrap_or("")
                        .into(),
                )
            }
            ("len", [Value::Text(text)]) | ("strlen", [Value::Text(text)]) => {
                Value::Number(text.len() as i64)
            }
            ("len", [Value::List(items)]) => Value::Number(items.len() as i64),
            ("len", [Value::Dictionary(items)]) => Value::Number(items.len() as i64),
            ("empty", [value]) => Value::Number(i64::from(match value {
                Value::Number(n) => *n == 0,
                Value::Text(s) => s.is_empty(),
                Value::List(v) => v.is_empty(),
                Value::Dictionary(v) => v.is_empty(),
                _ => return Err("unsupported setup empty() type".into()),
            })),
            ("type", [value]) => Value::Number(match value {
                Value::Number(_) => 0,
                Value::Text(_) => 1,
                Value::List(_) => 3,
                Value::Dictionary(_) | Value::Scope(_) => 4,
            }),
            ("has_key", [collection, key]) => {
                Value::Number(i64::from(self.lookup(collection, key)?.is_some()))
            }
            ("keys" | "values" | "items", [Value::Dictionary(items)]) => Value::List(
                items
                    .iter()
                    .map(|(key, value)| match name {
                        "keys" => Value::Text(key.clone()),
                        "values" => value.clone(),
                        _ => Value::List(vec![Value::Text(key.clone()), value.clone()]),
                    })
                    .collect(),
            ),
            ("tolower", [Value::Text(text)]) => Value::Text(text.to_lowercase()),
            ("toupper", [Value::Text(text)]) => Value::Text(text.to_uppercase()),
            ("escape", [Value::Text(text), Value::Text(characters)]) => {
                let mut result = String::new();
                for ch in text.chars() {
                    if characters.contains(ch) {
                        result.push('\\');
                    }
                    result.push(ch);
                }
                Value::Text(result)
            }
            ("stridx", [Value::Text(text), Value::Text(needle)]) => {
                Value::Number(text.find(needle).map_or(-1, |n| n as i64))
            }
            ("min" | "max", [Value::List(items)]) => {
                let values = items
                    .iter()
                    .map(Value::number)
                    .collect::<Result<Vec<_>, _>>()?;
                Value::Number(
                    if name == "min" {
                        values.into_iter().min()
                    } else {
                        values.into_iter().max()
                    }
                    .unwrap_or(0),
                )
            }
            ("range", [Value::Number(end)]) => {
                bounded_range(0, end.checked_sub(1).ok_or("setup range overflow")?, 1)?
            }
            ("range", [Value::Number(start), Value::Number(end)]) => {
                bounded_range(*start, *end, 1)?
            }
            ("range", [Value::Number(start), Value::Number(end), Value::Number(step)]) => {
                bounded_range(*start, *end, *step)?
            }
            ("split", [Value::Text(text)]) => Value::List(
                text.split(|c: char| c.is_ascii() && c <= ' ')
                    .filter(|s| !s.is_empty())
                    .map(|s| Value::Text(s.into()))
                    .collect(),
            ),
            ("split", [Value::Text(text), Value::Text(pattern)]) => {
                self.split(text, pattern, false)?
            }
            ("split", [Value::Text(text), Value::Text(pattern), keep]) => {
                self.split(text, pattern, keep.truth()?)?
            }
            ("join", [Value::List(items)]) => self.call(
                "join",
                vec![Value::List(items.clone()), Value::Text(" ".into())],
                0,
            )?,
            ("repeat", [Value::Text(text), Value::Number(count)]) => {
                let count = usize::try_from(*count).unwrap_or(0);
                if text.len().saturating_mul(count) > MAX_VALUE_BYTES {
                    return Err("syntax setup repeat byte budget exceeded".into());
                }
                Value::Text(text.repeat(count))
            }
            ("strpart", [Value::Text(text), Value::Number(start), Value::Number(length)]) => {
                let end = start.saturating_add(*length).clamp(0, text.len() as i64) as usize;
                let start = (*start).clamp(0, text.len() as i64) as usize;
                Value::Text(
                    if end < start {
                        ""
                    } else {
                        text.get(start..end)
                            .ok_or("setup strpart splits a Unicode character")?
                    }
                    .into(),
                )
            }
            ("tr", [Value::Text(text), Value::Text(from), Value::Text(to)]) => {
                let to: Vec<_> = to.chars().collect();
                let from: Vec<_> = from.chars().collect();
                if from.len() != to.len() {
                    return Err("setup tr requires equally sized character sets".into());
                }
                Value::Text(
                    text.chars()
                        .map(|ch| {
                            from.iter()
                                .position(|c| *c == ch)
                                .map_or(ch, |index| to[index])
                        })
                        .collect(),
                )
            }
            (
                "match" | "matchend" | "matchstr" | "matchlist",
                [Value::Text(text), Value::Text(pattern)],
            ) => {
                let pattern = VimPattern::compile(pattern, false, VimRegexLimits::default())?;
                let cancel = self.cancel;
                let found = pattern.find_text_with_control(text, 0, &mut self.fuel, &mut || {
                    cancel.is_some_and(|c| c.load(super::Ordering::Relaxed))
                })?;
                match name {
                    "match" => Value::Number(found.as_ref().map_or(-1, |m| m.start as i64)),
                    "matchend" => Value::Number(found.as_ref().map_or(-1, |m| m.end as i64)),
                    "matchstr" => {
                        Value::Text(found.as_ref().map_or("", |m| &text[m.start..m.end]).into())
                    }
                    _ => Value::List(found.as_ref().map_or_else(Vec::new, |m| {
                        std::iter::once(Some(m.start..m.end))
                            .chain(m.captures.iter().skip(1).cloned())
                            .take(10)
                            .map(|r| Value::Text(r.map_or("", |r| &text[r]).into()))
                            .collect()
                    })),
                }
            }
            (
                "substitute",
                [Value::Text(text), Value::Text(pattern), Value::Text(replacement), Value::Text(flags)],
            ) => self.substitute(text, pattern, replacement, flags, depth)?,
            _ => self.environment_builtin(name, &values)?,
        };
        if out.bytes() > MAX_VALUE_BYTES {
            return Err("syntax setup value byte budget exceeded".into());
        }
        self.charge(out.bytes())?;
        Ok(out)
    }

    fn split(&mut self, text: &str, source: &str, keep: bool) -> Result<Value, String> {
        let pattern = VimPattern::compile(
            if source.is_empty() {
                "[\\x01- ]\\+"
            } else {
                source
            },
            false,
            VimRegexLimits::default(),
        )?;
        let mut items = Vec::new();
        let mut begin = 0;
        let mut search = 0;
        let cancel = self.cancel;
        while search <= text.len() {
            let Some(found) =
                pattern.find_text_with_control(text, search, &mut self.fuel, &mut || {
                    cancel.is_some_and(|c| c.load(super::Ordering::Relaxed))
                })?
            else {
                break;
            };
            if found.start > begin || keep {
                items.push(Value::Text(text[begin..found.start].into()));
            }
            begin = found.end;
            if items.len() > MAX_BINDINGS {
                return Err("syntax setup split item budget exceeded".into());
            }
            search = if found.end > found.start {
                found.end
            } else if let Some(ch) = text[found.end..].chars().next() {
                found.end + ch.len_utf8()
            } else {
                break;
            };
        }
        if begin < text.len() || keep {
            items.push(Value::Text(text[begin..].into()));
        }
        Ok(Value::List(items))
    }

    fn substitute(
        &mut self,
        text: &str,
        source: &str,
        replacement: &str,
        flags: &str,
        depth: usize,
    ) -> Result<Value, String> {
        if !matches!(flags, "" | "g") {
            return Err(
                "setup substitute supports only literal replacement and optional g flag".into(),
            );
        }
        let expression = replacement
            .strip_prefix("\\=")
            .map(Parser::parse)
            .transpose()?;
        let pattern = VimPattern::compile(source, false, VimRegexLimits::default())?;
        let cancel = self.cancel;
        let mut result = String::new();
        let mut begin = 0;
        let mut search = 0;
        while search <= text.len() {
            let Some(found) =
                pattern.find_text_with_control(text, search, &mut self.fuel, &mut || {
                    cancel.is_some_and(|c| c.load(super::Ordering::Relaxed))
                })?
            else {
                break;
            };
            result.push_str(&text[begin..found.start]);
            if let Some(expression) = &expression {
                let mut arguments = BTreeMap::new();
                for index in 0..10 {
                    let text = if index == 0 {
                        &text[found.start..found.end]
                    } else {
                        found
                            .captures
                            .get(index)
                            .and_then(Option::as_ref)
                            .map_or("", |range| &text[range.clone()])
                    };
                    arguments.insert(format!("v:setup_submatch{index}"), Value::Text(text.into()));
                }
                let value = self.eval(expression, &arguments, depth + 1)?.text()?;
                result.push_str(&value);
                if result.len() > MAX_VALUE_BYTES {
                    return Err("syntax setup substitution byte budget exceeded".into());
                }
            } else {
                let mut chars = replacement.chars();
                while let Some(ch) = chars.next() {
                    if ch == '&' {
                        result.push_str(&text[found.start..found.end]);
                    } else if ch == '\\' {
                        let next = chars
                            .next()
                            .ok_or("unterminated setup substitute replacement escape")?;
                        match next {
                            '0' => result.push_str(&text[found.start..found.end]),
                            '1'..='9' => {
                                if let Some(Some(range)) =
                                    found.captures.get(next as usize - '0' as usize)
                                {
                                    result.push_str(&text[range.clone()]);
                                }
                            }
                            '&' | '\\' => result.push(next),
                            'r' => result.push('\r'),
                            'n' => result.push('\n'),
                            't' => result.push('\t'),
                            _ => {
                                return Err(format!(
                                    "unsupported setup substitute replacement escape: \\{next}"
                                ))
                            }
                        }
                    } else {
                        result.push(ch);
                    }
                    if result.len() > MAX_VALUE_BYTES {
                        return Err("syntax setup substitution byte budget exceeded".into());
                    }
                }
            }
            begin = found.end;
            if flags != "g" {
                break;
            }
            search = if found.end > found.start {
                found.end
            } else if let Some(ch) = text[found.end..].chars().next() {
                found.end + ch.len_utf8()
            } else {
                break;
            };
        }
        result.push_str(&text[begin..]);
        Ok(Value::Text(result))
    }
}

fn bounded_range(start: i64, end: i64, step: i64) -> Result<Value, String> {
    if step == 0 {
        return Err("setup range step is zero".into());
    }
    let mut values = Vec::new();
    let mut next = start;
    while if step > 0 { next <= end } else { next >= end } {
        if values.len() >= MAX_BINDINGS {
            return Err("syntax setup range item budget exceeded".into());
        }
        values.push(Value::Number(next));
        let Some(value) = next.checked_add(step) else {
            break;
        };
        next = value;
    }
    Ok(Value::List(values))
}

pub(super) fn slice(value: Value, start: Option<i64>, end: Option<i64>) -> Result<Value, String> {
    let len = match &value {
        Value::List(values) => values.len(),
        Value::Text(text) => text.len(),
        _ => return Err("setup slicing requires a list or string".into()),
    };
    let normalize = |value: i64| {
        if value < 0 {
            (len as i64).saturating_add(value)
        } else {
            value
        }
    };
    let start = normalize(start.unwrap_or(0)).clamp(0, len as i64) as usize;
    let end = normalize(end.unwrap_or(-1))
        .saturating_add(1)
        .clamp(0, len as i64) as usize;
    let end = end.max(start);
    Ok(match value {
        Value::List(values) => Value::List(values[start..end].to_vec()),
        Value::Text(text) => Value::Text(
            text.get(start..end)
                .ok_or("setup byte slice splits a Unicode character")?
                .into(),
        ),
        _ => unreachable!(),
    })
}

fn literal(value: &Value) -> Result<String, String> {
    let result = match value {
        Value::Number(value) => value.to_string(),
        Value::Text(value) => format!("'{}'", value.replace('\'', "''")),
        Value::List(values) => format!(
            "[{}]",
            values
                .iter()
                .map(literal)
                .collect::<Result<Vec<_>, _>>()?
                .join(", ")
        ),
        Value::Dictionary(values) => format!(
            "{{{}}}",
            values
                .iter()
                .map(|(key, value)| Ok(format!(
                    "'{}': {}",
                    key.replace('\'', "''"),
                    literal(value)?
                )))
                .collect::<Result<Vec<_>, String>>()?
                .join(", ")
        ),
        Value::Scope(_) => return Err("setup string() requires a concrete value".into()),
    };
    if result.len() > MAX_VALUE_BYTES {
        return Err("syntax setup literal byte budget exceeded".into());
    }
    Ok(result)
}
