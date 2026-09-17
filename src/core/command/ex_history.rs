//! Vim spelling and informational output for chronological undo navigation.
use super::ex::{ExParseError, ExParseErrorKind};
use crate::document::{Document, HistoryTimeAmount};
use std::fmt::Write;
use std::time::{SystemTime, UNIX_EPOCH};

pub(super) fn parse_amount(args: &str, offset: usize) -> Result<HistoryTimeAmount, ExParseError> {
    if args.is_empty() {
        return Ok(HistoryTimeAmount::Changes(1));
    }
    let digits = args.bytes().take_while(u8::is_ascii_digit).count();
    let invalid = || ExParseError {
        offset,
        kind: ExParseErrorKind::InvalidNumber(args.to_owned()),
    };
    let number = args[..digits].parse::<u64>().map_err(|_| invalid())?;
    let seconds = |factor| {
        number
            .checked_mul(factor)
            .map(HistoryTimeAmount::Seconds)
            .ok_or_else(invalid)
    };
    match &args[digits..] {
        "" => Ok(HistoryTimeAmount::Changes(number)),
        "s" => seconds(1),
        "m" => seconds(60),
        "h" => seconds(3_600),
        "d" => seconds(86_400),
        "f" => Ok(HistoryTimeAmount::Writes(number)),
        _ => Err(invalid()),
    }
}

pub(super) fn format_undo_list(document: &Document) -> String {
    let entries = document.history_timeline_leaves();
    if entries.is_empty() {
        return "Nothing to undo".to_owned();
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let mut output = String::from("number changes  when                 saved");
    for entry in entries {
        let age = now.saturating_sub(entry.timestamp);
        let saved = entry
            .saved_write
            .map(|number| number.to_string())
            .unwrap_or_default();
        let _ = write!(
            output,
            "\n{:6} {:7}  {:<21}{}",
            entry.location.change,
            entry.changes,
            format!("{age} seconds ago"),
            saved
        );
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::ex::{parse_ex as parse, ExAction};

    #[test]
    fn history_grammar_validates_units_abbreviations_and_overflow() {
        for (text, later, amount) in [
            (":ea", false, HistoryTimeAmount::Changes(1)),
            (":lat 2", true, HistoryTimeAmount::Changes(2)),
            (":earlier 0", false, HistoryTimeAmount::Changes(0)),
            (":earlier 10s", false, HistoryTimeAmount::Seconds(10)),
            (":later 2m", true, HistoryTimeAmount::Seconds(120)),
            (":later 3h", true, HistoryTimeAmount::Seconds(10_800)),
            (":earlier 4d", false, HistoryTimeAmount::Seconds(345_600)),
            (":earlier 2f", false, HistoryTimeAmount::Writes(2)),
        ] {
            assert_eq!(
                parse(text).unwrap().action,
                ExAction::HistoryTime { later, amount }
            );
        }
        assert_eq!(parse(":undol").unwrap().action, ExAction::UndoList);
        for text in [
            ":earlier +1",
            ":later -1",
            ":ea s",
            ":ea 1 s",
            ":ea 1S",
            ":lat 1f extra",
            ":earlier 18446744073709551615d",
            ":earlier!",
            ":1later",
            ":undolist x",
            ":undolist!",
            ":%undolist",
        ] {
            assert!(parse(text).is_err(), "{text}");
        }
    }
}
