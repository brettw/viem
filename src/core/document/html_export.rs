//! Standalone presentation export. The source adapters remain the persistence
//! authority; exporting resolves the same cascade as layout without editing it.
use super::{
    syntax::{service::SyntaxService, SyntaxInputSnapshot},
    BlockKind, BlockProperties, CharacterProperties, ContainerKind, Document, DocumentError,
    Encoding, FileFormat, FontSize, Format, FormattedDocument, Revision, WritingDirection,
};
use crate::layout::{
    BlockBoxStyle, ContainerLayoutStyle, DocumentLayoutStyles, DocumentStyleError,
    ParagraphLayoutStyle, ResolvedTextPaint, ResolvedTextStyle, TextDirection,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    ops::Range,
    sync::Arc,
};

#[derive(Debug)]
pub enum HtmlExportError {
    Document(DocumentError),
    Style(DocumentStyleError),
}
impl From<DocumentError> for HtmlExportError {
    fn from(value: DocumentError) -> Self {
        Self::Document(value)
    }
}
impl From<DocumentStyleError> for HtmlExportError {
    fn from(value: DocumentStyleError) -> Self {
        Self::Style(value)
    }
}

/// Immutable presentation capture. Rendering may wait for syntax workers and
/// therefore belongs outside the coordinator lease. No layout provider is used.
pub struct HtmlExport {
    projection: FormattedDocument,
    format: Format,
    source: Option<(
        super::source::SourceSnapshot,
        Encoding,
        FileFormat,
        Revision,
    )>,
    syntax: Option<(SyntaxService, SyntaxInputSnapshot)>,
    tabstop: u32,
}
impl Document {
    pub fn prepare_html_export(&self) -> HtmlExport {
        HtmlExport {
            projection: self.projection().clone(),
            format: self.format(),
            source: self.format().is_markdown().then(|| {
                (
                    self.state().source.clone(),
                    self.encoding(),
                    self.file_format(),
                    self.revision(),
                )
            }),
            syntax: None,
            tabstop: super::IndentationOptions::default().tabstop,
        }
    }
}
impl HtmlExport {
    pub(crate) fn with_tabstop(mut self, tabstop: u32) -> Self {
        self.tabstop = tabstop.max(1);
        self
    }
    pub(crate) fn with_syntax(
        mut self,
        service: SyntaxService,
        input: SyntaxInputSnapshot,
    ) -> Self {
        self.syntax = Some((service, input));
        self
    }

    pub fn render(mut self) -> Result<Vec<u8>, HtmlExportError> {
        let mut links = Vec::new();
        if let Some((source, encoding, endings, revision)) = &self.source {
            let bytes = source.bytes();
            let decoded = encoding.decode(&bytes).map_err(DocumentError::from)?;
            let input = super::line_endings::normalize(&decoded, *endings);
            if self.format.is_source_view() {
                let mut semantic = super::projection::project(
                    &input,
                    self.format.wysiwyg(),
                    *revision,
                    decoded.bom_len,
                    bytes.len(),
                );
                semantic.install_configuration_styles(
                    *revision,
                    self.projection.style_sheet().clone(),
                    self.projection.document_style().clone(),
                );
                self.projection = semantic;
            }
            links = export_links(&self.projection, &input);
        }
        if let Some((service, input)) = self.syntax.take() {
            let sheet = Arc::new(self.projection.style_sheet().clone());
            let runs = service.export_runs(input);
            self.projection.install_code_styles(sheet, &runs);
        }
        render_projection(
            &self.projection,
            self.format.is_literal(),
            &links,
            self.tabstop,
        )
        .map(String::into_bytes)
        .map_err(Into::into)
    }
}

fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
fn attribute(css: &str) -> String {
    format!(" style=\"{}\"", escape(css))
}

#[derive(Default)]
struct CssClasses {
    by_css: BTreeMap<String, usize>,
}
impl CssClasses {
    fn intern(&mut self, css: String) -> usize {
        let next = self.by_css.len();
        *self.by_css.entry(css).or_insert(next)
    }
    fn stylesheet(&self) -> String {
        let mut output = String::new();
        for (css, index) in &self.by_css {
            // Style elements are HTML raw text: CSS string quoting alone cannot
            // protect a family name containing </style>. CSS escapes preserve
            // the exact name without introducing an HTML tag boundary.
            let css = css.replace('<', "\\3c ").replace('>', "\\3e ");
            output.push_str(&format!(".c{index}{{{css}}}\n"));
        }
        output
    }
}

