//! Structural exits from inline formatting retain original source tokens. A
//! caret can cross closing syntax without changing the document; a mid-run
//! insertion locally splits the enclosing scopes in one source transaction.
use super::ScriptPosition;
use super::html::{self, Token, TokenKind};
use super::{
    BoundaryAffinity, CharacterProperties, Document, DocumentError, FontSlant, Format,
    StyleProperty, StylePropertyValue,
};
use std::ops::Range;

/// The first replaced paragraph and character contexts, captured before their
/// source is deleted. These are source syntax values, not stale source offsets.
/// Paragraph separators do not supply character scopes; retaining the selected
/// text's scopes also retains semantic identity such as a link destination.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ReplacementTypingContext {
    scopes: Vec<(String, String)>,
    pub character: super::ResolvedCharacterStyle,
    pub named: Option<super::StyleId>,
    pub link: Option<String>,
    pub paragraph: ReplacementParagraphStyle,
    paragraph_scope: Option<(String, String)>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ReplacementParagraphStyle {
    pub style: super::StyleId,
    pub direct: super::BlockProperties,
    pub defaults: CharacterProperties,
}

impl ReplacementParagraphStyle {
    pub(crate) fn matches(&self, document: &Document, at: usize) -> bool {
        document.projection().blocks_for_region(&(at..at)).iter()
            .find(|block| block.range.contains(&at) || block.range.start == at)
            .is_some_and(|block| block.style == self.style && block.direct_paragraph == self.direct
                && block.direct_default_character == self.defaults)
    }
}

fn inline_scopes<'a>(tokens: &'a [Token], at: usize) -> Vec<&'a Token> {
    let stack = super::html_paragraph::stack_at(tokens, at);
    let first = stack.iter().rposition(|token| matches!(&token.kind,
        TokenKind::Tag(tag) if super::html_paragraph::structural(&tag.name)))
        .map_or(0, |index| index + 1);
    stack[first..].to_vec()
}

impl Document {
    pub(crate) fn replacement_typing_context(
        &self,
        range: Range<usize>,
    ) -> Result<Option<ReplacementTypingContext>, DocumentError> {
        if !self.format().is_wysiwyg() || range.is_empty() {
            return Ok(None);
        }
        self.validate_range(&range)?;
        let blocks = self.projection().blocks_for_region(&range);
        let Some(owner) = blocks.iter().find(|block|
            block.range.start <= range.start && range.start <= block.range.end) else { return Ok(None) };
        let paragraph = ReplacementParagraphStyle { style: owner.style.clone(), direct: owner.direct_paragraph.clone(),
            defaults: owner.direct_default_character.clone() };
        // Paragraph separators have no character style of their own. Skip
        // only structural separators, retaining authored hard breaks/spaces.
        let mut sample = range.start;
        while sample < range.end && blocks.iter().any(|block| block.range.end == sample)
            && self.projection().text_tree().slice(sample..sample + 1).as_deref() == Ok("\n")
        { sample += 1; }
        let separators_only = sample == range.end;
        let character = if separators_only {
            crate::layout::DocumentLayoutStyles::semantic_character_at(self.projection(), range.start, true)
                .map_err(|_| DocumentError::AmbiguousProjection)?
        } else {
            super::rich_text::resolved_character_at(self.projection(), sample)
                .ok_or(DocumentError::AmbiguousProjection)?
        };
        let named = self.projection().selected_named_styles(
            if separators_only { range.start..range.start } else { sample..sample },
            if separators_only { BoundaryAffinity::Upstream } else { BoundaryAffinity::Downstream },
        ).character;
        let link = if separators_only { None } else { self.link_at(self.text_point(sample)?)? };
        if self.format() != Format::Html {
            return Ok(Some(ReplacementTypingContext { scopes: Vec::new(), character, named, link, paragraph, paragraph_scope: None }));
        }
        let paragraph_source = super::rich_text::block_source_point(self.projection(), owner)?;
        let source = if separators_only { paragraph_source } else {
            let end = self.hard_line_snapshot().next_grapheme_boundary(sample)
                .ok_or(DocumentError::AmbiguousProjection)?;
            let contributor = super::source_edit::complete_contributors(
                self.projection(), &super::TextEdit::new(sample..end, ""),
            )?;
            super::rich_text::text_source_runs(self, &contributor.range)?[0].start
        };
        let decoded = self.encoding().decode(&self.source_bytes())?;
        let input = super::line_endings::normalize(&decoded, self.file_format());
        let position = input.units.get(input.units.partition_point(|unit| unit.source.end <= source))
            .map_or(input.text.len(), |unit| unit.normalized.start);
        let tokens = html::tokenize(&input.text);
        let paragraph_position = input.units.get(input.units.partition_point(|unit| unit.source.end <= paragraph_source))
            .map_or(input.text.len(), |unit| unit.normalized.start);
        // A cleared document loses enclosing paragraph context as well as its
        // immediate tag: quote/container defaults and nested list ancestry
        // belong to that first paragraph. The document envelope survives the
        // clear itself, and opaque tables are never reconstructed as text.
        let owners = super::html_paragraph::stack_at(&tokens, paragraph_position).into_iter()
            .filter(|token| matches!(&token.kind, TokenKind::Tag(tag)
                if super::html_paragraph::structural(&tag.name)
                    && !matches!(tag.name.as_str(), "html" | "body" | "head" | "table")))
            .collect::<Vec<_>>();
        let paragraph_scope = (!owners.is_empty()).then(|| (
            owners.iter().map(|token| &input.text[token.range.clone()]).collect(),
            owners.iter().rev().map(|token| closing(token)).collect(),
        ));
        let scopes = if separators_only { Vec::new() } else { inline_scopes(&tokens, position).into_iter()
            .map(|token| (input.text[token.range.clone()].to_owned(), closing(token)))
            .collect() };
        Ok(Some(ReplacementTypingContext { scopes, character, named, link, paragraph, paragraph_scope }))
    }
}

