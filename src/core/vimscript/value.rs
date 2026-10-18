//! Legacy Vim script values. Strings are byte strings, as in Vim: indexing,
//! slicing and `strlen()` count bytes. Lists, dictionaries and blobs are
//! reference types whose mutations are visible through every alias.
use super::error::VimError;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard};

/// An immutable Vim string. Vim strings are NUL-free byte sequences that are
/// usually, but not necessarily, valid UTF-8.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct VStr(Arc<[u8]>);

impl VStr {
    pub fn new(bytes: impl Into<Arc<[u8]>>) -> Self {
        Self(bytes.into())
    }
    pub fn empty() -> Self {
        Self(Arc::from(&b""[..]))
    }
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
    pub fn len(&self) -> usize {
        self.0.len()
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    /// UTF-8 text when valid; invalid sequences become U+FFFD.
    pub fn to_str_lossy(&self) -> std::borrow::Cow<'_, str> {
        String::from_utf8_lossy(&self.0)
    }
    pub fn as_str(&self) -> Option<&str> {
        std::str::from_utf8(&self.0).ok()
    }
}

impl std::borrow::Borrow<[u8]> for VStr {
    fn borrow(&self) -> &[u8] {
        &self.0
    }
}

impl std::fmt::Debug for VStr {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{:?}", self.to_str_lossy())
    }
}

impl From<&str> for VStr {
    fn from(value: &str) -> Self {
        Self(Arc::from(value.as_bytes()))
    }
}
impl From<String> for VStr {
    fn from(value: String) -> Self {
        Self(Arc::from(value.into_bytes()))
    }
}
impl From<&[u8]> for VStr {
    fn from(value: &[u8]) -> Self {
        Self(Arc::from(value))
    }
}
impl From<Vec<u8>> for VStr {
    fn from(value: Vec<u8>) -> Self {
        Self(Arc::from(value))
    }
}

/// `:lockvar` state of a container or variable.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Lock {
    #[default]
    Unlocked,
    Locked,
    /// Read-only Vim variables and `:const` bindings cannot be unlocked.
    Fixed,
}

impl Lock {
    pub fn is_locked(self) -> bool {
        self != Lock::Unlocked
    }
}

#[derive(Debug, Default)]
pub struct ListData {
    pub items: Vec<Value>,
    pub lock: Lock,
}

#[derive(Clone, Debug, Default)]
pub struct List(Arc<Mutex<ListData>>);

impl List {
    pub fn new(items: Vec<Value>) -> Self {
        Self(Arc::new(Mutex::new(ListData { items, lock: Lock::Unlocked })))
    }
    /// Never hold the guard while evaluating or touching another container.
    pub fn data(&self) -> MutexGuard<'_, ListData> {
        self.0.lock().unwrap_or_else(|poison| poison.into_inner())
    }
    pub fn snapshot(&self) -> Vec<Value> {
        self.data().items.clone()
    }
    pub fn len(&self) -> usize {
        self.data().items.len()
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub fn get(&self, index: usize) -> Option<Value> {
        self.data().items.get(index).cloned()
    }
    pub fn push(&self, value: Value) {
        self.data().items.push(value);
    }
    pub fn ptr_eq(&self, other: &List) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
    pub fn id(&self) -> usize {
        Arc::as_ptr(&self.0) as *const () as usize
    }
    pub fn check_writable(&self) -> Result<(), VimError> {
        if self.data().lock.is_locked() {
            Err(VimError::new(741, "Value is locked"))
        } else {
            Ok(())
        }
    }
    /// Locked check for a builtin's argument, e.g. "add() argument".
    pub fn check_argument(&self, function: &str) -> Result<(), VimError> {
        if self.data().lock.is_locked() {
            Err(VimError::new(741, format!("Value is locked: {function}() argument")))
        } else {
            Ok(())
        }
    }
}

#[derive(Clone, Debug)]
pub struct DictItem {
    pub value: Value,
    pub lock: Lock,
}

impl DictItem {
    pub fn new(value: Value) -> Self {
        Self { value, lock: Lock::Unlocked }
    }
}

#[derive(Debug, Default)]
pub struct DictData {
    pub items: BTreeMap<VStr, DictItem>,
    pub lock: Lock,
}

#[derive(Clone, Debug, Default)]
pub struct Dict(Arc<Mutex<DictData>>);

