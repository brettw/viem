//! Portable, caret-local assistance for authored text. Clipboard/register
//! payloads remain literal; generated HTML is changed only while it is owned by
//! the most recent insertion and its identity-backed boundaries still resolve.

use super::{
    CommandInterpreter, CommandOutput, EditSessionStep, InputEvent, Key, Mode, RegisterValue,
};
use crate::document::{
    Association, BoundaryAffinity, DeletionRecovery, Document, DocumentError, Format,
    MappingOutcome, TextAnchor,
};
use unicode_segmentation::UnicodeSegmentation;

const MAX_TAG_BYTES: usize = 8192;
const CONTEXT_BYTES: usize = 4096;

#[derive(Clone, Debug, Default)]
pub(crate) struct InputAssistance {
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
        (self.input_assistance.smart_quotes && matches!(character, Some('\'' | '"')))
            || (document.format() == Format::HtmlSource
                && (self.input_assistance.tag.is_some() || character == Some('<')))
    }

    pub(crate) fn smart_quotes_input(&self, document: &Document, input: &str) -> String {
        let Some(quote @ ('\'' | '"')) = one_character(input) else {
            return input.to_owned();
        };
        if !self.input_assistance.smart_quotes || self.mode != Mode::Insert {
            return input.to_owned();
        }
        if document.format() == Format::HtmlSource
            && !document
                .html_source_prose_at(self.cursor, BoundaryAffinity::Downstream)
                .unwrap_or(false)
        {
            return input.to_owned();
        }
        let prefix = if document.format() == Format::HtmlSource {
            let Ok(Some(prose)) = document.html_source_prose_prefix(self.cursor, CONTEXT_BYTES)
            else {
                return input.to_owned();
            };
            prose
        } else {
            local_prefix(document, self.cursor, CONTEXT_BYTES)
        };
        if matches!(document.format(), Format::Markdown | Format::MarkdownSource) {
            if document
                .projection()
                .markdown_replacement_begins_in_code(&(self.cursor..self.cursor))
                || (document.format() == Format::MarkdownSource
                    && markdown_syntax_requires_quote(&prefix))
            {
                return input.to_owned();
            }
        }
        let previous = prefix.chars().next_back();
        let opening = previous.map_or(true, |value| {
            value.is_whitespace()
                || matches!(value, '(' | '[' | '{' | '<' | '-' | '–' | '—' | '“' | '‘')
        });
        match (quote, opening) {
            ('"', true) => "“",
            ('"', false) => "”",
            ('\'', true) => "‘",
            _ => "’",
        }
        .to_owned()
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
                            if let Some((at, _)) = inserted.text.grapheme_indices(true).next_back() {
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

fn local_prefix(document: &Document, cursor: usize, limit: usize) -> String {
    let text = document.projection().text_tree();
    let mut start = cursor.saturating_sub(limit);
    while start < cursor && !text.is_char_boundary(start).unwrap_or(false) {
        start += 1;
    }
    text.slice(start..cursor).unwrap_or_default()
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

fn markdown_syntax_requires_quote(prefix: &str) -> bool {
    let line = prefix.rsplit('\n').next().unwrap_or(prefix);
    if line
        .chars()
        .rev()
        .take_while(|value| *value == '\\')
        .count()
        % 2
        == 1
    {
        return true;
    }
    // Inline-code and fences have semantic style coverage. Also guard partial
    // constructs while their closing delimiter has not yet been authored.
    if line.chars().filter(|value| *value == '`').count() % 2 == 1 {
        return true;
    }
    if line
        .rfind("](")
        .is_some_and(|start| line.rfind(')').map_or(true, |end| start > end))
    {
        return true;
    }
    line.rfind('<').is_some_and(|start| {
        line.rfind('>').map_or(true, |end| start > end)
            && line[start + 1..]
                .chars()
                .next()
                .is_some_and(|value| value.is_ascii_alphabetic() || matches!(value, '/' | '!'))
    })
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
    fn smart_quotes_use_prose_context_and_preserve_explicit_batches() {
        for (prefix, quote, expected) in [
            ("", "\"", "“"),
            ("-", "'", "‘"),
            ("word", "'", "’"),
            ("word.", "\"", "”"),
            ("(", "\"", "“"),
            ("a ", "'", "‘"),
            ("a\n", "\"", "“"),
        ] {
            let document = Document::from_bytes(
                prefix.as_bytes().to_vec(),
                Encoding::Utf8,
                Format::PlainText,
            )
            .unwrap();
            let mut commands = CommandInterpreter::new();
            commands.mode = Mode::Insert;
            commands.cursor = prefix.len();
            assert_eq!(commands.smart_quotes_input(&document, quote), quote);
            commands.set_smart_quotes(true);
            assert_eq!(commands.smart_quotes_input(&document, quote), expected);
            assert_eq!(
                commands.smart_quotes_input(&document, "\"pasted\""),
                "\"pasted\""
            );
        }
    }

    #[test]
    fn markdown_partial_syntax_keeps_required_quotes_literal() {
        for source in ["`code ", "[link](url ", "<span title=", "<img alt=", "\\"] {
            assert!(markdown_syntax_requires_quote(source), "{source}");
        }
        for source in ["prose ", "[link](url) ", "<b>prose ", "`code` prose "] {
            assert!(!markdown_syntax_requires_quote(source), "{source}");
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