pub(super) fn replacement_insertion(
    document: &Document,
    at: usize,
    affinity: BoundaryAffinity,
    text: &str,
    protective_spaces: &[usize],
    inherited: &ReplacementTypingContext,
) -> Result<Option<Insertion>, DocumentError> {
    if document.format() != Format::Html || text.is_empty() {
        return Ok(None);
    }
    let source = if document.projection().text_tree().byte_len() == 0 {
        super::rich_text::text_source_range(document, &(at..at))?.start
    } else {
        super::source_edit::insertion_point(document.projection(), at, Some(affinity))
            .ok_or(DocumentError::AmbiguousProjection)?
    };
    let decoded = document.encoding().decode(&document.source_bytes())?;
    let input = super::line_endings::normalize(&decoded, document.file_format());
    let position = input.units.get(input.units.partition_point(|unit| unit.source.end <= source))
        .map_or(input.text.len(), |unit| unit.normalized.start);
    let tokens = html::tokenize(&input.text);
    if document.projection().text_tree().byte_len() == 0
        && !inherited.paragraph.matches(document, at)
        && !super::html_paragraph::stack_at(&tokens, position).iter().any(|token|
            matches!(&token.kind, TokenKind::Tag(tag) if super::html_paragraph::structural(&tag.name)
                && !matches!(tag.name.as_str(), "html" | "body" | "head" | "table")))
    {
        if let Some((opening, closing)) = &inherited.paragraph_scope {
            let prefix = inherited.scopes.iter().map(|(open, _)| open.as_str()).collect::<String>();
            let suffix = inherited.scopes.iter().rev().map(|(_, close)| close.as_str()).collect::<String>();
            let mut edit = super::TextEdit::new(at..at, text);
            edit.html_protective_spaces = protective_spaces.to_vec();
            let escaped = super::rich_text::escape_html_text_edit(document, source, &edit)?;
            return Ok(Some(Insertion {
                source: source..source,
                source_caret: position + opening.len() + prefix.len() + escaped.len(),
                syntax: format!("{opening}{prefix}{escaped}{suffix}{closing}"),
            }));
        }
    }
    let current = inline_scopes(&tokens, position);
    let common = current.iter().zip(&inherited.scopes)
        .take_while(|(token, (open, _))| input.text[token.range.clone()] == *open)
        .count();
    if common == current.len() && common == inherited.scopes.len() {
        return Ok(None);
    }
    // Consume no syntax when exiting a fully removed inline run. Inserting
    // after its existing closing tags avoids creating a second empty scope
    // after the new text, which would capture the next typing event again.
    let mut exit = position;
    let mut remaining = current.len();
    for token in tokens.iter().filter(|token| token.range.start >= position) {
        if remaining == common || token.range.start != exit { break; }
        let TokenKind::Tag(tag) = &token.kind else { break };
        let TokenKind::Tag(expected) = &current[remaining - 1].kind else { unreachable!() };
        if !tag.end || tag.name != expected.name { break; }
        remaining -= 1;
        exit = token.range.end;
    }
    if remaining == common && exit != position {
        let prefix = inherited.scopes[common..].iter().map(|(open, _)| open.as_str()).collect::<String>();
        let suffix = inherited.scopes[common..].iter().rev().map(|(_, close)| close.as_str()).collect::<String>();
        let mapper = super::rich_text::Builder::new(&input, document.revision());
        let source = mapper.source_range(exit..exit).start;
        let mut edit = super::TextEdit::new(at..at, text);
        edit.html_protective_spaces = protective_spaces.to_vec();
        let escaped = super::rich_text::escape_html_text_edit(document, source, &edit)?;
        return Ok(Some(Insertion {
            source: source..source,
            source_caret: exit + prefix.len() + escaped.len(),
            syntax: format!("{prefix}{escaped}{suffix}"),
        }));
    }
    let prefix = current[common..].iter().rev().map(|token| closing(token)).collect::<String>()
        + &inherited.scopes[common..].iter().map(|(open, _)| open.as_str()).collect::<String>();
    let suffix = inherited.scopes[common..].iter().rev().map(|(_, close)| close.as_str()).collect::<String>()
        + &current[common..].iter().map(|token| &input.text[token.range.clone()]).collect::<String>();
    let mut edit = super::TextEdit::new(at..at, text);
    edit.html_protective_spaces = protective_spaces.to_vec();
    let escaped = super::rich_text::escape_html_text_edit(document, source, &edit)?;
    Ok(Some(Insertion {
        source: source..source,
        source_caret: position + prefix.len() + escaped.len(),
        syntax: format!("{prefix}{escaped}{suffix}"),
    }))
}

