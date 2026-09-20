use std::ptr;
use viem_core::ffi::*;

#[test]
fn selection_option_abi_validates_utf8_values_and_output_regions() {
    let mut core = 0;
    let mut revision = 0;
    assert_eq!(
        unsafe {
            viem_core_create(
                b"text".as_ptr(),
                4,
                &ViemDocumentOptions::default(),
                &mut core,
                &mut revision,
            )
        },
        ViemStatus::Ok
    );
    let mut required = 999;
    assert_eq!(
        unsafe {
            viem_core_copy_selection_option(
                core,
                VIEM_EX_OPTION_AUTOSELECT,
                ptr::null_mut(),
                0,
                &mut required,
            )
        },
        ViemStatus::BufferTooSmall
    );
    assert_eq!(required, 1);
    let mut buffer = vec![0; 16];
    assert_eq!(
        unsafe {
            viem_core_copy_selection_option(
                core,
                VIEM_EX_OPTION_AUTOSELECT,
                buffer.as_mut_ptr(),
                buffer.len() as u64,
                &mut required,
            )
        },
        ViemStatus::Ok
    );
    assert_eq!(&buffer[..required as usize], b"1");
    for invalid in [b"false".as_slice(), b"", b"2", &[0xff]] {
        assert_ne!(
            unsafe {
                viem_core_set_selection_option(
                    core,
                    VIEM_EX_OPTION_AUTOSELECT,
                    invalid.as_ptr(),
                    invalid.len() as u64,
                )
            },
            ViemStatus::Ok
        );
    }
    assert_eq!(
        unsafe {
            viem_core_set_selection_option(core, VIEM_EX_OPTION_AUTOSELECT, b"0".as_ptr(), 1)
        },
        ViemStatus::Ok
    );
    assert_eq!(
        unsafe {
            viem_core_copy_selection_option(
                core,
                VIEM_EX_OPTION_AUTOSELECT,
                buffer.as_mut_ptr(),
                buffer.len() as u64,
                &mut required,
            )
        },
        ViemStatus::Ok
    );
    assert_eq!(&buffer[..required as usize], b"0");
    for bad in [b"key,invalid".as_slice(), b"mouse,,key", &[0xff]] {
        assert_ne!(
            unsafe {
                viem_core_set_selection_option(
                    core,
                    VIEM_EX_OPTION_SELECTMODE,
                    bad.as_ptr(),
                    bad.len() as u64,
                )
            },
            ViemStatus::Ok
        );
    }
    assert_eq!(
        unsafe {
            viem_core_set_selection_option(core, VIEM_EX_OPTION_SELECTMODE, b"cmd".as_ptr(), 3)
        },
        ViemStatus::Ok
    );
    buffer.fill(0);
    assert_eq!(
        unsafe {
            viem_core_copy_selection_option(
                core,
                VIEM_EX_OPTION_SELECTMODE,
                buffer.as_mut_ptr(),
                buffer.len() as u64,
                &mut required,
            )
        },
        ViemStatus::Ok
    );
    assert_eq!(&buffer[..required as usize], b"cmd");
    assert_eq!(
        unsafe { viem_core_set_selection_option(core, VIEM_EX_OPTION_KEYMODEL, ptr::null(), 0) },
        ViemStatus::Ok
    );
    assert_eq!(
        unsafe {
            viem_core_copy_selection_option(
                core,
                VIEM_EX_OPTION_KEYMODEL,
                ptr::null_mut(),
                0,
                &mut required,
            )
        },
        ViemStatus::Ok
    );
    assert_eq!(required, 0);
    assert_eq!(
        unsafe { viem_core_copy_selection_option(core, 9999, ptr::null_mut(), 0, &mut required) },
        ViemStatus::InvalidArgument
    );
    assert_eq!(
        unsafe {
            viem_core_copy_selection_option(
                core,
                VIEM_EX_OPTION_SELECTMODE,
                ptr::null_mut(),
                0,
                ptr::null_mut(),
            )
        },
        ViemStatus::NullPointer
    );
    let mut storage = [0_u64; 4];
    assert_eq!(
        unsafe {
            viem_core_copy_selection_option(
                core,
                VIEM_EX_OPTION_SELECTMODE,
                storage.as_mut_ptr().cast(),
                8,
                storage.as_mut_ptr(),
            )
        },
        ViemStatus::InvalidArgument
    );
    assert_eq!(viem_core_destroy(core), ViemStatus::Ok);
}
