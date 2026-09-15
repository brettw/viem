//! Bounded integer and byte-string printf conversions used by syntax setup.
use super::{Value, MAX_VALUE_BYTES};

pub(super) fn format(source: &str, values: &[Value]) -> Result<String, String> {
    let mut chars = source.chars().peekable();
    let mut arguments = values.iter();
    let mut output = String::new();
    while let Some(ch) = chars.next() {
        if ch != '%' {
            if output.len().saturating_add(ch.len_utf8()) > MAX_VALUE_BYTES {
                return Err("setup printf output byte budget exceeded".into());
            }
            output.push(ch);
            continue;
        }
        if chars.peek() == Some(&'%') {
            chars.next();
            append(&mut output, "%")?;
            continue;
        }
        let (mut left, mut zero, mut plus, mut space, mut alternate) =
            (false, false, false, false, false);
        while let Some(flag) = chars.peek().copied() {
            match flag {
                '-' => left = true,
                '0' => zero = true,
                '+' => plus = true,
                ' ' => space = true,
                '#' => alternate = true,
                _ => break,
            }
            chars.next();
        }
        let width = count(&mut chars)?;
        let precision = if chars.peek() == Some(&'.') {
            chars.next();
            Some(count(&mut chars)?)
        } else {
            None
        };
        let kind = chars.next().ok_or("unterminated setup printf conversion")?;
        if !matches!(kind, 's' | 'd' | 'b' | 'B' | 'o' | 'x' | 'X' | 'c') {
            return Err(format!("unsupported setup printf conversion: {kind}"));
        }
        let value = arguments.next().ok_or("missing setup printf argument")?;
        let mut prefix = String::new();
        let text = if kind == 's' {
            let mut text = value.text()?;
            if let Some(precision) = precision.filter(|n| *n < text.len()) {
                if !text.is_char_boundary(precision) {
                    return Err("setup printf precision splits a Unicode character".into());
                }
                text.truncate(precision);
            }
            text
        } else if kind == 'c' {
            let number = value.number()?;
            if !(1..=127).contains(&number) {
                return Err("setup printf %c requires a non-NUL ASCII byte".into());
            }
            char::from(number as u8).to_string()
        } else {
            let number = value.number()?;
            let mut digits = match kind {
                'd' => {
                    if number < 0 {
                        prefix.push('-');
                    } else if plus {
                        prefix.push('+');
                    } else if space {
                        prefix.push(' ');
                    }
                    number.unsigned_abs().to_string()
                }
                'b' | 'B' => format!("{:b}", number as u64),
                'o' => format!("{:o}", number as u64),
                'x' => format!("{:x}", number as u64),
                'X' => format!("{:X}", number as u64),
                _ => unreachable!(),
            };
            if precision == Some(0) && number == 0 {
                digits.clear();
            }
            if alternate && number != 0 {
                prefix.push_str(match kind {
                    'b' => "0b",
                    'B' => "0B",
                    'x' => "0x",
                    'X' => "0X",
                    _ => "",
                });
            }
            let minimum = precision.unwrap_or(0).max(
                digits.len()
                    + usize::from(
                        alternate && kind == 'o' && !digits.starts_with('0') && !digits.is_empty(),
                    ),
            );
            if minimum > digits.len() {
                digits = "0".repeat(minimum - digits.len()) + &digits;
            }
            digits
        };
        let padding = width.saturating_sub(prefix.len() + text.len());
        if output
            .len()
            .saturating_add(prefix.len())
            .saturating_add(text.len())
            .saturating_add(padding)
            > MAX_VALUE_BYTES
        {
            return Err("setup printf output byte budget exceeded".into());
        }
        if left {
            output.push_str(&prefix);
            output.push_str(&text);
            output.extend(std::iter::repeat_n(' ', padding));
        } else if zero && (precision.is_none() || matches!(kind, 's' | 'c')) {
            output.push_str(&prefix);
            output.extend(std::iter::repeat_n('0', padding));
            output.push_str(&text);
        } else {
            output.extend(std::iter::repeat_n(' ', padding));
            output.push_str(&prefix);
            output.push_str(&text);
        }
    }
    if arguments.next().is_some() {
        return Err("too many setup printf arguments".into());
    }
    Ok(output)
}

fn count(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) -> Result<usize, String> {
    let mut value = 0usize;
    while let Some(digit) = chars.peek().and_then(|ch| ch.to_digit(10)) {
        chars.next();
        value = value
            .checked_mul(10)
            .and_then(|v| v.checked_add(digit as usize))
            .filter(|v| *v <= MAX_VALUE_BYTES)
            .ok_or("setup printf width or precision budget exceeded")?;
    }
    Ok(value)
}

fn append(output: &mut String, value: &str) -> Result<(), String> {
    if output.len().saturating_add(value.len()) > MAX_VALUE_BYTES {
        return Err("setup printf output byte budget exceeded".into());
    }
    output.push_str(value);
    Ok(())
}
