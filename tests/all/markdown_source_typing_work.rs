//! Typing ordinary prose in Markdown Source must stay local to the edit.
use viem_core::command::{CommandStatus, InputEvent, Key};
use viem_core::document::{
    measure_document_work, BoundaryAffinity, Document, DocumentWorkStatistics, Encoding, Format,
};
use viem_core::layout::MockTextMeasurementProvider;
use viem_core::{Core, CoreEvent, ViewId};

type Editor = Core<MockTextMeasurementProvider>;

const CHAPTER: &str = "# Heading with *emphasis*\n\n\
Paragraph text with **bold**, `code`, and a [link](https://example.test).\n\
A continuation line of the same paragraph.\n\n\
- first item\n- second *item*\n  continued\n\n\
1. one\n2. two\n\n\
> quoted text\n\n\
```rust\nfn main() {}\n```\n\n\
Closing prose paragraph.\n";

fn input(core: &mut Editor, view: ViewId, event: InputEvent) {
    let outcome = core.handle_with_layout(view, CoreEvent::Input(event)).unwrap();
    assert!(outcome.command.is_none_or(|command| matches!(
        command.status,
        CommandStatus::Complete | CommandStatus::Pending
    )));
}

fn keys(core: &mut Editor, view: ViewId, keys: &str) {
    for character in keys.chars() {
        input(core, view, InputEvent::Key(Key::Char(character)));
    }
}

/// Opens `chapters` copies of the fixture and enters Insert mode at the end
/// of the continuation prose line in the middle chapter.
fn fixture(chapters: usize, line: &str) -> (Editor, ViewId) {
    fixture_of(CHAPTER, chapters, line)
}

fn fixture_of(chapter: &str, chapters: usize, line: &str) -> (Editor, ViewId) {
    let source = chapter.repeat(chapters);
    let document = Document::from_bytes(source.into_bytes(), Encoding::Utf8, Format::MarkdownSource)
        .unwrap();
    let mut core = Core::new(document);
    let view = core.add_view(MockTextMeasurementProvider::new(), 600., 400.);
    let (start, _) = core.document().text().match_indices(line).nth(chapters / 2).unwrap();
    let offset = start + line.len() - 1;
    core.handle(
        view,
        CoreEvent::PlaceCursor {
            document_revision: core.document().revision(),
            text_offset: offset,
            affinity: BoundaryAffinity::Downstream,
            extend_selection: false,
        },
    )
    .unwrap();
    keys(&mut core, view, "A");
    (core, view)
}

/// Everything a fresh parse determines, without the identities that an
/// incremental projection carries forward from the previous revision.
fn projection_summary(document: &Document) -> String {
    let projection = document.projection();
    let blocks = projection
        .blocks()
        .iter()
        .map(|block| {
            let mut block = block.clone();
            block.containers = block.containers.iter().enumerate().map(|(slot, member)| {
                let mut member = member.clone();
                std::sync::Arc::make_mut(&mut member.container).id = viem_core::document::ContainerIdentity { anchor: 0, slot: slot as u32 };
                member
            }).collect::<Vec<_>>().into();
            format!("{:?} {:?}", block.range, block.attributes)
        })
        .collect::<Vec<_>>();
    let hard_lines = (0..projection.hard_line_count())
        .map(|line| projection.hard_line_range(line))
        .collect::<Vec<_>>();
    let presentation = [false, true].map(|flow| {
        (0..projection.presentation_line_count(flow))
            .map(|line| projection.presentation_line_range(line, flow))
            .collect::<Vec<_>>()
    });
    format!(
        "text {:?}\nblocks {blocks:#?}\nstyles {:#?}\nhard lines {hard_lines:?}\npresentation {presentation:?}",
        document.text(),
        projection.style_spans(),
    )
}

fn assert_matches_fresh_parse(document: &Document, context: &str) {
    let fresh =
        Document::from_bytes(document.source_bytes(), Encoding::Utf8, Format::MarkdownSource).unwrap();
    let (incremental, fresh) = (projection_summary(document), projection_summary(&fresh));
    if incremental != fresh {
        let line = incremental
            .lines()
            .zip(fresh.lines())
            .position(|(a, b)| a != b)
            .unwrap_or(0);
        panic!(
            "{context}: incremental projection differs from a fresh parse at summary line {line}\n\
             incremental: {:?}\nfresh:       {:?}\nsource: {:?}",
            incremental.lines().nth(line),
            fresh.lines().nth(line),
            String::from_utf8_lossy(&document.source_bytes()),
        );
    }
}

