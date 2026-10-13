//! Boundary cases that shared transaction and projection changes have broken
//! before: whitespace-only separator lines, CR/LF seams, list and quote edges,
//! headings without blank lines, and table neighbors, each edited at every
//! position by typing, Enter, deletion, `dd`/`ddP`/`yyp` and replacing a
//! selection with Enter.
//!
//! Every case must either fail without changing source, or reopen exactly as
//! displayed and undo to the original bytes. Shadow verification (on for
//! Cargo-run tests) also compares each regional candidate with complete
//! reprojection. This runs in about a second; run it first after changing
//! candidate strategies, separator handling or list/quote structure:
//! `cargo test --test all edit_boundary_matrix::`.
use crate::support::{shape, Editor};
use unicode_segmentation::UnicodeSegmentation;
use viem_core::command::{InputEvent, Key, NavigationKey};
use viem_core::document::{BoundaryAffinity, Document, Format, HistoryNavigationRequest};
use viem_core::CoreEvent;

const SEEDS: &[&str] = &[
    // Whitespace-only separator lines are blank lines, not content.
    "a\n \nb",
    "a\n\t\nb",
    "a\n  \n  \nb",
    " \nb",
    // Line-ending seams.
    "a\r\n\r\nb\r\n",
    "- a\r\n- b\r\n",
    "a\r\n \r\nb",
    // List and quote boundaries.
    "- a\n\n- b",
    "- a\n- b\n\nc",
    "1. a\n2. b",
    "text\n- item",
    "> a\n>\n> b",
    "> quote\n\nafter",
    // Headings that interrupt or are followed directly by a paragraph.
    "# h\nbody",
    "body\n# h",
    // A table beside prose.
    "before\n\n| a | b |\n| - | - |\n| 1 | 2 |\n\nafter",
];

/// Operations that currently fail cleanly (no source change) for a seed. They
/// are long-standing consistency findings, not regressions; remove an entry
/// when the operation starts to succeed so the matrix guards the fix.
const KNOWN_FAILURES: &[(&str, &str)] = &[
    // `dd` on the last item of a list followed by a paragraph fails
    // verification, and `P` cannot put a deleted list item back there.
    ("- a\n- b\n\nc", "DeleteLine"),
    ("- a\n- b\n\nc", "DeleteLinePut"),
    // `P` cannot put a deleted paragraph back beside a list or table.
    ("text\n- item", "DeleteLinePut"),
    ("before\n\n| a | b |\n| - | - |\n| 1 | 2 |\n\nafter", "DeleteLinePut"),
    // Forward Delete at a paragraph end before a table fails verification
    // instead of reporting an unsupported structural join.
    ("before\n\n| a | b |\n| - | - |\n| 1 | 2 |\n\nafter", "Delete"),
    // The same paragraph-to-table boundary refuses a selection replacement.
    ("before\n\n| a | b |\n| - | - |\n| 1 | 2 |\n\nafter", "SelectionEnter"),
];

#[derive(Clone, Copy, Debug)]
enum Operation {
    Type,
    Enter,
    Backspace,
    Delete,
    DeleteLine,
    DeleteLinePut,
    YankLinePut,
    SelectionEnter,
}

const OPERATIONS: &[Operation] = &[
    Operation::Type,
    Operation::Enter,
    Operation::Backspace,
    Operation::Delete,
    Operation::DeleteLine,
    Operation::DeleteLinePut,
    Operation::YankLinePut,
    Operation::SelectionEnter,
];

fn input(editor: &mut Editor, event: InputEvent) -> Result<(), String> {
    editor.core.handle(editor.view, CoreEvent::Input(event)).map(|_| ()).map_err(|error| format!("{error:?}"))
}

