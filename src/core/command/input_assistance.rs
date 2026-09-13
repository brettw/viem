//! Portable assistance for document input. Every input payload shares the same
//! quote policy; format-aware context and rich-fragment remapping belong to the
//! document layer. Generated HTML remains owned by the most recent insertion.

use super::{
    CommandInterpreter, CommandOutput, EditSessionStep, InputEvent, Key, Mode, RegisterValue,
};
use crate::document::{
    Association, BoundaryAffinity, DeletionRecovery, Document, DocumentError, Format,
    MappingOutcome, TextAnchor,
};
use unicode_segmentation::UnicodeSegmentation;

const MAX_TAG_BYTES: usize = 8192;

#[derive(Clone, Debug, Default)]
pub(crate) struct InputAssistance {
    pub(super) literal: bool,
    smart_quotes: bool,
    tag: Option<AutoTag>,
}

impl InputAssistance {
    pub(crate) fn clear_tag(&mut self) {
        self.tag = None;
    }
}

#[derive(Clone, Debug)]
struct AutoTag {
    start: TextAnchor,
    frontier: TextAnchor,
    end: TextAnchor,
    /// The opening-tag characters are authored; only `suffix` is generated.
    prefix: String,
    suffix: String,
}

impl AutoTag {
    fn capture(document: &Document, start: usize, prefix: String, suffix: String) -> Option<Self> {
        let anchor = |at| {
            document
                .text_anchor(
                    document.text_point(at).ok()?,
                    Association::BeforeInsertion,
                    BoundaryAffinity::Downstream,
                    DeletionRecovery::Unresolvable,
                )
                .ok()
        };
        Some(Self {
            start: anchor(start)?,
            frontier: anchor(start + prefix.len())?,
            end: anchor(start + prefix.len() + suffix.len())?,
            prefix,
            suffix,
        })
    }

    fn resolve(&self, document: &Document, cursor: usize) -> Option<(usize, usize)> {
        let resolve = |anchor| match document.resolve_text_anchor(anchor).ok()? {
            MappingOutcome::Exact(point) | MappingOutcome::Moved(point) => Some(point.offset()),
            _ => None,
        };
        let start = resolve(self.start)?;
        let frontier = resolve(self.frontier)?;
        let end = resolve(self.end)?;
        if frontier != cursor
            || frontier.checked_sub(start)? != self.prefix.len()
            || end.checked_sub(frontier)? != self.suffix.len()
        {
            return None;
        }
        let text = document.projection().text_tree();
        (text.slice(start..frontier).ok()? == self.prefix
            && text.slice(frontier..end).ok()? == self.suffix)
            .then_some((start, end))
    }
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

