//! Portable Vim process arguments. Paths remain literal strings; the native
//! host resolves them and opens files only after parsing succeeds.
use serde::Serialize;

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LaunchArguments {
    pub filenames: Vec<String>,
    /// None opens one view; zero requests one stacked pane per argument.
    pub split_count: Option<usize>,
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
        } else if !literal && argument.starts_with("-o") {
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
        for value in ["-x", "-O", "-"] {
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