impl Dict {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn from_pairs(pairs: impl IntoIterator<Item = (VStr, Value)>) -> Self {
        let dict = Self::new();
        {
            let mut data = dict.data();
            for (key, value) in pairs {
                data.items.insert(key, DictItem::new(value));
            }
        }
        dict
    }
    pub fn data(&self) -> MutexGuard<'_, DictData> {
        self.0.lock().unwrap_or_else(|poison| poison.into_inner())
    }
    pub fn get(&self, key: &[u8]) -> Option<Value> {
        self.data().items.get(key).map(|item| item.value.clone())
    }
    pub fn get_str(&self, key: &str) -> Option<Value> {
        self.get(key.as_bytes())
    }
    pub fn contains(&self, key: &[u8]) -> bool {
        self.data().items.contains_key(key)
    }
    /// Insert ignoring locks; callers check writability first.
    pub fn insert(&self, key: impl Into<VStr>, value: Value) {
        let key = key.into();
        let mut data = self.data();
        match data.items.get_mut(&key) {
            Some(item) => item.value = value,
            None => {
                data.items.insert(key, DictItem::new(value));
            }
        }
    }
    pub fn remove(&self, key: &[u8]) -> Option<Value> {
        self.data().items.remove(key).map(|item| item.value)
    }
    pub fn keys(&self) -> Vec<VStr> {
        self.data().items.keys().cloned().collect()
    }
    pub fn snapshot(&self) -> Vec<(VStr, Value)> {
        self.data().items.iter().map(|(key, item)| (key.clone(), item.value.clone())).collect()
    }
    pub fn len(&self) -> usize {
        self.data().items.len()
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub fn ptr_eq(&self, other: &Dict) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
    pub fn id(&self) -> usize {
        Arc::as_ptr(&self.0) as *const () as usize
    }
    /// Break reference cycles when a scope is discarded.
    pub fn clear_recursive(&self) {
        let values: Vec<Value> = std::mem::take(&mut self.data().items)
            .into_values()
            .map(|item| item.value)
            .collect();
        let mut seen = std::collections::HashSet::new();
        for value in values {
            value.clear_recursive(&mut seen);
        }
    }
}

#[derive(Debug, Default)]
pub struct BlobData {
    pub bytes: Vec<u8>,
    pub lock: Lock,
}

#[derive(Clone, Debug, Default)]
pub struct Blob(Arc<Mutex<BlobData>>);

