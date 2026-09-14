//! Exact-generation native completion presentation and bounded search polling.
use super::*;

pub const VIEM_COMPLETION_ACTIVE: u32 = 1 << 0;
pub const VIEM_COMPLETION_SEARCHING: u32 = 1 << 1;
pub const VIEM_COMPLETION_TRUNCATED: u32 = 1 << 2;
pub const VIEM_COMPLETION_HAS_ANCHOR: u32 = 1 << 3;
pub const VIEM_COMPLETION_RIGHT_TO_LEFT: u32 = 1 << 4;

/// The frontend renders this backend-owned ordering and selection unchanged.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViemCompletionInfoV1 {
    pub struct_size: u32,
    pub flags: u32,
    pub session_id: u64,
    pub generation: u64,
    pub document_id: u64,
    pub document_revision: u64,
    pub view_id: u64,
    pub selected_index: i64,
    pub item_count: u64,
    pub utf8_length: u64,
    pub anchor_layout: ViemLayoutSnapshotIdentityV1,
    pub anchor_rect: ViemLayoutRectV1,
}

pub const VIEM_COMPLETION_INFO_V1_SIZE: u32 = size_of::<ViemCompletionInfoV1>() as u32;

impl Default for ViemCompletionInfoV1 {
    fn default() -> Self {
        Self {
            struct_size: VIEM_COMPLETION_INFO_V1_SIZE,
            flags: 0,
            session_id: 0,
            generation: 0,
            document_id: 0,
            document_revision: 0,
            view_id: 0,
            selected_index: -1,
            item_count: 0,
            utf8_length: 0,
            anchor_layout: ViemLayoutSnapshotIdentityV1::default(),
            anchor_rect: ViemLayoutRectV1::default(),
        }
    }
}

/// A candidate's byte range in the concatenated UTF-8 arena.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ViemCompletionItemV1 {
    pub text_offset: u64,
    pub text_length: u64,
}

pub const VIEM_COMPLETION_ITEM_V1_SIZE: u32 = size_of::<ViemCompletionItemV1>() as u32;

/// Keep the serial core lease throughout validation and export. The token
/// registry mutex is already released before this operation begins.
fn with_completion<R>(
    handle: ViemCoreHandle,
    view: ViemViewId,
    expected: Option<ViemCompletionInfoV1>,
    operation: impl FnOnce(ViemCompletionInfoV1, &[String]) -> Result<R, ViemStatus>,
) -> Result<R, ViemStatus> {
    with_core(handle, |core| {
        core.command_state(ViewId(view))
            .ok_or(ViemStatus::InvalidView)?;
        let presentation = core
            .completion_presentation(ViewId(view))
            .map_err(core_status)?;
        let mut info = ViemCompletionInfoV1 {
            document_id: core.document().id().0,
            document_revision: core.document().revision().0,
            view_id: view,
            ..ViemCompletionInfoV1::default()
        };
        let items = if let Some(presentation) = presentation.as_ref() {
            info.flags = VIEM_COMPLETION_ACTIVE
                | if presentation.searching {
                    VIEM_COMPLETION_SEARCHING
                } else {
                    0
                }
                | if presentation.truncated {
                    VIEM_COMPLETION_TRUNCATED
                } else {
                    0
                };
            info.session_id = presentation.session_id;
            info.generation = presentation.generation;
            info.document_id = presentation.document_id.0;
            info.document_revision = presentation.revision.0;
            info.selected_index = presentation
                .selected_index
                .map(i64::try_from)
                .transpose()
                .map_err(|_| ViemStatus::LengthOverflow)?
                .unwrap_or(-1);
            info.item_count = checked_export_count(presentation.items.len())?;
            info.utf8_length = presentation.items.iter().try_fold(0_u64, |total, item| {
                total
                    .checked_add(checked_export_count(item.len())?)
                    .ok_or(ViemStatus::LengthOverflow)
            })?;
            if let Some(anchor) = core
                .completion_popup_anchor(ViewId(view))
                .map_err(core_status)?
            {
                let snapshot = current_ffi_layout_snapshot(core, ViewId(view))?;
                info.flags |= VIEM_COMPLETION_HAS_ANCHOR;
                if anchor.right_to_left {
                    info.flags |= VIEM_COMPLETION_RIGHT_TO_LEFT;
                }
                info.anchor_layout = snapshot_identity(snapshot, ViewId(view));
                info.anchor_rect = ViemLayoutRectV1 {
                    x: anchor.rect.x,
                    y: anchor.rect.y,
                    width: anchor.rect.width,
                    height: anchor.rect.height,
                };
            }
            presentation.items.as_slice()
        } else {
            &[]
        };
        if let Some(expected) = expected {
            validate_completion_info(expected, info)?;
        }
        operation(info, items)
    })
}

