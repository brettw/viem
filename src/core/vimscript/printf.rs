//! `printf()` with Vim's conversions: `%d %i %u %x %X %o %b %B %c %s %S %e
//! %E %f %F %g %G %%`, flags `-+ 0#`, width, precision, `*` and `%N$`.
use super::builtins::MAX_STRING_BYTES;
use super::error::{EvalResult, Fail, VimError};
use super::value::{format_float_precision, Value};

const MAX_FIELD_SIZE: usize = 1_000_000;

fn too_large() -> Fail {
    Fail::Interrupt("printf() result or field too large".into())
}

fn field_size(size: u64) -> EvalResult<usize> {
    if size > MAX_FIELD_SIZE as u64 { Err(too_large()) } else { Ok(size as usize) }
}

fn parse_size(digits: &[u8]) -> EvalResult<usize> {
    if digits.is_empty() {
        return Ok(0);
    }
    let size = String::from_utf8_lossy(digits).parse::<u64>().map_err(|_| too_large())?;
    field_size(size)
}

fn append_output(out: &mut Vec<u8>, field: &[u8]) -> EvalResult<()> {
    if field.len() > MAX_STRING_BYTES.saturating_sub(out.len()) {
        return Err(too_large());
    }
    out.extend_from_slice(field);
    Ok(())
}

