use super::*;
/// Pure config normalization. JSON envelope contains configuration or error.
/// # Safety
/// Input/output/count regions are valid and disjoint. Output has `capacity` bytes.
#[no_mangle]
pub unsafe extern "C" fn viem_application_config(
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
        unsafe {
            required.write(0);
        }
        let result =
            crate::application_config::normalize(unsafe { input_bytes(input, input_length)? });
        let response = match result {
            Ok(value) => serde_json::json!({"configuration":value,"error":null}),
            Err(error) => serde_json::json!({"configuration":null,"error":error}),
        };
        let bytes = serde_json::to_vec(&response).map_err(|_| ViemStatus::InternalError)?;
        unsafe { copy_clipboard_json_bytes(&bytes, output, capacity, required) }
    })
}