fn run(editor: &mut Editor, operation: Operation, at: usize) -> Result<(), String> {
    let revision = editor.document().revision();
    let normal = matches!(operation, Operation::DeleteLine | Operation::DeleteLinePut | Operation::YankLinePut);
    if !normal {
        input(editor, InputEvent::key('i'))?;
    }
    editor
        .core
        .handle(editor.view, CoreEvent::PlaceCursor {
            document_revision: revision,
            text_offset: at,
            affinity: BoundaryAffinity::Downstream,
            extend_selection: false,
        })
        .map_err(|error| format!("place: {error:?}"))?;
    let keys: Vec<InputEvent> = match operation {
        Operation::Type => vec![InputEvent::key('x')],
        Operation::Enter => vec![InputEvent::Key(Key::Enter)],
        Operation::Backspace => vec![InputEvent::Key(Key::Backspace)],
        Operation::Delete => vec![InputEvent::Key(Key::Delete)],
        Operation::DeleteLine => "dd".chars().map(InputEvent::key).collect(),
        Operation::DeleteLinePut => "ddP".chars().map(InputEvent::key).collect(),
        Operation::YankLinePut => "yyp".chars().map(InputEvent::key).collect(),
        Operation::SelectionEnter => vec![
            InputEvent::Key(Key::ModifiedNavigation { key: NavigationKey::Right, modifiers: 1 }),
            InputEvent::Key(Key::Enter),
        ],
    };
    for key in keys {
        let before = editor.document().source_bytes();
        if let Err(error) = input(editor, key) {
            // A failed key leaves the source exactly as the previous key left it.
            return Err(if editor.document().source_bytes() == before { error }
                else { format!("{error} (and changed source)") });
        }
    }
    input(editor, InputEvent::Key(Key::Escape))
}

fn check(source: &str, operation: Operation, at: usize) -> Option<String> {
    let mut editor = Editor::open(source, Format::Markdown);
    let original = editor.document().source_bytes();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(&mut editor, operation, at)));
    let context = format!("{source:?} {operation:?} at {at}");
    match result {
        Err(panic) => {
            let message = panic.downcast_ref::<String>().cloned()
                .or_else(|| panic.downcast_ref::<&str>().map(|text| text.to_string())).unwrap_or_default();
            return Some(format!("{context}: panicked: {message}"));
        }
        Ok(Err(error)) if error.ends_with("(and changed source)") => {
            return Some(format!("{context}: failed key changed source: {error}"));
        }
        Ok(Err(error)) if KNOWN_FAILURES.iter().any(|(seed, known)| *seed == source && *known == format!("{operation:?}")) => {
            let _ = error;
        }
        // Table cells refuse edits across their boundaries by design.
        Ok(Err(error)) if error.contains("UnsupportedTableEdit") => {}
        Ok(Err(error)) => return Some(format!("{context}: failed: {error}")),
        Ok(Ok(())) => {}
    }
    let saved = editor.document().source_bytes();
    let reopened = Document::from_bytes(saved.clone(), editor.document().encoding(), Format::Markdown)
        .expect("saved bytes reopen");
    if shape(editor.document()) != shape(&reopened) {
        return Some(format!("{context}: reopens differently: saved {:?}\n  shown  {:?}\n  reopen {:?}",
            String::from_utf8_lossy(&saved), shape(editor.document()), shape(&reopened)));
    }
    if saved != original {
        for _ in 0..4 {
            if editor.document().source_bytes() == original { break; }
            if editor.core.handle(editor.view, CoreEvent::NavigateHistory(HistoryNavigationRequest::Undo)).is_err() { break; }
        }
        if editor.document().source_bytes() != original {
            return Some(format!("{context}: undo restored {:?}", String::from_utf8_lossy(&editor.document().source_bytes())));
        }
    }
    None
}

#[test]
fn boundary_edits_reopen_as_displayed_and_undo_exactly() {
    let mut failures = Vec::new();
    let mut cases = 0;
    for source in SEEDS {
        let text = Editor::open(source, Format::Markdown).document().text().to_owned();
        let positions: Vec<usize> = text.grapheme_indices(true).map(|(at, _)| at).chain([text.len()]).collect();
        for &at in &positions {
            for &operation in OPERATIONS {
                cases += 1;
                if let Some(failure) = check(source, operation, at) {
                    failures.push(failure);
                }
            }
        }
    }
    assert!(failures.is_empty(), "{} of {cases} boundary cases failed:\n{}", failures.len(), failures.join("\n"));
}
