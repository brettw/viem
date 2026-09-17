//! Read-only confirmation prompt. Decisions use the ordinary serialized key API.
use super::*;

/// Copy the current UTF-8 prompt; an empty result means no confirmation is active.
/// # Safety
/// Output and length regions must be writable and disjoint. Null/zero queries
/// return BufferTooSmall when a nonempty prompt is present.
#[no_mangle]
pub unsafe extern "C" fn viem_core_view_copy_substitute_confirmation(
    handle: ViemCoreHandle,
    view: ViemViewId,
    output: *mut u8,
    capacity: u64,
    out_length: *mut u64,
) -> ViemStatus {
    ffi_boundary(|| {
        validate_disjoint_regions(&[
            typed_pointer_region(output, capacity)?,
            typed_pointer_region(out_length, 1)?,
        ])?;
        unsafe {
            out_length.write(0);
        }
        let prompt = with_core(handle, |core| {
            core.command_state(ViewId(view))
                .ok_or(ViemStatus::InvalidView)
                .map(|state| state.substitute_confirmation_prompt().unwrap_or_default())
        })?;
        unsafe {
            out_length.write(prompt.len() as u64);
        }
        if capacity < prompt.len() as u64 {
            return Err(ViemStatus::BufferTooSmall);
        }
        unsafe {
            copy_output(prompt.as_bytes(), output);
        }
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn prompt_export_validates_and_clears_outputs() {
        let mut length = 99;
        assert_eq!(
            unsafe {
                viem_core_view_copy_substitute_confirmation(
                    0,
                    0,
                    std::ptr::null_mut(),
                    0,
                    &mut length,
                )
            },
            ViemStatus::InvalidHandle
        );
        assert_eq!(length, 0);
        assert_eq!(
            unsafe {
                viem_core_view_copy_substitute_confirmation(
                    0,
                    0,
                    std::ptr::null_mut(),
                    0,
                    std::ptr::null_mut(),
                )
            },
            ViemStatus::NullPointer
        );
    }
}
