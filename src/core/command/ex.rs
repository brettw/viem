//! Parser and typed syntax model for the supported Ex command surface.
//!
//! Parsing is deliberately independent of a document snapshot. Addresses such
//! as `.` and `$` remain symbolic until the command coordinator resolves them
//! against the exact hard-line state on which the command executes.

use std::fmt;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExCommand {
    pub range: Option<ExRange>,
    pub bang: bool,
    pub action: ExAction,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExRange {
    Single(ExAddress),
    Between {
        start: ExAddress,
        end: ExAddress,
        separator: RangeSeparator,
    },
    WholeFile,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExAddress {
    pub base: AddressBase,
    pub offset: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AddressBase {
    Absolute(u64),
    Current,
    Last,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RangeSeparator {
    /// Both addresses are resolved relative to the original current line.
    Comma,
    /// The first address becomes current while the second is resolved.
    Semicolon,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExAction {
    EditNewWindow {
        path: Option<String>,
    },
    PrintWorkingDirectory,
    CheckTime,
    ChangeDirectory {
        path: Option<String>,
    },
    Update,
    Split {
        path: Option<String>,
    },
    Edit {
        path: Option<String>,
    },
    New,
    Write {
        path: Option<String>,
    },
    SaveAs {
        path: String,
    },
    Quit,
    QuitAll,
    WriteQuit {
        path: Option<String>,
    },
    Xit {
        path: Option<String>,
    },
    WriteAll,
    Undo {
        /// Exact monotonically increasing history change number. With no
        /// number, `:undo` performs one ordinary parent step.
        change: Option<u64>,
    },
    Redo,
    Delete(RegisterCount),
    Yank(RegisterCount),
    Put {
        register: Option<char>,
    },
    Join {
        count: Option<u64>,
    },
    Copy {
        destination: ExAddress,
    },
    Move {
        destination: ExAddress,
    },
    Normal {
        commands: String,
    },
    Sort(SortOptions),
    Substitute(Substitute),
    RepeatSubstitute {
        pattern: RepeatPattern,
        flags: SubstituteFlags,
        count: Option<u64>,
    },
    GoToLine(ExAddress),
    GoToByte {
        /// Explicit argument after `:goto`. When absent, the last range
        /// address supplies the byte count; with neither, Vim defaults to 1.
        count: Option<u64>,
    },
    Marks {
        names: Vec<char>,
    },
    Registers {
        names: Vec<char>,
    },
    Jumps,
    Set(SetCommand),
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RegisterCount {
    pub register: Option<char>,
    pub count: Option<u64>,
}

/// Portable sort keys use Unicode scalar order, independent of host locale.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SortOptions {
    pub ignore_case: bool,
    pub unique: bool,
    pub match_only: bool,
    /// 10=n, 16=x, 8=o, 2=b. Numeric modes are mutually exclusive.
    pub radix: Option<u32>,
    pub pattern: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Substitute {
    pub delimiter: char,
    pub pattern: String,
    pub replacement: String,
    pub flags: SubstituteFlags,
    pub count: Option<u64>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RepeatPattern {
    LastSubstitute,
    LastSearch,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SubstituteFlags {
    pub global: bool,
    pub confirm: bool,
    pub ignore_case: Option<bool>,
    pub print: bool,
    pub number: bool,
    pub list: bool,
    pub suppress_errors: bool,
    pub use_previous_flags: bool,
    pub occurrence: Option<u64>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SetScope {
    GlobalAndLocal,
    Local,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SetCommand {
    pub scope: SetScope,
    pub operation: SetOperation,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SetOperation {
    ShowChanged,
    ShowAll,
    Options(Vec<OptionOperation>),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OptionOperation {
    pub name: String,
    pub action: OptionAction,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OptionAction {
    Enable,
    Disable,
    Toggle,
    Query,
    Reset,
    Assign(String),
    Append(String),
    Prepend(String),
    Remove(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExParseError {
    pub offset: usize,
    pub kind: ExParseErrorKind,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExParseErrorKind {
    Empty,
    InvalidAddress,
    AddressOverflow,
    TooManyAddresses,
    MissingCommand,
    UnknownCommand(String),
    AmbiguousCommand(String),
    UnexpectedBang(String),
    UnexpectedRange(String),
    MissingArgument(&'static str),
    UnexpectedArgument(String),
    InvalidNumber(String),
    InvalidRegister(String),
    InvalidSubstituteDelimiter(char),
    UnterminatedSubstitutePattern,
    InvalidSubstituteFlag(char),
    DuplicateSubstituteFlag(char),
    InvalidSortArgument(String),
    InvalidOption(String),
}

impl fmt::Display for ExParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "Ex parse error at byte {}: ", self.offset)?;
        match &self.kind {
            ExParseErrorKind::InvalidSortArgument(argument) => write!(
                formatter,
                "invalid sort argument {argument:?}; supported flags: i, u, r, n, x, o, b"
            ),
            ExParseErrorKind::Empty => formatter.write_str("empty command"),
            ExParseErrorKind::InvalidAddress => formatter.write_str("invalid address"),
            ExParseErrorKind::AddressOverflow => formatter.write_str("address offset overflow"),
            ExParseErrorKind::TooManyAddresses => {
                formatter.write_str("only two-address ranges are supported")
            }
            ExParseErrorKind::MissingCommand => formatter.write_str("range requires a command"),
            ExParseErrorKind::UnknownCommand(command) => {
                write!(formatter, "unknown command {command:?}")
            }
            ExParseErrorKind::AmbiguousCommand(command) => {
                write!(formatter, "ambiguous command {command:?}")
            }
            ExParseErrorKind::UnexpectedBang(command) => {
                write!(formatter, "command {command:?} does not accept !")
            }
            ExParseErrorKind::UnexpectedRange(command) => {
                write!(formatter, "command {command:?} does not accept a range")
            }
            ExParseErrorKind::MissingArgument(argument) => {
                write!(formatter, "missing {argument}")
            }
            ExParseErrorKind::UnexpectedArgument(argument) => {
                write!(formatter, "unexpected argument {argument:?}")
            }
            ExParseErrorKind::InvalidNumber(number) => {
                write!(formatter, "invalid number {number:?}")
            }
            ExParseErrorKind::InvalidRegister(register) => {
                write!(formatter, "invalid register {register:?}")
            }
            ExParseErrorKind::InvalidSubstituteDelimiter(delimiter) => {
                write!(formatter, "invalid substitute delimiter {delimiter:?}; :s substitutes text — use :w <file> or :saveas <file> to save")
            }
            ExParseErrorKind::UnterminatedSubstitutePattern => {
                formatter.write_str("substitute pattern has no closing delimiter")
            }
            ExParseErrorKind::InvalidSubstituteFlag(flag) => {
                write!(formatter, "invalid substitute flag {flag:?}")
            }
            ExParseErrorKind::DuplicateSubstituteFlag(flag) => {
                write!(formatter, "duplicate substitute flag {flag:?}")
            }
            ExParseErrorKind::InvalidOption(option) => {
                write!(formatter, "invalid option expression {option:?}")
            }
        }
    }
}

impl std::error::Error for ExParseError {}

pub fn parse_ex(input: &str) -> Result<ExCommand, ExParseError> {
    Parser::new(input).parse()
}

pub(super) struct FilenameArgument {
    pub range: std::ops::Range<usize>,
    pub directories_only: bool,
}

/// Locate the literal filename prefix before a prompt caret. Command names,
/// abbreviations, ranges, and bangs use the same grammar as execution.
pub(super) fn filename_argument(input: &str, cursor: usize) -> Option<FilenameArgument> {
    let prefix = input.get(..cursor)?;
    let mut parser = Parser::new(prefix);
    parser.skip_space();
    if parser.peek() == Some(':') {
        parser.bump();
    }
    parser.skip_space();
    let range = parser.parse_range().ok()?;
    parser.skip_space();
    let (name, _) = parser.parse_command_head(range.is_some()).ok()?;
    let directories_only = match name {
        CommandName::ChangeDirectory => true,
        CommandName::EditNewWindow
        | CommandName::Split
        | CommandName::Edit
        | CommandName::Write
        | CommandName::SaveAs
        | CommandName::WriteQuit
        | CommandName::Xit => false,
        _ => return None,
    };
    // A bare command still names the command, rather than an empty filename.
    if parser.at == prefix.len() {
        return None;
    }
    parser.skip_space();
    Some(FilenameArgument {
        range: parser.at..cursor,
        directories_only,
    })
}

struct Parser<'a> {
    input: &'a str,
    at: usize,
}

impl<'a> Parser<'a> {
    fn new(input: &'a str) -> Self {
        Self { input, at: 0 }
    }

    fn parse(mut self) -> Result<ExCommand, ExParseError> {
        self.skip_space();
        if self.peek() == Some(':') {
            self.bump();
        }
        self.skip_space();
        if self.at == self.input.len() {
            return self.error(ExParseErrorKind::Empty);
        }

        let range = self.parse_range()?;
        self.skip_space();
        if self.at == self.input.len() {
            return match range {
                Some(ExRange::Single(address)) => Ok(ExCommand {
                    range: None,
                    bang: false,
                    action: ExAction::GoToLine(address),
                }),
                Some(_) => self.error(ExParseErrorKind::MissingCommand),
                None => self.error(ExParseErrorKind::Empty),
            };
        }

        let (name, bang) = self.parse_command_head(range.is_some())?;

        let args_offset = self.at;
        let raw_args = &self.input[self.at..];
        let args = match name {
            // A trailing Space is a real Normal-mode command.  Only the
            // whitespace separating `:normal[!]` from its payload is trivia.
            CommandName::Normal => raw_args.trim_start(),
            // For `:&` and `:~`, whitespace distinguishes a following line
            // count from an immediately adjacent occurrence number.  Keep
            // that leading separator for the substitute-tail parser.
            CommandName::RepeatSubstitute | CommandName::RepeatWithSearch => raw_args.trim_end(),
            _ => raw_args.trim(),
        };
        let action = self.parse_action(name, args, args_offset)?;
        Ok(ExCommand {
            range,
            bang,
            action,
        })
    }

    fn parse_command_head(&mut self, has_range: bool) -> Result<(CommandName, bool), ExParseError> {
        let command_offset = self.at;
        let command = if matches!(self.peek(), Some('&' | '~')) {
            self.bump().unwrap().to_string()
        } else {
            let start = self.at;
            while self.peek().is_some_and(|ch| ch.is_ascii_alphabetic()) {
                self.bump();
            }
            if start == self.at {
                return self.error(ExParseErrorKind::UnknownCommand(
                    self.input[self.at..].to_owned(),
                ));
            }
            if &self.input[start..self.at] == "E" {
                "E".to_owned()
            } else {
                self.input[start..self.at].to_ascii_lowercase()
            }
        };
        let name = (if command == "E" {
            Ok(CommandName::EditNewWindow)
        } else {
            resolve_command(&command)
        })
        .map_err(|kind| ExParseError {
            offset: command_offset,
            kind,
        })?;
        let bang = if self.peek() == Some('!') {
            self.bump();
            true
        } else {
            false
        };
        if bang && !name.accepts_bang() {
            return Err(ExParseError {
                offset: self.at - 1,
                kind: ExParseErrorKind::UnexpectedBang(name.canonical().to_owned()),
            });
        }
        if has_range && !name.accepts_range() {
            return Err(ExParseError {
                offset: command_offset,
                kind: ExParseErrorKind::UnexpectedRange(name.canonical().to_owned()),
            });
        }

        Ok((name, bang))
    }

    fn parse_range(&mut self) -> Result<Option<ExRange>, ExParseError> {
        if self.peek() == Some('%') {
            self.bump();
            if self.peek().is_some_and(|ch| matches!(ch, ',' | ';')) {
                return self.error(ExParseErrorKind::TooManyAddresses);
            }
            return Ok(Some(ExRange::WholeFile));
        }

        let Some(first) = self.parse_address()? else {
            return Ok(None);
        };
        self.skip_space();
        let separator = match self.peek() {
            Some(',') => Some(RangeSeparator::Comma),
            Some(';') => Some(RangeSeparator::Semicolon),
            _ => None,
        };
        let Some(separator) = separator else {
            return Ok(Some(ExRange::Single(first)));
        };
        self.bump();
        self.skip_space();
        let second = self.parse_address()?.ok_or(ExParseError {
            offset: self.at,
            kind: ExParseErrorKind::InvalidAddress,
        })?;
        self.skip_space();
        if self.peek().is_some_and(|ch| matches!(ch, ',' | ';')) {
            return self.error(ExParseErrorKind::TooManyAddresses);
        }
        Ok(Some(ExRange::Between {
            start: first,
            end: second,
            separator,
        }))
    }

    fn parse_address(&mut self) -> Result<Option<ExAddress>, ExParseError> {
        let start = self.at;
        let base = match self.peek() {
            Some('.') => {
                self.bump();
                AddressBase::Current
            }
            Some('$') => {
                self.bump();
                AddressBase::Last
            }
            Some(ch) if ch.is_ascii_digit() => AddressBase::Absolute(self.parse_u64()?),
            Some('+' | '-') => AddressBase::Current,
            _ => return Ok(None),
        };

        let mut offset = 0_i64;
        loop {
            self.skip_space();
            let sign = match self.peek() {
                Some('+') => 1_i64,
                Some('-') => -1_i64,
                _ => break,
            };
            self.bump();
            let amount = if self.peek().is_some_and(|ch| ch.is_ascii_digit()) {
                self.parse_u64()?
            } else {
                1
            };
            let signed = i64::try_from(amount)
                .ok()
                .and_then(|amount| amount.checked_mul(sign))
                .ok_or(ExParseError {
                    offset: start,
                    kind: ExParseErrorKind::AddressOverflow,
                })?;
            offset = offset.checked_add(signed).ok_or(ExParseError {
                offset: start,
                kind: ExParseErrorKind::AddressOverflow,
            })?;
        }
        Ok(Some(ExAddress { base, offset }))
    }

    fn parse_action(
        &self,
        name: CommandName,
        args: &str,
        args_offset: usize,
    ) -> Result<ExAction, ExParseError> {
        let no_args = |action| {
            if args.is_empty() {
                Ok(action)
            } else {
                Err(ExParseError {
                    offset: args_offset,
                    kind: ExParseErrorKind::UnexpectedArgument(args.to_owned()),
                })
            }
        };
        match name {
            CommandName::EditNewWindow => Ok(ExAction::EditNewWindow {
                path: optional_string(args),
            }),
            CommandName::PrintWorkingDirectory => no_args(ExAction::PrintWorkingDirectory),
            CommandName::CheckTime => no_args(ExAction::CheckTime),
            CommandName::ChangeDirectory => Ok(ExAction::ChangeDirectory {
                path: optional_string(args),
            }),
            CommandName::Update => no_args(ExAction::Update),
            CommandName::Split => Ok(ExAction::Split {
                path: optional_string(args),
            }),
            CommandName::Edit => Ok(ExAction::Edit {
                path: optional_string(args),
            }),
            CommandName::New => no_args(ExAction::New),
            CommandName::Write => Ok(ExAction::Write {
                path: optional_string(args),
            }),
            CommandName::SaveAs => Ok(ExAction::SaveAs {
                path: required_string(args, args_offset, "file name")?,
            }),
            CommandName::Quit => no_args(ExAction::Quit),
            CommandName::QuitAll => no_args(ExAction::QuitAll),
            CommandName::WriteQuit => Ok(ExAction::WriteQuit {
                path: optional_string(args),
            }),
            CommandName::Xit => Ok(ExAction::Xit {
                path: optional_string(args),
            }),
            CommandName::WriteAll => no_args(ExAction::WriteAll),
            CommandName::Undo => Ok(ExAction::Undo {
                change: parse_optional_count(args, args_offset)?,
            }),
            CommandName::Redo => no_args(ExAction::Redo),
            CommandName::Delete => Ok(ExAction::Delete(parse_register_count(args, args_offset)?)),
            CommandName::Yank => Ok(ExAction::Yank(parse_register_count(args, args_offset)?)),
            CommandName::Put => Ok(ExAction::Put {
                register: parse_optional_register(args, args_offset)?,
            }),
            CommandName::Join => Ok(ExAction::Join {
                count: parse_optional_count(args, args_offset)?,
            }),
            CommandName::Copy => Ok(ExAction::Copy {
                destination: parse_address_argument(args, args_offset)?,
            }),
            CommandName::Move => Ok(ExAction::Move {
                destination: parse_address_argument(args, args_offset)?,
            }),
            CommandName::Normal => Ok(ExAction::Normal {
                commands: required_string(args, args_offset, "Normal-mode command")?,
            }),
            CommandName::Sort => parse_sort(args, args_offset).map(ExAction::Sort),
            CommandName::Substitute => parse_substitute(args, args_offset),
            CommandName::RepeatSubstitute => {
                let (flags, count) = parse_repeat_substitute_tail(args, args_offset)?;
                Ok(ExAction::RepeatSubstitute {
                    pattern: RepeatPattern::LastSubstitute,
                    flags,
                    count,
                })
            }
            CommandName::RepeatWithSearch => {
                let (flags, count) = parse_repeat_substitute_tail(args, args_offset)?;
                Ok(ExAction::RepeatSubstitute {
                    pattern: RepeatPattern::LastSearch,
                    flags,
                    count,
                })
            }
            CommandName::Goto => Ok(ExAction::GoToByte {
                count: (!args.is_empty())
                    .then(|| parse_required_count(args, args_offset, "byte offset"))
                    .transpose()?,
            }),
            CommandName::Marks => Ok(ExAction::Marks {
                names: parse_name_list(args),
            }),
            CommandName::Registers => Ok(ExAction::Registers {
                names: parse_name_list(args),
            }),
            CommandName::Jumps => no_args(ExAction::Jumps),
            CommandName::Set => parse_set(args, SetScope::GlobalAndLocal, args_offset),
            CommandName::SetLocal => parse_set(args, SetScope::Local, args_offset),
        }
    }

    fn parse_u64(&mut self) -> Result<u64, ExParseError> {
        let start = self.at;
        while self.peek().is_some_and(|ch| ch.is_ascii_digit()) {
            self.bump();
        }
        self.input[start..self.at]
            .parse()
            .map_err(|_| ExParseError {
                offset: start,
                kind: ExParseErrorKind::InvalidNumber(self.input[start..self.at].to_owned()),
            })
    }

    fn skip_space(&mut self) {
        while self.peek().is_some_and(char::is_whitespace) {
            self.bump();
        }
    }

    fn peek(&self) -> Option<char> {
        self.input[self.at..].chars().next()
    }

    fn bump(&mut self) -> Option<char> {
        let ch = self.peek()?;
        self.at += ch.len_utf8();
        Some(ch)
    }

    fn error<T>(&self, kind: ExParseErrorKind) -> Result<T, ExParseError> {
        Err(ExParseError {
            offset: self.at,
            kind,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CommandName {
    EditNewWindow,
    PrintWorkingDirectory,
    CheckTime,
    ChangeDirectory,
    Update,
    Split,
    Edit,
    New,
    Write,
    SaveAs,
    Quit,
    QuitAll,
    WriteQuit,
    Xit,
    WriteAll,
    Undo,
    Redo,
    Delete,
    Yank,
    Put,
    Join,
    Copy,
    Move,
    Normal,
    Sort,
    Substitute,
    RepeatSubstitute,
    RepeatWithSearch,
    Goto,
    Marks,
    Registers,
    Jumps,
    Set,
    SetLocal,
}

impl CommandName {
    fn canonical(self) -> &'static str {
        match self {
            Self::EditNewWindow => "E",
            Self::PrintWorkingDirectory => "pwd",
            Self::CheckTime => "checktime",
            Self::ChangeDirectory => "cd",
            Self::Update => "update",
            Self::Split => "split",
            Self::Edit => "edit",
            Self::New => "enew",
            Self::Write => "write",
            Self::SaveAs => "saveas",
            Self::Quit => "quit",
            Self::QuitAll => "qall",
            Self::WriteQuit => "wq",
            Self::Xit => "xit",
            Self::WriteAll => "wall",
            Self::Undo => "undo",
            Self::Redo => "redo",
            Self::Delete => "delete",
            Self::Yank => "yank",
            Self::Put => "put",
            Self::Join => "join",
            Self::Copy => "copy",
            Self::Move => "move",
            Self::Normal => "normal",
            Self::Sort => "sort",
            Self::Substitute => "substitute",
            Self::RepeatSubstitute => "&",
            Self::RepeatWithSearch => "~",
            Self::Goto => "goto",
            Self::Marks => "marks",
            Self::Registers => "registers",
            Self::Jumps => "jumps",
            Self::Set => "set",
            Self::SetLocal => "setlocal",
        }
    }

    fn accepts_bang(self) -> bool {
        matches!(
            self,
            Self::Edit
                | Self::EditNewWindow
                | Self::Update
                | Self::New
                | Self::Write
                | Self::SaveAs
                | Self::Quit
                | Self::QuitAll
                | Self::WriteQuit
                | Self::Xit
                | Self::WriteAll
                | Self::Put
                | Self::Join
                | Self::Normal
                | Self::Sort
        )
    }

    fn accepts_range(self) -> bool {
        matches!(
            self,
            Self::Write
                | Self::WriteQuit
                | Self::Delete
                | Self::Yank
                | Self::Put
                | Self::Join
                | Self::Copy
                | Self::Move
                | Self::Normal
                | Self::Sort
                | Self::Substitute
                | Self::RepeatSubstitute
                | Self::RepeatWithSearch
                | Self::Goto
        )
    }
}

struct CommandSpec {
    name: CommandName,
    spelling: &'static str,
    minimum: usize,
}

const COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        name: CommandName::Sort,
        spelling: "sort",
        minimum: 3,
    },
    CommandSpec {
        name: CommandName::CheckTime,
        spelling: "checktime",
        minimum: 5,
    },
    CommandSpec {
        name: CommandName::PrintWorkingDirectory,
        spelling: "pwd",
        minimum: 2,
    },
    CommandSpec {
        name: CommandName::ChangeDirectory,
        spelling: "cd",
        minimum: 2,
    },
    CommandSpec {
        name: CommandName::ChangeDirectory,
        spelling: "chdir",
        minimum: 2,
    },
    CommandSpec {
        name: CommandName::Update,
        spelling: "update",
        minimum: 2,
    },
    CommandSpec {
        name: CommandName::Split,
        spelling: "split",
        minimum: 2,
    },
    CommandSpec {
        name: CommandName::Split,
        spelling: "vsplit",
        minimum: 2,
    },
    CommandSpec {
        name: CommandName::Quit,
        spelling: "close",
        minimum: 3,
    },
    CommandSpec {
        name: CommandName::Edit,
        spelling: "edit",
        minimum: 1,
    },
    CommandSpec {
        name: CommandName::New,
        spelling: "enew",
        minimum: 2,
    },
    CommandSpec {
        name: CommandName::Write,
        spelling: "write",
        minimum: 1,
    },
    CommandSpec {
        name: CommandName::SaveAs,
        spelling: "saveas",
        minimum: 3,
    },
    CommandSpec {
        name: CommandName::Quit,
        spelling: "quit",
        minimum: 1,
    },
    CommandSpec {
        name: CommandName::QuitAll,
        spelling: "qall",
        minimum: 2,
    },
    CommandSpec {
        name: CommandName::WriteQuit,
        spelling: "wq",
        minimum: 2,
    },
    CommandSpec {
        name: CommandName::Xit,
        spelling: "xit",
        minimum: 1,
    },
    CommandSpec {
        name: CommandName::WriteAll,
        spelling: "wall",
        minimum: 2,
    },
    CommandSpec {
        name: CommandName::Undo,
        spelling: "undo",
        minimum: 1,
    },
    CommandSpec {
        name: CommandName::Redo,
        spelling: "redo",
        minimum: 3,
    },
    CommandSpec {
        name: CommandName::Delete,
        spelling: "delete",
        minimum: 1,
    },
    CommandSpec {
        name: CommandName::Yank,
        spelling: "yank",
        minimum: 1,
    },
    CommandSpec {
        name: CommandName::Put,
        spelling: "put",
        minimum: 2,
    },
    CommandSpec {
        name: CommandName::Join,
        spelling: "join",
        minimum: 1,
    },
    CommandSpec {
        name: CommandName::Copy,
        spelling: "copy",
        minimum: 2,
    },
    CommandSpec {
        name: CommandName::Copy,
        spelling: "t",
        minimum: 1,
    },
    CommandSpec {
        name: CommandName::Move,
        spelling: "move",
        minimum: 1,
    },
    CommandSpec {
        name: CommandName::Normal,
        spelling: "normal",
        minimum: 4,
    },
    CommandSpec {
        name: CommandName::Substitute,
        spelling: "substitute",
        minimum: 1,
    },
    CommandSpec {
        name: CommandName::Goto,
        spelling: "goto",
        minimum: 2,
    },
    CommandSpec {
        name: CommandName::Marks,
        spelling: "marks",
        minimum: 3,
    },
    CommandSpec {
        name: CommandName::Registers,
        spelling: "registers",
        minimum: 3,
    },
    CommandSpec {
        name: CommandName::Jumps,
        spelling: "jumps",
        minimum: 2,
    },
    CommandSpec {
        name: CommandName::Set,
        spelling: "set",
        minimum: 2,
    },
    CommandSpec {
        name: CommandName::SetLocal,
        spelling: "setlocal",
        minimum: 4,
    },
];

fn resolve_command(input: &str) -> Result<CommandName, ExParseErrorKind> {
    if input == "&" {
        return Ok(CommandName::RepeatSubstitute);
    }
    if input == "~" {
        return Ok(CommandName::RepeatWithSearch);
    }
    let mut matches = COMMANDS
        .iter()
        .filter(|spec| input.len() >= spec.minimum && spec.spelling.starts_with(input));
    let Some(first) = matches.next() else {
        return Err(ExParseErrorKind::UnknownCommand(input.to_owned()));
    };
    if matches.any(|candidate| candidate.name != first.name) {
        return Err(ExParseErrorKind::AmbiguousCommand(input.to_owned()));
    }
    Ok(first.name)
}

fn optional_string(args: &str) -> Option<String> {
    (!args.is_empty()).then(|| args.to_owned())
}

fn required_string(args: &str, offset: usize, name: &'static str) -> Result<String, ExParseError> {
    if args.is_empty() {
        Err(ExParseError {
            offset,
            kind: ExParseErrorKind::MissingArgument(name),
        })
    } else {
        Ok(args.to_owned())
    }
}

fn parse_register_count(args: &str, offset: usize) -> Result<RegisterCount, ExParseError> {
    let mut parts = args.split_whitespace();
    let first = parts.next();
    let second = parts.next();
    if let Some(extra) = parts.next() {
        return Err(ExParseError {
            offset,
            kind: ExParseErrorKind::UnexpectedArgument(extra.to_owned()),
        });
    }
    let Some(first) = first else {
        return Ok(RegisterCount::default());
    };
    if let Ok(count) = first.parse::<u64>() {
        if let Some(second) = second {
            return Err(ExParseError {
                offset,
                kind: ExParseErrorKind::UnexpectedArgument(second.to_owned()),
            });
        }
        return Ok(RegisterCount {
            register: None,
            count: Some(count),
        });
    }
    let register = parse_register(first, offset)?;
    let count = second
        .map(|value| parse_number(value, offset))
        .transpose()?;
    Ok(RegisterCount {
        register: Some(register),
        count,
    })
}

fn parse_optional_register(args: &str, offset: usize) -> Result<Option<char>, ExParseError> {
    if args.is_empty() {
        return Ok(None);
    }
    if args.split_whitespace().count() != 1 {
        return Err(ExParseError {
            offset,
            kind: ExParseErrorKind::UnexpectedArgument(args.to_owned()),
        });
    }
    parse_register(args, offset).map(Some)
}

fn parse_register(value: &str, offset: usize) -> Result<char, ExParseError> {
    if value == "\"" {
        return Ok('"');
    }
    let value = value.strip_prefix('"').unwrap_or(value);
    let mut chars = value.chars();
    let Some(register) = chars.next() else {
        return Err(ExParseError {
            offset,
            kind: ExParseErrorKind::InvalidRegister(value.to_owned()),
        });
    };
    if chars.next().is_some() || !is_valid_register(register) {
        return Err(ExParseError {
            offset,
            kind: ExParseErrorKind::InvalidRegister(value.to_owned()),
        });
    }
    Ok(register)
}

fn is_valid_register(register: char) -> bool {
    register.is_ascii_alphanumeric() || matches!(register, '"' | '-' | '_' | '+' | '*' | '.' | '%')
}

fn parse_optional_count(args: &str, offset: usize) -> Result<Option<u64>, ExParseError> {
    if args.is_empty() {
        Ok(None)
    } else {
        parse_number(args, offset).map(Some)
    }
}

fn parse_required_count(
    args: &str,
    offset: usize,
    name: &'static str,
) -> Result<u64, ExParseError> {
    if args.is_empty() {
        Err(ExParseError {
            offset,
            kind: ExParseErrorKind::MissingArgument(name),
        })
    } else {
        parse_number(args, offset)
    }
}

fn parse_number(value: &str, offset: usize) -> Result<u64, ExParseError> {
    value.parse().map_err(|_| ExParseError {
        offset,
        kind: ExParseErrorKind::InvalidNumber(value.to_owned()),
    })
}

fn parse_address_argument(value: &str, offset: usize) -> Result<ExAddress, ExParseError> {
    if value.is_empty() {
        return Err(ExParseError {
            offset,
            kind: ExParseErrorKind::MissingArgument("destination address"),
        });
    }
    let mut parser = Parser::new(value);
    let address = parser.parse_address()?.ok_or(ExParseError {
        offset,
        kind: ExParseErrorKind::InvalidAddress,
    })?;
    parser.skip_space();
    if parser.at != value.len() {
        return Err(ExParseError {
            offset: offset + parser.at,
            kind: ExParseErrorKind::UnexpectedArgument(value[parser.at..].to_owned()),
        });
    }
    Ok(address)
}

fn parse_name_list(args: &str) -> Vec<char> {
    args.chars().filter(|ch| !ch.is_whitespace()).collect()
}

fn parse_sort(args: &str, offset: usize) -> Result<SortOptions, ExParseError> {
    let mut options = SortOptions::default();
    let mut at = 0;
    while at < args.len() {
        let ch = args[at..].chars().next().unwrap();
        let invalid = || ExParseError {
            offset: offset + at,
            kind: ExParseErrorKind::InvalidSortArgument(args[at..].into()),
        };
        match ch {
            ch if ch.is_whitespace() => {}
            'i' => options.ignore_case = true,
            'u' => options.unique = true,
            'r' => options.match_only = true,
            'n' | 'x' | 'o' | 'b' => {
                if options.radix.is_some() {
                    return Err(invalid());
                }
                options.radix = Some(match ch {
                    'n' => 10,
                    'x' => 16,
                    'o' => 8,
                    _ => 2,
                });
            }
            ch if !ch.is_alphanumeric()
                && !matches!(ch, '\\' | '"' | '|')
                && options.pattern.is_none() =>
            {
                let (pattern, next, closed) = read_delimited(args, at + ch.len_utf8(), ch);
                if !closed {
                    return Err(invalid());
                }
                options.pattern = Some(pattern);
                at = next;
                continue;
            }
            _ => return Err(invalid()),
        }
        at += ch.len_utf8();
    }
    Ok(options)
}

fn parse_substitute(args: &str, offset: usize) -> Result<ExAction, ExParseError> {
    if args.is_empty() {
        return Ok(ExAction::RepeatSubstitute {
            pattern: RepeatPattern::LastSubstitute,
            flags: SubstituteFlags::default(),
            count: None,
        });
    }
    let delimiter = args.chars().next().unwrap();
    if delimiter.is_ascii_alphanumeric()
        || delimiter.is_whitespace()
        || matches!(delimiter, '\\' | '"' | '|')
    {
        return Err(ExParseError {
            offset,
            kind: ExParseErrorKind::InvalidSubstituteDelimiter(delimiter),
        });
    }
    let mut at = delimiter.len_utf8();
    let (pattern, next, closed) = read_delimited(args, at, delimiter);
    if !closed {
        return Err(ExParseError {
            offset: offset + at,
            kind: ExParseErrorKind::UnterminatedSubstitutePattern,
        });
    }
    at = next;
    let (replacement, next, replacement_closed) = read_delimited(args, at, delimiter);
    at = next;
    let tail = if replacement_closed { &args[at..] } else { "" };
    let (flags, count) = parse_flags_and_count(tail, offset + at)?;
    Ok(ExAction::Substitute(Substitute {
        delimiter,
        pattern,
        replacement,
        flags,
        count,
    }))
}

/// Returns field, byte after delimiter/end, and whether a delimiter was found.
fn read_delimited(input: &str, mut at: usize, delimiter: char) -> (String, usize, bool) {
    let mut output = String::new();
    while at < input.len() {
        let ch = input[at..].chars().next().unwrap();
        if ch == delimiter {
            return (output, at + ch.len_utf8(), true);
        }
        if ch == '\\' {
            let slash_at = at;
            at += 1;
            if at < input.len() {
                let escaped = input[at..].chars().next().unwrap();
                if escaped == delimiter {
                    output.push(delimiter);
                    at += escaped.len_utf8();
                    continue;
                }
            }
            output.push_str(&input[slash_at..at]);
            continue;
        }
        output.push(ch);
        at += ch.len_utf8();
    }
    (output, at, false)
}

fn parse_repeat_substitute_tail(
    args: &str,
    offset: usize,
) -> Result<(SubstituteFlags, Option<u64>), ExParseError> {
    parse_flags_and_count(args, offset)
}

fn parse_flags_and_count(
    tail: &str,
    offset: usize,
) -> Result<(SubstituteFlags, Option<u64>), ExParseError> {
    if tail.trim().is_empty() {
        return Ok((SubstituteFlags::default(), None));
    }
    // Vim gives digits immediately following the final delimiter flag
    // semantics (the Nth occurrence), while whitespace makes the number a
    // line count.  Trimming before this split silently changes `:s/x/y/ 2`
    // into `:s/x/y/2`.
    let (flag_text, count_text) = if tail.starts_with(char::is_whitespace) {
        ("", tail.trim())
    } else {
        match tail.find(char::is_whitespace) {
            Some(at) => (&tail[..at], tail[at..].trim()),
            None => (tail, ""),
        }
    };
    let mut flags = SubstituteFlags::default();
    let mut digits = String::new();
    for ch in flag_text.chars() {
        if ch.is_ascii_digit() {
            digits.push(ch);
            continue;
        }
        if !digits.is_empty() {
            return Err(ExParseError {
                offset,
                kind: ExParseErrorKind::InvalidSubstituteFlag(ch),
            });
        }
        let slot = match ch {
            'g' => &mut flags.global,
            'c' => &mut flags.confirm,
            'p' => &mut flags.print,
            '#' => &mut flags.number,
            'l' => &mut flags.list,
            'e' => &mut flags.suppress_errors,
            '&' => &mut flags.use_previous_flags,
            'i' => {
                if flags.ignore_case.is_some() {
                    return Err(ExParseError {
                        offset,
                        kind: ExParseErrorKind::DuplicateSubstituteFlag(ch),
                    });
                }
                flags.ignore_case = Some(true);
                continue;
            }
            'I' => {
                if flags.ignore_case.is_some() {
                    return Err(ExParseError {
                        offset,
                        kind: ExParseErrorKind::DuplicateSubstituteFlag(ch),
                    });
                }
                flags.ignore_case = Some(false);
                continue;
            }
            _ => {
                return Err(ExParseError {
                    offset,
                    kind: ExParseErrorKind::InvalidSubstituteFlag(ch),
                });
            }
        };
        if *slot {
            return Err(ExParseError {
                offset,
                kind: ExParseErrorKind::DuplicateSubstituteFlag(ch),
            });
        }
        *slot = true;
    }
    if !digits.is_empty() {
        flags.occurrence = Some(parse_number(&digits, offset)?);
    }
    let count = if count_text.is_empty() {
        None
    } else {
        if count_text.split_whitespace().count() != 1 {
            return Err(ExParseError {
                offset,
                kind: ExParseErrorKind::UnexpectedArgument(count_text.to_owned()),
            });
        }
        Some(parse_number(count_text, offset)?)
    };
    Ok((flags, count))
}

fn parse_set(args: &str, scope: SetScope, offset: usize) -> Result<ExAction, ExParseError> {
    let operation = if args.is_empty() {
        SetOperation::ShowChanged
    } else if args == "all" {
        SetOperation::ShowAll
    } else {
        let options = args
            .split_whitespace()
            .map(|expression| parse_option(expression, offset))
            .collect::<Result<Vec<_>, _>>()?;
        SetOperation::Options(options)
    };
    Ok(ExAction::Set(SetCommand { scope, operation }))
}

fn parse_option(expression: &str, offset: usize) -> Result<OptionOperation, ExParseError> {
    if expression.is_empty() {
        return Err(ExParseError {
            offset,
            kind: ExParseErrorKind::InvalidOption(expression.to_owned()),
        });
    }
    for (operator, make_action) in [
        ("+=", OptionAction::Append as fn(String) -> OptionAction),
        ("^=", OptionAction::Prepend),
        ("-=", OptionAction::Remove),
        ("=", OptionAction::Assign),
    ] {
        if let Some((name, value)) = expression.split_once(operator) {
            validate_option_name(name, expression, offset)?;
            return Ok(OptionOperation {
                name: name.to_owned(),
                action: make_action(value.to_owned()),
            });
        }
    }

    let (name, action) = if let Some(name) = expression.strip_suffix('?') {
        (name, OptionAction::Query)
    } else if let Some(name) = expression.strip_suffix('!') {
        (name, OptionAction::Toggle)
    } else if let Some(name) = expression.strip_suffix('&') {
        (name, OptionAction::Reset)
    } else if let Some(name) = expression.strip_prefix("inv") {
        (name, OptionAction::Toggle)
    } else if let Some(name) = expression.strip_prefix("no") {
        (name, OptionAction::Disable)
    } else {
        (expression, OptionAction::Enable)
    };
    validate_option_name(name, expression, offset)?;
    Ok(OptionOperation {
        name: name.to_owned(),
        action,
    })
}

fn validate_option_name(name: &str, expression: &str, offset: usize) -> Result<(), ExParseError> {
    if name.is_empty()
        || !name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
    {
        Err(ExParseError {
            offset,
            kind: ExParseErrorKind::InvalidOption(expression.to_owned()),
        })
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(input: &str) -> ExCommand {
        parse_ex(input).unwrap()
    }

    #[test]
    fn parses_symbolic_addresses_offsets_and_separator_semantics() {
        assert_eq!(
            parse(":.-2,$+1delete").range,
            Some(ExRange::Between {
                start: ExAddress {
                    base: AddressBase::Current,
                    offset: -2,
                },
                end: ExAddress {
                    base: AddressBase::Last,
                    offset: 1,
                },
                separator: RangeSeparator::Comma,
            })
        );
        assert_eq!(
            parse("1;.+2join").range,
            Some(ExRange::Between {
                start: ExAddress {
                    base: AddressBase::Absolute(1),
                    offset: 0,
                },
                end: ExAddress {
                    base: AddressBase::Current,
                    offset: 2,
                },
                separator: RangeSeparator::Semicolon,
            })
        );
    }

    #[test]
    fn percent_is_a_first_class_whole_file_range() {
        let command = parse("%s/old/new/g");
        assert_eq!(command.range, Some(ExRange::WholeFile));
        assert!(matches!(command.action, ExAction::Substitute(_)));
    }

    #[test]
    fn bare_numeric_address_is_a_line_jump() {
        assert_eq!(
            parse(":42").action,
            ExAction::GoToLine(ExAddress {
                base: AddressBase::Absolute(42),
                offset: 0,
            })
        );
    }

    #[test]
    fn parses_file_commands_abbreviations_and_bangs() {
        assert_eq!(
            parse(":e! notes.md"),
            ExCommand {
                range: None,
                bang: true,
                action: ExAction::Edit {
                    path: Some("notes.md".to_owned()),
                },
            }
        );
        assert_eq!(parse(":ene!").action, ExAction::New);
        assert_eq!(
            parse(":w draft.txt").action,
            ExAction::Write {
                path: Some("draft.txt".to_owned()),
            }
        );
        assert_eq!(
            parse(":sav copy.md").action,
            ExAction::SaveAs {
                path: "copy.md".to_owned(),
            }
        );
        assert_eq!(parse(":q!").action, ExAction::Quit);
        assert_eq!(parse(":qa!").action, ExAction::QuitAll);
        assert_eq!(parse(":wq").action, ExAction::WriteQuit { path: None });
        assert_eq!(parse(":x").action, ExAction::Xit { path: None });
        assert_eq!(parse(":wa").action, ExAction::WriteAll);
    }

    #[test]
    fn editing_commands_have_typed_arguments() {
        assert_eq!(
            parse(":2,4d a 3").action,
            ExAction::Delete(RegisterCount {
                register: Some('a'),
                count: Some(3),
            })
        );
        assert_eq!(
            parse(":y \"0").action,
            ExAction::Yank(RegisterCount {
                register: Some('0'),
                count: None,
            })
        );
        assert_eq!(
            parse(":put! +").action,
            ExAction::Put {
                register: Some('+')
            }
        );
        assert_eq!(
            parse(":put %").action,
            ExAction::Put {
                register: Some('%')
            }
        );
        assert_eq!(
            parse(":put \"").action,
            ExAction::Put {
                register: Some('"')
            }
        );
        assert_eq!(parse(":j! 4").action, ExAction::Join { count: Some(4) });
        assert_eq!(
            parse(":copy $-1").action,
            ExAction::Copy {
                destination: ExAddress {
                    base: AddressBase::Last,
                    offset: -1,
                }
            }
        );
        assert_eq!(
            parse(":m .+2").action,
            ExAction::Move {
                destination: ExAddress {
                    base: AddressBase::Current,
                    offset: 2,
                }
            }
        );
        assert_eq!(
            parse(":%norm! gU$").action,
            ExAction::Normal {
                commands: "gU$".to_owned()
            }
        );
    }

    #[test]
    fn parses_substitute_escaped_delimiters_flags_occurrence_and_count() {
        let ExAction::Substitute(substitute) = parse(":1,$s/a\\/b/c\\/d/gci2 5").action else {
            panic!("expected substitute")
        };
        assert_eq!(substitute.pattern, "a/b");
        assert_eq!(substitute.replacement, "c/d");
        assert!(substitute.flags.global);
        assert!(substitute.flags.confirm);
        assert_eq!(substitute.flags.ignore_case, Some(true));
        assert_eq!(substitute.flags.occurrence, Some(2));
        assert_eq!(substitute.count, Some(5));
    }

    #[test]
    fn substitute_whitespace_distinguishes_line_count_from_occurrence() {
        let ExAction::Substitute(counted) = parse(":s/x/y/ 2").action else {
            panic!("expected substitute")
        };
        assert_eq!(counted.flags.occurrence, None);
        assert_eq!(counted.count, Some(2));

        let ExAction::Substitute(occurrence) = parse(":s/x/y/2").action else {
            panic!("expected substitute")
        };
        assert_eq!(occurrence.flags.occurrence, Some(2));
        assert_eq!(occurrence.count, None);

        assert!(matches!(
            parse(":& 3").action,
            ExAction::RepeatSubstitute {
                pattern: RepeatPattern::LastSubstitute,
                count: Some(3),
                ..
            }
        ));
        assert!(matches!(
            parse(":~ 4").action,
            ExAction::RepeatSubstitute {
                pattern: RepeatPattern::LastSearch,
                count: Some(4),
                ..
            }
        ));
    }

    #[test]
    fn normal_preserves_significant_trailing_space() {
        assert_eq!(
            parse(":normal! A ").action,
            ExAction::Normal {
                commands: "A ".to_owned(),
            }
        );
    }

    #[test]
    fn substitute_allows_an_omitted_final_delimiter() {
        let ExAction::Substitute(substitute) = parse(":s/foo/bar").action else {
            panic!("expected substitute")
        };
        assert_eq!(substitute.pattern, "foo");
        assert_eq!(substitute.replacement, "bar");
        assert_eq!(substitute.flags, SubstituteFlags::default());
    }

    #[test]
    fn ampersand_tilde_and_bare_substitute_are_typed_repeats() {
        assert!(matches!(
            parse(":s").action,
            ExAction::RepeatSubstitute {
                pattern: RepeatPattern::LastSubstitute,
                ..
            }
        ));
        assert!(matches!(
            parse(":&g").action,
            ExAction::RepeatSubstitute {
                pattern: RepeatPattern::LastSubstitute,
                ..
            }
        ));
        assert!(matches!(
            parse(":~c").action,
            ExAction::RepeatSubstitute {
                pattern: RepeatPattern::LastSearch,
                ..
            }
        ));
    }

    #[test]
    fn parses_set_queries_boolean_forms_and_assignments() {
        assert_eq!(
            parse(":set").action,
            ExAction::Set(SetCommand {
                scope: SetScope::GlobalAndLocal,
                operation: SetOperation::ShowChanged,
            })
        );
        assert_eq!(
            parse(":setlocal wrap? nowrap invlinebreak ff=dos path+=foo").action,
            ExAction::Set(SetCommand {
                scope: SetScope::Local,
                operation: SetOperation::Options(vec![
                    OptionOperation {
                        name: "wrap".to_owned(),
                        action: OptionAction::Query,
                    },
                    OptionOperation {
                        name: "wrap".to_owned(),
                        action: OptionAction::Disable,
                    },
                    OptionOperation {
                        name: "linebreak".to_owned(),
                        action: OptionAction::Toggle,
                    },
                    OptionOperation {
                        name: "ff".to_owned(),
                        action: OptionAction::Assign("dos".to_owned()),
                    },
                    OptionOperation {
                        name: "path".to_owned(),
                        action: OptionAction::Append("foo".to_owned()),
                    },
                ]),
            })
        );
    }

    #[test]
    fn navigation_and_information_commands_parse() {
        assert_eq!(parse(":undo").action, ExAction::Undo { change: None });
        assert_eq!(parse(":u 42").action, ExAction::Undo { change: Some(42) });
        assert_eq!(
            parse(":go 100").action,
            ExAction::GoToByte { count: Some(100) }
        );
        assert_eq!(parse(":goto").action, ExAction::GoToByte { count: None });
        assert_eq!(
            parse(":5goto").range,
            Some(ExRange::Single(ExAddress {
                base: AddressBase::Absolute(5),
                offset: 0,
            }))
        );
        assert_eq!(
            parse(":marks ab").action,
            ExAction::Marks {
                names: vec!['a', 'b']
            }
        );
        assert_eq!(
            parse(":reg 0a").action,
            ExAction::Registers {
                names: vec!['0', 'a']
            }
        );
        assert_eq!(parse(":ju").action, ExAction::Jumps);
    }

    #[test]
    fn stacked_split_aliases_preserve_paths_and_reject_unsupported_modifiers() {
        for command in [":sp", ":split", ":vs", ":vsplit"] {
            assert_eq!(parse(command).action, ExAction::Split { path: None });
            assert_eq!(
                parse(&format!("{command} notes file.md")).action,
                ExAction::Split {
                    path: Some("notes file.md".to_owned())
                }
            );
            assert!(parse_ex(&format!("{command}!")).is_err());
        }
        assert!(parse_ex(":1,2split").is_err());
        assert_eq!(parse(":clo").action, ExAction::Quit);
    }

    #[test]
    fn errors_are_structured_and_non_approximating() {
        assert!(matches!(
            parse_ex(":frobnicate"),
            Err(ExParseError {
                kind: ExParseErrorKind::UnknownCommand(_),
                ..
            })
        ));
        assert!(matches!(
            parse_ex(":undo!"),
            Err(ExParseError {
                kind: ExParseErrorKind::UnexpectedBang(_),
                ..
            })
        ));
        assert!(matches!(
            parse_ex(":1,2quit"),
            Err(ExParseError {
                kind: ExParseErrorKind::UnexpectedRange(_),
                ..
            })
        ));
        assert!(matches!(
            parse_ex(":s foo"),
            Err(ExParseError {
                kind: ExParseErrorKind::InvalidSubstituteDelimiter('f'),
                ..
            })
        ));
        assert!(matches!(
            parse_ex(":s/a/b/gg"),
            Err(ExParseError {
                kind: ExParseErrorKind::DuplicateSubstituteFlag('g'),
                ..
            })
        ));
        assert!(matches!(
            parse_ex(":set no"),
            Err(ExParseError {
                kind: ExParseErrorKind::InvalidOption(_),
                ..
            })
        ));
    }

    #[test]
    fn incomplete_and_overlong_ranges_are_rejected() {
        assert!(matches!(
            parse_ex(":1,"),
            Err(ExParseError {
                kind: ExParseErrorKind::InvalidAddress,
                ..
            })
        ));
        assert!(matches!(
            parse_ex(":1,2,3delete"),
            Err(ExParseError {
                kind: ExParseErrorKind::TooManyAddresses,
                ..
            })
        ));
        assert!(matches!(
            parse_ex(":%"),
            Err(ExParseError {
                kind: ExParseErrorKind::MissingCommand,
                ..
            })
        ));
    }
}
