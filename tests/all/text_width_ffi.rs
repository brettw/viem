use viem_core::ffi::*;

struct Core(ViemCoreHandle);
impl Core {
    fn new() -> Self {
        let mut handle = 0;
        let mut revision = 0;
        let source = b"alpha beta\ngamma\n";
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
}
impl Drop for Core {
    fn drop(&mut self) {
        assert_eq!(viem_core_destroy(self.0), ViemStatus::Ok);
    }
}

#[test]
fn text_width_default_requires_a_positive_width_and_a_live_handle() {
    let core = Core::new();
    assert_eq!(
        viem_core_set_text_width_default(core.0, 0),
        ViemStatus::InvalidArgument
    );
    assert_eq!(viem_core_set_text_width_default(core.0, 72), ViemStatus::Ok);
    assert_eq!(
        viem_core_set_text_width_default(core.0, u32::MAX),
        ViemStatus::Ok
    );
    assert_ne!(viem_core_set_text_width_default(0, 72), ViemStatus::Ok);
    assert_eq!(VIEM_EX_OPTION_TEXTWIDTH, 8);
    assert_eq!(VIEM_EX_OPTION_VALUE_NUMBER, 4);
}
