//! Portable Vim process arguments. Paths remain literal strings; the native
//! host resolves them and opens files only after parsing succeeds.
use serde::Serialize;

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LaunchArguments {
    pub filenames: Vec<String>,
    /// None opens one view; zero requests one stacked pane per argument.
    pub split_count: Option<usize>,
    pub vertical_splits: bool,
    /// One-based line in the first file; u64::MAX means its last line.
    pub initial_line: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LaunchArgumentError {
    UnknownOption(String),
    InvalidSplitCount(String),
    InvalidInitialLine(String),
    EmptyFilename,
}

impl std::fmt::Display for LaunchArgumentError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownOption(value) => write!(formatter, "Unsupported option: {value}. Use -- before filenames starting with - or +."),
            Self::InvalidSplitCount(value) => write!(formatter, "Invalid split count: {value}. Use -o or -o followed immediately by a nonnegative number."),
            Self::InvalidInitialLine(value) => write!(formatter, "Invalid initial line: {value}. Use + followed by a line number, or + for the last line."),
            Self::EmptyFilename => formatter.write_str("A filename cannot be empty."),
        }
    }
}
impl std::error::Error for LaunchArgumentError {}

/// Parse argv excluding its executable name. Options may appear between files.
pub fn parse_launch_arguments<I, S>(arguments: I) -> Result<LaunchArguments, LaunchArgumentError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut result = LaunchArguments::default();
    let mut literal = false;
    for argument in arguments {
        let argument = argument.as_ref();
        if !literal && argument == "--" {
            literal = true;
        } else if !literal && (argument.starts_with("-o") || argument.starts_with("-O")) {
            result.vertical_splits = argument.starts_with("-O");
            let digits = &argument[2..];
            result.split_count = Some(if digits.is_empty() {
                0
            } else if digits.bytes().all(|byte| byte.is_ascii_digit()) {
                digits
                    .parse::<usize>()
                    .ok()
                    .filter(|count| *count <= isize::MAX as usize)
                    .ok_or_else(|| LaunchArgumentError::InvalidSplitCount(argument.to_owned()))?
            } else {
                return Err(LaunchArgumentError::InvalidSplitCount(argument.to_owned()));
            });
        } else if !literal && argument.starts_with('+') {
            let digits = &argument[1..];
            result.initial_line = Some(if digits.is_empty() {
                u64::MAX
            } else if digits.bytes().all(|byte| byte.is_ascii_digit()) {
                digits
                    .parse()
                    .map_err(|_| LaunchArgumentError::InvalidInitialLine(argument.to_owned()))?
            } else {
                return Err(LaunchArgumentError::InvalidInitialLine(argument.to_owned()));
            });
        } else if !literal && argument.starts_with('-') {
            return Err(LaunchArgumentError::UnknownOption(argument.to_owned()));
        } else if argument.is_empty() {
            return Err(LaunchArgumentError::EmptyFilename);
        } else {
            result.filenames.push(argument.to_owned());
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn files_preserve_order_spelling_duplicates_and_wildcards() {
        let parsed =
            parse_launch_arguments(["first file.md", "../β.txt", "*.rs", "first file.md"]).unwrap();
        assert_eq!(
            parsed.filenames,
            ["first file.md", "../β.txt", "*.rs", "first file.md"]
        );
        assert_eq!(parsed.split_count, None);
        assert_eq!(parsed.initial_line, None);
        assert_eq!(
            parse_launch_arguments::<[&str; 0], &str>([]).unwrap(),
            LaunchArguments::default()
        );
    }
    #[test]
    fn interspersed_repeated_options_use_last_value() {
        let parsed = parse_launch_arguments(["+12", "a", "-o2", "b", "+123", "-o3"]).unwrap();
        assert_eq!(parsed.filenames, ["a", "b"]);
        assert_eq!(parsed.split_count, Some(3));
        assert_eq!(parsed.initial_line, Some(123));
    }
    #[test]
    fn zero_or_omitted_split_count_means_all_and_count_is_attached() {
        for flag in ["-o", "-o0", "-o000"] {
            let parsed = parse_launch_arguments([flag, "2", "a"]).unwrap();
            assert_eq!(parsed.split_count, Some(0));
            assert_eq!(parsed.filenames, ["2", "a"]);
        }
    }
    #[test]
    fn vertical_startup_and_last_orientation_win() {
        let parsed=parse_launch_arguments(["-o2","-O3","first","second"]).unwrap();
        assert_eq!(parsed.split_count,Some(3));assert!(parsed.vertical_splits);
        assert!(!parse_launch_arguments(["-O","-o"]).unwrap().vertical_splits);
        assert_eq!(parse_launch_arguments(["-O"]).unwrap().split_count,Some(0));
    }
    #[test]
    fn plus_defaults_to_last_line_and_accepts_zero_and_leading_zeroes() {
        assert_eq!(
            parse_launch_arguments(["+"]).unwrap().initial_line,
            Some(u64::MAX)
        );
        assert_eq!(
            parse_launch_arguments(["+0"]).unwrap().initial_line,
            Some(0)
        );
        assert_eq!(
            parse_launch_arguments(["+00123"]).unwrap().initial_line,
            Some(123)
        );
    }
    #[test]
    fn terminator_makes_all_remaining_arguments_literal() {
        let parsed = parse_launch_arguments(["-o", "--", "-o2", "+123", "--", "-"]).unwrap();
        assert_eq!(parsed.filenames, ["-o2", "+123", "--", "-"]);
        assert_eq!(parsed.split_count, Some(0));
        assert_eq!(parsed.initial_line, None);
    }
    #[test]
    fn invalid_or_overflowing_options_do_not_become_filenames() {
        for value in ["-x", "-"] {
            assert!(matches!(
                parse_launch_arguments([value]),
                Err(LaunchArgumentError::UnknownOption(_))
            ));
        }
        for value in ["-o-1", "-o1x", "-o18446744073709551616"] {
            assert!(matches!(
                parse_launch_arguments([value]),
                Err(LaunchArgumentError::InvalidSplitCount(_))
            ));
        }
        for value in ["+-1", "+1x", "+/word", "+18446744073709551616"] {
            assert!(matches!(
                parse_launch_arguments([value]),
                Err(LaunchArgumentError::InvalidInitialLine(_))
            ));
        }
        assert_eq!(
            parse_launch_arguments([""]),
            Err(LaunchArgumentError::EmptyFilename)
        );
    }
}

/// A startup file error. Other lines still apply, in source order.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct StartupDiagnostic {
    pub line: usize,
    pub message: String,
}

