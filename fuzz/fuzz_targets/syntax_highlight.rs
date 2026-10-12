//! The worker-side syntax provider (Tree-sitter primary, injections, bundled
//! Vim syntax fallback/compiler and buffer-dependent setup) over untrusted
//! text, followed by one edited revision.
//! Layout: [language selector][filename selector][edit position][text...].
#![no_main]
use libfuzzer_sys::fuzz_target;
use std::sync::atomic::AtomicBool;
use viem_core::document::syntax::languages::supported_languages;
use viem_core::document::syntax::service::{
    BackendProvider, SyntaxConfiguration, SyntaxProvider, SyntaxRequest,
};
use viem_core::document::syntax::{treesitter, SyntaxInputIdentity, SyntaxInputSnapshot};
use viem_core::document::FormattedTextTree;

/// Tree-sitter primaries (Markdown and C-family injections reach other
/// providers). `vim_highlight` covers Vim programs with cached compilation.
const PRIMARY: &[&str] = &[
    "c", "cpp", "rust", "swift", "objc", "c_sharp", "javascript", "typescript", "tsx", "python",
    "json", "markdown", "html", "sh", "zsh", "gitcommit",
];
const FILENAMES: &[Option<&str>] = &[None, Some("file.txt"), Some(".zshrc"), Some("Makefile")];

fn request(text: &str, revision: u64, language: &str, filename: Option<&str>) -> Option<SyntaxRequest> {
    Some(SyntaxRequest {
        input: SyntaxInputSnapshot::new(
            SyntaxInputIdentity { document: 7, revision, generation: 1 },
            FormattedTextTree::try_from_text(text).ok()?,
        ),
        configuration: SyntaxConfiguration {
            generation: 1,
            registry_generation: treesitter::package_registry_generation(),
            language: Some(language.into()),
            vim_directory: concat!(env!("CARGO_MANIFEST_DIR"), "/../assets/vim/runtime/syntax").into(),
            filename: filename.map(str::to_owned),
        },
        range: 0..text.len(),
    })
}

fn finish(provider: &mut BackendProvider, request: &SyntaxRequest) {
    let cancelled = AtomicBool::new(false);
    for _ in 0..4200 {
        let result = provider.analyze(request, &cancelled);
        for run in &result.runs {
            assert!(run.range.start <= run.range.end && run.range.end <= request.input.byte_len());
        }
        if !result.continuation {
            return;
        }
    }
    panic!("syntax provider did not finish within its continuation budget");
}

fuzz_target!(|data: &[u8]| {
    let [language, filename, at, rest @ ..] = data else {
        return;
    };
    let languages = supported_languages();
    let language = if language & 7 != 7 {
        PRIMARY[usize::from(language >> 3) % PRIMARY.len()].to_owned()
    } else {
        let index = (usize::from(language >> 3) * 7 + usize::from(*filename)) % languages.len();
        languages[index].id.clone()
    };
    let filename = FILENAMES[usize::from(filename & 3)];
    let text = String::from_utf8_lossy(rest).into_owned();
    let mut provider = BackendProvider::default();
    let Some(first) = request(&text, 1, &language, filename) else {
        return;
    };
    finish(&mut provider, &first);

    let mut at = usize::from(*at) * text.len() / 256;
    while !text.is_char_boundary(at) {
        at -= 1;
    }
    let mut changed = text.clone();
    changed.insert_str(at, "\"*/\n");
    if let Some(second) = request(&changed, 2, &language, filename) {
        finish(&mut provider, &second);
    }
});