impl Blob {
    pub fn new(bytes: Vec<u8>) -> Self {
        Self(Arc::new(Mutex::new(BlobData { bytes, lock: Lock::Unlocked })))
    }
    pub fn data(&self) -> MutexGuard<'_, BlobData> {
        self.0.lock().unwrap_or_else(|poison| poison.into_inner())
    }
    pub fn bytes(&self) -> Vec<u8> {
        self.data().bytes.clone()
    }
    pub fn len(&self) -> usize {
        self.data().bytes.len()
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub fn ptr_eq(&self, other: &Blob) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

/// A function reference, optionally a partial with bound arguments or a
/// bound dictionary (`self`).
#[derive(Clone, Debug)]
pub struct Funcref(Arc<FuncrefData>);

#[derive(Debug)]
pub struct FuncrefData {
    pub name: VStr,
    pub args: Vec<Value>,
    pub dict: Option<Dict>,
    /// A dictionary bound implicitly by `dict.Func` access, which a later
    /// explicit binding replaces.
    pub auto_bound: bool,
}

impl Funcref {
    pub fn named(name: impl Into<VStr>) -> Self {
        Self(Arc::new(FuncrefData { name: name.into(), args: Vec::new(), dict: None, auto_bound: false }))
    }
    pub fn partial(name: VStr, args: Vec<Value>, dict: Option<Dict>, auto_bound: bool) -> Self {
        Self(Arc::new(FuncrefData { name, args, dict, auto_bound }))
    }
    pub fn name(&self) -> &VStr {
        &self.0.name
    }
    pub fn args(&self) -> &[Value] {
        &self.0.args
    }
    pub fn dict(&self) -> Option<&Dict> {
        self.0.dict.as_ref()
    }
    pub fn auto_bound(&self) -> bool {
        self.0.auto_bound
    }
    pub fn is_partial(&self) -> bool {
        !self.0.args.is_empty() || self.0.dict.is_some()
    }
}

#[derive(Clone, Debug)]
pub enum Value {
    Number(i64),
    Float(f64),
    String(VStr),
    List(List),
    Dict(Dict),
    Funcref(Funcref),
    Blob(Blob),
    Bool(bool),
    /// `v:none`
    None,
    /// `v:null`
    Null,
}

pub const VAR_NUMBER: i64 = 0;
pub const VAR_STRING: i64 = 1;
pub const VAR_FUNC: i64 = 2;
pub const VAR_LIST: i64 = 3;
pub const VAR_DICT: i64 = 4;
pub const VAR_FLOAT: i64 = 5;
pub const VAR_BOOL: i64 = 6;
pub const VAR_SPECIAL: i64 = 7;
pub const VAR_BLOB: i64 = 10;

impl Default for Value {
    fn default() -> Self {
        Value::Number(0)
    }
}

impl From<i64> for Value {
    fn from(value: i64) -> Self {
        Value::Number(value)
    }
}
impl From<bool> for Value {
    fn from(value: bool) -> Self {
        Value::Number(value as i64)
    }
}
impl From<&str> for Value {
    fn from(value: &str) -> Self {
        Value::String(value.into())
    }
}
impl From<String> for Value {
    fn from(value: String) -> Self {
        Value::String(value.into())
    }
}
impl From<VStr> for Value {
    fn from(value: VStr) -> Self {
        Value::String(value)
    }
}
impl From<Vec<u8>> for Value {
    fn from(value: Vec<u8>) -> Self {
        Value::String(value.into())
    }
}

impl Value {
    pub fn str(text: &str) -> Self {
        Value::String(text.into())
    }
    pub fn list(items: Vec<Value>) -> Self {
        Value::List(List::new(items))
    }
    pub fn type_number(&self) -> i64 {
        match self {
            Value::Number(_) => VAR_NUMBER,
            Value::String(_) => VAR_STRING,
            Value::Funcref(_) => VAR_FUNC,
            Value::List(_) => VAR_LIST,
            Value::Dict(_) => VAR_DICT,
            Value::Float(_) => VAR_FLOAT,
            Value::Bool(_) => VAR_BOOL,
            Value::None | Value::Null => VAR_SPECIAL,
            Value::Blob(_) => VAR_BLOB,
        }
    }
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Number(_) => "Number",
            Value::String(_) => "String",
            Value::Funcref(_) => "Funcref",
            Value::List(_) => "List",
            Value::Dict(_) => "Dictionary",
            Value::Float(_) => "Float",
            Value::Bool(_) => "Boolean",
            Value::None | Value::Null => "Special",
            Value::Blob(_) => "Blob",
        }
    }

    /// `tv_get_number()`: legacy conversion of scalars to Number.
    pub fn to_number(&self) -> Result<i64, VimError> {
        match self {
            Value::Number(number) => Ok(*number),
            Value::String(text) => Ok(str2nr(text.as_bytes(), true).0),
            Value::Bool(value) => Ok(*value as i64),
            Value::None | Value::Null => Ok(0),
            Value::Float(_) => Err(VimError::new(805, "Using a Float as a Number")),
            Value::List(_) => Err(VimError::new(745, "Using a List as a Number")),
            Value::Dict(_) => Err(VimError::new(728, "Using a Dictionary as a Number")),
            Value::Funcref(_) => Err(VimError::new(703, "Using a Funcref as a Number")),
            Value::Blob(_) => Err(VimError::new(974, "Using a Blob as a Number")),
        }
    }

    /// Conversion used by `:if`, `&&`, `||`, `!` and the ternary operator.
    pub fn to_bool(&self) -> Result<bool, VimError> {
        Ok(self.to_number()? != 0)
    }

    pub fn to_float(&self) -> Result<f64, VimError> {
        match self {
            Value::Float(value) => Ok(*value),
            Value::Number(number) => Ok(*number as f64),
            Value::Bool(value) => Ok(*value as i64 as f64),
            Value::None | Value::Null => Ok(0.0),
            Value::String(_) => Err(VimError::new(892, "Using a String as a Float")),
            Value::List(_) => Err(VimError::new(893, "Using a List as a Float")),
            Value::Dict(_) => Err(VimError::new(894, "Using a Dictionary as a Float")),
            Value::Funcref(_) => Err(VimError::new(891, "Using a Funcref as a Float")),
            Value::Blob(_) => Err(VimError::new(975, "Using a Blob as a Float")),
        }
    }

    /// `tv_get_string()`: legacy conversion of scalars to String.
    pub fn to_vstr(&self) -> Result<VStr, VimError> {
        match self {
            Value::String(text) => Ok(text.clone()),
            Value::Number(number) => Ok(number.to_string().into()),
            Value::Float(value) => Ok(format_float(*value).into()),
            Value::Bool(true) => Ok("v:true".into()),
            Value::Bool(false) => Ok("v:false".into()),
            Value::None => Ok("v:none".into()),
            Value::Null => Ok("v:null".into()),
            Value::List(_) => Err(VimError::new(730, "Using a List as a String")),
            Value::Dict(_) => Err(VimError::new(731, "Using a Dictionary as a String")),
            Value::Funcref(_) => Err(VimError::new(729, "Using a Funcref as a String")),
            Value::Blob(_) => Err(VimError::new(976, "Using a Blob as a String")),
        }
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>, VimError> {
        Ok(self.to_vstr()?.as_bytes().to_vec())
    }

    /// Text for host APIs; invalid UTF-8 is replaced.
    pub fn to_text(&self) -> Result<String, VimError> {
        Ok(self.to_vstr()?.to_str_lossy().into_owned())
    }

    /// The value as `:echo` displays it.
    pub fn echo_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        let mut seen = Vec::new();
        write_value(&mut out, self, false, &mut seen, 0);
        out
    }

    pub fn echo_string(&self) -> String {
        String::from_utf8_lossy(&self.echo_bytes()).into_owned()
    }

    /// `string()`: an expression that evaluates to this value.
    pub fn string_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        let mut seen = Vec::new();
        write_value(&mut out, self, true, &mut seen, 0);
        out
    }

    pub fn string_repr(&self) -> String {
        String::from_utf8_lossy(&self.string_bytes()).into_owned()
    }

    /// A deep or shallow copy (`deepcopy()` / `copy()`). `noref` deep copies
    /// share converted children for repeated references, like Vim.
    pub fn copy(&self, deep: bool) -> Value {
        let mut copies = Vec::new();
        self.copy_inner(deep, &mut copies, 0)
    }

    fn copy_inner(&self, deep: bool, copies: &mut Vec<(usize, Value)>, depth: usize) -> Value {
        match self {
            Value::List(list) => {
                if let Some((_, copied)) = copies.iter().find(|(id, _)| *id == list.id()) {
                    return copied.clone();
                }
                let new = List::new(Vec::new());
                copies.push((list.id(), Value::List(new.clone())));
                let items = list.snapshot();
                let items = if deep && depth < 100 {
                    items.iter().map(|item| item.copy_inner(true, copies, depth + 1)).collect()
                } else {
                    items
                };
                new.data().items = items;
                Value::List(new)
            }
            Value::Dict(dict) => {
                if let Some((_, copied)) = copies.iter().find(|(id, _)| *id == dict.id()) {
                    return copied.clone();
                }
                let new = Dict::new();
                copies.push((dict.id(), Value::Dict(new.clone())));
                for (key, value) in dict.snapshot() {
                    let value = if deep && depth < 100 { value.copy_inner(true, copies, depth + 1) } else { value };
                    new.insert(key, value);
                }
                Value::Dict(new)
            }
            Value::Blob(blob) => Value::Blob(Blob::new(blob.bytes())),
            other => other.clone(),
        }
    }

    fn clear_recursive(self, seen: &mut std::collections::HashSet<usize>) {
        match self {
            Value::List(list) => {
                if seen.insert(list.id()) {
                    let items = std::mem::take(&mut list.data().items);
                    for item in items {
                        item.clear_recursive(seen);
                    }
                }
            }
            Value::Dict(dict) => {
                if seen.insert(dict.id()) {
                    let items = std::mem::take(&mut dict.data().items);
                    for (_, item) in items {
                        item.value.clear_recursive(seen);
                    }
                }
            }
            Value::Funcref(funcref) => {
                if let Some(dict) = funcref.dict() {
                    if seen.insert(dict.id()) {
                        let items = std::mem::take(&mut dict.data().items);
                        for (_, item) in items {
                            item.value.clear_recursive(seen);
                        }
                    }
                }
            }
            _ => {}
        }
    }

    pub fn is_empty_value(&self) -> bool {
        match self {
            Value::Number(number) => *number == 0,
            Value::Float(value) => *value == 0.0,
            Value::String(text) => text.is_empty(),
            Value::List(list) => list.is_empty(),
            Value::Dict(dict) => dict.is_empty(),
            Value::Blob(blob) => blob.is_empty(),
            Value::Funcref(funcref) => funcref.name().is_empty(),
            Value::Bool(value) => !value,
            Value::None | Value::Null => true,
        }
    }
}

