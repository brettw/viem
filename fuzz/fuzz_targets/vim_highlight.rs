//! Bundled Vim syntax programs (regex VM, regions, keywords, sync) over
//! untrusted text, then one incremental edit. Programs are compiled once per
//! language with a neutral setup context so throughput goes to matching; the
//! `syntax_highlight` target covers buffer-dependent setup.
//! Layout: [language index (u16 LE)][edit position][text...].
#![no_main]
use libfuzzer_sys::fuzz_target;
use std::collections::BTreeMap;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, OnceLock};
use viem_core::document::syntax::languages::supported_languages;
use viem_core::document::syntax::vim::{
    VimBudget, VimLoadLimits, VimProgram, VimSession, VimSetupContext,
};
use viem_core::document::syntax::{SyntaxInputIdentity, SyntaxInputSnapshot};
use viem_core::document::FormattedTextTree;

const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../assets/vim/runtime/syntax");

fn program(language: &str) -> Option<Arc<VimProgram>> {
    static PROGRAMS: OnceLock<Mutex<BTreeMap<String, Option<Arc<VimProgram>>>>> = OnceLock::new();
    let programs = PROGRAMS.get_or_init(Default::default);
    if let Some(program) = programs.lock().unwrap().get(language) {
        return program.clone();
    }
    let program = VimProgram::load_directory_with_context(
        ROOT,
        language,
        VimLoadLimits::default(),
        &VimSetupContext::default(),
        &AtomicBool::new(false),
    )
    .ok();
    programs.lock().unwrap().insert(language.to_owned(), program.clone());
    program
}

fn snapshot(text: &str, revision: u64) -> Option<SyntaxInputSnapshot> {
    Some(SyntaxInputSnapshot::new(
        SyntaxInputIdentity { document: 3, revision, generation: 1 },
        FormattedTextTree::try_from_text(text).ok()?,
    ))
}

fn finish(session: &mut VimSession, input: &SyntaxInputSnapshot) {
    let budget = VimBudget { allow_provisional: false, ..Default::default() };
    for _ in 0..2048 {
        let result = session.highlight(input, 0..input.byte_len(), budget);
        for run in &result.runs {
            assert!(run.range.start <= run.range.end && run.range.end <= input.byte_len());
        }
        if !result.stats.yielded {
            return;
        }
    }
}

fuzz_target!(|data: &[u8]| {
    let [low, high, at, rest @ ..] = data else {
        return;
    };
    let languages = supported_languages();
    let language = &languages[usize::from(u16::from_le_bytes([*low, *high])) % languages.len()];
    let Some(program) = program(&language.id) else {
        return;
    };
    let text = String::from_utf8_lossy(rest).into_owned();
    let Some(before) = snapshot(&text, 1) else {
        return;
    };
    let mut session = VimSession::new(program);
    finish(&mut session, &before);

    let mut at = usize::from(*at) * text.len() / 256;
    while !text.is_char_boundary(at) {
        at -= 1;
    }
    let inserted = "\"*/\n";
    let mut changed = text.clone();
    changed.insert_str(at, inserted);
    let Some(after) = snapshot(&changed, 2) else {
        return;
    };
    if session.apply_edit(before.identity(), after.identity(), at..at, at + inserted.len()).is_ok() {
        finish(&mut session, &after);
    }
});