struct ExportLink {
    range: Range<usize>,
    destination: String,
}

fn safe_destination(destination: &str) -> bool {
    if destination.chars().any(char::is_control) {
        return false;
    }
    let prefix = destination.split(['/', '?', '#']).next().unwrap_or("");
    match prefix.split_once(':') {
        None => true, // Relative files and document fragments remain usable.
        Some((scheme, _)) => matches!(
            scheme.to_ascii_lowercase().as_str(),
            "http" | "https" | "file" | "mailto"
        ),
    }
}

/// Use the adapters' link recognition and authoritative Link spans. Source
/// provenance provides the label pieces; reference/image exceptions never
/// receive a Link span, and Code never calls this path.
fn export_links(
    document: &FormattedDocument,
    input: &super::line_endings::NormalizedText,
) -> Vec<ExportLink> {
    use super::links::InlineLink;
    use pulldown_cmark::{Event, LinkType, Parser, Tag};
    let spans = document
        .style_spans()
        .iter()
        .filter(|span| span.application == super::StyleApplication::Automatic("Link".into()))
        .collect::<Vec<_>>();
    if spans.is_empty() {
        return Vec::new();
    }
    let mut candidates = super::links::markdown_links_in(&input.text, 0..input.text.len());
    candidates.extend(super::links::html_links(&input.text));
    for (event, range) in Parser::new(&input.text).into_offset_iter() {
        if let Event::Start(Tag::Link {
            link_type: link_type @ (LinkType::Autolink | LinkType::Email),
            dest_url,
            ..
        }) = event
        {
            let email = link_type == LinkType::Email;
            candidates.push(InlineLink {
                label: range.clone(),
                range,
                destination: if email {
                    format!("mailto:{dest_url}")
                } else {
                    dest_url.into_string()
                },
            });
        }
    }
    let normalized_at = |source: usize| {
        input
            .units
            .get(
                input
                    .units
                    .partition_point(|unit| unit.source.end <= source),
            )
            .map(|unit| unit.normalized.start)
    };
    for span in &spans {
        if let Some(at) = document
            .provenance_for_region(&span.range)
            .iter()
            .find(|p| !p.source.is_empty() && !p.formatted.is_empty())
            .and_then(|p| normalized_at(p.source.start))
        {
            if let Some((end, destination)) =
                super::markdown_syntax::autolink(&input.text, at, input.text.len())
            {
                candidates.push(InlineLink {
                    range: at..end,
                    label: at..end,
                    destination,
                });
            }
        }
    }
    candidates.sort_by_key(|link| (link.label.start, std::cmp::Reverse(link.label.end)));
    let mut output: Vec<ExportLink> = Vec::new();
    for span in spans {
        for provenance in document.provenance_for_region(&span.range) {
            if provenance.source.is_empty() || provenance.formatted.is_empty() {
                continue;
            }
            let Some(at) = normalized_at(provenance.source.start) else {
                continue;
            };
            let end = candidates.partition_point(|link| link.label.start <= at);
            let Some(link) = candidates[..end]
                .last()
                .filter(|link| link.label.contains(&at))
            else {
                continue;
            };
            if !safe_destination(&link.destination) {
                continue;
            }
            let range = provenance.formatted.start.max(span.range.start)
                ..provenance.formatted.end.min(span.range.end);
            if range.is_empty() {
                continue;
            }
            if let Some(last) = output.last_mut().filter(|last| {
                last.range.end == range.start && last.destination == link.destination
            }) {
                last.range.end = range.end;
            } else {
                output.push(ExportLink {
                    range,
                    destination: link.destination.clone(),
                });
            }
        }
    }
    output.sort_by_key(|link| link.range.start);
    output
}