fn write_string_repr(out: &mut Vec<u8>, bytes: &[u8]) {
    out.push(b'\'');
    for &byte in bytes {
        if byte == b'\'' {
            out.push(b'\'');
        }
        out.push(byte);
    }
    out.push(b'\'');
}

fn write_value(out: &mut Vec<u8>, value: &Value, repr: bool, seen: &mut Vec<usize>, depth: usize) {
    match value {
        Value::String(text) => {
            if repr {
                write_string_repr(out, text.as_bytes());
            } else {
                out.extend_from_slice(text.as_bytes());
            }
        }
        Value::Number(number) => {
            out.extend_from_slice(number.to_string().as_bytes());
        }
        Value::Float(float) => out.extend_from_slice(format_float(*float).as_bytes()),
        Value::Bool(true) => out.extend_from_slice(b"v:true"),
        Value::Bool(false) => out.extend_from_slice(b"v:false"),
        Value::None => out.extend_from_slice(b"v:none"),
        Value::Null => out.extend_from_slice(b"v:null"),
        Value::Blob(blob) => {
            out.extend_from_slice(b"0z");
            for byte in blob.bytes().iter() {
                out.extend_from_slice(format!("{byte:02X}").as_bytes());
            }
        }
        Value::Funcref(funcref) => {
            if !repr && depth == 0 && !funcref.is_partial() {
                out.extend_from_slice(funcref.name().as_bytes());
                return;
            }
            out.extend_from_slice(b"function(");
            write_string_repr(out, funcref.name().as_bytes());
            if !funcref.args().is_empty() {
                out.extend_from_slice(b", [");
                for (index, arg) in funcref.args().iter().enumerate() {
                    if index > 0 {
                        out.extend_from_slice(b", ");
                    }
                    write_value(out, arg, true, seen, depth + 1);
                }
                out.push(b']');
            }
            if let Some(dict) = funcref.dict() {
                out.extend_from_slice(b", ");
                write_value(out, &Value::Dict(dict.clone()), true, seen, depth + 1);
            }
            out.push(b')');
        }
        Value::List(list) => {
            if seen.contains(&list.id()) || depth > 100 {
                out.extend_from_slice(b"[...]");
                return;
            }
            seen.push(list.id());
            out.push(b'[');
            for (index, item) in list.snapshot().iter().enumerate() {
                if index > 0 {
                    out.extend_from_slice(b", ");
                }
                write_value(out, item, true, seen, depth + 1);
            }
            out.push(b']');
            seen.pop();
        }
        Value::Dict(dict) => {
            if seen.contains(&dict.id()) || depth > 100 {
                out.extend_from_slice(b"{...}");
                return;
            }
            seen.push(dict.id());
            out.push(b'{');
            for (index, (key, item)) in dict.snapshot().iter().enumerate() {
                if index > 0 {
                    out.extend_from_slice(b", ");
                }
                write_string_repr(out, key.as_bytes());
                out.extend_from_slice(b": ");
                write_value(out, item, true, seen, depth + 1);
            }
            out.push(b'}');
            seen.pop();
        }
    }
}