    pub(crate) fn needs_input_assistance(&self, document: &Document, event: &InputEvent) -> bool {
        if self.mode != Mode::Insert {
            return false;
        }
        let character = match event {
            InputEvent::Key(Key::Char(value)) => Some(*value),
            InputEvent::Text(value) => one_character(value),
            _ => None,
        };
        document.format() == Format::HtmlSource
            && (self.input_assistance.tag.is_some() || character == Some('<'))
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
                let previous = document.input_prose_previous(range.start, affinity)?;
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
            // Rich HTML/RTF can escape generated Unicode; raw source and
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

    pub(crate) fn try_insert_html_assistance(
        &mut self,
        document: &mut Document,
        input: &str,
    ) -> Result<Option<CommandOutput>, DocumentError> {
        if self.mode != Mode::Insert || document.format() != Format::HtmlSource {
            self.input_assistance.clear_tag();
            return Ok(None);
        }
        let Some(character) = one_character(input) else {
            // Pasted/IME batches are exact authored source, never a request to
            // manufacture or repair tags in the supplied text.
            self.input_assistance.clear_tag();
            return Ok(None);
        };
        if let Some(tag) = self.input_assistance.tag.clone() {
            if let Some((start, end)) = tag.resolve(document, self.cursor) {
                if character == '>' && !inside_attribute_quote(&tag.prefix) {
                    self.cursor += 1;
                    self.input_assistance.clear_tag();
                    self.record_assisted_input(input);
                    return Ok(Some(CommandOutput {
                        cursor_moved: true,
                        ..CommandOutput::complete()
                    }));
                }
                if tag.prefix.len() + input.len() <= MAX_TAG_BYTES
                    && !matches!(character, '<' | '\n' | '\r')
                {
                    let prefix = format!("{}{input}", tag.prefix);
                    let suffix = automatic_suffix(&prefix);
                    document.replace(self.cursor..end, &format!("{input}{suffix}"))?;
                    self.cursor += input.len();
                    self.input_assistance.tag = AutoTag::capture(document, start, prefix, suffix);
                    self.record_assisted_input(input);
                    return Ok(Some(CommandOutput {
                        document_changed: true,
                        cursor_moved: true,
                        ..CommandOutput::complete()
                    }));
                }
            }
            self.input_assistance.clear_tag();
        }
        if character == '<'
            && document.html_source_prose_at(self.cursor, BoundaryAffinity::Downstream)?
        {
            let start = self.cursor;
            document.insert(start, "<>")?;
            self.cursor += 1;
            self.input_assistance.tag =
                AutoTag::capture(document, start, "<".to_owned(), ">".to_owned());
            self.record_assisted_input(input);
            return Ok(Some(CommandOutput {
                document_changed: true,
                cursor_moved: true,
                ..CommandOutput::complete()
            }));
        }
        Ok(None)
    }

    pub(crate) fn try_html_assistance_key(
        &mut self,
        document: &mut Document,
        key: Key,
    ) -> Result<Option<CommandOutput>, DocumentError> {
        if self.mode != Mode::Insert || document.format() != Format::HtmlSource {
            self.input_assistance.clear_tag();
            return Ok(None);
        }
        if key == Key::Backspace {
            if let Some(tag) = self.input_assistance.tag.clone() {
                if let Some((start, end)) = tag.resolve(document, self.cursor) {
                    let prefix_end = tag
                        .prefix
                        .grapheme_indices(true)
                        .next_back()
                        .map_or(0, |(at, _)| at);
                    let prefix = tag.prefix[..prefix_end].to_owned();
                    let suffix = if prefix.is_empty() {
                        String::new()
                    } else {
                        automatic_suffix(&prefix)
                    };
                    document.replace(start + prefix_end..end, &suffix)?;
                    self.cursor = start + prefix_end;
                    self.input_assistance.tag = (!prefix.is_empty())
                        .then(|| AutoTag::capture(document, start, prefix, suffix))
                        .flatten();
                    if let Some(session) = self.insert_session.as_mut() {
                        session.record_edit(|program, inserted| {
                            if let Some(program) = program {
                                program.push(EditSessionStep::Backspace);
                            }
                            if let Some((at, _)) = inserted.text.grapheme_indices(true).next_back()
                            {
                                inserted.text.truncate(at);
                            }
                        });
                    }
                    return Ok(Some(CommandOutput {
                        document_changed: true,
                        cursor_moved: true,
                        ..CommandOutput::complete()
                    }));
                }
            }
            self.input_assistance.clear_tag();
        } else if !matches!(key, Key::Char(_) | Key::Tab) {
            self.input_assistance.clear_tag();
        }
        Ok(None)
    }

    fn record_assisted_input(&mut self, input: &str) {
        if let Some(session) = self.insert_session.as_mut() {
            session.record_edit(|program, inserted| {
                if let Some(program) = program {
                    program.push(EditSessionStep::AssistedText(input.to_owned()));
                }
                inserted.append_inserted_payload(&RegisterValue::characterwise(input));
            });
        }
    }
}

fn one_character(input: &str) -> Option<char> {
    let mut characters = input.chars();
    let first = characters.next()?;
    characters.next().is_none().then_some(first)
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

fn inside_attribute_quote(prefix: &str) -> bool {
    matches!(tag_prefix_state(prefix).1, TagPrefixState::Quoted(_))
}

#[derive(Clone, Copy)]
enum TagPrefixState {
    BetweenAttributes,
    AttributeName,
    AfterAttributeName,
    BeforeValue,
    Quoted(char),
    Unquoted,
    SelfClosing,
}

/// Only classify the currently owned opening-tag prefix. In particular a slash
/// inside an unquoted URL is content, whereas a slash after the name, a quoted
/// value, or separating whitespace introduces the self-closing delimiter.
fn tag_prefix_state(prefix: &str) -> (&str, TagPrefixState) {
    use TagPrefixState::*;
    let body = prefix.strip_prefix('<').unwrap_or("");
    let name_end = body
        .find(|value: char| value.is_ascii_whitespace() || matches!(value, '/' | '>'))
        .unwrap_or(body.len());
    let name = &body[..name_end];
    let mut state = BetweenAttributes;
    for character in body[name_end..].chars() {
        state = match state {
            Quoted(active) => {
                if character == active {
                    BetweenAttributes
                } else {
                    Quoted(active)
                }
            }
            Unquoted => {
                if character.is_ascii_whitespace() {
                    BetweenAttributes
                } else {
                    Unquoted
                }
            }
            BeforeValue => match character {
                value if value.is_ascii_whitespace() => BeforeValue,
                '\'' | '"' => Quoted(character),
                _ => Unquoted,
            },
            AttributeName => match character {
                value if value.is_ascii_whitespace() => AfterAttributeName,
                '=' => BeforeValue,
                '/' => SelfClosing,
                _ => AttributeName,
            },
            BetweenAttributes | AfterAttributeName | SelfClosing => match character {
                value if value.is_ascii_whitespace() => {
                    if matches!(state, AfterAttributeName) {
                        AfterAttributeName
                    } else {
                        BetweenAttributes
                    }
                }
                '=' if matches!(state, AfterAttributeName) => BeforeValue,
                '/' => SelfClosing,
                _ => AttributeName,
            },
        };
    }
    (name, state)
}

fn automatic_suffix(prefix: &str) -> String {
    let (name, state) = tag_prefix_state(prefix);
    let is_start = name
        .chars()
        .next()
        .is_some_and(|value| value.is_ascii_alphabetic());
    let is_void = matches!(
        name.to_ascii_lowercase().as_str(),
        "area"
            | "base"
            | "br"
            | "col"
            | "embed"
            | "hr"
            | "img"
            | "input"
            | "link"
            | "meta"
            | "param"
            | "source"
            | "track"
            | "wbr"
    );
    if !is_start || is_void || matches!(state, TagPrefixState::SelfClosing) {
        ">".to_owned()
    } else {
        format!("></{name}>")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Encoding;

    #[test]
    fn suffix_tracks_start_void_custom_and_self_closing_tags() {
        for (prefix, expected) in [
            ("<", ">"),
            ("<b", "></b>"),
            ("<br", ">"),
            ("<BR", ">"),
            ("<my-element", "></my-element>"),
            ("<my.widget-tag", "></my.widget-tag>"),
            ("</b", ">"),
            ("<!", ">"),
            ("<b/", ">"),
            ("<b x='/'", "></b>"),
            ("<b x='/", "></b>"),
            ("<a href=https://example.com/", "></a>"),
            ("<a href = /", "></a>"),
            ("<a href=https://example.com/ /", ">"),
            ("<a href='url/'/", ">"),
        ] {
            assert_eq!(automatic_suffix(prefix), expected, "{prefix}");
        }
    }

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

    fn source_document(text: &str) -> Document {
        Document::from_bytes(text.as_bytes().to_vec(), Encoding::Utf8, Format::HtmlSource).unwrap()
    }

    fn type_keys(commands: &mut CommandInterpreter, document: &mut Document, text: &str) {
        for character in text.chars() {
            let outcome = commands
                .handle(document, InputEvent::key(character))
                .unwrap();
            assert!(
                matches!(
                    outcome.status,
                    super::super::CommandStatus::Complete | super::super::CommandStatus::Pending
                ),
                "{character:?}: {:?}",
                outcome.status
            );
        }
    }

    #[test]
    fn generated_tags_follow_name_edits_void_conversion_and_backspace_in_one_undo_unit() {
        let mut document = source_document("");
        let mut commands = CommandInterpreter::new();
        type_keys(&mut commands, &mut document, "i<");
        assert_eq!((document.text(), commands.cursor()), ("<>", 1));
        type_keys(&mut commands, &mut document, "b");
        assert_eq!((document.text(), commands.cursor()), ("<b></b>", 2));
        type_keys(&mut commands, &mut document, "r");
        assert_eq!((document.text(), commands.cursor()), ("<br>", 3));
        commands
            .handle(&mut document, InputEvent::Key(Key::Backspace))
            .unwrap();
        assert_eq!((document.text(), commands.cursor()), ("<b></b>", 2));
        type_keys(&mut commands, &mut document, ">text");
        assert_eq!(document.text(), "<b>text</b>");
        commands
            .handle(&mut document, InputEvent::Key(Key::Escape))
            .unwrap();
        commands
            .handle(&mut document, InputEvent::key('u'))
            .unwrap();
        assert_eq!(document.source_bytes(), b"");
        commands
            .handle(&mut document, InputEvent::Key(Key::Ctrl('r')))
            .unwrap();
        assert_eq!(document.source_bytes(), b"<b>text</b>");
    }

    #[test]
    fn authored_custom_names_and_unquoted_url_slashes_retain_the_owned_end_tag() {
        for opening in ["my.widget-tag", "a href=https://example.com/", "a href = /"] {
            let mut document = source_document("");
            let mut commands = CommandInterpreter::new();
            type_keys(&mut commands, &mut document, &format!("i<{opening}"));
            let name = opening.split_ascii_whitespace().next().unwrap();
            assert_eq!(document.text(), format!("<{opening}></{name}>"));
            type_keys(&mut commands, &mut document, ">body");
            assert_eq!(document.text(), format!("<{opening}>body</{name}>"));
            commands
                .handle(&mut document, InputEvent::Key(Key::Escape))
                .unwrap();
            commands
                .handle(&mut document, InputEvent::key('u'))
                .unwrap();
            assert_eq!(document.source_bytes(), b"");
        }
    }

    #[test]
    fn empty_pair_backspace_removes_only_the_generated_pair() {
        let mut document = source_document("before after");
        let mut commands = CommandInterpreter::new();
        commands.set_cursor(&document, 7);
        type_keys(&mut commands, &mut document, "i<");
        commands
            .handle(&mut document, InputEvent::Key(Key::Backspace))
            .unwrap();
        assert_eq!(document.text(), "before after");
        assert_eq!(commands.cursor(), 7);
    }

    #[test]
    fn existing_or_manually_touched_generated_source_is_never_repaired() {
        let mut existing = source_document("<b></b>");
        let mut commands = CommandInterpreter::new();
        commands.set_cursor(&existing, 2);
        type_keys(&mut commands, &mut existing, "ir");
        assert_eq!(existing.text(), "<br></b>");

        let mut document = source_document("");
        let mut commands = CommandInterpreter::new();
        type_keys(&mut commands, &mut document, "i<b");
        // An external/source-view edit touches the generated closing name.
        document.replace(5..6, "strong").unwrap();
        type_keys(&mut commands, &mut document, "r");
        assert_eq!(document.text(), "<br></strong>");
    }

    #[test]
    fn pasted_source_and_raw_html_contexts_are_literal() {
        let mut document = source_document("");
        let mut commands = CommandInterpreter::new();
        type_keys(&mut commands, &mut document, "i");
        commands
            .handle(&mut document, InputEvent::text("<b"))
            .unwrap();
        assert_eq!(document.text(), "<b");
        for source in [
            "<p title=\"here\">text</p>",
            "<script>here</script>",
            "<style>here</style>",
            "<!--here-->",
        ] {
            let mut document = source_document(source);
            let mut commands = CommandInterpreter::new();
            let at = source.find("here").unwrap();
            commands.set_cursor(&document, at);
            type_keys(&mut commands, &mut document, "i<");
            let mut expected = source.to_owned();
            expected.insert(at, '<');
            assert_eq!(document.text(), expected, "{source}");
        }
    }

    #[test]
    fn smart_quotes_skip_html_attributes_and_raw_text_but_convert_prose() {
        for (source, expected_quote) in [
            ("<p title=\"here\">text</p>", "\""),
            ("<script>here</script>", "\""),
            ("<style>here</style>", "\""),
            ("<!--here-->", "\""),
            ("<!--\nhere-->", "\""),
            ("<p title=\nhere>text</p>", "\""),
            ("<p>here</p>", "“"),
            ("<p>&nbsp;here</p>", "“"),
            ("<p>&#32;here</p>", "“"),
            ("<p>&#x20;here</p>", "“"),
            ("<script>hidden</script><span>here</span>", "“"),
            ("<!-- > hidden --><span>here</span>", "“"),
            ("<p>word<span>here</span></p>", "”"),
        ] {
            let mut document = source_document(source);
            let mut commands = CommandInterpreter::new();
            let at = source.find("here").unwrap();
            commands.set_cursor(&document, at);
            commands.set_smart_quotes(true);
            type_keys(&mut commands, &mut document, "i\"");
            let expected = format!("{}{}{}", &source[..at], expected_quote, &source[at..]);
            assert_eq!(document.text(), expected, "{source}");
        }
    }

    #[test]
    fn wysiwyg_html_typing_escapes_syntax_without_changing_the_requested_text() {
        let mut document =
            Document::from_bytes(b"<p>text</p>".to_vec(), Encoding::Utf8, Format::Html).unwrap();
        let mut commands = CommandInterpreter::new();
        type_keys(&mut commands, &mut document, "i<&\"");
        assert_eq!(document.text(), "<&\"text");
        let reopened =
            Document::from_bytes(document.source_bytes(), Encoding::Utf8, Format::Html).unwrap();
        assert_eq!(document.text(), reopened.text());
        assert!(String::from_utf8(document.source_bytes())
            .unwrap()
            .contains("&lt;"));
    }

    #[test]
    fn auto_tags_repeat_through_macros_and_dot_without_inventing_unowned_repairs() {
        let mut document = source_document("");
        let mut commands = CommandInterpreter::new();
        type_keys(&mut commands, &mut document, "qai<b>text");
        commands
            .handle(&mut document, InputEvent::Key(Key::Escape))
            .unwrap();
        type_keys(&mut commands, &mut document, "q");
        assert_eq!(document.text(), "<b>text</b>");
        // Open a separate physical source line for the recorded insertion.
        commands.set_cursor(&document, 0);
        type_keys(&mut commands, &mut document, "@a");
        assert_eq!(document.text(), "<b>text</b><b>text</b>");
        commands.set_cursor(&document, 0);
        type_keys(&mut commands, &mut document, ".");
        assert_eq!(document.text(), "<b>text</b><b>text</b><b>text</b>");
    }

    #[test]
    fn assistance_treats_the_boundary_before_an_existing_tag_as_prose() {
        let mut document = source_document("<p>old</p>");
        let mut commands = CommandInterpreter::new();
        type_keys(&mut commands, &mut document, "i<b");
        assert_eq!(document.text(), "<b></b><p>old</p>");
        let mut document = source_document("");
        let mut commands = CommandInterpreter::new();
        type_keys(&mut commands, &mut document, "2i<b>x");
        commands
            .handle(&mut document, InputEvent::Key(Key::Escape))
            .unwrap();
        assert_eq!(document.text(), "<b>x<b>x</b></b>");
    }
}
