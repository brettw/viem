//! Errors, exceptions and the non-local exits of script evaluation.
use super::value::Value;

/// An error message as Vim reports it, e.g. `E121: Undefined variable: x`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VimError {
    /// The `E` number, or zero for messages without one.
    pub code: u32,
    pub message: String,
}

impl VimError {
    pub fn new(code: u32, message: impl Into<String>) -> Self {
        Self { code, message: message.into() }
    }
    /// A message without an `E` number, used for Viem-specific failures.
    pub fn plain(message: impl Into<String>) -> Self {
        Self { code: 0, message: message.into() }
    }
    /// The complete text, e.g. "E492: Not an editor command: x".
    pub fn text(&self) -> String {
        if self.code == 0 {
            self.message.clone()
        } else {
            format!("E{}: {}", self.code, self.message)
        }
    }
    pub fn invalid_expression(text: &[u8]) -> Self {
        Self::new(15, format!("Invalid expression: \"{}\"", String::from_utf8_lossy(text)))
    }
    pub fn trailing(text: &[u8]) -> Self {
        Self::new(488, format!("Trailing characters: {}", String::from_utf8_lossy(text)))
    }
    pub fn undefined_variable(name: &[u8]) -> Self {
        Self::new(121, format!("Undefined variable: {}", String::from_utf8_lossy(name)))
    }
    pub fn unknown_function(name: &[u8]) -> Self {
        Self::new(117, format!("Unknown function: {}", String::from_utf8_lossy(name)))
    }
    pub fn argument(code: u32, name: &str) -> Self {
        match code {
            118 => Self::new(118, format!("Too many arguments for function: {name}")),
            119 => Self::new(119, format!("Not enough arguments for function: {name}")),
            _ => Self::new(code, name.to_string()),
        }
    }
}

impl std::fmt::Display for VimError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.text())
    }
}

impl std::error::Error for VimError {}

/// A thrown Vim exception, either from `:throw` or an error inside `:try`.
#[derive(Clone, Debug)]
pub struct Exception {
    pub value: Value,
    /// `v:throwpoint`, e.g. "function Foo, line 3".
    pub throwpoint: String,
    /// Converted from an error message rather than thrown explicitly.
    pub from_error: bool,
    pub interrupt: bool,
}

impl Exception {
    pub fn text(&self) -> String {
        self.value.to_text().unwrap_or_else(|_| self.value.echo_string())
    }
}

/// Why evaluation of a command or expression stopped early.
#[derive(Clone, Debug)]
pub enum Fail {
    /// An error that has not been reported yet.
    Error(VimError),
    /// An error that was already reported (or silenced); stop without
    /// another message, like Vim's `aborting()`.
    Aborted,
    /// A propagating exception.
    Exception(Box<Exception>),
    /// Cancellation or an exhausted work budget; never catchable.
    Interrupt(String),
}

impl From<VimError> for Fail {
    fn from(value: VimError) -> Self {
        Fail::Error(value)
    }
}

pub type EvalResult<T> = Result<T, Fail>;
