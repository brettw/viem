//! Portable assistance shared by committed document input paths.
use super::{CommandInterpreter, Mode, RegisterValue};
use crate::document::{BoundaryAffinity, Document, DocumentError};

#[derive(Clone, Debug)]
pub(crate) struct InputAssistance {
    pub(super) literal: bool,
    smart_quotes: bool,
    markdown_autodetect: bool,
    markdown_exit: Option<(crate::document::TextPoint, crate::document::SourcePoint)>,
}

impl Default for InputAssistance {
    fn default() -> Self {
        Self {
            literal: false,
            smart_quotes: false,
            markdown_autodetect: true,
            markdown_exit: None,
        }
    }
}

impl CommandInterpreter {
    pub(super) fn retire_typing_context(&mut self) {
        self.typing_style = Default::default();
        self.input_assistance.markdown_exit = None;
    }
    pub(crate) fn note_markdown_typing_exit(
        &mut self,
        document: &Document,
        source: Option<usize>,
    ) -> Result<(), DocumentError> {
        self.input_assistance.markdown_exit = source
            .map(|at| {
                Ok::<_, DocumentError>((
                    document.text_point(self.cursor)?,
                    document
                        .source_point(at)
                        .map_err(|_| DocumentError::AmbiguousProjection)?,
                ))
            })
            .transpose()?;
        Ok(())
    }
    pub(super) fn markdown_typing_exit(
        &self,
        document: &Document,
    ) -> Option<crate::document::SourcePoint> {
        self.input_assistance
            .markdown_exit
            .filter(|(point, _)| {
                point.document() == document.id()
                    && point.revision() == document.revision()
                    && point.offset() == self.cursor
            })
            .map(|(_, source)| source)
    }

    pub(super) fn markdown_typing_affinity(&self, document: &Document) -> BoundaryAffinity {
        if self.markdown_typing_exit(document).is_some() {
            BoundaryAffinity::Downstream
        } else {
            self.insertion_boundary_affinity()
        }
    }

    pub fn set_markdown_autodetect(&mut self, enabled: bool) {
        self.input_assistance.markdown_autodetect = enabled;
    }

    pub fn markdown_autodetect(&self) -> bool {
        self.input_assistance.markdown_autodetect
    }

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
        let mut protected = literal_ranges.to_vec();
        if self.markdown_autodetect()
            && document.format() == crate::document::Format::Markdown
            && value.clipboard_fragment().is_none()
        {
            let Ok(mut width) = document.markdown_pending_code_delimiter(range.start) else {
                return Ok(value.clone());
            };
            let mut start = width.map(|_| 0);
            let mut offset = 0;
            while let Some(relative) = value.text[offset..].find('`') {
                let at = offset + relative;
                let run = value.text[at..]
                    .bytes()
                    .take_while(|byte| *byte == b'`')
                    .count();
                offset = at + run;
                if literal_ranges
                    .iter()
                    .any(|range| range.start < offset && at < range.end)
                {
                    continue;
                }
                if width == Some(run) {
                    protected.push(start.take().unwrap_or(at)..offset);
                    width = None;
                } else if width.is_none() {
                    width = Some(run);
                    start = Some(at);
                }
            }
            if let Some(start) = start {
                protected.push(start..value.text.len());
            }
        }
        protected.sort_by_key(|range| range.start);
        let mut merged: Vec<std::ops::Range<usize>> = Vec::new();
        for range in protected {
            if let Some(last) = merged.last_mut().filter(|last| range.start <= last.end) {
                last.end = last.end.max(range.end);
            } else {
                merged.push(range);
            }
        }
        let literal_ranges = merged;
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
                            if literal_ranges
                                .get(index)
                                .is_some_and(|range| range.contains(&at))
                            {
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
        if document.encoding().encode_fragment(&text).is_err() {
            // Generated prose must remain in the document's encoding.
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