/// Vim's own `%g`: fixed notation for 0.001 <= |f| < 1e7, otherwise
/// exponent notation, both with six fractional digits and trailing zeros
/// removed, keeping one digit after the point.
pub fn format_float(value: f64) -> String {
    format_float_precision(value, 6, false)
}

pub fn format_float_precision(value: f64, precision: usize, upper: bool) -> String {
    if value.is_nan() {
        return if upper { "NAN".into() } else { "nan".into() };
    }
    if value.is_infinite() {
        let text = if value < 0.0 { "-inf" } else { "inf" };
        return if upper { text.to_uppercase() } else { text.into() };
    }
    let magnitude = value.abs();
    let fixed = (0.001..10_000_000.0).contains(&magnitude) || magnitude == 0.0;
    let mut text = if fixed {
        format!("{value:.precision$}")
    } else {
        let formatted = format!("{value:.precision$e}");
        // Rust prints e.g. "1.234568e8"; Vim's exponent already has no '+'.
        formatted
    };
    // Remove trailing zeroes from the mantissa, keeping one after the point.
    let (mantissa, exponent) = match text.find(['e', 'E']) {
        Some(at) => (text[..at].to_string(), text[at + 1..].to_string()),
        None => (text.clone(), String::new()),
    };
    let mut mantissa = mantissa;
    if mantissa.contains('.') {
        while mantissa.ends_with('0') && !mantissa.ends_with(".0") {
            mantissa.pop();
        }
        if mantissa.ends_with('.') {
            mantissa.push('0');
        }
    } else {
        mantissa.push_str(".0");
    }
    text = if exponent.is_empty() {
        mantissa
    } else {
        let exponent = exponent.trim_start_matches('+');
        let (sign, digits) = match exponent.strip_prefix('-') {
            Some(digits) => ("-", digits),
            None => ("", exponent),
        };
        let digits = digits.trim_start_matches('0');
        let digits = if digits.is_empty() { "0" } else { digits };
        format!("{mantissa}{}{sign}{digits}", if upper { 'E' } else { 'e' })
    };
    text
}