fn validate_completion_info(
    mut expected: ViemCompletionInfoV1,
    current: ViemCompletionInfoV1,
) -> Result<(), ViemStatus> {
    // Larger caller structures are accepted consistently with other v1 ABI
    // records, but all state-bearing fields must match the queried record.
    expected.struct_size = VIEM_COMPLETION_INFO_V1_SIZE;
    if expected != current {
        return Err(ViemStatus::StaleRevision);
    }
    Ok(())
}

unsafe fn read_completion_info(
    expected: *const ViemCompletionInfoV1,
) -> Result<ViemCompletionInfoV1, ViemStatus> {
    let expected = unsafe { expected.read() };
    if expected.struct_size < VIEM_COMPLETION_INFO_V1_SIZE
        || expected.flags
            & !(VIEM_COMPLETION_ACTIVE
                | VIEM_COMPLETION_SEARCHING
                | VIEM_COMPLETION_TRUNCATED
                | VIEM_COMPLETION_HAS_ANCHOR
                | VIEM_COMPLETION_RIGHT_TO_LEFT)
            != 0
    {
        return Err(ViemStatus::InvalidArgument);
    }
    Ok(expected)
}

/// Query completion without searching or changing its selected item.
///
/// # Safety
/// `out_info` must identify one aligned writable record.
#[no_mangle]
pub unsafe extern "C" fn viem_core_view_completion_info(
    handle: ViemCoreHandle,
    view: ViemViewId,
    out_info: *mut ViemCompletionInfoV1,
) -> ViemStatus {
    ffi_boundary(|| {
        typed_pointer_region(out_info, 1)?;
        unsafe { out_info.write(ViemCompletionInfoV1::default()) };
        let info = with_completion(handle, view, None, |info, _| Ok(info))?;
        unsafe { out_info.write(info) };
        Ok(())
    })
}

/// Copy all candidate references for one exact completion generation.
///
/// # Safety
/// All pointer regions must be valid, aligned, and pairwise disjoint. A null
/// `items` pointer is accepted only with zero capacity. `out_count` is required.
#[no_mangle]
pub unsafe extern "C" fn viem_core_view_copy_completion_items(
    handle: ViemCoreHandle,
    view: ViemViewId,
    expected: *const ViemCompletionInfoV1,
    items: *mut ViemCompletionItemV1,
    capacity: u64,
    out_count: *mut u64,
) -> ViemStatus {
    ffi_boundary(|| {
        validate_disjoint_regions(&[
            typed_pointer_region(expected, 1)?,
            typed_pointer_region(items, capacity)?,
            typed_pointer_region(out_count, 1)?,
        ])?;
        let expected = unsafe { read_completion_info(expected)? };
        unsafe { out_count.write(0) };
        with_completion(handle, view, Some(expected), |info, candidates| {
            unsafe { out_count.write(info.item_count) };
            if capacity < info.item_count {
                return Err(ViemStatus::BufferTooSmall);
            }
            let mut offset = 0;
            for (index, candidate) in candidates.iter().enumerate() {
                // Total size was checked before writing any caller memory.
                let length = candidate.len() as u64;
                unsafe {
                    items.add(index).write(ViemCompletionItemV1 {
                        text_offset: offset,
                        text_length: length,
                    });
                }
                offset += length;
            }
            Ok(())
        })
    })
}

