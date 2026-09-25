use std::ptr;
use viem_core::ffi::*;

struct Core(ViemCoreHandle);
impl Core {
    fn new() -> Self {
        let mut handle = 0;
        let mut revision = 0;
        let source = b"print(\"literal\")\r\n";
        assert_eq!(
            unsafe {
                viem_core_create(
                    source.as_ptr(),
                    source.len() as u64,
                    &ViemDocumentOptions::default(),
                    &mut handle,
                    &mut revision,
                )
            },
            ViemStatus::Ok
        );
        Self(handle)
    }
    fn state(&self) -> ViemDocumentStateV1 {
        let mut state = ViemDocumentStateV1::default();
        assert_eq!(
            unsafe { viem_core_document_state(self.0, &mut state) },
            ViemStatus::Ok
        );
        state
    }
}
impl Drop for Core {
    fn drop(&mut self) {
        assert_eq!(viem_core_destroy(self.0), ViemStatus::Ok);
    }
}

#[test]
fn syntax_style_name_export_is_read_only_and_checks_output_storage() {
    let core = Core::new();
    let before = core.state();
    let mut required = 0;
    unsafe {
        assert_eq!(
            viem_core_copy_syntax_style_names(core.0, ptr::null_mut(), 0, &mut required),
            ViemStatus::BufferTooSmall
        );
        assert_eq!(required, 2);
        let mut bytes = [0u8; 2];
        assert_eq!(
            viem_core_copy_syntax_style_names(core.0, bytes.as_mut_ptr(), 2, &mut required),
            ViemStatus::Ok
        );
        assert_eq!(&bytes, b"[]");
        assert_eq!(
            viem_core_copy_syntax_style_names(core.0, ptr::null_mut(), 0, ptr::null_mut()),
            ViemStatus::NullPointer
        );
        let mut aliased = 99u64;
        let alias = &mut aliased as *mut u64;
        assert_eq!(
            viem_core_copy_syntax_style_names(core.0, alias.cast(), 8, alias),
            ViemStatus::InvalidArgument
        );
        assert_eq!(aliased, 99);
        assert_eq!(
            viem_core_copy_syntax_style_names(0, ptr::null_mut(), 0, &mut required),
            ViemStatus::InvalidHandle
        );
    }
    assert_eq!(core.state(), before);
}

#[test]
fn code_detection_abi_preserves_source_and_clean_history() {
    let core = Core::new();
    let before = core.state();
    let table = br#"[{"pattern":"*.custom","language":"python"}]"#;
    let filename = b"example.custom";
    unsafe {
        assert_eq!(
            viem_core_set_code_filename_associations_json(
                core.0,
                table.as_ptr(),
                table.len() as u64
            ),
            ViemStatus::Ok
        );
        assert_eq!(
            viem_core_initialize_code_detection(
                core.0,
                filename.as_ptr(),
                filename.len() as u64,
                1
            ),
            ViemStatus::Ok
        );
        // Unavailable packages and an explicit None remain ordinary editable Code.
        assert_eq!(
            viem_core_configure_syntax(core.0, b"/missing".as_ptr(), 8),
            ViemStatus::Ok
        );
        assert_eq!(
            viem_core_set_code_language(core.0, 2, b"unavailable".as_ptr(), 11),
            ViemStatus::Ok
        );
        assert_eq!(
            viem_core_redetect_code_language(core.0, b"renamed.rs".as_ptr(), 10),
            ViemStatus::Ok
        );
        assert_eq!(
            viem_core_set_code_language(core.0, 1, ptr::null(), 0),
            ViemStatus::Ok
        );
        let mut changed = 0;
        assert_eq!(viem_core_poll_syntax(core.0, &mut changed), ViemStatus::Ok);
        let mut required = 0;
        let mut source = [0; 19];
        assert_eq!(
            viem_core_copy_source_bytes(
                core.0,
                before.document_revision,
                source.as_mut_ptr(),
                source.len() as u64,
                &mut required
            ),
            ViemStatus::Ok
        );
        assert_eq!(&source[..required as usize], b"print(\"literal\")\r\n");
    }
    let after = core.state();
    assert_eq!(after.format, VIEM_FORMAT_CODE);
    assert_eq!(after.document_revision, before.document_revision);
    assert_eq!(
        after.flags & (VIEM_DOCUMENT_STATE_IS_DIRTY | VIEM_DOCUMENT_STATE_CAN_UNDO),
        0
    );
}

#[test]
fn code_configuration_abi_rejects_invalid_and_oversized_data_atomically() {
    let core = Core::new();
    let original = core.state();
    unsafe {
        assert_eq!(
            viem_core_set_code_filename_associations_json(core.0, ptr::null(), 128 * 1024 + 1),
            ViemStatus::InvalidArgument
        );
        assert_eq!(
            viem_core_configure_syntax(core.0, ptr::null(), 16 * 1024 + 1),
            ViemStatus::InvalidArgument
        );
        assert_eq!(
            viem_core_initialize_code_detection(core.0, ptr::null(), 0, 2),
            ViemStatus::InvalidArgument
        );
        assert_eq!(
            viem_core_configure_syntax(core.0, b"a\0b".as_ptr(), 3),
            ViemStatus::InvalidArgument
        );
        assert_eq!(
            viem_core_set_code_language(core.0, 0, b"rust".as_ptr(), 4),
            ViemStatus::InvalidArgument
        );
        assert_eq!(
            viem_core_set_code_language(core.0, 2, b"../rust".as_ptr(), 7),
            ViemStatus::InvalidArgument
        );
        assert_eq!(
            viem_core_set_code_language(core.0, 2, [255].as_ptr(), 1),
            ViemStatus::InvalidUtf8
        );
        assert_eq!(
            viem_core_set_code_language(core.0, 3, ptr::null(), 0),
            ViemStatus::InvalidArgument
        );
        assert_eq!(
            viem_core_redetect_code_language(core.0, b"x\0.py".as_ptr(), 5),
            ViemStatus::InvalidArgument
        );
        assert_eq!(
            viem_core_poll_syntax(core.0, ptr::null_mut()),
            ViemStatus::NullPointer
        );
        for json in [
            r#"{}"#,
            r#"[{"pattern":"","language":"rust"}]"#,
            r#"[{"pattern":"*","language":"../../x"}]"#,
        ] {
            assert_eq!(
                viem_core_set_code_filename_associations_json(
                    core.0,
                    json.as_ptr(),
                    json.len() as u64
                ),
                ViemStatus::InvalidArgument
            );
        }
        assert_eq!(
            viem_core_set_code_filename_associations_json(core.0, ptr::null(), 0),
            ViemStatus::Ok
        );
    }
    assert_eq!(core.state(), original);
}