/// Documents whose line structure a single typed character can disturb:
/// setext underlines, lists that interrupt paragraphs, lazy continuations,
/// nested items, quotes, emphasis spanning source lines and code fences.
const DIFFERENTIAL_SOURCES: [&str; 10] = [
    CHAPTER,
    "Title\n---\nText *spans\ntwo lines* here.\n- item\ntext after item\n\n  indented\n",
    "Para one\n1. first\n   - nested\n     more\n2. second\n> quote\ncontinued\n\n***\nEnd",
    "a\n\n\n\nb\n=\n\n# h #\n\n    code\n\n<div>\nhtml\n</div>\n\n[x]: /url\n",
    "one *two\nthree\nfour\nfive* six [a\nb\nc\nd](u) `x\ny\nz\nw` end\nlast line\n",
    "- item one\n  line two with *emph\n  line three\n  line four* done\n  > quoted in item\n  > more\n- item two\n",
    "~~~\nfenced\n~~~\ntext\n<!-- c\nd -->\n\n| a | b |\n|---|---|\n| c | d |\n\nx  \ny\\\nz\n",
    "x y\nl2\nl3\nl4\nend* z\nl6 [label\nl7\nl8\nl9](u)\n\nnext\n",
    "- a\n  - b\n    - c\n      deep text\n      more *x\n      y* z\n      last\n- tail\n",
    "one\ntwo\nthree\nfour](u) five\n- item\n  one\n  two\n  three](v)\n",
];

const BACKSPACE: &str = "<BS>";
const ENTER: &str = "<CR>";

const TYPED: [&str; 17] = [
    BACKSPACE,
    "a", "1", " ", ".", ",", "*", "-", "=", "_", "`", "#", ">", "[", "]", "\\", ")",
];

/// Types `typed` at `offset` in a fresh document for `source`, then checks
/// the literal source change and compares the result with a fresh parse.
fn check_typing(source: &str, offset: usize, typed: &str) {
    let document =
        Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::MarkdownSource).unwrap();
    if !document.text().is_char_boundary(offset) {
        return;
    }
    let mut core = Core::new(document);
    let view = core.add_view(MockTextMeasurementProvider::new(), 600., 400.);
    core.handle(
        view,
        CoreEvent::PlaceCursor {
            document_revision: core.document().revision(),
            text_offset: offset,
            affinity: BoundaryAffinity::Downstream,
            extend_selection: false,
        },
    )
    .unwrap();
    keys(&mut core, view, "i");
    let event = match typed {
        BACKSPACE => InputEvent::Key(Key::Backspace),
        ENTER => InputEvent::Key(Key::Enter),
        _ => InputEvent::text(typed),
    };
    core.handle_with_layout(view, CoreEvent::Input(event))
        .unwrap_or_else(|error| panic!("typing {typed:?} at {offset} in {source:?}: {error:?}"));
    if typed != BACKSPACE && typed != ENTER && !typed.contains('\n') {
        // Source view writes the typed characters as literal source.
        let (before, after) = (source.as_bytes(), core.document().source_bytes());
        let at = before.iter().zip(&after).take_while(|(a, b)| a == b).count();
        assert_eq!(
            [&before[..at], typed.as_bytes(), &before[at..]].concat(),
            after,
            "typing {typed:?} at {offset} in {source:?}"
        );
    }
    assert_matches_fresh_parse(core.document(), &format!("typing {typed:?} at {offset} in {source:?}"));
}

fn text_len(source: &str) -> usize {
    Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::MarkdownSource)
        .unwrap()
        .text()
        .len()
}

/// Types each character at every caret position of each source, then
/// compares the incremental result with a fresh parse of the same bytes.
#[test]
fn markdown_source_typing_matches_a_fresh_parse_at_every_position() {
    for source in DIFFERENTIAL_SOURCES {
        for offset in 0..=text_len(source) {
            for typed in TYPED {
                check_typing(source, offset, typed);
            }
        }
    }
}

/// Enter adds rows, and Backspace at a row start removes one; both reach
/// the regional parse with rows after the edit moved by the change.
#[test]
fn markdown_source_breaks_match_a_fresh_parse_at_every_position() {
    for source in DIFFERENTIAL_SOURCES {
        for offset in 0..=text_len(source) {
            for typed in [ENTER, "\n", "a\nb", "\n\n- x"] {
                check_typing(source, offset, typed);
            }
        }
    }
}