pub(super) fn retained_inline_syntax(
    document: &Document,
    source: Range<usize>,
    retained: &[Range<usize>],
) -> Result<Vec<Range<usize>>, DocumentError> {
    let bytes = document.state().source.bytes_in(source.clone()).ok_or(DocumentError::AmbiguousProjection)?;
    let decoded = document.encoding().decode_region(&bytes, source.start)?;
    let input = super::line_endings::normalize(&decoded, document.file_format());
    let mapper = super::rich_text::Builder::new(&input, document.revision());
    let mut open = Vec::<Token>::new();
    let mut result = Vec::new();
    for token in html::tokenize(&input.text) {
        let TokenKind::Tag(tag) = &token.kind else { continue };
        if tag.end {
            if let Some(index) = open.iter().rposition(|token| matches!(&token.kind, TokenKind::Tag(other) if other.name == tag.name)) {
                let opening = &open[index];
                if !html::block(&tag.name) && !html::atomic(&tag.name) && !html::hidden(&tag.name)
                    && !matches!(tag.name.as_str(), "tr" | "td" | "th" | "thead" | "tbody" | "tfoot" | "caption" | "colgroup")
                {
                    let before = mapper.source_range(opening.range.clone());
                    let after = mapper.source_range(token.range.clone());
                    if retained.iter().any(|range| before.end <= range.start && range.end <= after.start) {
                        result.extend([before, after]);
                    }
                }
                open.truncate(index);
            }
        } else if !html::void(&tag.name) { open.push(token); }
    }
    Ok(result)
}