impl super::CommandInterpreter {
    /// Run `startup.viem` as Vim script. Startup may configure options,
    /// mappings, abbreviations, variables, functions, commands and
    /// autocommands, but cannot change the source artifact or invoke host
    /// file/window actions. Errors become diagnostics with their line and
    /// later lines still run, as in Vim.
    pub fn configure_startup(
        &mut self,
        document: &mut crate::document::Document,
        text: &str,
    ) -> Vec<StartupDiagnostic> {
        self.configure_startup_with_effects(document, text).0
    }

    pub(crate) fn configure_startup_with_effects(
        &mut self, document: &mut crate::document::Document, text: &str,
    ) -> (Vec<StartupDiagnostic>, Vec<super::ex_execute::ExFrontendRequest>) {
        use crate::vimscript::Runtime;
        let mut runtime = Runtime::new(self.script.clone());
        runtime.restricted = true;
        if let Err(error) = runtime.push_script("startup.viem", text.as_bytes()) {
            return (vec![StartupDiagnostic { line: 1, message: error.text() }], Vec::new());
        }
        self.script_depth += 1;
        let mut host = super::scripting::InterpreterHost::new(self, document);
        host.startup = true;
        runtime.run(&mut host);
        host.commands.script_depth -= 1;
        let effects = host.merged.take().and_then(|output| output.ex_outcome)
            .map(|outcome| outcome.frontend_requests).unwrap_or_default();
        let diagnostics = runtime
            .errors
            .iter()
            .map(|error| StartupDiagnostic { line: error.lnum.unwrap_or(1), message: error.error.text() })
            .collect();
        (diagnostics, effects)
    }

    pub(crate) fn startup_view_options(&self) -> (bool, super::VisibleWhitespaceSetting, bool) {
        (self.wrap, self.visible_whitespace.clone(), self.number)
    }
    pub(crate) fn install_startup_view_options(
        &mut self,
        options: &(bool, super::VisibleWhitespaceSetting, bool),
    ) {
        self.wrap = options.0;
        self.visible_whitespace = options.1.clone();
        self.number = options.2;
    }
}