/// Parse the longest number prefix of `bytes` like Vim's `vim_str2nr()`.
/// With `octal_leading_zero`, "017" is octal; hex "0x", binary "0b" and
/// octal "0o" prefixes are always recognized. Returns the value and the
/// number of bytes used. No leading whitespace is skipped.
pub fn str2nr(bytes: &[u8], octal_leading_zero: bool) -> (i64, usize) {
    let mut at = 0;
    let negative = bytes.first() == Some(&b'-');
    if negative {
        at = 1;
    }
    let digits_start = at;
    let (radix, mut position) = if bytes.get(at) == Some(&b'0') {
        match bytes.get(at + 1) {
            Some(b'x' | b'X') if bytes.get(at + 2).is_some_and(|b| b.is_ascii_hexdigit()) => (16, at + 2),
            Some(b'b' | b'B') if bytes.get(at + 2).is_some_and(|b| matches!(b, b'0' | b'1')) => (2, at + 2),
            Some(b'o' | b'O') if bytes.get(at + 2).is_some_and(|b| (b'0'..=b'7').contains(b)) => (8, at + 2),
            Some(b'0'..=b'9') if octal_leading_zero => {
                // Octal only when every following digit is octal.
                let end = bytes[at..].iter().position(|b| !b.is_ascii_digit()).map_or(bytes.len(), |n| at + n);
                if bytes[at..end].iter().all(|b| (b'0'..=b'7').contains(b)) {
                    (8, at + 1)
                } else {
                    (10, at)
                }
            }
            _ => (10, at),
        }
    } else {
        (10, at)
    };
    let start = position;
    let mut value: u64 = 0;
    let mut overflow = false;
    while let Some(&byte) = bytes.get(position) {
        let digit = match byte {
            b'0'..=b'9' => (byte - b'0') as u64,
            b'a'..=b'f' => (byte - b'a' + 10) as u64,
            b'A'..=b'F' => (byte - b'A' + 10) as u64,
            _ => break,
        };
        if digit >= radix {
            break;
        }
        match value.checked_mul(radix).and_then(|v| v.checked_add(digit)) {
            Some(next) => value = next,
            None => overflow = true,
        }
        position += 1;
    }
    if position == start {
        if radix != 10 {
            // "0x" without digits: only the "0" is a number.
            return (0, digits_start + 1);
        }
        return (0, 0);
    }
    let value = if overflow {
        if negative { i64::MIN } else { i64::MAX }
    } else if negative {
        if value > i64::MAX as u64 { i64::MIN } else { -(value as i64) }
    } else if value > i64::MAX as u64 {
        i64::MAX
    } else {
        value as i64
    };
    (value, position)
}

/// Comparison operators of `expr4`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompareOp {
    Equal,
    NotEqual,
    Greater,
    GreaterEqual,
    Less,
    LessEqual,
    Match,
    NoMatch,
    Is,
    IsNot,
}

/// Case handling selected by the `#`/`?` operator suffix.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaseMode {
    /// Plain operator: follow 'ignorecase'.
    Option,
    Match,
    Ignore,
}

pub fn strings_equal(left: &[u8], right: &[u8], ignore_case: bool) -> bool {
    if ignore_case {
        compare_bytes(left, right, true) == std::cmp::Ordering::Equal
    } else {
        left == right
    }
}

/// Byte order, or case-folded character order for `?` comparisons.
pub fn compare_bytes(left: &[u8], right: &[u8], ignore_case: bool) -> std::cmp::Ordering {
    if !ignore_case {
        return left.cmp(right);
    }
    match (std::str::from_utf8(left), std::str::from_utf8(right)) {
        (Ok(left), Ok(right)) => {
            let mut left = left.chars().flat_map(char::to_lowercase);
            let mut right = right.chars().flat_map(char::to_lowercase);
            loop {
                match (left.next(), right.next()) {
                    (None, None) => return std::cmp::Ordering::Equal,
                    (None, Some(_)) => return std::cmp::Ordering::Less,
                    (Some(_), None) => return std::cmp::Ordering::Greater,
                    (Some(a), Some(b)) if a != b => return a.cmp(&b),
                    _ => {}
                }
            }
        }
        _ => left.to_ascii_lowercase().cmp(&right.to_ascii_lowercase()),
    }
}

