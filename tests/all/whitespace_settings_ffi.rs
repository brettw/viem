use viem_core::ffi::*;

#[test]
fn settings_json_is_validated_before_any_buffer_changes() {
    let mut handle = 0;
    let mut revision = 0;
    let source = b"\tword  \n";
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
    for json in [
        r#"{"tabstop":0}"#,
        r#"{"softtabstop":-2}"#,
        r#"{"shiftwidth":1025}"#,
        r#"{"expandtab":null}"#,
    ] {
        assert_eq!(
            unsafe { viem_core_set_indentation_defaults(handle, json.as_ptr(), json.len() as u64) },
            ViemStatus::InvalidArgument
        );
    }
    for json in [
        r#"{"visibleWhitespace":{"listchars":"tab:x"}}"#,
        r#"{"visibleWhitespace":{"listchars":"trail:界"}}"#,
        r#"{"visibleWhitespace":{"style":{"size":-1}}}"#,
        r#"{"codeWhitespace":null}"#,
        r#"{"codeWrappedLineIndent":-1}"#,
        r#"{"codeWrappedLineIndent":1025}"#,
        r#"{"codeWrappedLineIndent":null}"#,
        r#"{"codeWrappedLineIndent":true}"#,
        r#"{"codeWrappedLineIndent":1.5}"#,
        r#"{"codeWrappedLineIndent":"4"}"#,
    ] {
        assert_eq!(
            unsafe { viem_validate_whitespace_presentation(json.as_ptr(), json.len() as u64) },
            ViemStatus::InvalidArgument
        );
        assert_eq!(
            unsafe {
                viem_core_set_whitespace_presentation_defaults(
                    handle,
                    json.as_ptr(),
                    json.len() as u64,
                )
            },
            ViemStatus::InvalidArgument
        );
    }
    for json in [
        "{}",
        r#"{"codeWrappedLineIndent":0}"#,
        r#"{"codeWrappedLineIndent":1024}"#,
        r#"{"visibleWhitespace":{"listchars":"tab:>-,trail:*","style":{"foreground":{"red":0,"green":0,"blue":0.5,"alpha":1}}}}"#,
    ] {
        assert_eq!(
            unsafe { viem_validate_whitespace_presentation(json.as_ptr(), json.len() as u64) },
            ViemStatus::Ok
        );
        assert_eq!(
            unsafe {
                viem_core_set_whitespace_presentation_defaults(
                    handle,
                    json.as_ptr(),
                    json.len() as u64,
                )
            },
            ViemStatus::Ok
        );
    }
    let json = br#"{"tabstop":8,"shiftwidth":0,"softtabstop":-1,"expandtab":false}"#;
    assert_eq!(
        unsafe { viem_core_set_indentation_defaults(handle, json.as_ptr(), json.len() as u64) },
        ViemStatus::Ok
    );
    assert_eq!(
        unsafe { viem_core_set_indentation_defaults(handle, std::ptr::null(), 1) },
        ViemStatus::NullPointer
    );
    assert_ne!(
        unsafe { viem_core_set_indentation_defaults(0, b"{}".as_ptr(), 2) },
        ViemStatus::Ok
    );
    assert_eq!(viem_core_destroy(handle), ViemStatus::Ok);
}
