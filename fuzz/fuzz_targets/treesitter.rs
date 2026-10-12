//! Native Tree-sitter grammars (C parsers and external scanners) and the
//! bundled highlight queries over untrusted text, including one incremental
//! reparse. Build with AddressSanitizer for the C code; see docs/fuzzing.md.
//! Layout: [grammar][edit start][edit old length][edit insert length][text...].
#![no_main]
use libfuzzer_sys::fuzz_target;
use std::sync::atomic::AtomicBool;
use viem_core::document::syntax::treesitter::{
    self, ParseOutcome, SyntaxInputEdit, TreeSitterBudget, TreeSitterSession,
};
use viem_core::document::syntax::{SyntaxInputIdentity, SyntaxInputSnapshot};
use viem_core::document::FormattedTextTree;

const GRAMMARS: &[&str] = &[
    "c", "cpp", "rust", "swift", "objc", "c_sharp", "javascript", "typescript", "tsx", "python",
    "json", "markdown", "markdown_inline",
];

fn snapshot(text: &str, revision: u64) -> Option<SyntaxInputSnapshot> {
    Some(SyntaxInputSnapshot::new(
        SyntaxInputIdentity { document: 1, revision, generation: 1 },
        FormattedTextTree::try_from_text(text).ok()?,
    ))
}

fn parse(
    session: &mut TreeSitterSession,
    input: SyntaxInputSnapshot,
    edits: &[SyntaxInputEdit],
    budget: &TreeSitterBudget,
) {
    let cancelled = AtomicBool::new(false);
    for _ in 0..512 {
        match session.parse(input.clone(), edits, &[], budget, &cancelled) {
            Ok(ParseOutcome::Complete { snapshot, .. }) => {
                let _ = treesitter::highlight(&snapshot, 0..input.byte_len(), budget, &cancelled);
                return;
            }
            Ok(ParseOutcome::Yielded { .. }) => {}
            Err(_) => return,
        }
    }
}

/// Known upstream bug (tree-sitter-swift ≤ 0.7.4): a directive name exactly
/// at end of input makes the scanner read a few bytes past its string
/// literal. The core supplies NUL as U+0001, which bounds that read; skip the
/// residual end-of-input case so campaigns look for new failures.
fn known_swift_overread(id: &str, text: &str) -> bool {
    id == "swift" && ["#if", "#else", "#elseif", "#endif"].iter().any(|end| text.ends_with(end))
}

fn boundary(text: &str, at: usize) -> usize {
    let mut at = at.min(text.len());
    while !text.is_char_boundary(at) {
        at -= 1;
    }
    at
}

fuzz_target!(|data: &[u8]| {
    let [grammar, start, removed, inserted, rest @ ..] = data else {
        return;
    };
    let text = String::from_utf8_lossy(rest).into_owned();
    let id = GRAMMARS[usize::from(*grammar) % GRAMMARS.len()];
    let Ok(package) = treesitter::package_for_language(id) else {
        return;
    };
    let Ok(mut session) = TreeSitterSession::new(package) else {
        return;
    };
    let budget = TreeSitterBudget::default();
    if known_swift_overread(id, &text) {
        return;
    }
    let Some(before) = snapshot(&text, 1) else {
        return;
    };
    parse(&mut session, before.clone(), &[], &budget);

    // Incremental reparse after one splice derived from the input itself.
    let start = boundary(&text, usize::from(*start) * text.len() / 256);
    let end = boundary(&text, start + usize::from(*removed % 64)).max(start);
    let insertion = text[..boundary(&text, usize::from(*inserted % 32))].to_owned();
    let mut changed = text.clone();
    changed.replace_range(start..end, &insertion);
    if known_swift_overread(id, &changed) {
        return;
    }
    let Some(after) = snapshot(&changed, 2) else {
        return;
    };
    let Ok(edit) = SyntaxInputEdit::new(&before, &after, start..end, start..start + insertion.len())
    else {
        return;
    };
    parse(&mut session, after, &[edit], &budget);
});
