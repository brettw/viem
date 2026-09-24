//! Exact native formatting snapshots and atomic set/clear requests.
use super::*;

pub const VIEM_STYLE_PROPERTY_MIXED: u32 = 1 << 3;

/// # Safety
/// The request array, nested values, and output must be valid and disjoint.
#[no_mangle]
pub unsafe extern "C" fn viem_core_view_edit_direct_properties(
    handle: ViemCoreHandle,
    view: ViemViewId,
    requests: *const ViemDirectStyleEditV1,
    count: u64,
    out_outcome: *mut ViemCoreOutcomeV1,
) -> ViemStatus {
    ffi_boundary(|| {
        if !(1..=32).contains(&count) {
            return Err(ViemStatus::InvalidArgument);
        }
        let input = typed_pointer_region(requests, count)?;
        let output = typed_pointer_region(out_outcome, 1)?;
        if regions_overlap(input, output) {
            return Err(ViemStatus::InvalidArgument);
        }
        let requests = unsafe { std::slice::from_raw_parts(requests, count as usize) };
        let expected_selection = requests[0].expected_selection;
        let mut values = Vec::with_capacity(count as usize);
        let mut seen = std::collections::BTreeSet::new();
        for request in requests {
            if request.struct_size < VIEM_DIRECT_STYLE_EDIT_V1_SIZE || request.reserved != 0 {
                return Err(ViemStatus::InvalidArgument);
            }
            validate_logical_selection_identity(request.expected_selection, expected_selection)?;
            let property = parse_style_property(request.property)?;
            if property < StyleProperty::ParagraphSpacingBefore || !seen.insert(property) {
                return Err(ViemStatus::InvalidArgument);
            }
            let value = match request.operation {
                VIEM_STYLE_EDIT_SET_DECLARATION => Some(unsafe {
                    parse_direct_style_property_value(property, &request.value, out_outcome)?
                }),
                VIEM_STYLE_EDIT_CLEAR_DECLARATION
                    if request.value.struct_size >= VIEM_STYLE_EDIT_VALUE_V1_SIZE
                        && request.value.kind == VIEM_STYLE_VALUE_NONE
                        && request.value.reserved == 0
                        && request.value.item_count == 0
                        && request.value.text.length == 0 =>
                {
                    None
                }
                _ => return Err(ViemStatus::InvalidArgument),
            };
            values.push((property, value));
        }
        unsafe {
            clear_outcome(out_outcome)?;
        }
        let outcome = with_core_mut(handle, |core| {
            let expected = core
                .list_selection_identity(ViewId(view))
                .map_err(core_status)?;
            validate_logical_selection_identity(
                expected_selection,
                logical_selection_identity_to_ffi(&expected)?,
            )?;
            dispatch_event(
                core,
                view,
                CoreEvent::EditDirectProperties { expected, values },
            )
        })?;
        unsafe {
            out_outcome.write(outcome);
        }
        Ok(())
    })
}

