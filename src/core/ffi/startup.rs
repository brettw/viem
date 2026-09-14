//! Stateless, batched launch-argument parser for frontend composition.
use super::*;
use crate::command::startup::parse_launch_arguments;

/// JSON argv input; JSON `{arguments, error}` output. Parse errors are successful
/// ABI calls with null arguments. Null/zero count queries return BufferTooSmall.
///
/// # Safety
/// Nonempty pointer regions must be valid, aligned, and mutually disjoint.
#[no_mangle]
pub unsafe extern "C" fn viem_parse_launch_arguments(
    input: *const u8,
    input_length: u64,
    output: *mut u8,
    capacity: u64,
    required: *mut u64,
) -> ViemStatus {
    ffi_boundary(|| {
        validate_disjoint_regions(&[
            typed_pointer_region(input, input_length)?,
            typed_pointer_region(output, capacity)?,
            typed_pointer_region(required, 1)?,
        ])?;
        unsafe { required.write(0) };
        if input_length > 16 * 1024 * 1024 {
            return Err(ViemStatus::InvalidArgument);
        }
        let arguments: Vec<String> =
            serde_json::from_slice(unsafe { input_bytes(input, input_length)? })
                .map_err(|_| ViemStatus::InvalidArgument)?;
        let value = match parse_launch_arguments(arguments) {
            Ok(arguments) => serde_json::json!({ "arguments": arguments, "error": null }),
            Err(error) => serde_json::json!({ "arguments": null, "error": error.to_string() }),
        };
        let bytes = serde_json::to_vec(&value).map_err(|_| ViemStatus::CoreFailure)?;
        unsafe { required.write(bytes.len() as u64) };
        if capacity < bytes.len() as u64 {
            return Err(ViemStatus::BufferTooSmall);
        }
        unsafe { copy_output(&bytes, output) };
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parser_reports_success_errors_and_required_storage() {
        for (input, error) in [
            (br#"["-o","+123","one","two"]"#.as_slice(), false),
            (br#"["-unknown"]"#.as_slice(), true),
        ] {
            let mut length = 0;
            assert_eq!(
                unsafe {
                    viem_parse_launch_arguments(
                        input.as_ptr(),
                        input.len() as u64,
                        std::ptr::null_mut(),
                        0,
                        &mut length,
                    )
                },
                ViemStatus::BufferTooSmall
            );
            let mut output = vec![0u8; length as usize];
            assert_eq!(
                unsafe {
                    viem_parse_launch_arguments(
                        input.as_ptr(),
                        input.len() as u64,
                        output.as_mut_ptr(),
                        length,
                        &mut length,
                    )
                },
                ViemStatus::Ok
            );
            let result: serde_json::Value = serde_json::from_slice(&output).unwrap();
            assert_eq!(result["error"].is_string(), error);
            if !error {
                assert_eq!(
                    result["arguments"]["filenames"],
                    serde_json::json!(["one", "two"])
                );
                assert_eq!(result["arguments"]["initialLine"], 123);
            }
        }
    }
    #[test]
    fn malformed_json_and_overlapping_buffers_are_rejected() {
        let input = b"{}";
        let mut length = 0;
        assert_eq!(
            unsafe {
                viem_parse_launch_arguments(
                    input.as_ptr(),
                    input.len() as u64,
                    std::ptr::null_mut(),
                    0,
                    &mut length,
                )
            },
            ViemStatus::InvalidArgument
        );
        let mut input = br#"["file"]"#.to_vec();
        assert_eq!(
            unsafe {
                viem_parse_launch_arguments(
                    input.as_ptr(),
                    input.len() as u64,
                    input.as_mut_ptr(),
                    input.len() as u64,
                    &mut length,
                )
            },
            ViemStatus::InvalidArgument
        );
    }
}
