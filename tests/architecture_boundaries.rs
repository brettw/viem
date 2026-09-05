use std::fs;
use std::path::{Path, PathBuf};

fn rust_sources_below(relative: &str) -> Vec<PathBuf> {
    fn visit(directory: &Path, result: &mut Vec<PathBuf>) {
        for entry in fs::read_dir(directory).expect("architecture source directory must exist") {
            let path = entry
                .expect("source directory entry must be readable")
                .path();
            if path.is_dir() {
                visit(&path, result);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                result.push(path);
            }
        }
    }

    let mut result = Vec::new();
    visit(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join(relative),
        &mut result,
    );
    result.sort();
    result
}

fn assert_sources_omit(relative: &str, forbidden: &[&str]) {
    for path in rust_sources_below(relative) {
        let source = fs::read_to_string(&path).expect("Rust source must be valid UTF-8");
        for needle in forbidden {
            assert!(
                !source.contains(needle),
                "{} crosses an architecture boundary with {needle:?}",
                path.display()
            );
        }
    }
}

#[test]
fn document_model_has_no_controller_or_platform_dependency() {
    assert_sources_omit(
        "src/core/document",
        &[
            "crate::command::",
            "use crate::command",
            "super::command::",
            "use appkit",
            "extern crate appkit",
            "use core_text",
            "extern crate core_text",
            "use metal",
            "extern crate metal",
        ],
    );
}

#[test]
fn command_controller_does_not_reach_private_document_storage() {
    assert_sources_omit(
        "src/core/command",
        &[
            "document::source",
            "document::range_index",
            "document::history",
            "document::projection",
            "document::formatted_text",
        ],
    );
}

#[test]
fn portable_core_has_no_native_ui_framework_dependency() {
    assert_sources_omit(
        "src/core",
        &[
            "use appkit",
            "extern crate appkit",
            "use core_text",
            "extern crate core_text",
            "use metal",
            "extern crate metal",
        ],
    );
}