/// Deep equality used by `==` for Lists, Dictionaries, Funcrefs and Blobs.
pub fn values_equal(left: &Value, right: &Value, ignore_case: bool, depth: usize) -> Result<bool, VimError> {
    if depth > 100 {
        return Ok(false);
    }
    Ok(match (left, right) {
        (Value::List(a), Value::List(b)) => {
            if a.ptr_eq(b) {
                return Ok(true);
            }
            let (a, b) = (a.snapshot(), b.snapshot());
            if a.len() != b.len() {
                return Ok(false);
            }
            for (x, y) in a.iter().zip(&b) {
                if !values_equal(x, y, ignore_case, depth + 1)? {
                    return Ok(false);
                }
            }
            true
        }
        (Value::Dict(a), Value::Dict(b)) => {
            if a.ptr_eq(b) {
                return Ok(true);
            }
            let (a, b) = (a.snapshot(), b.snapshot());
            if a.len() != b.len() {
                return Ok(false);
            }
            for ((ka, va), (kb, vb)) in a.iter().zip(&b) {
                if ka != kb || !values_equal(va, vb, ignore_case, depth + 1)? {
                    return Ok(false);
                }
            }
            true
        }
        (Value::Funcref(a), Value::Funcref(b)) => {
            if a.name() != b.name() || a.args().len() != b.args().len() {
                return Ok(false);
            }
            for (x, y) in a.args().iter().zip(b.args()) {
                if !values_equal(x, y, ignore_case, depth + 1)? {
                    return Ok(false);
                }
            }
            match (a.dict(), b.dict()) {
                (None, None) => true,
                (Some(x), Some(y)) => x.ptr_eq(y),
                _ => false,
            }
        }
        (Value::Blob(a), Value::Blob(b)) => a.ptr_eq(b) || a.bytes() == b.bytes(),
        (Value::List(_) | Value::Dict(_) | Value::Funcref(_) | Value::Blob(_), _)
        | (_, Value::List(_) | Value::Dict(_) | Value::Funcref(_) | Value::Blob(_)) => false,
        (Value::String(a), Value::String(b)) => strings_equal(a.as_bytes(), b.as_bytes(), ignore_case),
        (Value::Float(_), _) | (_, Value::Float(_)) => {
            // Strings inside containers compare by type first.
            if matches!(left, Value::String(_)) || matches!(right, Value::String(_)) {
                false
            } else {
                left.to_float()? == right.to_float()?
            }
        }
        (Value::String(_), _) | (_, Value::String(_)) => false,
        _ => left.to_number()? == right.to_number()?,
    })
}

/// `typval_compare()` for legacy script. `regex` handles `=~`/`!~` on strings.
pub fn compare_values(
    left: &Value,
    right: &Value,
    op: CompareOp,
    ignore_case: bool,
    regex: &mut dyn FnMut(&[u8], &[u8], bool) -> Result<bool, VimError>,
) -> Result<bool, VimError> {
    use CompareOp::*;
    let is_container = |value: &Value| matches!(value, Value::List(_) | Value::Dict(_) | Value::Blob(_));
    if matches!(op, Is | IsNot) {
        let same = match (left, right) {
            (Value::List(a), Value::List(b)) => a.ptr_eq(b),
            (Value::Dict(a), Value::Dict(b)) => a.ptr_eq(b),
            (Value::Blob(a), Value::Blob(b)) => a.ptr_eq(b),
            (Value::Funcref(a), Value::Funcref(b)) => values_equal(&Value::Funcref(a.clone()), &Value::Funcref(b.clone()), ignore_case, 0)?,
            _ if is_container(left) || is_container(right) => false,
            _ if left.type_number() != right.type_number() => false,
            (Value::String(a), Value::String(b)) => strings_equal(a.as_bytes(), b.as_bytes(), ignore_case),
            (Value::Float(a), Value::Float(b)) => a == b,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::None, Value::None) | (Value::Null, Value::Null) => true,
            (Value::None, Value::Null) | (Value::Null, Value::None) => false,
            _ => left.to_number()? == right.to_number()?,
        };
        return Ok(if op == Is { same } else { !same });
    }
    match (left, right) {
        (Value::List(_), _) | (_, Value::List(_)) => {
            if !matches!((left, right), (Value::List(_), Value::List(_))) {
                return Err(VimError::new(691, "Can only compare List with List"));
            }
            if !matches!(op, Equal | NotEqual) {
                return Err(VimError::new(692, "Invalid operation for List"));
            }
            let equal = values_equal(left, right, ignore_case, 0)?;
            Ok(if op == Equal { equal } else { !equal })
        }
        (Value::Dict(_), _) | (_, Value::Dict(_)) => {
            if !matches!((left, right), (Value::Dict(_), Value::Dict(_))) {
                return Err(VimError::new(735, "Can only compare Dictionary with Dictionary"));
            }
            if !matches!(op, Equal | NotEqual) {
                return Err(VimError::new(736, "Invalid operation for Dictionary"));
            }
            let equal = values_equal(left, right, ignore_case, 0)?;
            Ok(if op == Equal { equal } else { !equal })
        }
        (Value::Blob(_), _) | (_, Value::Blob(_)) => {
            if !matches!((left, right), (Value::Blob(_), Value::Blob(_))) {
                return Err(VimError::new(977, "Can only compare Blob with Blob"));
            }
            if !matches!(op, Equal | NotEqual) {
                return Err(VimError::new(978, "Invalid operation for Blob"));
            }
            let equal = values_equal(left, right, ignore_case, 0)?;
            Ok(if op == Equal { equal } else { !equal })
        }
        (Value::Funcref(_), _) | (_, Value::Funcref(_)) => {
            if !matches!(op, Equal | NotEqual) {
                return Err(VimError::new(694, "Invalid operation for Funcrefs"));
            }
            let equal = values_equal(left, right, ignore_case, 0)?;
            Ok(if op == Equal { equal } else { !equal })
        }
        (Value::Float(_), _) | (_, Value::Float(_)) => {
            if matches!(op, Match | NoMatch) {
                return compare_strings(left, right, op, ignore_case, regex);
            }
            let (a, b) = (left.to_float()?, right.to_float()?);
            Ok(match op {
                Equal => a == b,
                NotEqual => a != b,
                Greater => a > b,
                GreaterEqual => a >= b,
                Less => a < b,
                LessEqual => a <= b,
                _ => unreachable!(),
            })
        }
        (Value::String(_), Value::String(_)) => compare_strings(left, right, op, ignore_case, regex),
        _ if matches!(op, Match | NoMatch) => compare_strings(left, right, op, ignore_case, regex),
        (Value::String(_), _) | (_, Value::String(_)) | _ => {
            let (a, b) = (left.to_number()?, right.to_number()?);
            Ok(match op {
                Equal => a == b,
                NotEqual => a != b,
                Greater => a > b,
                GreaterEqual => a >= b,
                Less => a < b,
                LessEqual => a <= b,
                _ => unreachable!(),
            })
        }
    }
}

