use super::*;
use crate::ffi::*;

fn exported_bytes(handle: ViemHtmlExportHandle) -> Vec<u8> {
    let mut required = 0;
    unsafe {
        assert_eq!(
            viem_html_export_copy_utf8(handle, ptr::null_mut(), 0, &mut required),
            ViemStatus::BufferTooSmall
        );
        let mut bytes = vec![0; required as usize];
        assert_eq!(
            viem_html_export_copy_utf8(handle, bytes.as_mut_ptr(), required, &mut required),
            ViemStatus::Ok
        );
        bytes
    }
}

#[test]
fn prepared_html_export_survives_core_destruction_and_can_be_discarded() {
    let mut storage = PaintTestProviderStorage::default();
    let mut core = Core::new(Document::new("captured <text>"));
    let revision = core.document().revision().0;
    let view = core.add_view(paint_test_provider(&mut storage, 501, 601), 240.0, 480.0);
    let handle = register_core(core).unwrap();
    let mut prepared = 0;
    let mut discarded = 0;
    unsafe {
        assert_eq!(
            viem_core_prepare_html_export(handle, view.0, revision, &mut prepared),
            ViemStatus::Ok
        );
        assert_eq!(
            viem_core_prepare_html_export(handle, view.0, revision, &mut discarded),
            ViemStatus::Ok
        );
    }
    assert_eq!(viem_html_export_release(discarded), ViemStatus::Ok);
    assert_eq!(
        viem_html_export_render(discarded),
        ViemStatus::InvalidHandle
    );
    assert_eq!(viem_core_destroy(handle), ViemStatus::Ok);
    assert_eq!(viem_html_export_render(prepared), ViemStatus::Ok);
    assert!(String::from_utf8(exported_bytes(prepared))
        .unwrap()
        .contains("captured &lt;text&gt;"));
    assert_eq!(viem_html_export_release(prepared), ViemStatus::Ok);
}

#[test]
fn html_export_validates_pointers_identity_and_owned_result_lifetime() {
    let mut storage = PaintTestProviderStorage::default();
    let mut core = Core::new(
        Document::from_bytes(
            b"# Heading\n\n**bold** & text".to_vec(),
            Encoding::Utf8,
            Format::MarkdownSource,
        )
        .unwrap(),
    );
    let revision = core.document().revision().0;
    let mut provider = paint_test_provider(&mut storage, 501, 601);
    provider.threading = ProviderThreading::FrontendMainThread;
    let view = core.add_view(provider, 240.0, 480.0);
    let original = summarize_document_state(core.document());
    let handle = register_core(core).unwrap();
    let mut export = 99;
    unsafe {
        assert_eq!(
            viem_core_prepare_html_export(handle, view.0, revision, ptr::null_mut()),
            ViemStatus::NullPointer
        );
        let mut misaligned = [99u64; 2];
        assert_eq!(
            viem_core_prepare_html_export(
                handle,
                view.0,
                revision,
                misaligned.as_mut_ptr().cast::<u8>().add(1).cast()
            ),
            ViemStatus::InvalidArgument
        );
        assert_eq!(misaligned, [99; 2]);
        assert_eq!(
            viem_core_prepare_html_export(0, view.0, revision, &mut export),
            ViemStatus::InvalidHandle
        );
        assert_eq!(export, 0);
        assert_eq!(
            viem_core_prepare_html_export(handle, view.0, revision + 1, &mut export),
            ViemStatus::StaleRevision
        );
        assert_eq!(export, 0);
        assert_eq!(
            viem_core_prepare_html_export(handle, view.0 + 1, revision, &mut export),
            ViemStatus::InvalidView
        );
        assert_eq!(export, 0);
    }
    // Even a main-thread-only shaping provider is never needed by export.
    let calls = storage.responses.len();
    assert_eq!(
        unsafe { viem_core_prepare_html_export(handle, view.0, revision, &mut export) },
        ViemStatus::Ok
    );
    let mut required = 0;
    assert_eq!(
        unsafe { viem_html_export_copy_utf8(export, ptr::null_mut(), 0, &mut required) },
        ViemStatus::InvalidArgument
    );
    // Rendering must not touch a live core, even while it is checked out.
    let busy = checkout_core(handle).unwrap();
    std::thread::spawn(move || {
        assert_eq!(viem_html_export_render(export), ViemStatus::Ok);
        assert_eq!(viem_html_export_render(export), ViemStatus::InvalidArgument);
    })
    .join()
    .unwrap();
    drop(busy);
    assert_ne!(export, 0);
    assert_eq!(storage.responses.len(), calls);
    let initial = exported_bytes(export);
    let html = String::from_utf8(initial.clone()).unwrap();
    assert!(html.contains("<h1"));
    assert!(html.contains("&amp;"));
    assert!(!html.contains("**bold**"));
    {
        let mut core = checkout_core(handle).unwrap();
        assert_eq!(summarize_document_state(core.core().document()), original);
        core.core_mut()
            .handle(view, CoreEvent::Input(InputEvent::Key(Key::Char('i'))))
            .unwrap();
        core.core_mut()
            .handle(view, CoreEvent::Input(InputEvent::Text("new ".into())))
            .unwrap();
    }
    assert_eq!(exported_bytes(export), initial);
    assert_eq!(viem_core_destroy(handle), ViemStatus::Ok);
    assert_eq!(exported_bytes(export), initial);
    unsafe {
        let mut required = 999;
        let mut short = [0xa5; 4];
        assert_eq!(
            viem_html_export_copy_utf8(export, short.as_mut_ptr(), 4, &mut required),
            ViemStatus::BufferTooSmall
        );
        assert_eq!(short, [0xa5; 4]);
        assert_eq!(required as usize, initial.len());
        assert_eq!(
            viem_html_export_copy_utf8(export, ptr::null_mut(), 1, &mut required),
            ViemStatus::NullPointer
        );
        assert_eq!(
            viem_html_export_copy_utf8(export, ptr::null_mut(), 0, ptr::null_mut()),
            ViemStatus::NullPointer
        );
        let alias = &mut required as *mut u64;
        required = 999;
        assert_eq!(
            viem_html_export_copy_utf8(export, alias.cast(), 8, alias),
            ViemStatus::InvalidArgument
        );
        assert_eq!(required, 999);
    }
    assert_eq!(viem_html_export_release(export), ViemStatus::Ok);
    assert_eq!(viem_html_export_release(export), ViemStatus::InvalidHandle);
    assert_eq!(viem_html_export_release(0), ViemStatus::InvalidHandle);
    let mut required = 999;
    assert_eq!(
        unsafe { viem_html_export_copy_utf8(export, ptr::null_mut(), 0, &mut required) },
        ViemStatus::InvalidHandle
    );
    assert_eq!(required, 0);
}
