//! Automatic Code language detection from an untrusted filename and content
//! (modelines, shebangs, content signatures), through the load-time path.
//! Layout: [filename length][filename bytes][document bytes].
#![no_main]
use libfuzzer_sys::fuzz_target;
use viem_core::document::syntax::detection::{self, LanguageSelection};
use viem_core::document::syntax::{SyntaxInputIdentity, SyntaxInputSnapshot};
use viem_core::document::{Document, Format, FormattedTextTree};
use viem_core::layout::MockTextMeasurementProvider;
use viem_core::Core;

fuzz_target!(|data: &[u8]| {
    let Some((&length, rest)) = data.split_first() else {
        return;
    };
    let (name, source) = rest.split_at(usize::from(length).min(rest.len()));
    let filename = String::from_utf8_lossy(name);
    let Ok(document) = Document::from_bytes_detect_encoding(source.to_vec(), Format::PlainText) else {
        return;
    };
    let Ok(text) = FormattedTextTree::try_from_text(document.text()) else {
        return;
    };
    let mut core = Core::<MockTextMeasurementProvider>::new(document);
    let _ = core.initialize_code_detection(&filename, true);
    assert_eq!(core.document().source_bytes(), source, "detection must be read-only");
    let input = SyntaxInputSnapshot::new(
        SyntaxInputIdentity { document: 1, revision: 1, generation: 1 },
        text,
    );
    let detected = detection::detect(&input, &filename, &LanguageSelection::Automatic, &[]);
    if let Some(language) = detected.language {
        assert!(!language.is_empty());
    }
});
