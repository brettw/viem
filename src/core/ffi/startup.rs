//! Stateless, batched launch-argument parser for frontend composition.
use super::*;
use crate::command::startup::parse_launch_arguments;

pub type ViemStartupDiagnosticCallback =
    Option<unsafe extern "C" fn(*mut c_void, u64, *const u8, u64)>;

/// Initialize a buffer's startup settings before its first view is created.
/// Unsupported commands produce diagnostics and later lines still apply.
/// Callbacks run synchronously after the serial core lease is released; their
/// message bytes are borrowed for the duration of the callback only.
///
/// # Safety
/// Input must be readable UTF-8 for this call. The callback and context must
/// be valid for synchronous invocation and must not unwind through C.
#[no_mangle]
pub unsafe extern "C" fn viem_core_initialize_startup(
    handle: ViemCoreHandle,
    input: *const u8,
    length: u64,
    diagnostic: ViemStartupDiagnosticCallback,
    context: *mut c_void,
) -> ViemStatus {
    ffi_boundary(|| {
        if length > 1024 * 1024 {
            return Err(ViemStatus::InvalidArgument);
        }
        let text = str::from_utf8(unsafe { input_bytes(input, length)? })
            .map_err(|_| ViemStatus::InvalidUtf8)?;
        let diagnostics = with_core_mut(handle, |core| Ok(core.initialize_startup(text)))?;
        if let Some(callback) = diagnostic {
            for item in diagnostics {
                unsafe {
                    callback(
                        context,
                        item.line as u64,
                        item.message.as_ptr(),
                        item.message.len() as u64,
                    )
                };
            }
        }
        Ok(())
    })
}

/// # Safety
/// `pending` identifies one writable byte.
#[no_mangle]
pub unsafe extern "C" fn viem_core_view_has_pending_mapping(
    handle: ViemCoreHandle,
    view: ViemViewId,
    pending: *mut u8,
) -> ViemStatus {
    ffi_boundary(|| {
        typed_pointer_region(pending, 1)?;
        unsafe { pending.write(0) };
        let value = with_core(handle, |core| {
            core.command_state(ViewId(view))
                .ok_or(ViemStatus::InvalidView)?;
            Ok(core.has_pending_mapping(ViewId(view)))
        })?;
        unsafe { pending.write(u8::from(value)) };
        Ok(())
    })
}

/// Resolve a pending mapping prefix after the host's input timeout. Host
/// clipboard capabilities and owned effects follow the ordinary key protocol.
///
/// # Safety
/// The context and its nested input slices remain readable; writable outputs
/// are aligned, valid and disjoint from each other and all input regions.
#[no_mangle]
pub unsafe extern "C" fn viem_core_view_flush_mapping_with_host_context_v2(
    handle: ViemCoreHandle,
    view: ViemViewId,
    context: *const ViemCommandTurnContextV2,
    out_outcome: *mut ViemCoreOutcomeV1,
    out_effect_batch: *mut ViemEffectBatchHandle,
) -> ViemStatus {
    ffi_boundary(|| {
        let outcome_region = typed_pointer_region(out_outcome, 1)?;
        let effect_region = typed_pointer_region(out_effect_batch, 1)?;
        validate_disjoint_regions(&[outcome_region, effect_region])?;
        let parsed =
            unsafe { read_command_turn_context(context, &[outcome_region, effect_region]) };
        unsafe {
            clear_outcome(out_outcome)?;
            out_effect_batch.write(0);
        }
        let clipboard = parsed?;
        let reservation = reserve_effect_batch()?;
        let (outcome, effects) = with_core_mut(handle, |core| {
            dispatch_event_with_effects(
                core,
                view,
                CoreEvent::FlushMappingPrefixWithClipboard(clipboard.clone()),
                clipboard,
            )
        })?;
        let effect_handle = match effects {
            Some(effects) => reservation.commit(effects)?,
            None => {
                drop(reservation);
                0
            }
        };
        unsafe {
            out_outcome.write(outcome);
            out_effect_batch.write(effect_handle);
        }
        Ok(())
    })
}

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
    use std::ptr;

    #[test]
    fn function_key_transport_checks_number_and_modifier_domain() {
        let input = ViemKeyInputV1 {
            struct_size: VIEM_KEY_INPUT_V1_SIZE,
            kind: VIEM_KEY_FUNCTION,
            codepoint: 2,
            modifiers: VIEM_KEY_MODIFIER_CONTROL,
        };
        assert_eq!(
            parse_key(input),
            Ok(Key::Function {
                number: 2,
                modifiers: 2
            })
        );
        for codepoint in [0, 36, u32::MAX] {
            assert_eq!(
                parse_key(ViemKeyInputV1 { codepoint, ..input }),
                Err(ViemStatus::InvalidKey)
            );
        }
        assert_eq!(
            parse_key(ViemKeyInputV1 {
                modifiers: 16,
                ..input
            }),
            Err(ViemStatus::InvalidArgument)
        );
        assert_eq!(
            parse_key(ViemKeyInputV1 {
                kind: VIEM_KEY_CHARACTER,
                codepoint: 65,
                ..input
            }),
            Err(ViemStatus::InvalidArgument)
        );
    }

    #[test]
    fn startup_reports_lines_after_releasing_core_and_rejects_invalid_input() {
        #[derive(Default)]
        struct Captured {
            handle: ViemCoreHandle,
            messages: Vec<(u64, String)>,
            readable: bool,
        }
        unsafe extern "C" fn collect(
            context: *mut c_void,
            line: u64,
            bytes: *const u8,
            length: u64,
        ) {
            let captured = unsafe { &mut *context.cast::<Captured>() };
            captured.messages.push((
                line,
                String::from_utf8_lossy(unsafe { slice::from_raw_parts(bytes, length as usize) })
                    .into_owned(),
            ));
            captured.readable = with_core(captured.handle, |_| Ok(())).is_ok();
        }
        let mut captured = Captured::default();
        let mut revision = 0;
        assert_eq!(
            unsafe {
                viem_core_create(
                    ptr::null(),
                    0,
                    &ViemDocumentOptions::default(),
                    &mut captured.handle,
                    &mut revision,
                )
            },
            ViemStatus::Ok
        );
        let startup = b"\" comment\nnotacommand\nmap Y y$\n";
        assert_eq!(
            unsafe {
                viem_core_initialize_startup(
                    captured.handle,
                    startup.as_ptr(),
                    startup.len() as u64,
                    Some(collect),
                    (&mut captured as *mut Captured).cast(),
                )
            },
            ViemStatus::Ok
        );
        assert_eq!(captured.messages.len(), 1);
        assert_eq!(captured.messages[0].0, 2);
        assert!(
            captured.readable,
            "diagnostic callbacks must not hold the core lease"
        );
        assert_eq!(
            unsafe {
                viem_core_initialize_startup(
                    captured.handle,
                    [255].as_ptr(),
                    1,
                    None,
                    ptr::null_mut(),
                )
            },
            ViemStatus::InvalidUtf8
        );
        assert_eq!(
            unsafe {
                viem_core_initialize_startup(captured.handle, ptr::null(), 1, None, ptr::null_mut())
            },
            ViemStatus::NullPointer
        );
        assert_eq!(
            unsafe {
                viem_core_initialize_startup(
                    captured.handle,
                    ptr::null(),
                    1024 * 1024 + 1,
                    None,
                    ptr::null_mut(),
                )
            },
            ViemStatus::InvalidArgument
        );
        assert_eq!(viem_core_destroy(captured.handle), ViemStatus::Ok);
    }
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