pub fn format(spec: &[u8], args: &[Value]) -> EvalResult<Vec<u8>> {
    let mut out = Vec::new();
    let mut next_arg = 0usize;
    let mut at = 0;
    let take = |position: Option<usize>, next_arg: &mut usize| -> EvalResult<Value> {
        let index = match position {
            Some(position) => position,
            None => {
                let index = *next_arg;
                *next_arg += 1;
                index
            }
        };
        args.get(index).cloned().ok_or_else(|| Fail::Error(VimError::new(766, "Insufficient arguments for printf()")))
    };
    while at < spec.len() {
        let byte = spec[at];
        if byte != b'%' {
            append_output(&mut out, &[byte])?;
            at += 1;
            continue;
        }
        at += 1;
        if spec.get(at) == Some(&b'%') {
            append_output(&mut out, b"%")?;
            at += 1;
            continue;
        }
        // %N$ positional argument.
        let mut position = None;
        let digits_start = at;
        while spec.get(at).is_some_and(u8::is_ascii_digit) {
            at += 1;
        }
        if at > digits_start && spec.get(at) == Some(&b'$') {
            let index: usize = String::from_utf8_lossy(&spec[digits_start..at]).parse().unwrap_or(1);
            position = Some(index.saturating_sub(1));
            at += 1;
        } else {
            at = digits_start;
        }
        let mut left = false;
        let mut plus = false;
        let mut space = false;
        let mut zero = false;
        let mut alternate = false;
        while let Some(&flag) = spec.get(at) {
            match flag {
                b'-' => left = true,
                b'+' => plus = true,
                b' ' => space = true,
                b'0' => zero = true,
                b'#' => alternate = true,
                b'\'' => {}
                _ => break,
            }
            at += 1;
        }
        let mut width: Option<usize> = None;
        if spec.get(at) == Some(&b'*') {
            at += 1;
            let value = take(None, &mut next_arg)?.to_number()?;
            if value < 0 {
                left = true;
            }
            width = Some(field_size(value.unsigned_abs())?);
        } else {
            let start = at;
            while spec.get(at).is_some_and(u8::is_ascii_digit) {
                at += 1;
            }
            if at > start {
                width = Some(parse_size(&spec[start..at])?);
            }
        }
        let mut precision: Option<usize> = None;
        if spec.get(at) == Some(&b'.') {
            at += 1;
            if spec.get(at) == Some(&b'*') {
                at += 1;
                let value = take(None, &mut next_arg)?.to_number()?;
                precision = if value >= 0 { Some(field_size(value as u64)?) } else { None };
            } else {
                let start = at;
                while spec.get(at).is_some_and(u8::is_ascii_digit) {
                    at += 1;
                }
                precision = Some(parse_size(&spec[start..at])?);
            }
        }
        // Length modifiers are accepted and ignored.
        while matches!(spec.get(at), Some(b'h' | b'l' | b'L' | b'q' | b'j' | b'z' | b't')) {
            at += 1;
        }
        let Some(&conversion) = spec.get(at) else {
            append_output(&mut out, b"%")?;
            break;
        };
        at += 1;
        let field: Vec<u8> = match conversion {
            b'd' | b'i' | b'u' | b'x' | b'X' | b'o' | b'b' | b'B' => {
                let value = take(position, &mut next_arg)?;
                let number = match value {
                    Value::Float(_) => return Err(Fail::Error(VimError::new(805, "Using a Float as a Number"))),
                    other => other.to_number()?,
                };
                let negative = number < 0 && matches!(conversion, b'd' | b'i');
                let magnitude = if matches!(conversion, b'd' | b'i') { number.unsigned_abs() } else { number as u64 };
                let mut digits = match conversion {
                    b'x' => format!("{magnitude:x}"),
                    b'X' => format!("{magnitude:X}"),
                    b'o' => format!("{magnitude:o}"),
                    b'b' | b'B' => format!("{magnitude:b}"),
                    _ => magnitude.to_string(),
                };
                if let Some(precision) = precision {
                    if digits.len() < precision {
                        let mut padded = String::with_capacity(precision);
                        padded.extend(std::iter::repeat_n('0', precision - digits.len()));
                        padded.push_str(&digits);
                        digits = padded;
                    }
                    if precision == 0 && magnitude == 0 {
                        digits.clear();
                    }
                }
                let prefix = match (conversion, alternate && magnitude != 0) {
                    (b'x', true) => "0x",
                    (b'X', true) => "0X",
                    (b'o', true) if !digits.starts_with('0') => "0",
                    (b'b', true) => "0b",
                    (b'B', true) => "0B",
                    _ => "",
                };
                let sign = if negative { "-" } else if plus && matches!(conversion, b'd' | b'i') { "+" } else if space && matches!(conversion, b'd' | b'i') { " " } else { "" };
                pad_number(sign, prefix, &digits, width, left, zero && precision.is_none())
            }
            b'c' => {
                // Vim converts the Number to a single byte.
                let value = take(position, &mut next_arg)?.to_number()?;
                pad(vec![value as u8], width, left, false)
            }
            b's' | b'S' => {
                let value = take(position, &mut next_arg)?;
                let mut text = match &value {
                    Value::String(text) => text.as_bytes().to_vec(),
                    Value::List(_) | Value::Dict(_) | Value::Funcref(_) | Value::Blob(_) => value.string_bytes(),
                    other => other.to_bytes()?,
                };
                if text.len() > MAX_STRING_BYTES {
                    return Err(too_large());
                }
                if let Some(precision) = precision {
                    // %s counts bytes, %S characters (both truncate to whole characters).
                    let mut end = 0;
                    let mut count = 0;
                    let as_text = String::from_utf8_lossy(&text).into_owned();
                    for (index, c) in as_text.char_indices() {
                        let size = if conversion == b'S' { 1 } else { c.len_utf8() };
                        if count + size > precision {
                            break;
                        }
                        count += size;
                        end = index + c.len_utf8();
                    }
                    text = as_text.as_bytes()[..end].to_vec();
                }
                let display = |text: &[u8]| -> usize {
                    if conversion == b'S' {
                        String::from_utf8_lossy(text).chars().count()
                    } else {
                        unicode_width::UnicodeWidthStr::width(String::from_utf8_lossy(text).as_ref())
                    }
                };
                let actual = display(&text);
                match width {
                    Some(width) if width > actual => {
                        if text.len().saturating_add(width - actual) > MAX_STRING_BYTES {
                            return Err(too_large());
                        }
                        let padding = vec![b' '; width - actual];
                        if left {
                            let mut text = text;
                            text.extend(padding);
                            text
                        } else {
                            let mut padded = padding;
                            padded.extend(text);
                            padded
                        }
                    }
                    _ => text,
                }
            }
            b'e' | b'E' | b'f' | b'F' | b'g' | b'G' => {
                let value = take(position, &mut next_arg)?;
                let float = match value {
                    Value::String(_) => return Err(Fail::Error(VimError::new(892, "Using a String as a Float"))),
                    other => other.to_float()?,
                };
                let precision = precision.unwrap_or(6);
                let upper = conversion.is_ascii_uppercase();
                let mut text = if float.is_nan() || float.is_infinite() {
                    format_float_precision(float, precision, upper)
                } else {
                    match conversion {
                        b'f' | b'F' => format!("{float:.precision$}"),
                        b'e' | b'E' => c_exponent(float, precision, upper),
                        // Vim's own %g, as :echo uses: fixed notation for
                        // 0.001 <= |f| < 1e7, trailing zeroes removed.
                        _ => format_float_precision(float, precision, upper),
                    }
                };
                if float.is_sign_positive() && !float.is_nan() {
                    if plus {
                        text.insert(0, '+');
                    } else if space {
                        text.insert(0, ' ');
                    }
                }
                let (sign, digits) = match text.strip_prefix(['-', '+', ' ']) {
                    Some(rest) => (&text[..1], rest.to_string()),
                    None => ("", text.clone()),
                };
                pad_number(sign, "", &digits, width, left, zero && float.is_finite())
            }
            other => {
                return Err(Fail::Error(VimError::new(767, format!("Too many arguments for printf(): {}", other as char))));
            }
        };
        append_output(&mut out, &field)?;
    }
    if next_arg < args.len() && !spec.contains(&b'$') {
        return Err(Fail::Error(VimError::new(767, "Too many arguments for printf()")));
    }
    Ok(out)
}

