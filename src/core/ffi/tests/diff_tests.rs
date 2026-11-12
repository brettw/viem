use super::*;
use crate::ffi::*;

fn code_core(text: &str, storage: &mut PaintTestProviderStorage, seed: u64) -> (ViemCoreHandle, ViewId) {
    let document = Document::from_bytes(text.as_bytes().to_vec(), Encoding::Utf8, Format::Code).unwrap();
    let mut core = Core::new(document);
    let view = core.add_view(paint_test_provider(storage, seed, seed + 100), 400.0, 300.0);
    (register_core(core).unwrap(), view)
}

fn request(kind: u32, path: &str) -> ViemDiffBaseRequest {
    ViemDiffBaseRequest {
        kind,
        file_path: ViemUtf8Slice { data: path.as_ptr(), length: path.len() as u64 },
        revision: ViemUtf8Slice::default(),
        git_program: ViemUtf8Slice::default(),
        cancellation: 0,
        unsaved_paths: ptr::null(),
        unsaved_path_count: 0,
        scope_path: ViemUtf8Slice::default(),
    }
}

fn info(result: ViemDiffResultHandle) -> ViemDiffResultInfo {
    let mut info = ViemDiffResultInfo { ..Default::default() };
    assert_eq!(unsafe { viem_diff_result_info(result, &mut info) }, ViemStatus::Ok);
    info
}

fn result_bytes(result: ViemDiffResultHandle) -> Vec<u8> {
    let mut required = 0;
    unsafe {
        let status = viem_diff_result_copy_bytes(result, ptr::null_mut(), 0, &mut required);
        assert!(matches!(status, ViemStatus::Ok | ViemStatus::BufferTooSmall));
        let mut bytes = vec![0; required as usize];
        assert_eq!(viem_diff_result_copy_bytes(result, bytes.as_mut_ptr(), required, &mut required), ViemStatus::Ok);
        bytes
    }
}

#[test]
fn sessions_sync_both_views_and_map_lines_through_the_abi() {
    let mut storage = PaintTestProviderStorage::default();
    let (base, base_view) = code_core("a\nx\nb\n", &mut storage, 701);
    let (new, new_view) = code_core("a\nb\n", &mut storage, 801);
    let mut session = 0;
    unsafe {
        assert_eq!(viem_diff_session_create(base, base, 0, &mut session), ViemStatus::InvalidArgument);
        assert_eq!(viem_diff_session_create(base, new, 1 << 5, &mut session), ViemStatus::InvalidArgument);
        assert_eq!(viem_diff_session_create(base, new, VIEM_DIFF_SESSION_ALIGNED, &mut session), ViemStatus::Ok);
        let mut sync = 9;
        assert_eq!(viem_diff_session_sync(session, base, base_view.0, new, new_view.0, &mut sync), ViemStatus::Ok);
        assert_eq!(sync, VIEM_DIFF_SYNC_UNCHANGED);
        let mut summary = ViemDiffSessionInfo { ..Default::default() };
        assert_eq!(viem_diff_session_info(session, &mut summary), ViemStatus::Ok);
        assert_eq!((summary.hunk_count, summary.base_line_count, summary.new_line_count), (1, 4, 3));
        assert_eq!(summary.flags, VIEM_DIFF_SESSION_ALIGNED);
        let mut line = 0;
        assert_eq!(viem_diff_session_corresponding_line(session, 1, 1, &mut line), ViemStatus::Ok);
        assert_eq!(line, 2);
        assert_eq!(viem_diff_session_corresponding_line(session, 2, 1, &mut line), ViemStatus::InvalidArgument);
        assert_eq!(viem_diff_session_set_aligned(session, 0), ViemStatus::Ok);
        assert_eq!(viem_diff_clear_view(new, new_view.0), ViemStatus::Ok);
        assert_eq!(viem_diff_session_destroy(session), ViemStatus::Ok);
        assert_eq!(viem_diff_session_destroy(session), ViemStatus::InvalidHandle);
    }
    assert_eq!(viem_core_destroy(base), ViemStatus::Ok);
    assert_eq!(viem_core_destroy(new), ViemStatus::Ok);
}

