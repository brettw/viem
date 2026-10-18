//! Document-leading YAML metadata keeps its literal source body in Preview.
//! Recognition depends on the envelope rather than valid YAML, so an unfinished
//! value can be edited without changing the surrounding Markdown interpretation.
use std::ops::Range;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct FrontMatter {
    /// Includes both delimiter lines and the closing line's ending, if present.
    pub range: Range<usize>,
    /// Includes the final body ending, which separates the body from the closer.
    pub body: Range<usize>,
}

/// `text` uses normalized grammar endings. Callers must establish that it starts
/// at the document's content start, rather than at an arbitrary regional slice.
pub(super) fn recognize(text: &str) -> Option<FrontMatter> {
    let opening_end = text.find('\n')?;
    if !is_delimiter(&text[..opening_end]) { return None; }
    let body_start = opening_end + 1;
    let mut start = body_start;
    for line in text[body_start..].split_inclusive('\n') {
        let content = line.strip_suffix('\n').unwrap_or(line);
        if is_delimiter(content) {
            return Some(FrontMatter { range: 0..start + line.len(), body: body_start..start });
        }
        start += line.len();
    }
    None
}

pub(super) fn is_delimiter(line: &str) -> bool {
    line.trim_end_matches([' ', '\t']) == "---"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn front_matter_recognition_keeps_invalid_and_empty_yaml_editable() {
        let source = "--- \t\nvalue: [unfinished\n---\t\n# Body";
        let found = recognize(source).unwrap();
        assert_eq!(&source[found.body.clone()], "value: [unfinished\n");
        assert_eq!(&source[found.range], "--- \t\nvalue: [unfinished\n---\t\n");
        assert_eq!(recognize("---\n---").unwrap().body, 4..4);
        assert_eq!(recognize("---\n\n---").unwrap().body, 4..5);
    }

    #[test]
    fn front_matter_recognition_requires_a_closed_document_leading_envelope() {
        for source in ["---", "---\nvalue: x", "\n---\nvalue: x\n---", " ---\nvalue: x\n---",
            "----\nvalue: x\n---", "---\nvalue: x\n ---", "---\nvalue: x\n--- extra"] {
            assert!(recognize(source).is_none(), "{source:?}");
        }
    }
}
