use viem_core::document::{FormattedPayloadEdit, FormattedPayloadEditRequest};
use viem_core::{Document, Encoding, Format};

#[test]
fn equal_payload_replacement_preserves_recovered_objects_and_original_source() {
    for source in [
        "<p>A</p><table><b>B</b><tr><td>keep</td></tr></table><p>C</p>",
        "<table><b>foster</b><tr><td>keep</td></tr></table>",
        "<p>&fjlig;&amp;</p>",
    ] {
        let mut document =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Html).unwrap();
        let revision = document.revision();
        let range = document
            .projection()
            .blocks()
            .iter()
            .find(|block| document.text()[block.range.clone()].contains('\u{fffc}'))
            .unwrap_or(&document.projection().blocks()[0])
            .range
            .clone();
        let payload = document.capture_formatted_payload(range.clone()).unwrap();
        let request = FormattedPayloadEditRequest::new(
            document.id(),
            revision,
            vec![FormattedPayloadEdit::new(range, payload)],
        );
        document.apply_formatted_payload_request(request).unwrap();
        assert_eq!(document.source_bytes(), source.as_bytes());
        assert_eq!(document.revision(), revision);
        assert!(!document.undo());
    }
}
