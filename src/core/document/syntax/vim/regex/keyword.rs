//! The syntax program's portable keyword environment. Option declarations are
//! evaluated at load time; every compiled pattern owns its exact environment.

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VimKeyword {
    bytes: [bool; 256],
    alphabetic: bool,
}

impl Default for VimKeyword {
    fn default() -> Self {
        Self::parse("@,48-57,_,192-255").expect("valid native keyword default")
    }
}

impl VimKeyword {
    pub fn parse(option: &str) -> Result<Self, String> {
        if option.len() > 8192 {
            return Err("Vim keyword option byte budget exceeded".into());
        }
        let mut result = Self {
            bytes: [false; 256],
            alphabetic: false,
        };
        let chars: Vec<char> = option.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            let include = !(chars[i] == '^' && i + 1 < chars.len());
            i += usize::from(!include);
            let begin = i;
            let first = endpoint(&chars, &mut i)?;
            let last = if chars.get(i) == Some(&'-') && i + 1 < chars.len() {
                i += 1;
                endpoint(&chars, &mut i)?
            } else {
                first
            };
            if first > last {
                return Err("inverted Vim keyword character range".into());
            }
            if chars.get(i).is_some_and(|c| *c != ',') {
                return Err("invalid Vim keyword option separator".into());
            }
            if chars[begin] == '@' && i == begin + 1 {
                result.alphabetic = include;
                for (code, member) in result.bytes.iter_mut().enumerate() {
                    if char::from_u32(code as u32).is_some_and(char::is_alphabetic) {
                        *member = include;
                    }
                }
            } else {
                result.bytes[first..=last].fill(include);
            }
            if i < chars.len() {
                i += 1;
                if i == chars.len() {
                    return Err("missing Vim keyword option item".into());
                }
            }
        }
        Ok(result)
    }

    pub fn contains(&self, c: char) -> bool {
        if c == '\n' {
            return false;
        }
        self.bytes
            .get(c as usize)
            .copied()
            .unwrap_or_else(|| self.alphabetic && c.is_alphabetic())
    }

    pub(super) fn class(&self, exclude_digits: bool) -> String {
        let mut out = String::from("[");
        let mut i = 0;
        while i < self.bytes.len() {
            if !self.bytes[i]
                || i == b'\n' as usize
                || exclude_digits && (b'0' as usize..=b'9' as usize).contains(&i)
            {
                i += 1;
                continue;
            }
            let begin = i;
            i += 1;
            while i < self.bytes.len()
                && self.bytes[i]
                && i != b'\n' as usize
                && !(exclude_digits && (b'0' as usize..=b'9' as usize).contains(&i))
            {
                i += 1;
            }
            out.push_str(&format!("\\x{{{begin:x}}}"));
            if i > begin + 1 {
                out.push_str(&format!("-\\x{{{:x}}}", i - 1));
            }
        }
        if self.alphabetic {
            out.push_str("[\\p{L}&&[^\\x00-\\xff]]");
        }
        // An empty class is represented by an explicitly empty set.
        if out == "[" {
            out.push_str("a&&[^a]");
        }
        out.push(']');
        out
    }
}

fn endpoint(chars: &[char], i: &mut usize) -> Result<usize, String> {
    let c = *chars.get(*i).ok_or("missing Vim keyword character")?;
    *i += 1;
    let mut value = c as usize;
    if c.is_ascii_digit() {
        value = c as usize - '0' as usize;
        while let Some(digit) = chars.get(*i).and_then(|c| c.to_digit(10)) {
            value = value
                .checked_mul(10)
                .and_then(|v| v.checked_add(digit as usize))
                .ok_or("Vim keyword character overflow")?;
            *i += 1;
        }
    }
    if value > 255 {
        return Err("Vim keyword option character exceeds 255".into());
    }
    Ok(value)
}
