//! Batched settings and exact-snapshot whitespace marker exports.
use super::*;
use crate::layout::WhitespacePresentationOptions;

#[no_mangle]
pub extern "C" fn viem_core_view_set_visible_whitespace(
    handle: ViemCoreHandle,
    view: ViemViewId,
    enabled: u8,
) -> ViemStatus {
    ffi_boundary(|| {
        if enabled > 1 {
            return Err(ViemStatus::InvalidArgument);
        }
        with_core_mut(handle, |core| {
            core.set_visible_whitespace(ViewId(view), enabled != 0)
                .map_err(core_status)
        })
    })
}

unsafe fn decode_options<T: serde::de::DeserializeOwned>(
    bytes: *const u8,
    length: u64,
) -> Result<T, ViemStatus> {
    if length > 128 * 1024 {
        return Err(ViemStatus::InvalidArgument);
    }
    serde_json::from_slice(unsafe { input_bytes(bytes, length)? })
        .map_err(|_| ViemStatus::InvalidArgument)
}

/// Validate the complete candidate before settings persistence.
#[no_mangle]
pub unsafe extern "C" fn viem_validate_whitespace_presentation(
    bytes: *const u8,
    length: u64,
) -> ViemStatus {
    ffi_boundary(|| {
        let options: WhitespacePresentationOptions = unsafe { decode_options(bytes, length)? };
        options
            .validate()
            .map_err(|_| ViemStatus::InvalidArgument)?;
        Ok(())
    })
}

#[no_mangle]
pub unsafe extern "C" fn viem_core_set_indentation_defaults(
    handle: ViemCoreHandle,
    bytes: *const u8,
    length: u64,
) -> ViemStatus {
    ffi_boundary(|| {
        let options: crate::document::IndentationOptions =
            unsafe { decode_options(bytes, length)? };
        options
            .validate()
            .map_err(|_| ViemStatus::InvalidArgument)?;
        with_core_mut(handle, |core| {
            core.set_indentation_defaults(options).map_err(core_status)
        })
    })
}

#[no_mangle]
pub unsafe extern "C" fn viem_core_set_whitespace_presentation_defaults(
    handle: ViemCoreHandle,
    bytes: *const u8,
    length: u64,
) -> ViemStatus {
    ffi_boundary(|| {
        let options: WhitespacePresentationOptions = unsafe { decode_options(bytes, length)? };
        options
            .validate()
            .map_err(|_| ViemStatus::InvalidArgument)?;
        with_core_mut(handle, |core| {
            core.set_whitespace_presentation_defaults(options)
                .map_err(core_status)
        })
    })
}

/// A UTF-8 JSON batch containing sparse character declarations and viewport
/// marker rectangles. Count queries use null/zero; all regions are disjoint.
/// The supplied identity and viewport must still identify the exact presentation.
/// Scrolling may reuse the same layout snapshot, but changes clipped markers.
#[no_mangle]
pub unsafe extern "C" fn viem_core_view_copy_whitespace_markers(
    handle: ViemCoreHandle,
    view: ViemViewId,
    expected: *const ViemLayoutSnapshotIdentityV1,
    expected_viewport: *const ViemLayoutRectV1,
    output: *mut u8,
    capacity: u64,
    out_length: *mut u64,
) -> ViemStatus {
    ffi_boundary(|| {
        validate_disjoint_regions(&[
            typed_pointer_region(expected, 1)?,
            typed_pointer_region(expected_viewport, 1)?,
            typed_pointer_region(output, capacity)?,
            typed_pointer_region(out_length, 1)?,
        ])?;
        let expected = unsafe { read_layout_identity(expected)? };
        let expected_viewport = unsafe { expected_viewport.read() };
        unsafe { out_length.write(0) };
        let bytes = with_core(handle, |core| {
            let id = ViewId(view);
            let snapshot = current_ffi_layout_snapshot(core, id)?;
            validate_snapshot_identity(expected, snapshot, id)?;
            let layout = core
                .presentation_layout(id)
                .ok_or(ViemStatus::InvalidView)?;
            validate_whitespace_viewport(
                expected_viewport,
                ViemLayoutRectV1 {
                    x: layout.viewport_left(),
                    y: layout.viewport_top(),
                    width: layout.width(),
                    height: layout.height(),
                },
            )?;
            let markers = core.whitespace_markers(id).map_err(core_status)?;
            let style = &core
                .presentation_layout(id)
                .ok_or(ViemStatus::InvalidArgument)?
                .whitespace_presentation()
                .visible_whitespace
                .style;
            let values: Vec<_> = markers
                .iter()
                .map(|marker| {
                    serde_json::json!({
                        "text": marker.text, "rowIndex": marker.row_index,
                        "x": marker.rect.x, "y": marker.rect.y,
                        "width": marker.rect.width, "height": marker.rect.height,
                    })
                })
                .collect();
            let applicable = !core.document().format().is_wysiwyg();
            let enabled = applicable
                && core
                    .presentation_layout(id)
                    .unwrap()
                    .whitespace_presentation()
                    .visible_whitespace
                    .enabled;
            serde_json::to_vec(&serde_json::json!({"style": style, "markers": values, "enabled": enabled, "applicable": applicable}))
                .map_err(|_| ViemStatus::InvalidArgument)
        })?;
        unsafe { out_length.write(checked_export_count(bytes.len())?) };
        if capacity < bytes.len() as u64 {
            return Err(ViemStatus::BufferTooSmall);
        }
        unsafe { copy_output(&bytes, output) };
        Ok(())
    })
}

fn validate_whitespace_viewport(
    expected: ViemLayoutRectV1,
    current: ViemLayoutRectV1,
) -> Result<(), ViemStatus> {
    if !expected.x.is_finite()
        || expected.x < 0.0
        || !expected.y.is_finite()
        || expected.y < 0.0
        || !expected.width.is_finite()
        || expected.width < 0.0
        || !expected.height.is_finite()
        || expected.height < 0.0
    {
        return Err(ViemStatus::InvalidArgument);
    }
    if expected != current {
        return Err(ViemStatus::StaleRevision);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marker_viewport_requires_exact_finite_nonnegative_geometry() {
        let current = ViemLayoutRectV1 {
            x: 10.0,
            y: 20.0,
            width: 400.0,
            height: 300.0,
        };
        assert_eq!(validate_whitespace_viewport(current, current), Ok(()));
        let collapsed = ViemLayoutRectV1 {
            width: 0.0,
            height: 0.0,
            ..current
        };
        assert_eq!(validate_whitespace_viewport(collapsed, collapsed), Ok(()));
        for expected in [
            ViemLayoutRectV1 { x: 11.0, ..current },
            ViemLayoutRectV1 { y: 21.0, ..current },
            ViemLayoutRectV1 {
                width: 401.0,
                ..current
            },
            ViemLayoutRectV1 {
                height: 301.0,
                ..current
            },
        ] {
            assert_eq!(
                validate_whitespace_viewport(expected, current),
                Err(ViemStatus::StaleRevision)
            );
        }
        for expected in [
            ViemLayoutRectV1 {
                x: f32::NAN,
                ..current
            },
            ViemLayoutRectV1 {
                y: f32::INFINITY,
                ..current
            },
            ViemLayoutRectV1 { x: -1.0, ..current },
            ViemLayoutRectV1 {
                width: -1.0,
                ..current
            },
            ViemLayoutRectV1 {
                height: -1.0,
                ..current
            },
        ] {
            assert_eq!(
                validate_whitespace_viewport(expected, current),
                Err(ViemStatus::InvalidArgument)
            );
        }
    }
}