#[test]
fn file_bases_return_owned_results_including_missing_files() {
    let directory = std::env::temp_dir().join(format!("viem-ffi-diff-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let file = directory.join("base.txt");
    std::fs::write(&file, b"base\r\n").unwrap();
    let path = file.to_str().unwrap().to_owned();
    let mut result = 0;
    unsafe {
        assert_eq!(viem_diff_load_base(&request(VIEM_DIFF_BASE_FILE, &path), &mut result), ViemStatus::Ok);
    }
    let summary = info(result);
    assert_eq!((summary.result, summary.flags, summary.byte_count), (VIEM_DIFF_RESULT_OK, VIEM_DIFF_RESULT_BASE_EXISTS, 6));
    assert_eq!(result_bytes(result), b"base\r\n");
    assert_eq!(viem_diff_result_release(result), ViemStatus::Ok);
    assert_eq!(viem_diff_result_release(result), ViemStatus::InvalidHandle);

    let missing = directory.join("missing").to_str().unwrap().to_owned();
    unsafe {
        assert_eq!(viem_diff_load_base(&request(VIEM_DIFF_BASE_LAST_SAVED, &missing), &mut result), ViemStatus::Ok);
    }
    assert_eq!(info(result).flags & VIEM_DIFF_RESULT_BASE_EXISTS, 0);
    assert_eq!(viem_diff_result_release(result), ViemStatus::Ok);

    // A chosen file has an empty list; Last saved lists only the document.
    unsafe {
        assert_eq!(viem_diff_list_files(&request(VIEM_DIFF_BASE_FILE, &path), &mut result), ViemStatus::Ok);
        assert_eq!(info(result).item_count, 0);
        assert_eq!(viem_diff_result_release(result), ViemStatus::Ok);
        assert_eq!(viem_diff_list_files(&request(VIEM_DIFF_BASE_LAST_SAVED, &path), &mut result), ViemStatus::Ok);
        assert_eq!(info(result).item_count, 1);
        let mut item = ViemDiffListItem { ..Default::default() };
        let mut required = 0;
        let mut text = vec![0u8; 4096];
        assert_eq!(viem_diff_result_copy_item(result, 0, &mut item, text.as_mut_ptr(), text.len() as u64, &mut required), ViemStatus::Ok);
        assert_eq!(&text[..item.path_bytes as usize], path.as_bytes());
        assert_eq!(viem_diff_result_copy_item(result, 1, &mut item, text.as_mut_ptr(), 4096, &mut required), ViemStatus::InvalidRange);
        assert_eq!(viem_diff_result_release(result), ViemStatus::Ok);
    }
    let _ = std::fs::remove_dir_all(&directory);
}

#[test]
fn base_requests_reject_relative_paths_and_unknown_kinds() {
    let mut result = 7;
    unsafe {
        assert_eq!(viem_diff_load_base(&request(VIEM_DIFF_BASE_FILE, "relative.txt"), &mut result), ViemStatus::InvalidArgument);
        assert_eq!(result, 0);
        assert_eq!(viem_diff_load_base(&request(99, "/absolute"), &mut result), ViemStatus::InvalidArgument);
        assert_eq!(viem_diff_load_base(ptr::null(), &mut result), ViemStatus::NullPointer);
    }
}

#[test]
fn a_cancelled_token_produces_a_cancelled_git_result() {
    let mut token = 0;
    let mut result = 0;
    unsafe {
        assert_eq!(viem_diff_cancellation_create(&mut token), ViemStatus::Ok);
        assert_eq!(viem_diff_cancellation_cancel(token), ViemStatus::Ok);
        let path = std::env::temp_dir().join("anything.txt").to_str().unwrap().to_owned();
        let mut cancelled = request(VIEM_DIFF_BASE_HEAD, &path);
        cancelled.cancellation = token;
        assert_eq!(viem_diff_load_base(&cancelled, &mut result), ViemStatus::Ok);
    }
    let summary = info(result);
    assert_eq!(summary.result, VIEM_DIFF_RESULT_CANCELLED);
    let mut required = 0;
    let mut message = vec![0u8; summary.message_bytes as usize];
    assert_eq!(unsafe { viem_diff_result_copy_message(result, message.as_mut_ptr(), message.len() as u64, &mut required) }, ViemStatus::Ok);
    assert!(String::from_utf8(message).unwrap().contains("cancelled"));
    assert_eq!(viem_diff_result_release(result), ViemStatus::Ok);
    assert_eq!(viem_diff_cancellation_release(token), ViemStatus::Ok);
    assert_eq!(viem_diff_cancellation_cancel(token), ViemStatus::InvalidHandle);
}

#[test]
fn saved_source_copies_the_last_saved_bytes() {
    let handle = register_core(Core::new(Document::from_bytes(b"saved\n".to_vec(), Encoding::Utf8, Format::Code).unwrap())).unwrap();
    let mut required = 0;
    let mut available = 0;
    unsafe {
        assert_eq!(viem_core_copy_saved_source(handle, ptr::null_mut(), 0, &mut required, &mut available), ViemStatus::BufferTooSmall);
        assert_eq!((required, available), (6, 1));
        let mut bytes = vec![0; 6];
        assert_eq!(viem_core_copy_saved_source(handle, bytes.as_mut_ptr(), 6, &mut required, &mut available), ViemStatus::Ok);
        assert_eq!(bytes, b"saved\n");
    }
    assert_eq!(viem_core_destroy(handle), ViemStatus::Ok);
}

#[test]
fn navigation_and_scrollbar_marks_cross_the_abi() {
    let mut storage = PaintTestProviderStorage::default();
    let (base, base_view) = code_core("a\nb\nc\nd\n", &mut storage, 901);
    let (new, new_view) = code_core("a\nb\nC\nd\n", &mut storage, 1001);
    let mut session = 0;
    unsafe {
        assert_eq!(viem_diff_session_create(base, new, VIEM_DIFF_SESSION_ALIGNED, &mut session), ViemStatus::Ok);
        let mut navigation = ViemDiffNavigation { ..Default::default() };
        assert_eq!(viem_diff_session_navigate(session, base, base_view.0, new, new_view.0, 7, &mut navigation),
            ViemStatus::InvalidArgument);
        assert_eq!(viem_diff_session_navigate(session, base, base_view.0, new, new_view.0,
            VIEM_DIFF_NAVIGATE_NEXT, &mut navigation), ViemStatus::Ok);
        assert_eq!(navigation.flags & VIEM_DIFF_NAVIGATION_FOUND, VIEM_DIFF_NAVIGATION_FOUND);
        assert_eq!((navigation.change_index, navigation.change_count, navigation.caret_line), (0, 1, 2));
        let (mut count, mut extent) = (0, 0.0);
        assert_eq!(viem_diff_session_scrollbar_marks(session, new, new_view.0, 1, ptr::null_mut(), 0, &mut count, &mut extent),
            ViemStatus::BufferTooSmall);
        assert_eq!(count, 1);
        assert!(extent > 0.0);
        let mut marks = vec![ViemDiffScrollbarMark::default(); count as usize];
        assert_eq!(viem_diff_session_scrollbar_marks(session, new, new_view.0, 1, marks.as_mut_ptr(), count, &mut count, &mut extent),
            ViemStatus::Ok);
        assert_eq!(marks[0].flags, VIEM_DIFF_MARK_ADDED);
        assert!(marks[0].bottom > marks[0].top);
        assert_eq!(viem_diff_session_destroy(session), ViemStatus::Ok);
    }
}
