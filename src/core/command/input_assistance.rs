//! Portable smart-quote assistance shared by document input paths.
use super::{CommandInterpreter, Mode, RegisterValue};
use crate::document::{BoundaryAffinity, Document, DocumentError};

#[derive(Clone, Debug, Default)]
pub(crate) struct InputAssistance {
    pub(super) literal: bool,
    smart_quotes: bool,
}

impl CommandInterpreter {
    /// An application preference copied to each view. Changing it never edits
    /// source or contributes an undo unit.
    pub fn set_smart_quotes(&mut self, enabled: bool) {
        self.input_assistance.smart_quotes = enabled;
    }

    pub fn smart_quotes(&self) -> bool {
        self.input_assistance.smart_quotes
    }

    /// Apply the view's quote preference exactly where an input payload is
    /// placed. Callers resolve counts/ranges first, and keep the original value
    /// for replay so the next destination gets its own context. The register
    /// itself is never rewritten.
    pub(crate) fn assist_input_payload(
        &self,
        document: &Document,
        range: std::ops::Range<usize>,
        affinity: BoundaryAffinity,
        value: &RegisterValue,
    ) -> Result<RegisterValue, DocumentError> {
        self.assist_input_payload_with_literals(document, range, affinity, value, &[])
    }

    pub(super) fn assist_input_payload_with_literals(
        &self,
        document: &Document,
        range: std::ops::Range<usize>,
        affinity: BoundaryAffinity,
        value: &RegisterValue,
        literal_ranges: &[std::ops::Range<usize>],
    ) -> Result<RegisterValue, DocumentError> {
        if self.input_assistance.literal
            || document.format().is_code()
            || !self.smart_quotes()
            || !value.text.contains(['\'', '"'])
        {
            return Ok(value.clone());
        }
        let transformed = (|| {
            if let Some(fragment) = value.clipboard_fragment() {
                if document.is_code_at(range.start, affinity)? {
                    return Ok((value.text.clone(), Some(fragment.clone())));
                }
                let previous = document.input_prose_previous(range.start)?;
                let (text, fragment) = fragment.transform_quotes(previous, smart_quote)?;
                Ok((text, Some(fragment)))
            } else {
                document
                    .transform_text_input_indexed(
                        range,
                        affinity,
                        &value.text,
                        |at, character, previous| {
                            let index = literal_ranges.partition_point(|range| range.end <= at);
                            if literal_ranges.get(index).is_some_and(|range| range.contains(&at)) {
                                character
                            } else {
                                smart_quote(character, previous)
                            }
                        },
                    )
                    .map(|text| (text, None))
            }
        })();
        // Assistance is optional: uncertain context must leave input literal,
        // not prevent an edit. The actual transaction still validates it.
        let Ok((text, fragment)) = transformed else {
            return Ok(value.clone());
        };
        if text == value.text {
            return Ok(value.clone());
        }
        if !document.format().is_rich_text()
            && document.encoding().encode_fragment(&text).is_err()
        {
            // RTF can escape generated Unicode; raw source and
            // plain/Markdown prose must remain in the document's encoding.
            return Ok(value.clone());
        }
        // Quote substitutions preserve scalars and line-break meaning, but
        // change UTF-8 widths. Remap semantic breaks before building a payload.
        let mut breaks = Vec::with_capacity(value.hard_break_offsets().len());
        for ((old, _), (new, _)) in value.text.char_indices().zip(text.char_indices()) {
            if value.is_hard_break(old) {
                breaks.push(new);
            }
        }
        let mut assisted = RegisterValue::try_new(text, value.kind, breaks)
            .expect("quote substitutions preserve hard-break identities");
        assisted.clipboard_fragment = fragment;
        Ok(assisted)
    }

    pub(crate) fn assist_typing_input_payload(
        &self,
        document: &Document,
        range: std::ops::Range<usize>,
        affinity: BoundaryAffinity,
        value: &RegisterValue,
    ) -> Result<RegisterValue, DocumentError> {
        let applies_typing_style = self.mode == Mode::Replace
            || !value.clipboard_fragment().is_some_and(|fragment| {
                fragment.can_insert_rich_source(document.format(), &value.text)
            });
        if applies_typing_style
            && self
                .typing_style
                .named
                .as_ref()
                .is_some_and(|style| document.character_style_is_code(style))
        {
            return Ok(value.clone());
        }
        self.assist_input_payload(document, range, affinity, value)
    }

    /// Physical-source puts use source byte ranges rather than formatted
    /// offsets, but share the same preference and quote-selection rule.
    pub(super) fn assist_source_input(
        &self,
        document: &Document,
        range: std::ops::Range<usize>,
        input: &str,
    ) -> Result<String, DocumentError> {
        if document.format().is_code() || !self.smart_quotes() || !input.contains(['\'', '"']) {
            return Ok(input.to_owned());
        }
        let transformed = document
            .transform_source_input(range, input, smart_quote)
            .unwrap_or_else(|_| input.to_owned());
        Ok(
            if document.encoding().encode_fragment(&transformed).is_ok() {
                transformed
            } else {
                input.to_owned()
            },
        )
    }

}

fn smart_quote(quote: char, previous: Option<char>) -> char {
    let opening = previous.map_or(true, |value| {
        value.is_whitespace()
            || matches!(value, '(' | '[' | '{' | '<' | '-' | '–' | '—' | '“' | '‘')
    });
    match (quote, opening) {
        ('"', true) => '“',
        ('"', false) => '”',
        ('\'', true) => '‘',
        ('\'', false) => '’',
        _ => quote,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn smart_quotes_choose_opening_and_closing_from_prose_context() {
        for (previous, quote, expected) in [
            (None, '"', '“'),
            (Some('-'), '\'', '‘'),
            (Some('d'), '\'', '’'),
            (Some('.'), '"', '”'),
            (Some('('), '"', '“'),
            (Some(' '), '\'', '‘'),
            (Some('\n'), '"', '“'),
        ] {
            assert_eq!(smart_quote(quote, previous), expected);
        }
    }

}