/// Adapt already-resolved layout values to the existing CSS writer. No cascade
/// or format-specific style interpretation is duplicated here.
fn character(shape: &ResolvedTextStyle, paint: &ResolvedTextPaint) -> CharacterProperties {
    CharacterProperties {
        font_families: Some(shape.font_families.clone()),
        font_axes: Some(shape.font_axes.clone()),
        size: Some(FontSize::Points(shape.size)),
        weight: Some(shape.weight.round() as u16),
        slant: Some(shape.slant),
        foreground: Some(paint.foreground),
        background: paint.background,
        underline: Some(paint.underline),
        strikethrough: Some(paint.strikethrough),
        direction: Some(match shape.direction {
            TextDirection::LeftToRight => WritingDirection::LeftToRight,
            TextDirection::RightToLeft => WritingDirection::RightToLeft,
            TextDirection::Auto => WritingDirection::Natural,
        }),
        open_type_features: Some(
            shape
                .features
                .iter()
                .map(|feature| {
                    (
                        String::from_utf8_lossy(&feature.tag).into_owned(),
                        feature.value,
                    )
                })
                .collect(),
        ),
        letter_spacing: Some(shape.letter_spacing),
        language: shape.language.clone(),
        ..Default::default()
    }
}
fn box_properties(value: &BlockBoxStyle) -> BlockProperties {
    BlockProperties {
        margin_top: Some(value.margin.top),
        margin_right: Some(value.margin.right),
        margin_bottom: Some(value.margin.bottom),
        margin_left: Some(value.margin.left),
        padding_top: Some(value.padding.top),
        padding_right: Some(value.padding.right),
        padding_bottom: Some(value.padding.bottom),
        padding_left: Some(value.padding.left),
        border_top_width: Some(value.border.top),
        border_right_width: Some(value.border.right),
        border_bottom_width: Some(value.border.bottom),
        border_left_width: Some(value.border.left),
        border_top_color: value.border_colors[0],
        border_right_color: value.border_colors[1],
        border_bottom_color: value.border_colors[2],
        border_left_color: value.border_colors[3],
        background: value.background,
        leading_indent: (value.inline_start != 0.).then_some(value.inline_start),
        trailing_indent: (value.inline_end != 0.).then_some(value.inline_end),
        ..Default::default()
    }
}
fn paragraph_css(paragraph: &ParagraphLayoutStyle) -> String {
    let mut block = box_properties(&paragraph.block_box);
    block.margin_top = Some(paragraph.margin_top);
    block.margin_bottom = Some(paragraph.margin_bottom);
    block.leading_indent = (paragraph.leading_indent != 0.).then_some(paragraph.leading_indent);
    block.trailing_indent = (paragraph.trailing_indent != 0.).then_some(paragraph.trailing_indent);
    block.first_line_indent = Some(paragraph.first_line_indent);
    block.line_spacing = Some(paragraph.line_spacing);
    block.alignment = Some(paragraph.alignment);
    block.base_direction = Some(paragraph.base_direction);
    let mut text = character(&paragraph.default_shaping_style, &paragraph.marker_paint);
    text.underline = None;
    text.strikethrough = None;
    format!(
        "{}; {}",
        super::html::character_css(&text),
        super::html_styles::block_css(&block)
    )
}
fn container_tag(kind: ContainerKind) -> &'static str {
    match kind {
        ContainerKind::Quote => "blockquote",
        ContainerKind::CodeBlock => "div",
        ContainerKind::List { ordered: true } => "ol",
        ContainerKind::List { ordered: false } => "ul",
        ContainerKind::ListItem => "li",
    }
}
fn close_containers(output: &mut String, containers: &[ContainerLayoutStyle]) {
    for container in containers.iter().rev() {
        output.push_str(&format!("</{}>\n", container_tag(container.kind)));
    }
}

