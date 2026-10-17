use super::*;
use crate::ffi::*;

#[test]
fn portable_theme_ffi_validates_names_json_and_two_pass_output() {
    unsafe {
        let mut required = 0;
        assert_eq!(
            viem_theme_default_json(0, ptr::null_mut(), 0, &mut required),
            ViemStatus::BufferTooSmall
        );
        assert!(required > 0);
        let mut bytes = vec![0; required as usize];
        assert_eq!(
            viem_theme_default_json(0, bytes.as_mut_ptr(), required, &mut required),
            ViemStatus::Ok
        );
        assert_eq!(
            viem_theme_validate_json(bytes.as_ptr(), required),
            ViemStatus::Ok
        );
        assert_eq!(
            viem_theme_default_json(2, bytes.as_mut_ptr(), required, &mut required),
            ViemStatus::InvalidArgument
        );
        assert_eq!(
            viem_theme_default_json(0, ptr::null_mut(), 0, ptr::null_mut()),
            ViemStatus::NullPointer
        );
        let mut overlap = [0u64; 2];
        assert_eq!(
            viem_theme_default_json(0, overlap.as_mut_ptr().cast(), 8, overlap.as_mut_ptr()),
            ViemStatus::InvalidArgument
        );
        for (name, expected) in [
            ("Midnight", ViemStatus::Ok),
            ("Default", ViemStatus::InvalidArgument),
            ("../Name", ViemStatus::InvalidArgument),
        ] {
            assert_eq!(
                viem_theme_validate_name(name.as_ptr(), name.len() as u64),
                expected
            );
        }
        assert_eq!(
            viem_theme_validate_name([0xff].as_ptr(), 1),
            ViemStatus::InvalidUtf8
        );
        assert_eq!(
            viem_theme_validate_json(ptr::null(), 20 * 1024 * 1024 + 1),
            ViemStatus::InvalidArgument
        );
    }
}

#[test]
fn live_defaults_ffi_requires_current_revision_and_keeps_edited_source() {
    let mut storage = PaintTestProviderStorage::default();
    let mut document = Document::new("text");
    document.insert(0, "edited ").unwrap();
    let revision = document.revision().0;
    let original = summarize_document_state(&document);
    let mut core = Core::new(document);
    core.add_view(paint_test_provider(&mut storage, 501, 601), 240.0, 480.0);
    let handle = register_core(core).unwrap();
    let json = br#"{"version":1}"#;
    unsafe {
        assert_eq!(
            viem_core_replace_style_defaults(
                handle,
                revision + 1,
                json.as_ptr(),
                json.len() as u64,
                None,
                ptr::null_mut()
            ),
            ViemStatus::StaleRevision
        );
        assert_eq!(
            viem_core_replace_style_defaults(
                handle,
                revision,
                json.as_ptr(),
                json.len() as u64,
                None,
                ptr::null_mut()
            ),
            ViemStatus::Ok
        );
    }
    let lease = checkout_core(handle).unwrap();
    let after = summarize_document_state(lease.core().document());
    assert!(after.style_sheet_revision > original.style_sheet_revision);
    assert_eq!(
        after,
        ViemDocumentStateV1 {
            style_sheet_revision: after.style_sheet_revision,
            ..original
        }
    );
    drop(lease);
    assert_eq!(viem_core_destroy(handle), ViemStatus::Ok);
}

#[test]
fn vim_import_ffi_uses_two_pass_output_and_reports_diagnostics_once() {
    unsafe extern "C" fn collect(context: *mut c_void, message: *const u8, length: u64) {
        let messages = unsafe { &mut *context.cast::<Vec<String>>() };
        let bytes = unsafe { std::slice::from_raw_parts(message, length as usize) };
        messages.push(String::from_utf8_lossy(bytes).into_owned());
    }
    let source = b"hi Normal guifg=#eeeeee guibg=#111111\naugroup Skipped\n";
    let mut messages = Vec::<String>::new();
    let context = (&mut messages as *mut Vec<String>).cast::<c_void>();
    unsafe {
        let mut required = 0;
        assert_eq!(
            viem_theme_import_vim(source.as_ptr(), source.len() as u64, ptr::null_mut(), 0, &mut required, Some(collect), context),
            ViemStatus::BufferTooSmall
        );
        // The first pass reports nothing; skipped statements arrive with output.
        assert!(messages.is_empty());
        let mut bytes = vec![0; required as usize];
        assert_eq!(
            viem_theme_import_vim(source.as_ptr(), source.len() as u64, bytes.as_mut_ptr(), required, &mut required, Some(collect), context),
            ViemStatus::Ok
        );
        assert_eq!(viem_theme_validate_json(bytes.as_ptr(), required), ViemStatus::Ok);
        assert_eq!(messages.len(), 1);
        assert!(messages[0].starts_with("line 2:"), "{messages:?}");
        messages.clear();
        let text = b"just prose\n";
        assert_eq!(
            viem_theme_import_vim(text.as_ptr(), text.len() as u64, ptr::null_mut(), 0, &mut required, Some(collect), context),
            ViemStatus::InvalidArgument
        );
        assert_eq!(messages, ["The file contains no Vim highlight commands"]);
    }
}

#[test]
fn unique_theme_name_ffi_numbers_case_insensitive_collisions() {
    let stem = "Desert";
    let existing = "Paper\ndesert\nDESERT (2)\n";
    unsafe {
        let mut required = 0;
        let mut output = [0u8; 64];
        assert_eq!(
            viem_theme_unique_name(stem.as_ptr(), stem.len() as u64, existing.as_ptr(), existing.len() as u64,
                output.as_mut_ptr(), output.len() as u64, &mut required),
            ViemStatus::Ok
        );
        assert_eq!(&output[..required as usize], b"Desert (3)");
        assert_eq!(
            viem_theme_unique_name([0xff].as_ptr(), 1, existing.as_ptr(), 0, output.as_mut_ptr(), 64, &mut required),
            ViemStatus::InvalidUtf8
        );
    }
}