fn pad(text: Vec<u8>, width: Option<usize>, left: bool, zero: bool) -> Vec<u8> {
    let Some(width) = width else { return text };
    if text.len() >= width {
        return text;
    }
    let fill = vec![if zero { b'0' } else { b' ' }; width - text.len()];
    if left {
        let mut text = text;
        text.extend(fill);
        text
    } else {
        let mut padded = fill;
        padded.extend(text);
        padded
    }
}

fn pad_number(sign: &str, prefix: &str, digits: &str, width: Option<usize>, left: bool, zero: bool) -> Vec<u8> {
    let length = sign.len() + prefix.len() + digits.len();
    let width = width.unwrap_or(0);
    if length >= width {
        return format!("{sign}{prefix}{digits}").into_bytes();
    }
    let fill = width - length;
    if left {
        format!("{sign}{prefix}{digits}{}", " ".repeat(fill)).into_bytes()
    } else if zero {
        format!("{sign}{prefix}{}{digits}", "0".repeat(fill)).into_bytes()
    } else {
        format!("{}{sign}{prefix}{digits}", " ".repeat(fill)).into_bytes()
    }
}

/// C's `%e`: mantissa with `precision` digits and an exponent of at least
/// two digits with a sign.
fn c_exponent(value: f64, precision: usize, upper: bool) -> String {
    let formatted = format!("{value:.precision$e}");
    let (mantissa, exponent) = formatted.split_once('e').unwrap_or((&formatted, "0"));
    let exponent: i32 = exponent.parse().unwrap_or(0);
    let sign = if exponent < 0 { '-' } else { '+' };
    format!("{mantissa}{}{sign}{:02}", if upper { 'E' } else { 'e' }, exponent.unsigned_abs())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fmt(spec: &str, args: Vec<Value>) -> String {
        String::from_utf8(format(spec.as_bytes(), &args).unwrap()).unwrap()
    }

    #[test]
    fn field_limits_reject_overflow_before_allocating() {
        for spec in ["%999999999999999999999999999d", "%.99999999999999999999999999f", "%1000001d", "%.1000001s"] {
            assert!(matches!(format(spec.as_bytes(), &[1.into()]), Err(Fail::Interrupt(_))));
        }
        for width in [i64::MIN, i64::MAX] {
            assert!(matches!(format(b"%*d", &[width.into(), 1.into()]), Err(Fail::Interrupt(_))));
        }
        assert!(matches!(format(b"%.*f", &[i64::MAX.into(), Value::Float(1.0)]), Err(Fail::Interrupt(_))));
        let digits = format(b"%.100000d", &[42.into()]).unwrap();
        assert_eq!(digits.len(), 100_000);
        assert!(digits[..99_998].iter().all(|byte| *byte == b'0'));
        assert_eq!(&digits[99_998..], b"42");
    }

    #[test]
    fn conversions_match_vim() {
        assert_eq!(fmt("%d", vec![42.into()]), "42");
        assert_eq!(fmt("%5d|%-5d|%05d", vec![42.into(), 42.into(), 42.into()]), "   42|42   |00042");
        assert_eq!(fmt("%x %X %o %b", vec![255.into(), 255.into(), 8.into(), 5.into()]), "ff FF 10 101");
        assert_eq!(fmt("%#x", vec![255.into()]), "0xff");
        assert_eq!(fmt("%s-%s", vec!["a".into(), Value::list(vec![1.into()])]), "a-[1]");
        assert_eq!(fmt("%.2s", vec!["abcdef".into()]), "ab");
        assert_eq!(fmt("%.3f", vec![Value::Float(3.14159)]), "3.142");
        assert_eq!(fmt("%e", vec![Value::Float(1234.5)]), "1.234500e+03");
        assert_eq!(fmt("%g", vec![Value::Float(0.0001)]), "1.0e-4");
        assert_eq!(fmt("%g", vec![Value::Float(100000.0)]), "100000.0");
        assert_eq!(fmt("%g", vec![Value::Float(1000000.0)]), "1000000.0");
        assert_eq!(fmt("%g", vec![Value::Float(123456789.0)]), "1.234568e8");
        assert_eq!(fmt("%.0f|%5.1f", vec![Value::Float(2.5), Value::Float(-2.25)]), "2| -2.2");
        assert_eq!(fmt("%c", vec![65.into()]), "A");
        assert_eq!(fmt("%*d", vec![4.into(), 7.into()]), "   7");
        assert_eq!(fmt("%2$s %1$s", vec!["a".into(), "b".into()]), "b a");
        assert_eq!(fmt("100%%", vec![]), "100%");
    }
}
