//! Portable status and Ex output. Hosts display these strings without rebuilding them.
use super::*;

pub(super) fn command_diagnostic(status: &CommandStatus) -> Option<String> {
    use crate::command::visual_block::VisualBlockError;
    use crate::command::{CountError, ExCommandError};
    Some(match status {
        CommandStatus::Unsupported(message) | CommandStatus::Error(message) => message.clone(),
        CommandStatus::RegisterReadError(error) => error.to_string(),
        CommandStatus::RegisterWriteError(error) => error.to_string(),
        CommandStatus::ExError(ExCommandError::Parse(error)) => error.to_string(),
        CommandStatus::ExError(ExCommandError::Execute(error)) => error.to_string(),
        CommandStatus::CountError(CountError::Overflow) => "Command count is too large".into(),
        CommandStatus::CountError(CountError::ReplayEventBudgetExceeded {
            count,
            events_per_iteration,
            limit,
        }) => format!(
            "Repeating {events_per_iteration} events {count} times exceeds the {limit}-event limit"
        ),
        CommandStatus::VisualBlockError(error) => match error {
            VisualBlockError::NeedsLayout(_)
            | VisualBlockError::EmptyLayout
            | VisualBlockError::EndpointNotInLayout(_) => "Block selection needs visible layout",
            VisualBlockError::WrongDocument { .. }
            | VisualBlockError::WrongDocumentRevision { .. }
            | VisualBlockError::StaleLayout { .. }
            | VisualBlockError::TextDoesNotMatchLayout => "Block selection is no longer current",
            VisualBlockError::NonGraphemeBoundary(_) => "Block selection splits a text character",
            VisualBlockError::OverlappingRanges => "Block selection ranges overlap",
            VisualBlockError::InvalidX => "Block selection has an invalid horizontal position",
            VisualBlockError::ReplacementTooLarge { .. } => "Block replacement is too large",
        }
        .into(),
        CommandStatus::SearchNotFound => "Pattern not found".into(),
        CommandStatus::Complete
        | CommandStatus::Pending
        | CommandStatus::Cancelled
        | CommandStatus::NeedsMoreLayout(_) => return None,
    })
}

fn visible(text: &str) -> String {
    text.chars()
        .map(|c| match c as u32 {
            0..=31 => format!("^{}", char::from_u32(c as u32 + 64).unwrap()),
            127 => "^?".into(),
            128..=159 => format!("<{:02x}>", c as u32),
            _ => c.to_string(),
        })
        .collect()
}
fn option_text(option: &crate::command::ex_execute::ExOptionDisplay) -> String {
    let name = option.name.vim_name();
    let value = match &option.value {
        ExOptionValue::Boolean(value) => {
            return format!("{}{name}", if *value { "" } else { "no" })
        }
        ExOptionValue::String(value) => value.clone(),
        ExOptionValue::Integer(value) => value.to_string(),
        ExOptionValue::Number(value) => value.to_string(),
        ExOptionValue::OptionalNumber(value) => value.unwrap_or(0).to_string(),
        ExOptionValue::FileFormat(value) => format_name(*value).into(),
        ExOptionValue::FileFormats(values) => values
            .iter()
            .map(|v| format_name(*v))
            .collect::<Vec<_>>()
            .join(","),
        // Only resolved display values reach an Info request.
        ExOptionValue::Indentation(_)
        | ExOptionValue::Reflow(_)
        | ExOptionValue::VisibleWhitespace(_) => return String::new(),
    };
    format!("{name}={value}")
}
fn format_name(format: FileFormat) -> &'static str {
    match format {
        FileFormat::Unix => "unix",
        FileFormat::Dos => "dos",
        FileFormat::Mac => "mac",
    }
}