fn compare_strings(
    left: &Value,
    right: &Value,
    op: CompareOp,
    ignore_case: bool,
    regex: &mut dyn FnMut(&[u8], &[u8], bool) -> Result<bool, VimError>,
) -> Result<bool, VimError> {
    use CompareOp::*;
    let (a, b) = (left.to_vstr()?, right.to_vstr()?);
    let order = compare_bytes(a.as_bytes(), b.as_bytes(), ignore_case);
    Ok(match op {
        Equal => order.is_eq(),
        NotEqual => order.is_ne(),
        Greater => order.is_gt(),
        GreaterEqual => order.is_ge(),
        Less => order.is_lt(),
        LessEqual => order.is_le(),
        Match => regex(a.as_bytes(), b.as_bytes(), ignore_case)?,
        NoMatch => !regex(a.as_bytes(), b.as_bytes(), ignore_case)?,
        Is | IsNot => unreachable!(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn floats_follow_vims_percent_g() {
        assert_eq!(format_float(1.0 / 3.0), "0.333333");
        assert_eq!(format_float(1.0e20), "1.0e20");
        assert_eq!(format_float(0.000001), "1.0e-6");
        assert_eq!(format_float(123456789.0), "1.234568e8");
        assert_eq!(format_float(1500.0), "1500.0");
        assert_eq!(format_float(100.0), "100.0");
        assert_eq!(format_float(1.0e-7), "1.0e-7");
        assert_eq!(format_float(-0.0), "-0.0");
        assert_eq!(format_float(1.5), "1.5");
        assert_eq!(format_float(f64::INFINITY), "inf");
        assert_eq!(format_float(f64::NEG_INFINITY), "-inf");
        assert_eq!(format_float(f64::NAN), "nan");
    }

    #[test]
    fn strings_convert_like_vim_str2nr() {
        assert_eq!(str2nr(b"010", true).0, 8);
        assert_eq!(str2nr(b"019", true).0, 19);
        assert_eq!(str2nr(b"0x1F", true).0, 31);
        assert_eq!(str2nr(b"0b101", true).0, 5);
        assert_eq!(str2nr(b"0o17", true).0, 15);
        assert_eq!(str2nr(b"3abc", true).0, 3);
        assert_eq!(str2nr(b"abc", true).0, 0);
        assert_eq!(str2nr(b" 5", true).0, 0);
        assert_eq!(str2nr(b"-12", true).0, -12);
        assert_eq!(str2nr(b"0x", true), (0, 1));
    }

    #[test]
    fn display_and_string_forms_match_vim() {
        let list = Value::list(vec![1.into(), "a".into(), Value::list(vec![2.into()])]);
        assert_eq!(list.echo_string(), "[1, 'a', [2]]");
        assert_eq!(Value::str("it's").string_repr(), "'it''s'");
        let dict = Value::Dict(Dict::from_pairs([("b".into(), 1.into()), ("a".into(), 2.into())]));
        assert_eq!(dict.string_repr(), "{'a': 2, 'b': 1}");
        assert_eq!(Value::Funcref(Funcref::named("strlen")).echo_string(), "strlen");
        assert_eq!(Value::Funcref(Funcref::named("strlen")).string_repr(), "function('strlen')");
        let recursive = List::new(Vec::new());
        recursive.push(Value::List(recursive.clone()));
        assert_eq!(Value::List(recursive.clone()).echo_string(), "[[...]]");
        Value::List(recursive).clear_recursive(&mut Default::default());
    }
}