/// Recovered text can inherit an unclosed inline scope whose source must stay
/// inside an atomic owner. Copy that scope around the moved contributor; do
/// not remove the original, which can also style later recovered content.
pub(super) fn recovered_inline_wrapper(
    document: &Document,
    owner: Range<usize>,
    retained: Range<usize>,
) -> Result<(String, String), DocumentError> {
    let bytes = document.state().source.bytes_in(owner.clone())
        .ok_or(DocumentError::AmbiguousProjection)?;
    let decoded = document.encoding().decode_region(&bytes, owner.start)?;
    let input = super::line_endings::normalize(&decoded, document.file_format());
    let at = input.units.get(input.units.partition_point(|unit| unit.source.end <= retained.start))
        .map_or(input.text.len(), |unit| unit.normalized.start);
    let tokens = html::tokenize(&input.text);
    let stack = super::html_paragraph::stack_at(&tokens, at);
    let scopes = stack.into_iter().filter(|token| {
        matches!(&token.kind, TokenKind::Tag(tag)
            if !html::block(&tag.name) && !html::atomic(&tag.name) && !html::hidden(&tag.name)
                && !matches!(tag.name.as_str(), "tr" | "td" | "th" | "thead" | "tbody" | "tfoot" | "caption" | "colgroup"))
    }).collect::<Vec<_>>();
    let opening = scopes.iter().map(|token| &input.text[token.range.clone()]).collect();
    let closing = scopes.iter().rev().map(|token| {
        let TokenKind::Tag(tag) = &token.kind else { unreachable!() };
        format!("</{}>", tag.name)
    }).collect();
    Ok((opening, closing))
}

/// A source file may end inside an opaque object. Its trailing visible caret
/// is outside that object, so materialize only the missing closing syntax
/// before inserting text at that edge. Existing object bytes stay untouched.
pub(super) fn opaque_closing_syntax(
    document: &Document,
    source: Range<usize>,
) -> Result<String, DocumentError> {
    let bytes = document.state().source.bytes_in(source.clone())
        .ok_or(DocumentError::AmbiguousProjection)?;
    let input = document.encoding().decode_region(&bytes, source.start)?.text;
    let mut open = Vec::<String>::new();
    for token in html::tokenize(&input) {
        let TokenKind::Tag(tag) = token.kind else { continue };
        if tag.end {
            if let Some(index) = open.iter().rposition(|name| *name == tag.name) {
                open.truncate(index);
            }
        } else if !html::void(&tag.name)
            && !(input[token.range].trim_end().ends_with("/>")
                && (matches!(tag.name.as_str(), "svg" | "math")
                    || open.iter().any(|name| matches!(name.as_str(), "svg" | "math"))))
        {
            open.push(tag.name);
        }
    }
    Ok(open.into_iter().rev().map(|name| format!("</{name}>")).collect())
}