/// # Safety
/// All output buffers must be valid for their capacities and disjoint from
/// each other and expected_selection. No arrays are written on sizing failure.
#[no_mangle]
pub unsafe extern "C" fn viem_core_view_copy_formatting(
    handle: ViemCoreHandle,
    view: ViemViewId,
    expected_selection: *const ViemLogicalSelectionIdentityV1,
    out_info: *mut ViemStyleSheetInfoV1,
    out_properties: *mut ViemStylePropertyV1,
    property_capacity: u64,
    out_items: *mut ViemStyleValueItemV1,
    item_capacity: u64,
    out_strings: *mut u8,
    string_capacity: u64,
) -> ViemStatus {
    ffi_boundary(|| {
        let regions = [
            typed_pointer_region(expected_selection, 1)?,
            typed_pointer_region(out_info, 1)?,
            typed_pointer_region(out_properties, property_capacity)?,
            typed_pointer_region(out_items, item_capacity)?,
            typed_pointer_region(out_strings, string_capacity)?,
        ];
        for (index, left) in regions.iter().enumerate() {
            if regions[index + 1..]
                .iter()
                .any(|right| regions_overlap(*left, *right))
            {
                return Err(ViemStatus::InvalidArgument);
            }
        }
        let expected = unsafe { expected_selection.read() };
        let (info, properties, items, strings) = with_core(handle, |core| {
            let selected = core
                .list_selection_identity(ViewId(view))
                .map_err(core_status)?;
            validate_logical_selection_identity(
                expected,
                logical_selection_identity_to_ffi(&selected)?,
            )?;
            let (character, _) = core
                .selected_typography(ViewId(view))
                .map_err(core_status)?;
            let paragraph = core
                .selected_paragraph_style(ViewId(view))
                .map_err(core_status)?;
            let projection = core.document().projection();
            let range = selected.range();
            let mut boundaries = std::collections::BTreeSet::new();
            let mut paragraph_boundaries = std::collections::BTreeSet::new();
            if selected.kind() != LogicalSelectionKind::None && !range.is_empty() {
                boundaries.insert(range.start);
                paragraph_boundaries.insert(range.start);
                for span in projection.style_spans_for_region(&range) {
                    boundaries.insert(span.range.start.max(range.start));
                    boundaries.insert(span.range.end.min(range.end));
                }
                for block in projection
                    .flow_blocks_for_region(&range)
                    .unwrap_or_else(|| projection.blocks_for_region(&range))
                {
                    boundaries.insert(block.range.start.max(range.start));
                    paragraph_boundaries.insert(block.range.start.max(range.start));
                }
            }
            let mut characters = Vec::new();
            let mut paragraphs = Vec::new();
            for at in boundaries.into_iter().filter(|at| *at < range.end) {
                characters.push(
                    crate::layout::DocumentLayoutStyles::semantic_character_at(
                        projection, at, false,
                    )
                    .map_err(|error| {
                        core_status(CoreError::Layout(crate::layout::LayoutError::from(error)))
                    })?,
                );
            }
            // Inline declarations may change inside one grapheme. Only real
            // paragraph boundaries are logical points for the paragraph query.
            for at in paragraph_boundaries
                .into_iter()
                .filter(|at| *at < range.end)
            {
                paragraphs.push(core.paragraph_style_at_offset(at).map_err(core_status)?);
            }
            let mut properties = Vec::new();
            let mut items = Vec::new();
            let mut strings = Vec::new();
            for id in
                VIEM_STYLE_PROPERTY_PARAGRAPH_SPACING_BEFORE..=VIEM_STYLE_PROPERTY_CHARACTER_BOLD
            {
                let property = parse_style_property(id)?;
                let value = if crate::document::is_character_property(property) {
                    effective_character_property(&character, property)
                } else {
                    effective_paragraph_property(&paragraph, property)
                };
                let mixed = if crate::document::is_character_property(property) {
                    characters
                        .iter()
                        .any(|current| effective_character_property(current, property) != value)
                } else {
                    paragraphs
                        .iter()
                        .any(|current| effective_paragraph_property(current, property) != value)
                };
                properties.push(ViemStylePropertyV1 {
                    struct_size: VIEM_STYLE_PROPERTY_V1_SIZE,
                    property: id,
                    flags: (if value.is_some() {
                        VIEM_STYLE_PROPERTY_EFFECTIVE_PRESENT
                    } else {
                        0
                    }) | (if mixed { VIEM_STYLE_PROPERTY_MIXED } else { 0 }),
                    effective: value
                        .as_ref()
                        .map(|v| style_value_to_ffi(v, &mut items, &mut strings))
                        .transpose()?
                        .unwrap_or_else(empty_style_value),
                    ..Default::default()
                });
            }
            let info = ViemStyleSheetInfoV1 {
                struct_size: VIEM_STYLE_SHEET_INFO_V1_SIZE,
                identity: style_sheet_identity(core.document()),
                property_count: properties.len() as u64,
                value_item_count: items.len() as u64,
                string_bytes: strings.len() as u64,
                ..Default::default()
            };
            Ok((info, properties, items, strings))
        })?;
        unsafe {
            out_info.write(info);
        }
        if property_capacity < properties.len() as u64
            || item_capacity < items.len() as u64
            || string_capacity < strings.len() as u64
        {
            return Err(ViemStatus::BufferTooSmall);
        }
        unsafe {
            if !properties.is_empty() {
                std::ptr::copy_nonoverlapping(
                    properties.as_ptr(),
                    out_properties,
                    properties.len(),
                );
            }
            if !items.is_empty() {
                std::ptr::copy_nonoverlapping(items.as_ptr(), out_items, items.len());
            }
            if !strings.is_empty() {
                std::ptr::copy_nonoverlapping(strings.as_ptr(), out_strings, strings.len());
            }
        }
        Ok(())
    })
}
