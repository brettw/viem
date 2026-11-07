//! Diff mode: the core services behind the side-by-side comparison. See
//! docs/diff_mode.md.

use viem_core::document::{
    execute_prepared_artifact_write, ArtifactOverwrite, ArtifactPath, ArtifactStorageProvider,
    ArtifactWriteIntent, ArtifactWriteScope, InMemoryArtifactStorage, LineEndingOpenPolicy,
};
use viem_core::{Document, Encoding, Format};

#[test]
fn last_saved_source_follows_loads_and_saves_but_not_edits_or_recovery() {
    let mut document = Document::from_bytes(b"one\r\ntwo\r\n".to_vec(), Encoding::Utf8, Format::Code).unwrap();
    assert_eq!(document.saved_source_bytes().unwrap(), b"one\r\ntwo\r\n");
    document.replace(0..3, "ONE").unwrap();
    assert_eq!(document.saved_source_bytes().unwrap(), b"one\r\ntwo\r\n", "an edit is not a save");
    document.mark_saved();
    assert_eq!(document.saved_source_bytes().unwrap(), b"ONE\r\ntwo\r\n");
    document.mark_recovered();
    assert_eq!(document.saved_source_bytes(), None, "recovered source has no known saved copy");
}

#[test]
fn last_saved_source_is_what_an_asynchronous_save_wrote_not_later_edits() {
    let mut storage = InMemoryArtifactStorage::new();
    let path = ArtifactPath::from("file.rs");
    storage.insert(path.clone(), b"old\n".to_vec()).unwrap();
    let mut document = Document::from_loaded_artifact(
        storage.read_artifact(&path).unwrap(),
        Encoding::Utf8,
        Format::Code,
        LineEndingOpenPolicy::default(),
    )
    .unwrap();
    document.replace(0..3, "new").unwrap();

    let failed = document
        .prepare_artifact_write(ArtifactWriteIntent::Save { overwrite: ArtifactOverwrite::ReplaceExisting })
        .unwrap();
    document.complete_artifact_write(failed.failed()).unwrap();
    assert_eq!(document.saved_source_bytes().unwrap(), b"old\n", "a failed save keeps the base");

    let alternate = document
        .prepare_artifact_write(ArtifactWriteIntent::WriteAlternate {
            destination: ArtifactPath::from("copy.rs"),
            scope: ArtifactWriteScope::WholeArtifact,
            overwrite: ArtifactOverwrite::RefuseExisting,
        })
        .unwrap();
    let completion = execute_prepared_artifact_write(&mut storage, &alternate).unwrap();
    document.complete_artifact_write(alternate.succeeded(completion)).unwrap();
    assert_eq!(document.saved_source_bytes().unwrap(), b"old\n", "writing a copy is not a save");

    let save = document
        .prepare_artifact_write(ArtifactWriteIntent::Save { overwrite: ArtifactOverwrite::ReplaceExisting })
        .unwrap();
    document.replace(0..3, "later").unwrap();
    let completion = execute_prepared_artifact_write(&mut storage, &save).unwrap();
    document.complete_artifact_write(save.succeeded(completion)).unwrap();
    assert_eq!(document.saved_source_bytes().unwrap(), b"new\n");
    assert_eq!(document.source_bytes(), b"later\n");
}