/// Copy the concatenated, non-NUL-terminated UTF-8 candidate arena.
///
/// # Safety
/// All pointer regions must be valid, aligned, and pairwise disjoint. A null
/// `bytes` pointer is accepted only with zero capacity. `out_count` is required.
#[no_mangle]
pub unsafe extern "C" fn viem_core_view_copy_completion_utf8(
    handle: ViemCoreHandle,
    view: ViemViewId,
    expected: *const ViemCompletionInfoV1,
    bytes: *mut u8,
    capacity: u64,
    out_count: *mut u64,
) -> ViemStatus {
    ffi_boundary(|| {
        validate_disjoint_regions(&[
            typed_pointer_region(expected, 1)?,
            typed_pointer_region(bytes, capacity)?,
            typed_pointer_region(out_count, 1)?,
        ])?;
        let expected = unsafe { read_completion_info(expected)? };
        unsafe { out_count.write(0) };
        with_completion(handle, view, Some(expected), |info, candidates| {
            unsafe { out_count.write(info.utf8_length) };
            if capacity < info.utf8_length {
                return Err(ViemStatus::BufferTooSmall);
            }
            let mut offset = 0;
            for candidate in candidates {
                if !candidate.is_empty() {
                    unsafe { copy_output(candidate.as_bytes(), bytes.add(offset)) };
                    offset += candidate.len();
                }
            }
            Ok(())
        })
    })
}

/// Run one bounded incremental search slice on the core's serial executor.
///
/// # Safety
/// `out_changed` must identify one writable byte.
#[no_mangle]
pub unsafe extern "C" fn viem_core_view_poll_completion(
    handle: ViemCoreHandle,
    view: ViemViewId,
    out_changed: *mut u8,
) -> ViemStatus {
    ffi_boundary(|| {
        typed_pointer_region(out_changed, 1)?;
        unsafe { out_changed.write(0) };
        let changed = with_core_mut(handle, |core| {
            core.poll_completion(ViewId(view)).map_err(core_status)
        })?;
        unsafe { out_changed.write(u8::from(changed)) };
        Ok(())
    })
}