pub(super) fn request_text(
    request: &ExFrontendRequest,
    payload: Option<&OwnedExInfoPayload>,
) -> Option<String> {
    let ExFrontendRequest::Info(request) = request else {
        return None;
    };
    Some(match request {
        ExInfoRequest::Message(text) => text.clone(),
        ExInfoRequest::Options(options) => options
            .iter()
            .map(option_text)
            .collect::<Vec<_>>()
            .join("  "),
        ExInfoRequest::Marks(_) => {
            let Some(OwnedExInfoPayload::Marks(marks)) = payload else {
                return None;
            };
            std::iter::once("mark  line  col  text".into())
                .chain(marks.iter().map(|mark| {
                    format!(
                        "{}  {}  {}  {}",
                        mark.name,
                        mark.point.hard_line_index + 1,
                        mark.point.grapheme_column,
                        visible(&mark.point.line_text)
                    )
                }))
                .collect::<Vec<_>>()
                .join("\n")
        }
        ExInfoRequest::Registers(_) => {
            let Some(OwnedExInfoPayload::Registers(registers)) = payload else {
                return None;
            };
            std::iter::once("--- Registers ---".into())
                .chain(registers.iter().map(|register| {
                    format!("\"{}   {}", register.name, visible(&register.value.text))
                }))
                .collect::<Vec<_>>()
                .join("\n")
        }
        ExInfoRequest::Jumps => {
            let Some(OwnedExInfoPayload::Jumps(jumps)) = payload else {
                return None;
            };
            std::iter::once(" jump  line  col  text".into())
                .chain(jumps.iter().map(|jump| {
                    format!(
                        "{} {}  {}  {}  {}",
                        if jump.current { ">" } else { " " },
                        jump.list_index,
                        jump.point.hard_line_index + 1,
                        jump.point.grapheme_column,
                        visible(&jump.point.line_text)
                    )
                }))
                .collect::<Vec<_>>()
                .join("\n")
        }
        ExInfoRequest::PrintLines { number, list, .. } => {
            let Some(OwnedExInfoPayload::TextLines(lines)) = payload else {
                return None;
            };
            lines
                .iter()
                .map(|line| {
                    let text = if *list {
                        visible(&line.text) + "$"
                    } else {
                        line.text.clone()
                    };
                    if *number {
                        format!("{}\t{text}", line.hard_line_index + 1)
                    } else {
                        text
                    }
                })
                .collect::<Vec<_>>()
                .join("\n")
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn options_use_signed_numbers_named_formats_and_vim_boolean_spelling() {
        use crate::command::ex_execute::ExOptionDisplay;
        for (name, value, text) in [
            (ExOptionName::Wrap, ExOptionValue::Boolean(false), "nowrap"),
            (
                ExOptionName::SoftTabStop,
                ExOptionValue::Integer(-1),
                "softtabstop=-1",
            ),
            (
                ExOptionName::FileFormat,
                ExOptionValue::FileFormat(FileFormat::Dos),
                "fileformat=dos",
            ),
            (
                ExOptionName::FileFormats,
                ExOptionValue::FileFormats(vec![FileFormat::Mac, FileFormat::Unix]),
                "fileformats=mac,unix",
            ),
        ] {
            assert_eq!(option_text(&ExOptionDisplay { name, value }), text);
        }
        assert_eq!(visible("a\0\t\n\r\u{7f}\u{80}é"), "a^@^I^J^M^?<80>é");
    }
    #[test]
    fn non_ex_failures_have_portable_diagnostics() {
        use crate::command::CountError;
        assert_eq!(
            command_diagnostic(&CommandStatus::CountError(CountError::Overflow)).as_deref(),
            Some("Command count is too large")
        );
        assert_eq!(
            command_diagnostic(&CommandStatus::Unsupported(
                "Cannot edit this selection".into()
            ))
            .as_deref(),
            Some("Cannot edit this selection")
        );
        assert_eq!(
            command_diagnostic(&CommandStatus::Error("test error".into())).as_deref(),
            Some("test error")
        );
        assert_eq!(command_diagnostic(&CommandStatus::Pending), None);
    }
}