fn declarations(token: &Token) -> CharacterProperties {
    let mut result = CharacterProperties::default();
    let TokenKind::Tag(tag) = &token.kind else {
        return result;
    };
    match tag.name.as_str() {
        "b" | "strong" => result.bold = Some(true),
        "i" | "em" => result.slant = Some(FontSlant::Italic),
        "u" => result.underline = Some(true),
        "sup" => result.script_position = Some(ScriptPosition::Superscript),
        "sub" => result.script_position = Some(ScriptPosition::Subscript),
        "s" | "strike" | "del" => result.strikethrough = Some(true),
        _ => {}
    }
    if let Some(css) = tag.attribute("style") {
        html::apply_css(css, &mut result, &mut Default::default());
    }
    if let Some(language) = tag.attribute("lang") {
        result.language = Some(language.to_owned());
    }
    result.direction = match tag.attribute("dir") {
        Some("ltr") => Some(super::WritingDirection::LeftToRight),
        Some("rtl") => Some(super::WritingDirection::RightToLeft),
        Some("auto") => Some(super::WritingDirection::Natural),
        _ => result.direction,
    };
    result
}
fn disabled(token: &Token, desired: &CharacterProperties) -> bool {
    let own = declarations(token);
    desired.bold == Some(false) && own.bold == Some(true)
        || desired.slant == Some(FontSlant::Upright)
            && own.slant.is_some_and(|value| value != FontSlant::Upright)
        || desired.underline == Some(false) && own.underline == Some(true)
        || desired.strikethrough == Some(false) && own.strikethrough == Some(true)
        || desired.script_position.zip(own.script_position).is_some_and(|(desired, own)| desired != own)
}
fn closing(token: &Token) -> String {
    let TokenKind::Tag(tag) = &token.kind else {
        unreachable!()
    };
    format!("</{}>", tag.name)
}
pub(super) struct Insertion {
    pub source: Range<usize>,
    pub syntax: String,
    pub source_caret: usize,
}
struct Context<'a> {
    open: Vec<&'a Token>,
    first: usize,
    exit: Option<usize>,
}
fn context<'a>(
    tokens: &'a [Token],
    at: usize,
    desired: &CharacterProperties,
) -> Option<Context<'a>> {
    let open = super::html_paragraph::stack_at(tokens, at);
    let structural = open.iter().rposition(|token| matches!(&token.kind, TokenKind::Tag(tag) if super::html_paragraph::structural(&tag.name))).map_or(0, |index| index + 1);
    let first = (structural..open.len()).find(|&index| disabled(open[index], desired))?;
    // Only cross exact immediately adjacent closing syntax. Visible prose,
    // comments, unknown nodes, and malformed nesting never get skipped.
    let mut remaining = open.len();
    let mut position = at;
    let mut exit = None;
    for token in tokens.iter().filter(|token| token.range.start >= at) {
        if token.range.start != position {
            break;
        }
        let TokenKind::Tag(tag) = &token.kind else {
            break;
        };
        let TokenKind::Tag(expected) = &open[remaining - 1].kind else {
            unreachable!()
        };
        if !tag.end || tag.name != expected.name {
            break;
        }
        remaining -= 1;
        position = token.range.end;
        if remaining == first {
            exit = Some(position);
            break;
        }
    }
    Some(Context { open, first, exit })
}
fn preserve(context: &Context<'_>, desired: &CharacterProperties) -> CharacterProperties {
    let mut result = CharacterProperties::default();
    for token in &context.open[context.first..] {
        let mut own = declarations(token);
        if desired.bold == Some(false) {
            own.bold = None;
        }
        if desired.slant == Some(FontSlant::Upright) {
            own.slant = None;
        }
        if desired.underline == Some(false) {
            own.underline = None;
        }
        if desired.strikethrough == Some(false) {
            own.strikethrough = None;
        }
        super::rich_text::overlay(&mut result, &own);
    }
    result
}
fn values(properties: CharacterProperties) -> Vec<(StyleProperty, StylePropertyValue)> {
    use StyleProperty as P;
    use StylePropertyValue as V;
    let mut result = Vec::new();
    macro_rules! add {
        ($field:ident, $property:ident, $variant:ident) => {
            if let Some(value) = properties.$field {
                result.push((P::$property, V::$variant(value)));
            }
        };
    }
    add!(font_families, CharacterFontFamilies, FontFamilies);
    add!(size, CharacterSize, Float);
    add!(weight, CharacterWeight, FontWeight);
    add!(bold, CharacterBold, Boolean);
    add!(slant, CharacterSlant, FontSlant);
    add!(foreground, CharacterForeground, Color);
    add!(background, CharacterBackground, Color);
    add!(underline, CharacterUnderline, Boolean);
    add!(strikethrough, CharacterStrikethrough, Boolean);
    add!(language, CharacterLanguage, Text);
    add!(direction, CharacterDirection, WritingDirection);
    add!(
        open_type_features,
        CharacterOpenTypeFeatures,
        OpenTypeFeatures
    );
    add!(letter_spacing, CharacterLetterSpacing, Float);
    add!(script_position, CharacterScriptPosition, ScriptPosition);
    result
}
impl Document {
    /// Return an exact source-visible caret exit, together with unrelated
    /// declarations whose inline scopes must remain active for later typing.
    pub(crate) fn html_typing_exit(
        &self,
        at: usize,
        requested: &[(StyleProperty, StylePropertyValue)],
    ) -> Result<Option<(usize, Vec<(StyleProperty, StylePropertyValue)>)>, DocumentError> {
        if self.format() != Format::HtmlSource {
            return Ok(None);
        }
        self.text_point(at)?;
        let desired = self.validate_typing_properties(requested)?;
        let decoded = self.encoding().decode(&self.source_bytes())?;
        let input = super::line_endings::normalize(&decoded, self.file_format());
        let tokens = html::tokenize(&input.text);
        let Some(context) = context(&tokens, at, &desired) else {
            return Ok(None);
        };
        Ok(context
            .exit
            .map(|exit| (exit, values(preserve(&context, &desired)))))
    }
}