/// Materialize a backend-selected candidate before a native non-key operation.
///
/// # Safety
/// `out_changed` must identify one writable byte.
#[no_mangle]
pub unsafe extern "C" fn viem_core_view_accept_completion(
    handle: ViemCoreHandle,
    view: ViemViewId,
    out_changed: *mut u8,
) -> ViemStatus {
    ffi_boundary(|| {
        typed_pointer_region(out_changed, 1)?;
        unsafe { out_changed.write(0) };
        let changed = with_core_mut(handle, |core| {
            core.accept_completion(ViewId(view))
                .map(|outcome| outcome.document_changed || outcome.layout_changed)
                .map_err(core_status)
        })?;
        unsafe { out_changed.write(u8::from(changed)) };
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ptr;

    // Completion is a logical service and must work independently of shaping.
    unsafe extern "C" fn metrics(_: *mut c_void) -> u64 {
        1
    }
    unsafe extern "C" fn unavailable_shape(
        _: *mut c_void,
        _: *const ViemShapeRequestV1,
        _: u64,
        _: *mut ViemShapeResponseV1,
        _: u64,
    ) -> u32 {
        ViemStatus::ProviderFailure as u32
    }

    struct Fixture {
        handle: ViemCoreHandle,
        view: ViemViewId,
    }

    impl Fixture {
        fn new(text: &str) -> Self {
            let provider = CTextMeasurementProvider {
                context: 0,
                measurement_environment_id: MeasurementEnvironmentId(1),
                threading: ProviderThreading::AnyWorker,
                render_run_policy: None,
                metrics_generation_callback: metrics,
                shape_batch_callback: unavailable_shape,
                retain_render_runs_callback: None,
                release_render_runs_callback: None,
            };
            let mut core = Core::new(Document::new(text));
            let view = core.add_view(provider, 400.0, 300.0).0;
            Self {
                handle: register_core(core).unwrap(),
                view,
            }
        }

        fn key(&self, key: Key) {
            with_core_mut(self.handle, |core| {
                core.handle(ViewId(self.view), CoreEvent::Input(InputEvent::Key(key)))
                    .map_err(core_status)
            })
            .unwrap();
        }

        fn info(&self) -> ViemCompletionInfoV1 {
            let mut info = ViemCompletionInfoV1::default();
            assert_eq!(
                unsafe { viem_core_view_completion_info(self.handle, self.view, &mut info) },
                ViemStatus::Ok
            );
            info
        }

        fn finish_search(&self) -> ViemCompletionInfoV1 {
            for _ in 0..1024 {
                let info = self.info();
                if info.flags & VIEM_COMPLETION_SEARCHING == 0 {
                    return info;
                }
                let mut changed = 99;
                assert_eq!(
                    unsafe { viem_core_view_poll_completion(self.handle, self.view, &mut changed) },
                    ViemStatus::Ok
                );
                assert!(changed <= 1);
            }
            panic!("small completion fixture did not finish its bounded search");
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            assert_eq!(viem_core_destroy(self.handle), ViemStatus::Ok);
        }
    }

    #[test]
    fn inactive_completion_has_current_identity_and_empty_exports() {
        let fixture = Fixture::new("words");
        let info = fixture.info();
        assert_eq!(info.struct_size, VIEM_COMPLETION_INFO_V1_SIZE);
        assert_ne!(info.document_id, 0);
        assert_eq!(info.view_id, fixture.view);
        assert_eq!((info.flags, info.session_id, info.generation), (0, 0, 0));
        assert_eq!(
            (info.selected_index, info.item_count, info.utf8_length),
            (-1, 0, 0)
        );
        let mut count = 99;
        assert_eq!(
            unsafe {
                viem_core_view_copy_completion_items(
                    fixture.handle,
                    fixture.view,
                    &info,
                    ptr::null_mut(),
                    0,
                    &mut count,
                )
            },
            ViemStatus::Ok
        );
        assert_eq!(count, 0);
        assert_eq!(
            unsafe {
                viem_core_view_copy_completion_utf8(
                    fixture.handle,
                    fixture.view,
                    &info,
                    ptr::null_mut(),
                    0,
                    &mut count,
                )
            },
            ViemStatus::Ok
        );
        assert_eq!(count, 0);
        let mut changed = 99;
        assert_eq!(
            unsafe { viem_core_view_poll_completion(fixture.handle, fixture.view, &mut changed) },
            ViemStatus::Ok
        );
        assert_eq!(changed, 0);
        changed = 99;
        assert_eq!(
            unsafe { viem_core_view_accept_completion(fixture.handle, fixture.view, &mut changed) },
            ViemStatus::Ok
        );
        assert_eq!(changed, 0);
    }

    #[test]
    fn native_acceptance_commits_selection_and_remains_in_insert_mode() {
        let fixture = Fixture::new("é\néclair élève");
        fixture.key(Key::Char('a'));
        fixture.key(Key::Ctrl('n'));
        let info = fixture.finish_search();
        assert_eq!(info.selected_index, 0);
        with_core(fixture.handle, |core| {
            assert_eq!(core.document().text(), "é\néclair élève");
            Ok(())
        })
        .unwrap();
        let mut changed = 99;
        assert_eq!(
            unsafe { viem_core_view_accept_completion(fixture.handle, fixture.view, &mut changed) },
            ViemStatus::Ok
        );
        assert_eq!(changed, 1);
        assert_eq!(fixture.info().flags, 0);
        with_core(fixture.handle, |core| {
            assert_eq!(core.document().text(), "éclair\néclair élève");
            assert_eq!(
                core.command_state(ViewId(fixture.view)).unwrap().mode(),
                Mode::Insert
            );
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn candidate_exports_are_exact_utf8_and_all_or_none() {
        let fixture = Fixture::new("é\néclair élève éclair");
        fixture.key(Key::Char('a'));
        fixture.key(Key::Ctrl('n'));
        let info = fixture.finish_search();
        assert_ne!(info.flags & VIEM_COMPLETION_ACTIVE, 0);
        assert_ne!(info.session_id, 0);
        assert_eq!(info.item_count, 2);
        assert!(info.selected_index >= 0);

        let mut count = 0;
        assert_eq!(
            unsafe {
                viem_core_view_copy_completion_items(
                    fixture.handle,
                    fixture.view,
                    &info,
                    ptr::null_mut(),
                    0,
                    &mut count,
                )
            },
            ViemStatus::BufferTooSmall
        );
        assert_eq!(count, info.item_count);
        let sentinel = ViemCompletionItemV1 {
            text_offset: 99,
            text_length: 99,
        };
        let mut short_items = [sentinel];
        assert_eq!(
            unsafe {
                viem_core_view_copy_completion_items(
                    fixture.handle,
                    fixture.view,
                    &info,
                    short_items.as_mut_ptr(),
                    1,
                    &mut count,
                )
            },
            ViemStatus::BufferTooSmall
        );
        assert_eq!(short_items, [sentinel]);

        let mut items = vec![ViemCompletionItemV1::default(); info.item_count as usize];
        assert_eq!(
            unsafe {
                viem_core_view_copy_completion_items(
                    fixture.handle,
                    fixture.view,
                    &info,
                    items.as_mut_ptr(),
                    items.len() as u64,
                    &mut count,
                )
            },
            ViemStatus::Ok
        );
        assert_eq!(count, info.item_count);
        assert_eq!(
            unsafe {
                viem_core_view_copy_completion_utf8(
                    fixture.handle,
                    fixture.view,
                    &info,
                    ptr::null_mut(),
                    0,
                    &mut count,
                )
            },
            ViemStatus::BufferTooSmall
        );
        assert_eq!(count, info.utf8_length);
        let mut bytes = vec![0xcc; info.utf8_length as usize];
        assert_eq!(
            unsafe {
                viem_core_view_copy_completion_utf8(
                    fixture.handle,
                    fixture.view,
                    &info,
                    bytes.as_mut_ptr(),
                    info.utf8_length - 1,
                    &mut count,
                )
            },
            ViemStatus::BufferTooSmall
        );
        assert!(bytes.iter().all(|byte| *byte == 0xcc));
        assert_eq!(
            unsafe {
                viem_core_view_copy_completion_utf8(
                    fixture.handle,
                    fixture.view,
                    &info,
                    bytes.as_mut_ptr(),
                    info.utf8_length,
                    &mut count,
                )
            },
            ViemStatus::Ok
        );
        assert_eq!(count, info.utf8_length);
        let words: Vec<_> = items
            .iter()
            .map(|item| {
                let start = item.text_offset as usize;
                std::str::from_utf8(&bytes[start..start + item.text_length as usize]).unwrap()
            })
            .collect();
        assert_eq!(words, ["éclair", "élève"]);
        assert_eq!(items[0].text_offset, 0);
        assert_eq!(items[1].text_offset, items[0].text_length);
        assert_eq!(
            items[1].text_offset + items[1].text_length,
            info.utf8_length
        );

        fixture.key(Key::Ctrl('n'));
        assert_ne!(fixture.info().generation, info.generation);
        count = 99;
        assert_eq!(
            unsafe {
                viem_core_view_copy_completion_utf8(
                    fixture.handle,
                    fixture.view,
                    &info,
                    bytes.as_mut_ptr(),
                    info.utf8_length,
                    &mut count,
                )
            },
            ViemStatus::StaleRevision
        );
        assert_eq!(count, 0);
        fixture.key(Key::Escape);
        assert_eq!(fixture.info().flags, 0);
        assert_eq!(
            unsafe {
                viem_core_view_copy_completion_items(
                    fixture.handle,
                    fixture.view,
                    &info,
                    items.as_mut_ptr(),
                    items.len() as u64,
                    &mut count,
                )
            },
            ViemStatus::StaleRevision
        );
    }

    #[test]
    fn completion_exports_reject_bad_tokens_pointers_and_overlap() {
        let fixture = Fixture::new("text");
        let info = fixture.info();
        let mut count = 99;
        assert_eq!(
            unsafe {
                viem_core_view_completion_info(fixture.handle, fixture.view, ptr::null_mut())
            },
            ViemStatus::NullPointer
        );
        assert_eq!(
            unsafe {
                viem_core_view_poll_completion(fixture.handle, fixture.view, ptr::null_mut())
            },
            ViemStatus::NullPointer
        );
        assert_eq!(
            unsafe {
                viem_core_view_accept_completion(fixture.handle, fixture.view, ptr::null_mut())
            },
            ViemStatus::NullPointer
        );
        assert_eq!(
            unsafe {
                viem_core_view_copy_completion_items(
                    fixture.handle,
                    fixture.view,
                    &info,
                    ptr::null_mut(),
                    1,
                    &mut count,
                )
            },
            ViemStatus::NullPointer
        );
        assert_eq!(
            unsafe {
                viem_core_view_copy_completion_utf8(
                    fixture.handle,
                    fixture.view,
                    &info,
                    ptr::null_mut(),
                    0,
                    ptr::null_mut(),
                )
            },
            ViemStatus::NullPointer
        );
        let mut overlapping = info;
        let pointer = &mut overlapping as *mut ViemCompletionInfoV1;
        assert_eq!(
            unsafe {
                viem_core_view_copy_completion_utf8(
                    fixture.handle,
                    fixture.view,
                    pointer,
                    pointer.cast(),
                    size_of::<ViemCompletionInfoV1>() as u64,
                    &mut count,
                )
            },
            ViemStatus::InvalidArgument
        );
        let mut malformed = info;
        malformed.struct_size = 0;
        assert_eq!(
            unsafe {
                viem_core_view_copy_completion_utf8(
                    fixture.handle,
                    fixture.view,
                    &malformed,
                    ptr::null_mut(),
                    0,
                    &mut count,
                )
            },
            ViemStatus::InvalidArgument
        );
        malformed = info;
        malformed.flags = 1 << 31;
        assert_eq!(
            unsafe {
                viem_core_view_copy_completion_items(
                    fixture.handle,
                    fixture.view,
                    &malformed,
                    ptr::null_mut(),
                    0,
                    &mut count,
                )
            },
            ViemStatus::InvalidArgument
        );
        assert_eq!(
            unsafe {
                viem_core_view_copy_completion_utf8(
                    0,
                    fixture.view,
                    &info,
                    ptr::null_mut(),
                    0,
                    &mut count,
                )
            },
            ViemStatus::InvalidHandle
        );
        assert_eq!(count, 0);
        let mut output = info;
        assert_eq!(
            unsafe { viem_core_view_completion_info(fixture.handle, u64::MAX, &mut output) },
            ViemStatus::InvalidView
        );
        assert_eq!(output, ViemCompletionInfoV1::default());
        let mut changed = 99;
        assert_eq!(
            unsafe { viem_core_view_poll_completion(fixture.handle, u64::MAX, &mut changed) },
            ViemStatus::InvalidView
        );
        assert_eq!(changed, 0);
        let lease = checkout_core(fixture.handle).unwrap();
        assert_eq!(
            unsafe { viem_core_view_completion_info(fixture.handle, fixture.view, &mut output) },
            ViemStatus::CoreBusy
        );
        drop(lease);
    }

    #[test]
    fn every_completion_presentation_field_participates_in_stale_validation() {
        let current = ViemCompletionInfoV1 {
            flags: VIEM_COMPLETION_ACTIVE,
            session_id: 1,
            generation: 2,
            document_id: 3,
            document_revision: 4,
            view_id: 5,
            selected_index: 0,
            item_count: 1,
            utf8_length: 7,
            ..ViemCompletionInfoV1::default()
        };
        for stale in [
            ViemCompletionInfoV1 {
                flags: VIEM_COMPLETION_ACTIVE | VIEM_COMPLETION_SEARCHING,
                ..current
            },
            ViemCompletionInfoV1 {
                session_id: 2,
                ..current
            },
            ViemCompletionInfoV1 {
                generation: 3,
                ..current
            },
            ViemCompletionInfoV1 {
                document_id: 4,
                ..current
            },
            ViemCompletionInfoV1 {
                document_revision: 5,
                ..current
            },
            ViemCompletionInfoV1 {
                view_id: 6,
                ..current
            },
            ViemCompletionInfoV1 {
                selected_index: -1,
                ..current
            },
            ViemCompletionInfoV1 {
                item_count: 2,
                ..current
            },
            ViemCompletionInfoV1 {
                utf8_length: 8,
                ..current
            },
            ViemCompletionInfoV1 {
                anchor_layout: ViemLayoutSnapshotIdentityV1 {
                    layout_revision: 1,
                    ..current.anchor_layout
                },
                ..current
            },
            ViemCompletionInfoV1 {
                anchor_rect: ViemLayoutRectV1 {
                    x: 1.0,
                    ..current.anchor_rect
                },
                ..current
            },
            ViemCompletionInfoV1 {
                flags: current.flags | VIEM_COMPLETION_RIGHT_TO_LEFT,
                ..current
            },
        ] {
            assert_eq!(
                validate_completion_info(stale, current),
                Err(ViemStatus::StaleRevision)
            );
        }
        assert_eq!(validate_completion_info(current, current), Ok(()));
    }
}