/// Blocks longer than the regional reparse limit: the region ends inside
/// them, so edits near their first and last rows exercise the rules for a
/// region that stops mid-block.
#[test]
fn markdown_source_typing_in_long_blocks_matches_a_fresh_parse() {
    let rows = |prefix: &str, count: usize| {
        (0..count).map(|n| format!("{prefix}row {n}\n")).collect::<String>()
    };
    let sources = [
        format!("Lead\n{}\nTail\n", rows("", 80)),
        format!("{}# mid\n{}", rows("", 70), rows("", 70)),
        (0..80).map(|n| format!("row {n} \n")).collect::<String>(),
        format!("{}row  \nbreak\\\nafter\n{}", rows("", 70), rows("", 70)),
        format!("Lead\n\n<div>\n{}</div>\n\nTail\n", rows("", 80)),
        format!("Lead\n\n- item\n{}\nTail\n", rows("  ", 80)),
        format!("Lead\n\n{}\nTail\n", rows("> ", 80)),
    ];
    for source in &sources {
        let text = Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::MarkdownSource)
            .unwrap()
            .text()
            .to_owned();
        let row_starts = std::iter::once(0)
            .chain(text.match_indices('\n').map(|(at, _)| at + 1))
            .collect::<Vec<_>>();
        let near_edges = row_starts.len().saturating_sub(5);
        let middle = row_starts.len() / 2;
        for (row, &start) in row_starts.iter().enumerate() {
            if (6..near_edges).contains(&row) && !(middle - 3..=middle + 3).contains(&row) {
                continue;
            }
            let end = row_starts.get(row + 1).map_or(text.len(), |next| next - 1);
            for offset in start..=end {
                for typed in ["a", " ", "\\", "[", "]", "#", "-", "<", ">", BACKSPACE, ENTER] {
                    check_typing(source, offset, typed);
                }
            }
        }
    }
}

fn type_text(core: &mut Editor, view: ViewId, text: &str) -> DocumentWorkStatistics {
    measure_document_work(|| input(core, view, InputEvent::text(text))).1
}

const LINES: [&str; 7] = [
    "Heading with *emphasis*",
    "Paragraph text with **bold**, `code`, and a [link](https://example.test).",
    "A continuation line of the same paragraph.",
    "second *item*",
    "  continued",
    "quoted text",
    "Closing prose paragraph.",
];

/// A keystroke decodes and projects only the rows around it, whatever the
/// document size: no full projection and no work proportional to the file.
#[test]
fn markdown_source_typing_stays_regional_in_a_large_document() {
    let chapters = 2048;
    for line in LINES {
        let (mut core, view) = fixture(chapters, line);
        for typed in ["x", " ", ".", "*"] {
            let work = type_text(&mut core, view, typed);
            assert_eq!(
                (work.full_projection_candidates, work.regional_projection_candidates),
                (0, 1),
                "typing {typed:?} at the end of {line:?}"
            );
            assert!(work.source_decoded_bytes < 4096, "{line:?}: {work:?}");
            assert!(work.projected_formatted_bytes < 4096, "{line:?}: {work:?}");
            assert_eq!(work.source_full_materialized_bytes, 0, "{line:?}: {work:?}");
        }
        let fresh = Document::from_bytes(core.document().source_bytes(), Encoding::Utf8, Format::MarkdownSource)
            .unwrap();
        assert_eq!(projection_summary(core.document()), projection_summary(&fresh), "{line:?}");
    }
}

/// Enter adds a row by splitting a paragraph, continuing a list or quote,
/// or ending a row whose trailing hard-break spaces the new paragraph trims.
/// It reparses only the rows around it, whatever the document size.
#[test]
fn markdown_source_enter_stays_regional_in_a_large_document() {
    const HARD_BREAKS: &str = "Prose that ends in a hard break  \nand continues\\\nto a third row.\n\n";
    const TYPED: [Key; 4] = [Key::Enter, Key::Char('x'), Key::Enter, Key::Backspace];
    // Enter twice at the end of a list leaves it through an empty item.
    const LEAVE_LIST: [Key; 4] = [Key::Enter, Key::Char('x'), Key::Enter, Key::Enter];
    let cases = LINES
        .iter()
        .map(|line| (CHAPTER, *line, TYPED))
        .chain([
            (CHAPTER, "2. two", LEAVE_LIST),
            (HARD_BREAKS, "Prose that ends in a hard break  ", TYPED),
            (HARD_BREAKS, "and continues\\", TYPED),
            (HARD_BREAKS, "to a third row.", TYPED),
        ]);
    for (chapter, line, typed) in cases {
        let (mut core, view) = fixture_of(chapter, 2048, line);
        for (step, key) in typed.into_iter().enumerate() {
            let work = measure_document_work(|| input(&mut core, view, InputEvent::Key(key))).1;
            assert_eq!(
                (work.full_projection_candidates, work.regional_projection_candidates),
                (0, 1),
                "step {step}, {key:?} after {line:?}"
            );
            assert!(work.source_decoded_bytes < 4096, "{line:?}: {work:?}");
            assert!(work.projected_formatted_bytes < 4096, "{line:?}: {work:?}");
            assert_eq!(work.source_full_materialized_bytes, 0, "{line:?}: {work:?}");
        }
        let fresh = Document::from_bytes(core.document().source_bytes(), Encoding::Utf8, Format::MarkdownSource)
            .unwrap();
        assert_eq!(projection_summary(core.document()), projection_summary(&fresh), "{line:?}");
    }
}