pub(super) fn insertion(
    document: &Document,
    at: usize,
    affinity: BoundaryAffinity,
    text: &str,
    protective_spaces: &[usize],
    desired: &CharacterProperties,
) -> Result<Option<Insertion>, DocumentError> {
    if !document.format().is_html() || text.is_empty() {
        return Ok(None);
    }
    if ![desired.bold, desired.underline, desired.strikethrough].contains(&Some(false))
        && desired.slant != Some(FontSlant::Upright)
    {
        return Ok(None);
    }
    let source = if document.format() == Format::HtmlSource {
        document.projection().source_insertion_point(at, true)
    } else if document.projection().text_tree().byte_len() == 0 {
        Some(super::rich_text::text_source_range(document, &(at..at))?.start)
    } else {
        super::source_edit::insertion_point(document.projection(), at, Some(affinity))
    };
    let Some(source) = source else {
        if super::source_edit::complete_contributors(document.projection(), &super::TextEdit::new(at..at, text))?.range != (at..at) {
            // Materialize the indivisible source contributor first, then apply
            // the requested style to only the newly inserted logical text.
            return Ok(None);
        }
        return Err(DocumentError::AmbiguousProjection);
    };
    let decoded = document.encoding().decode(&document.source_bytes())?;
    let input = super::line_endings::normalize(&decoded, document.file_format());
    let position = input
        .units
        .iter()
        .find(|unit| unit.source.start == source)
        .map(|unit| unit.normalized.start)
        .or_else(|| {
            input
                .units
                .last()
                .filter(|unit| unit.source.end == source)
                .map(|unit| unit.normalized.end)
        })
        .unwrap_or(0);
    let tokens = html::tokenize(&input.text);
    let Some(context) = context(&tokens, position, desired) else {
        return Ok(None);
    };
    let converter = super::rich_text::Builder::new(&input, document.revision());
    let preserved = preserve(&context, desired);
    let (opening, closing_preserved) = if preserved == CharacterProperties::default() {
        (String::new(), String::new())
    } else {
        html::character_wrapper(&preserved)
    };
    let escaped = if document.format() == Format::HtmlSource {
        text.to_owned()
    } else {
        let mut edit = super::TextEdit::new(at..at, text);
        edit.html_protective_spaces = protective_spaces.to_vec();
        super::rich_text::escape_html_text_edit(document, source, &edit)?
    };
    let (position, prefix, suffix) = if let Some(exit) = context.exit {
        (exit, opening, closing_preserved)
    } else {
        let close = context.open[context.first..]
            .iter()
            .rev()
            .map(|token| closing(token))
            .collect::<String>();
        let reopen = context.open[context.first..]
            .iter()
            .map(|token| &input.text[token.range.clone()])
            .collect::<String>();
        let reopen = if position == input.text.len() {
            String::new()
        } else {
            reopen
        };
        (
            position,
            format!("{close}{opening}"),
            format!("{closing_preserved}{reopen}"),
        )
    };
    let source_caret = position + prefix.len() + escaped.len();
    Ok(Some(Insertion {
        source: converter.source_range(position..position),
        syntax: format!("{prefix}{escaped}{suffix}"),
        source_caret,
    }))
}
