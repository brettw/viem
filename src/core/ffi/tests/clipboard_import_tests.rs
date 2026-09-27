use crate::ffi::*;
use std::ptr;

#[test]
fn clipboard_import_is_passive_styled_and_uses_dedicated_format_tags() {
    for (format, source) in [
        (
            VIEM_CLIPBOARD_FORMAT_HTML,
            b"<style>b{color:red}</style><script>bad()</script><p><b>A&amp;B</b></p>".as_slice(),
        ),
        (VIEM_CLIPBOARD_FORMAT_RTF, br"{\rtf1{\b A&B}}".as_slice()),
    ] {
        let mut required = 0;
        unsafe {
            assert_eq!(
                viem_import_clipboard_json(
                    format,
                    source.as_ptr(),
                    source.len() as u64,
                    ptr::null_mut(),
                    0,
                    &mut required
                ),
                ViemStatus::BufferTooSmall
            );
            let mut output = vec![0; required as usize];
            assert_eq!(
                viem_import_clipboard_json(
                    format,
                    source.as_ptr(),
                    source.len() as u64,
                    output.as_mut_ptr(),
                    required,
                    &mut required
                ),
                ViemStatus::Ok
            );
            let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
            assert_eq!(json["plain_text"], "A&B");
            assert_eq!(json["is_rich"], true);
            assert!(json["character_runs"]
                .as_array()
                .unwrap()
                .iter()
                .any(|run| run["bold"] == true));
            if format == VIEM_CLIPBOARD_FORMAT_HTML {
                assert_eq!(json["source_format"], VIEM_FORMAT_PLAIN_TEXT);
                assert_eq!(json["source_text"], "A&B");
            }
        }
    }
}

#[test]
fn clipboard_import_validates_aliasing_and_retired_document_formats_are_invalid() {
    let source = b"<p>x</p>";
    let mut required = 99;
    unsafe {
        assert_eq!(
            viem_import_clipboard_json(
                99,
                source.as_ptr(),
                source.len() as u64,
                ptr::null_mut(),
                0,
                &mut required
            ),
            ViemStatus::InvalidArgument
        );
        assert_eq!(required, 0);
        assert_eq!(
            viem_import_clipboard_json(
                VIEM_CLIPBOARD_FORMAT_HTML,
                ptr::null(),
                1,
                ptr::null_mut(),
                0,
                &mut required
            ),
            ViemStatus::NullPointer
        );
        assert_eq!(
            viem_import_clipboard_json(
                VIEM_CLIPBOARD_FORMAT_HTML,
                source.as_ptr(),
                source.len() as u64,
                (&mut required as *mut u64).cast(),
                8,
                &mut required
            ),
            ViemStatus::InvalidArgument
        );
        assert_eq!(
            viem_import_clipboard_json(
                VIEM_CLIPBOARD_FORMAT_HTML,
                source.as_ptr(),
                source.len() as u64,
                source.as_ptr().cast_mut(),
                source.len() as u64,
                &mut required
            ),
            ViemStatus::InvalidArgument
        );
        for format in [3, 6] {
            let options = ViemDocumentOptions {
                format,
                ..Default::default()
            };
            let mut core = 0;
            let mut revision = 0;
            assert_eq!(
                viem_core_create(
                    source.as_ptr(),
                    source.len() as u64,
                    &options,
                    &mut core,
                    &mut revision
                ),
                ViemStatus::InvalidFormat
            );
            assert_eq!(core, 0);
        }
    }
}
