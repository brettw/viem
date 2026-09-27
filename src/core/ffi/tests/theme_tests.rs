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