fn render_projection(
    document: &FormattedDocument,
    literal: bool,
    links: &[ExportLink],
    tabstop: u32,
) -> Result<String, DocumentStyleError> {
    let styles = DocumentLayoutStyles::resolve(document)?;
    let mut classes = CssClasses::default();
    let mut root = character(&styles.default_shaping_style, &styles.default_paint);
    root.background = Some(styles.canvas_background);
    // Decorations propagate through descendants in CSS and cannot be reset on
    // a child. Apply them only to the fully resolved text spans.
    root.underline = None;
    root.strikethrough = None;
    let root_box = BlockProperties {
        padding_top: Some(styles.document_insets.top),
        padding_right: Some(styles.document_insets.right),
        padding_bottom: Some(styles.document_insets.bottom),
        padding_left: Some(styles.document_insets.left),
        ..Default::default()
    };
    let mut output = String::from("<!DOCTYPE html>\n<html><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"><title>Exported document</title>\n<style>\nhtml{color-scheme:light}body{margin:2rem auto;max-width:72rem;padding:0 1.5rem}main{box-sizing:border-box}p,h1,h2,h3,h4,h5,h6,pre{white-space:pre-wrap;overflow-wrap:anywhere}li::marker{color:currentColor}hr{border:0;border-top:1px solid currentColor}\n</style></head>\n");
    output = output.replace("</style>", &format!("pre{{tab-size:{tabstop}}}\n</style>"));
    output.push_str(&format!(
        "<body dir=\"auto\"{}><main{}>\n",
        attribute(&super::html::character_css(&root)),
        attribute(&super::html_styles::block_css(&root_box))
    ));
    if literal {
        let css = styles
            .paragraphs
            .first()
            .map(paragraph_css)
            .unwrap_or_default();
        output.push_str(&format!("<pre dir=\"auto\"{}><code>", attribute(&css)));
        write_runs(
            &mut output,
            document,
            &styles,
            0..document.text().len(),
            links,
            &mut classes,
        );
        output.push_str("</code></pre>\n");
    } else {
        let numbers = super::markdown_serialization::containers::numbering(document.blocks());
        let mut previous: &[ContainerLayoutStyle] = &[];
        for (block, paragraph) in document.blocks().iter().zip(&styles.paragraphs) {
            let path = paragraph.containers.as_ref();
            let common = previous
                .iter()
                .zip(path)
                .take_while(|(a, b)| a.id == b.id)
                .count();
            close_containers(&mut output, &previous[common..]);
            for (depth, container) in path.iter().enumerate().skip(common) {
                let mut extra = String::new();
                if let ContainerKind::List { ordered: true } = container.kind {
                    let ordinal = path[depth + 1..]
                        .iter()
                        .find_map(|child| numbers.get(&child.id).map(|n| n.1))
                        .unwrap_or(1);
                    extra.push_str(&format!(" start=\"{ordinal}\""));
                }
                if container.kind == ContainerKind::ListItem {
                    if let Some((true, ordinal)) = numbers.get(&container.id) {
                        extra.push_str(&format!(" value=\"{ordinal}\""));
                    }
                }
                let mut css = super::html_styles::block_css(&box_properties(&container.style));
                if let ContainerKind::List { ordered } = container.kind {
                    let level = path[..depth]
                        .iter()
                        .filter(|ancestor| matches!(ancestor.kind, ContainerKind::List { .. }))
                        .count()
                        .min(3) as u8;
                    css.push_str(&format!(
                        "list-style-type:{}; ",
                        ParagraphLayoutStyle::list_marker_type(ordered, level)
                    ));
                }
                css.push_str(&super::html::character_css(&CharacterProperties {
                    foreground: Some(container.style.foreground),
                    ..Default::default()
                }));
                output.push_str(&format!(
                    "<{}{}{}>\n",
                    container_tag(container.kind),
                    extra,
                    attribute(&css)
                ));
            }
            let heading = match block.kind {
                BlockKind::Heading(level) => Some(level),
                _ => block
                    .style
                    .0
                    .strip_prefix("Heading")
                    .and_then(|n| n.parse::<u8>().ok())
                    .filter(|n| (1..=6).contains(n)),
            };
            let code = block.style.0 == "Code Block"
                || path.iter().any(|c| c.kind == ContainerKind::CodeBlock);
            let tag = if paragraph.thematic_break {
                "hr".to_owned()
            } else if code {
                "pre".to_owned()
            } else if let Some(level) = heading {
                format!("h{level}")
            } else {
                "p".to_owned()
            };
            let direction = if paragraph.base_direction == WritingDirection::Natural {
                " dir=\"auto\""
            } else {
                ""
            };
            let mut css = paragraph_css(paragraph);
            if paragraph.thematic_break && paragraph.block_box.border.top == 0. {
                css.push_str("; border-top-width:1px; border-top-style:solid");
            }
            output.push_str(&format!("<{tag}{direction}{}>", attribute(&css)));
            if !paragraph.thematic_break {
                if code {
                    output.push_str("<code>");
                }
                if paragraph.text_range.is_empty() {
                    output.push_str("<br>");
                } else {
                    write_runs(
                        &mut output,
                        document,
                        &styles,
                        paragraph.text_range.clone(),
                        links,
                        &mut classes,
                    );
                }
                if code {
                    output.push_str("</code>");
                }
                output.push_str(&format!("</{tag}>\n"));
            }
            previous = path;
        }
        close_containers(&mut output, previous);
    }
    output.push_str("</main></body></html>\n");
    Ok(output.replacen("</style>", &(classes.stylesheet() + "</style>"), 1))
}

fn write_runs(
    output: &mut String,
    document: &FormattedDocument,
    styles: &DocumentLayoutStyles,
    range: Range<usize>,
    links: &[ExportLink],
    classes: &mut CssClasses,
) {
    let mut boundaries = BTreeSet::from([range.start, range.end]);
    let shape_start = styles
        .shaping_runs
        .partition_point(|run| run.text_range.end <= range.start);
    for run in styles.shaping_runs[shape_start..]
        .iter()
        .take_while(|run| run.text_range.start < range.end)
    {
        boundaries.insert(run.text_range.start.max(range.start));
        boundaries.insert(run.text_range.end.min(range.end));
    }
    let paint_start = styles
        .paint_runs
        .partition_point(|run| run.text_range.end <= range.start);
    for run in styles.paint_runs[paint_start..]
        .iter()
        .take_while(|run| run.text_range.start < range.end)
    {
        boundaries.insert(run.text_range.start.max(range.start));
        boundaries.insert(run.text_range.end.min(range.end));
    }
    let link_start = links.partition_point(|link| link.range.end <= range.start);
    for link in links[link_start..]
        .iter()
        .take_while(|link| link.range.start < range.end)
    {
        boundaries.insert(link.range.start.max(range.start));
        boundaries.insert(link.range.end.min(range.end));
    }
    let boundaries = boundaries.into_iter().collect::<Vec<_>>();
    for pair in boundaries.windows(2) {
        if pair[0] == pair[1] {
            continue;
        }
        let shape = styles
            .shaping_runs
            .get(
                styles
                    .shaping_runs
                    .partition_point(|run| run.text_range.end <= pair[0]),
            )
            .filter(|run| run.text_range.contains(&pair[0]))
            .map_or(&styles.default_shaping_style, |run| &run.style);
        let paint = styles
            .paint_runs
            .get(
                styles
                    .paint_runs
                    .partition_point(|run| run.text_range.end <= pair[0]),
            )
            .filter(|run| run.text_range.contains(&pair[0]))
            .map_or(&styles.default_paint, |run| &run.paint);
        let properties = character(shape, paint);
        let language = properties
            .language
            .as_ref()
            .map_or(String::new(), |language| {
                format!(" lang=\"{}\"", escape(language))
            });
        let link = links
            .get(links.partition_point(|link| link.range.end <= pair[0]))
            .filter(|link| link.range.contains(&pair[0]));
        if let Some(link) = link {
            output.push_str(&format!(
                "<a href=\"{}\" style=\"color:inherit;text-decoration:none\">",
                escape(&link.destination)
            ));
        }
        output.push_str(&format!(
            "<span class=\"c{}\"{}>{}</span>",
            classes.intern(super::html::character_css(&properties)),
            language,
            escape(&document.text()[pair[0]..pair[1]])
        ));
        if link.is_some() {
            output.push_str("</a>");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::{
        code_style,
        syntax::{
            service::{SyntaxProvider, SyntaxRequest, SyntaxResult, MAX_REGION_BYTES},
            Coverage, SyntaxInputIdentity, SyntaxRun,
        },
        Color, StyleId,
    };
    use super::*;
    use crate::{coordinator::Core, layout::MockTextMeasurementProvider};
    use std::sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Mutex,
    };

    fn open(source: &str, format: Format) -> Document {
        Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap()
    }
    fn html(document: &Document) -> String {
        String::from_utf8(document.prepare_html_export().render().unwrap()).unwrap()
    }

    #[test]
    fn markdown_source_exports_the_same_semantic_styles_and_structure() {
        let source = "# A styled heading\n\nA **bold** and *italic* paragraph with ~~old~~ words & <literal>.\n\n> A quote\n>\n> 3. third\n> 4. fourth\n\n---\n\n```rust\nfn main() {\n    println!(\"hello\");\n}\n```\n\n![image](never-fetch.png)\n";
        let visual = open(source, Format::Markdown);
        let source_view = open(source, Format::MarkdownSource);
        let output = html(&visual);
        assert_eq!(output, html(&source_view));
        assert!(output.starts_with("<!DOCTYPE html>"));
        assert!(output.contains("<h1 dir=\"auto\" style="));
        assert!(output.contains("<blockquote"));
        assert!(output.contains("<ol start=\"3\""));
        assert!(output.contains("<li value=\"4\""));
        assert!(output.contains("<hr dir=\"auto\" style="));
        assert!(output.contains("font-weight: 700"));
        assert!(output.contains("font-style: italic"));
        assert!(output.contains("line-through"));
        assert!(output.contains("![image](never-fetch.png)"));
        assert!(!output.contains("<img"));
        assert!(!output.contains("**bold**"));
        assert_eq!(visual.source_bytes(), source.as_bytes());
        assert!(!visual.is_dirty());
    }

    #[test]
    fn export_uses_the_layout_cascade_and_safe_css_strings() {
        let document = open("# Heading\n\nA **bold** paragraph", Format::Markdown);
        let mut sheet = document.projection().style_sheet().clone();
        let mut heading = sheet
            .block_style(&StyleId::from("Heading1"))
            .unwrap()
            .clone();
        heading.character.size = Some(FontSize::Points(31.));
        heading.character.open_type_features = Some(BTreeMap::from([("a'b\\".into(), 1)]));
        heading.character.foreground = Some(Color {
            red: 0.25,
            green: 0.5,
            blue: 0.75,
            alpha: 1.,
        });
        heading.character.font_families = Some(vec![
            "Evil'</style><script>bad()</script>\"\r\u{c}".into(),
            "serif".into(),
        ]);
        let metadata = sheet.block_style_metadata(&heading.id).unwrap().clone();
        sheet.insert_block_style(heading, metadata).unwrap();
        let mut export = document.prepare_html_export();
        export.projection.install_configuration_styles(
            document.revision(),
            sheet,
            document.projection().document_style().clone(),
        );
        let styles = DocumentLayoutStyles::resolve(&export.projection).unwrap();
        let output = String::from_utf8(export.render().unwrap()).unwrap();
        assert!(output.contains(&format!(
            "font-size: {}pt",
            styles.paragraphs[0].default_shaping_style.size
        )));
        assert!(output.contains("font-size: 31pt"));
        assert!(output.contains("color(srgb 0.25 0.5 0.75 / 1)"));
        assert!(!output.contains("<script>"));
        assert_eq!(output.matches("</style>").count(), 1);
        assert!(output.contains("\\d ") && output.contains("\\c "));
        assert!(output.contains(", serif"));
    }

    struct WholeDocumentProvider {
        regions: Arc<Mutex<Vec<Range<usize>>>>,
        calls: Arc<AtomicUsize>,
        mode: u8,
    }
    impl SyntaxProvider for WholeDocumentProvider {
        fn analyze(&mut self, request: &SyntaxRequest, _: &AtomicBool) -> SyntaxResult {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.regions.lock().unwrap().push(request.range.clone());
            assert!(std::thread::current()
                .name()
                .unwrap_or("")
                .contains("syntax"));
            if self.mode == 1 {
                return SyntaxResult::missing(request, "unknown language");
            }
            if self.mode == 2 {
                panic!("fixture provider failure");
            }
            let mut range = request.range.clone();
            if self.mode == 3 && range.len() > 2 {
                range.end = range.start + 2;
            }
            SyntaxResult {
                input: request.input.identity(),
                configuration: request.configuration.clone(),
                range: range.clone(),
                coverage: Coverage::Exact,
                diagnostics: vec![],
                continuation: false,
                runs: vec![SyntaxRun {
                    range,
                    name: super::super::syntax::SyntaxStyleName("Comment".into()),
                    origin: "test".into(),
                    priority: 0,
                }],
            }
        }
    }

    #[test]
    fn code_export_analyzes_every_region_uses_custom_styles_and_never_mutates_session() {
        let source = format!(
            "<!-- <script>alert(1)</script> & -->\n{}TAIL",
            "éabc ".repeat(MAX_REGION_BYTES / 3)
        );
        let mut document = open(&source, Format::Code);
        let mut sheet = code_style::default_sheet();
        let mut base = sheet.block_style(&sheet.base_paragraph).unwrap().clone();
        base.block.line_spacing = Some(super::super::LineSpacing::Multiplier(1.7));
        sheet
            .apply_configuration_edit(
                &super::super::StyleDefinitionEdit::UpdateBlock(base),
                super::super::StyleSheetRevision(sheet.revision.0 + 1),
                true,
            )
            .unwrap();
        let comment_id = code_style::resolve_name(&sheet, "Comment").unwrap().clone();
        let mut comment = sheet.character_style(&comment_id).unwrap().clone();
        comment.properties.foreground = Some(Color {
            red: 1.,
            green: 0.,
            blue: 0.,
            alpha: 1.,
        });
        comment.properties.slant = Some(super::super::FontSlant::Italic);
        let metadata = sheet.character_style_metadata(&comment.id).unwrap().clone();
        sheet.insert_character_style(comment, metadata).unwrap();
        let sheet = Arc::new(sheet);
        document.install_code_presentation(sheet.clone(), &[]);
        let mut core = Core::<MockTextMeasurementProvider>::new(document);
        let calls = Arc::new(AtomicUsize::new(0));
        let regions = Arc::new(Mutex::new(Vec::new()));
        let calls_clone = calls.clone();
        let regions_clone = regions.clone();
        core.set_syntax_provider_factory(Arc::new(move || {
            Box::new(WholeDocumentProvider {
                regions: regions_clone.clone(),
                calls: calls_clone.clone(),
                mode: 0,
            })
        }));
        let view = core.add_view(MockTextMeasurementProvider::default(), 600., 100.);
        let revision = core.document().revision();
        let history = core.document().history_status();
        let mut export = core.prepare_html_export(view).unwrap();
        // Keep this test independent of the process-wide Code stylesheet.
        export.projection.install_code_styles(sheet, &[]);
        let output = String::from_utf8(export.render().unwrap()).unwrap();
        assert!(calls.load(Ordering::SeqCst) >= 2);
        let recorded = regions.lock().unwrap();
        let mut start = 0;
        while start < source.len() {
            let mut end = (start + MAX_REGION_BYTES).min(source.len());
            while !source.is_char_boundary(end) {
                end -= 1;
            }
            assert!(
                recorded.contains(&(start..end)),
                "export skipped {start}..{end}: {recorded:?}"
            );
            start = end;
        }
        assert!(output.contains("font-family: monospace"));
        assert!(output.contains("line-height: 1.7"));
        assert!(output.contains("&lt;script&gt;alert(1)&lt;/script&gt; &amp;"));
        assert!(!output.contains("<script>"));
        let tail = output.rfind("TAIL").unwrap();
        assert!(output[..tail].rfind("color: #ff0000ff").is_some());
        assert_eq!(core.document().revision(), revision);
        assert_eq!(core.document().history_status(), history);
        assert_eq!(core.document().source_bytes(), source.as_bytes());
        assert!(!core.document().is_dirty());
    }

    #[test]
    fn missing_or_panicking_syntax_and_unknown_language_still_export_literal_text() {
        for mode in [1, 2] {
            let mut core =
                Core::<MockTextMeasurementProvider>::new(open("<unknown>&", Format::Code));
            core.set_syntax_provider_factory(Arc::new(move || {
                Box::new(WholeDocumentProvider {
                    regions: Arc::default(),
                    calls: Arc::default(),
                    mode,
                })
            }));
            let view = core.add_view(MockTextMeasurementProvider::default(), 400., 100.);
            let output =
                String::from_utf8(core.prepare_html_export(view).unwrap().render().unwrap())
                    .unwrap();
            assert!(output.contains("&lt;unknown&gt;&amp;"));
        }
        let input = SyntaxInputSnapshot::new(
            SyntaxInputIdentity {
                document: 1,
                revision: 1,
                generation: 1,
            },
            open("anything", Format::Code)
                .projection()
                .text_tree()
                .clone(),
        );
        assert!(SyntaxService::default().export_runs(input).is_empty());
    }

    #[test]
    fn literal_text_retains_empty_lines_unicode_and_final_line_boundary() {
        let source = "\n  A\t<&> é\n\nlast\n";
        let output = html(&open(source, Format::PlainText));
        let decoded = Encoding::Utf8.decode(output.as_bytes()).unwrap();
        let input = super::super::line_endings::normalize(&decoded, FileFormat::Unix);
        let reopened = super::super::html::project_fragment(&input, Revision(0), 0, output.len());
        assert_eq!(reopened.text(), source);
    }

    #[test]
    fn final_syntax_subsets_visit_uncovered_text() {
        let input = SyntaxInputSnapshot::new(
            SyntaxInputIdentity {
                document: 987,
                revision: 1,
                generation: 1,
            },
            open("one two three", Format::Code)
                .projection()
                .text_tree()
                .clone(),
        );
        let service = SyntaxService::with_factory(Arc::new(|| {
            Box::new(WholeDocumentProvider {
                regions: Arc::default(),
                calls: Arc::default(),
                mode: 3,
            })
        }));
        let runs = service.export_runs(input.clone());
        assert_eq!(runs.first().unwrap().range.start, 0);
        assert_eq!(runs.last().unwrap().range.end, input.byte_len());
        assert!(runs
            .windows(2)
            .all(|pair| pair[0].range.end == pair[1].range.start));
    }

    #[test]
    fn links_keep_destinations_while_images_references_and_unsafe_schemes_stay_passive() {
        let source = "[**site** & label](https://example.com/?a=1&b=2) <mailto:writer@example.com> <reader@example.com> https://example.org/page [bad](javascript:alert) [relative](other.html#part) ![img](image.png) [ref][id]\n\n[id]: https://example.net/";
        let output = html(&open(source, Format::Markdown));
        assert_eq!(output, html(&open(source, Format::MarkdownSource)));
        assert!(
            output.contains("href=\"https://example.com/?a=1&amp;b=2\""),
            "{output}"
        );
        assert!(
            output.contains("href=\"mailto:writer@example.com\""),
            "{output}"
        );
        assert!(output.contains("href=\"mailto:reader@example.com\""));
        assert!(output.contains("href=\"https://example.org/page\""));
        assert!(output.contains("href=\"other.html#part\""));
        assert!(!output.contains("href=\"javascript:"));
        assert!(!output.contains("href=\"https://example.net/"));
        assert!(!output.contains("<img"));
    }

    #[test]
    fn bundled_tree_sitter_and_html_vim_syntax_export_styled_code() {
        for (filename, source) in [
            ("example.rs", "// An exported Rust document\nfn main() {\n\tlet answer = 42;\n\tprintln!(\"<hello> & world\");\n}\n"),
            ("example.html", "<!doctype html>\n<!-- Editable HTML source -->\n<html lang=\"en\">\n  <body><h1>Hello &amp; welcome</h1></body>\n</html>\n"),
        ] {
            let mut core = Core::<MockTextMeasurementProvider>::new(open(source, Format::Code));
            core.initialize_code_detection(filename, false).unwrap();
            core.configure_syntax(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/vim/runtime/syntax"));
            let view = core.add_view(MockTextMeasurementProvider::default(), 700., 200.);
            core.set_indentation_defaults(super::super::IndentationOptions { tabstop: 8, ..Default::default() }).unwrap();
            let output = String::from_utf8(core.prepare_html_export(view).unwrap().render().unwrap()).unwrap();
            assert!(output.matches(".c").count() > 2, "{filename}: {output}");
            assert!(output.contains("tab-size:8"));
            assert!(output.contains("font-family: monospace"));
            assert!(!output.contains("<h1>Hello"));
            if std::env::var_os("VIEM_EXPORT_REVIEW").is_some() {
                std::fs::write(if filename.ends_with(".rs") { "/tmp/viem-export-code.html" } else { "/tmp/viem-export-html-code.html" }, &output).unwrap();
            }
        }
        if std::env::var_os("VIEM_EXPORT_REVIEW").is_some() {
            let source = std::fs::read_to_string(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/docs/markdown_demo.md"
            ))
            .unwrap();
            std::fs::write(
                "/tmp/viem-export-markdown.html",
                html(&open(&source, Format::Markdown)),
            )
            .unwrap();
        }
    }
}
