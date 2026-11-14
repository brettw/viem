//! Container parity: line-oriented commands treat Markdown containers alike.
//!
//! A line that a command adds beside a line inside a quote, alert, code block,
//! front matter or list belongs to the same container, whichever command adds
//! it and wherever the caret's line sits in the container. Each cell runs one
//! operation on the first, a middle and the last line of one container and
//! asserts the new line's container path, that no container was created or
//! split, that the saved bytes reopen as displayed and that undo/redo are
//! exact.
//!
//! A deliberate difference for one container kind belongs in [`EXCEPTIONS`],
//! with the rule that requires it. See
//! [container parity](../../docs/markdown-feature-contracts.md#container-parity).
use crate::support::{assert_reopens_as_displayed, Editor};
use viem_core::command::{InputEvent, Key};
use viem_core::document::{AlertKind, CodeBlockKind, ContainerKind, Document, Format, ViewPresentation};
use viem_core::CoreEvent;

/// Each container holds the lines `aa`, `bb` and `cc`, between paragraphs `p`
/// and `z`, so a line that leaves the container lands in one of them.
const CONTAINERS: &[(&str, &str)] = &[
    ("quote", "p\n\n> aa\n>\n> bb\n>\n> cc\n\nz"),
    ("alert", "p\n\n> [!NOTE]\n> aa\n>\n> bb\n>\n> cc\n\nz"),
    ("fenced code", "p\n\n```\naa\nbb\ncc\n```\n\nz"),
    ("indented code", "p\n\n    aa\n    bb\n    cc\n\nz"),
    ("quoted code", "p\n\n> ```\n> aa\n> bb\n> cc\n> ```\n\nz"),
    ("front matter", "---\naa: 1\nbb: 2\ncc: 3\n---\n\nz"),
    ("bulleted list", "p\n\n- aa\n- bb\n- cc\n\nz"),
    ("numbered list", "p\n\n1. aa\n2. bb\n3. cc\n\nz"),
];

const LINES: &[&str] = &["aa", "bb", "cc"];

#[derive(Clone, Copy, Debug, PartialEq)]
enum Operation {
    /// `yyp`: a rich register put below.
    YankPut,
    /// `yyP`: a rich register put above.
    YankPutAbove,
    /// `yy2p`: a counted put below.
    YankPutCount,
    /// `"ap` with a plain linewise register (yanked in a source view).
    PlainPut,
    /// `"aP` with a plain linewise register.
    PlainPutAbove,
    /// `o` and typing.
    OpenBelow,
    /// `O` and typing.
    OpenAbove,
    /// `A`, Enter and typing.
    EnterAtEnd,
}

const OPERATIONS: &[Operation] = &[
    Operation::YankPut,
    Operation::YankPutAbove,
    Operation::YankPutCount,
    Operation::PlainPut,
    Operation::PlainPutAbove,
    Operation::OpenBelow,
    Operation::OpenAbove,
    Operation::EnterAtEnd,
];

/// Deliberate per-container differences: (container, operation, line, rule).
/// Every entry names the requirement that makes the container differ.
const EXCEPTIONS: &[(&str, Operation, &str, &str)] = &[];

impl Operation {
    /// Lines added by the operation, relative to the caret's line index.
    fn added(self) -> &'static [isize] {
        match self {
            Self::YankPutCount => &[1, 2],
            Self::YankPutAbove | Self::PlainPutAbove | Self::OpenAbove => &[0],
            _ => &[1],
        }
    }
}

/// The containers that own a line, outermost first. A new list item is still
/// in its list, so only the list (not the item) takes part in identity.
fn container_path(document: &Document, offset: usize) -> Vec<(ContainerKind, Option<AlertKind>, Option<CodeBlockKind>)> {
    let block = document
        .projection()
        .blocks()
        .iter()
        .find(|block| block.range.start <= offset && offset <= block.range.end)
        .expect("line has a block");
    block
        .containers
        .iter()
        .filter(|member| member.container.kind != ContainerKind::ListItem)
        .map(|member| {
            let kind = member.container.kind;
            (kind, member.container.alert, (kind == ContainerKind::CodeBlock).then_some(block.code_kind))
        })
        .collect()
}

/// Quote, code and list containers in the document; a line that splits or
/// opens one changes this count.
fn container_count(document: &Document) -> usize {
    document
        .projection()
        .blocks()
        .iter()
        .flat_map(|block| block.containers.iter())
        .filter(|member| member.starts_here && member.container.kind != ContainerKind::ListItem)
        .count()
}

fn line_starts(document: &Document) -> Vec<usize> {
    let text = document.text();
    std::iter::once(0).chain(text.match_indices('\n').map(|(at, _)| at + 1)).collect()
}

fn input(editor: &mut Editor, keys: &str) -> Result<(), String> {
    let mut events: Vec<InputEvent> = Vec::new();
    for character in keys.chars() {
        events.push(match character {
            '\r' => InputEvent::Key(Key::Enter),
            '\x1b' => InputEvent::Key(Key::Escape),
            other => InputEvent::key(other),
        });
    }
    for event in events {
        editor.core.handle(editor.view, CoreEvent::Input(event)).map_err(|error| format!("{error:?}"))?;
    }
    Ok(())
}

fn run(editor: &mut Editor, operation: Operation) -> Result<(), String> {
    match operation {
        Operation::YankPut => input(editor, "yyp"),
        Operation::YankPutAbove => input(editor, "yyP"),
        Operation::YankPutCount => input(editor, "yy2p"),
        Operation::PlainPut => input(editor, "\"ap"),
        Operation::PlainPutAbove => input(editor, "\"aP"),
        Operation::OpenBelow => input(editor, "oqq\x1b"),
        Operation::OpenAbove => input(editor, "Oqq\x1b"),
        Operation::EnterAtEnd => input(editor, "A\rqq\x1b"),
    }
}

fn check(name: &str, source: &str, operation: Operation, line: &str) -> Option<String> {
    let context = format!("{name} {operation:?} on {line:?}");
    let mut editor = Editor::open(source, Format::Markdown);
    let text = editor.document().text().to_owned();
    let at = text.find(line).unwrap_or_else(|| panic!("{context}: {line:?} in {text:?}"));
    let origin_line = line_starts(editor.document()).iter().rposition(|start| *start <= at).unwrap();
    let expected = container_path(editor.document(), at);
    let line_text = |document: &Document, index: usize| document.text().split('\n').nth(index).map(str::to_owned);
    let copied = line_text(editor.document(), origin_line).unwrap();
    let added_text = match operation {
        Operation::YankPut | Operation::YankPutAbove | Operation::YankPutCount => copied,
        Operation::PlainPut | Operation::PlainPutAbove => "z".to_owned(),
        _ => "qq".to_owned(),
    };
    let containers = container_count(editor.document());
    if matches!(operation, Operation::PlainPut | Operation::PlainPutAbove) {
        // A source view's yank holds the last line, `z`, without a rich
        // fragment, so the put takes the plain payload path.
        editor.core.set_view_presentation(editor.view, ViewPresentation::literal()).unwrap();
        input(&mut editor, "G\"ayy").unwrap();
        editor.core.set_view_presentation(editor.view, ViewPresentation::preview()).unwrap();
    }
    editor.place(at);
    if let Err(error) = run(&mut editor, operation) {
        return Some(format!("{context}: failed: {error}"));
    }
    let after = editor.source();
    let document = editor.document();
    let starts = line_starts(document);
    let text = document.text();
    for offset in operation.added() {
        let index = origin_line.checked_add_signed(*offset).unwrap();
        let Some(&start) = starts.get(index) else {
            return Some(format!("{context}: no line {index} in {text:?}"));
        };
        if line_text(document, index).as_deref() != Some(added_text.as_str()) {
            return Some(format!("{context}: line {index} is {:?}, expected {added_text:?}; source {after:?}",
                line_text(document, index)));
        }
        let path = container_path(document, start);
        if path != expected {
            return Some(format!("{context}: added line {index} has containers {path:?}, expected {expected:?}; source {after:?}"));
        }
    }
    if container_count(document) != containers {
        return Some(format!("{context}: container count changed; source {after:?}"));
    }
    assert_reopens_as_displayed(document, &context);
    editor.key(Key::Escape);
    editor.undo();
    if editor.source() != source {
        return Some(format!("{context}: undo gave {:?}", editor.source()));
    }
    editor.redo();
    if editor.source() != after {
        return Some(format!("{context}: redo gave {:?}, expected {after:?}", editor.source()));
    }
    None
}

#[test]
fn line_commands_keep_new_lines_in_the_caret_container() {
    let mut failures = Vec::new();
    for (name, source) in CONTAINERS {
        for operation in OPERATIONS {
            for line in LINES {
                let excepted = EXCEPTIONS.iter().any(|(container, excepted, excepted_line, _)|
                    container == name && excepted == operation && excepted_line == line);
                match (check(name, source, *operation, line), excepted) {
                    (Some(failure), false) => failures.push(failure),
                    (None, true) => failures.push(format!(
                        "{name} {operation:?} on {line:?} now matches other containers; remove its exception")),
                    _ => {}
                }
            }
        }
    }
    assert!(failures.is_empty(), "{} parity failures:\n{}", failures.len(), failures.join("\n"));
}
