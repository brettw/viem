use viem_core::ffi::{
    viem_core_abi_version, viem_core_copy_formatted_utf8, viem_core_copy_source_bytes,
    viem_core_create, viem_core_destroy,
    viem_core_revision, ViemCoreHandle, ViemDocumentOptions, ViemStatus,
    VIEM_CORE_ABI_VERSION, VIEM_DOCUMENT_OPTIONS_SIZE, VIEM_ENCODING_DETECT, VIEM_ENCODING_LATIN1,
    VIEM_ENCODING_UTF16_BE, VIEM_ENCODING_UTF16_LE, VIEM_ENCODING_UTF8, VIEM_FILE_FORMAT_DETECT,
    VIEM_FILE_FORMAT_DOS, VIEM_FILE_FORMAT_MAC, VIEM_FILE_FORMAT_UNIX, VIEM_FORMAT_MARKDOWN,
    VIEM_FORMAT_PLAIN_TEXT,
};
use std::ptr;

struct TestCore {
    handle: ViemCoreHandle,
    revision: u64,
}

impl Drop for TestCore {
    fn drop(&mut self) {
        if self.handle != 0 {
            let _ = viem_core_destroy(self.handle);
        }
    }
}

fn create(source: &[u8], options: ViemDocumentOptions) -> TestCore {
    let mut handle = 0;
    let mut revision = u64::MAX;
    // SAFETY: Every pointer is derived from a live Rust allocation/value and
    // remains valid for the complete synchronous call. Outputs are distinct.
    let status = unsafe {
        viem_core_create(
            source.as_ptr(),
            source.len() as u64,
            &options,
            &mut handle,
            &mut revision,
        )
    };
    assert_eq!(status, ViemStatus::Ok);
    assert_ne!(handle, 0);
    TestCore { handle, revision }
}

type CopyFunction =
    unsafe extern "C" fn(ViemCoreHandle, u64, *mut u8, u64, *mut u64) -> ViemStatus;

fn copy_bytes(function: CopyFunction, document: &TestCore, revision: u64) -> Vec<u8> {
    let mut required = u64::MAX;
    // SAFETY: `out_required` is writable. A null/zero destination is the
    // documented length-query form.
    let query = unsafe { function(document.handle, revision, ptr::null_mut(), 0, &mut required) };
    let expected_query = if required == 0 {
        ViemStatus::Ok
    } else {
        ViemStatus::BufferTooSmall
    };
    assert_eq!(query, expected_query);

    let mut result = vec![0; required as usize];
    let output = if result.is_empty() {
        ptr::null_mut()
    } else {
        result.as_mut_ptr()
    };
    let mut second_required = u64::MAX;
    // SAFETY: `result` provides exactly `required` writable bytes and the
    // length output is a distinct live value.
    let status = unsafe {
        function(
            document.handle,
            revision,
            output,
            result.len() as u64,
            &mut second_required,
        )
    };
    assert_eq!(status, ViemStatus::Ok);
    assert_eq!(second_required, required);
    result
}

fn source_bytes(document: &TestCore, revision: u64) -> Vec<u8> {
    copy_bytes(viem_core_copy_source_bytes, document, revision)
}

fn formatted_utf8(document: &TestCore, revision: u64) -> Vec<u8> {
    copy_bytes(viem_core_copy_formatted_utf8, document, revision)
}

#[test]
fn abi_create_validates_pointers_and_every_option_domain() {
    assert_eq!(viem_core_abi_version(), VIEM_CORE_ABI_VERSION);
    assert_eq!(VIEM_DOCUMENT_OPTIONS_SIZE, 16);

    let options = ViemDocumentOptions::default();
    let mut handle = 99;
    let mut revision = 99;
    // SAFETY: Options and outputs are valid; null with nonzero input length is
    // intentional and must be rejected before dereferencing.
    let status =
        unsafe { viem_core_create(ptr::null(), 1, &options, &mut handle, &mut revision) };
    assert_eq!(status, ViemStatus::NullPointer);
    // Invalid pointer relations are rejected before touching caller outputs.
    assert_eq!((handle, revision), (99, 99));

    // SAFETY: Outputs are live; the null options pointer is intentionally
    // invalid and must be checked before it is read.
    let status =
        unsafe { viem_core_create(ptr::null(), 0, ptr::null(), &mut handle, &mut revision) };
    assert_eq!(status, ViemStatus::NullPointer);

    for (mut invalid, expected) in [
        (
            ViemDocumentOptions {
                struct_size: VIEM_DOCUMENT_OPTIONS_SIZE - 1,
                ..options
            },
            ViemStatus::InvalidArgument,
        ),
        (
            ViemDocumentOptions {
                encoding: 99,
                ..options
            },
            ViemStatus::InvalidEncoding,
        ),
        (
            ViemDocumentOptions {
                format: 99,
                ..options
            },
            ViemStatus::InvalidFormat,
        ),
        (
            ViemDocumentOptions {
                file_format: 99,
                ..options
            },
            ViemStatus::InvalidFileFormat,
        ),
    ] {
        handle = 99;
        revision = 99;
        // Keep the value mutable to demonstrate that the implementation only
        // reads the versioned options prefix.
        let options_pointer = &mut invalid as *mut ViemDocumentOptions;
        // SAFETY: All pointers refer to live values. Each options value is
        // structurally readable but contains one intentionally invalid field.
        let status = unsafe {
            viem_core_create(ptr::null(), 0, options_pointer, &mut handle, &mut revision)
        };
        assert_eq!(status, expected);
        assert_eq!((handle, revision), (0, 0));
    }

    let empty = create(&[], options);
    assert_eq!(empty.revision, 0);
    assert!(source_bytes(&empty, 0).is_empty());
    assert!(formatted_utf8(&empty, 0).is_empty());
}

#[test]
fn automatic_encoding_choice_is_shared_by_document_creation_and_is_byte_exact() {
    let options = ViemDocumentOptions {
        encoding: VIEM_ENCODING_DETECT,
        file_format: VIEM_FILE_FORMAT_UNIX,
        ..ViemDocumentOptions::default()
    };
    let utf16_le = [0xff, 0xfe, b'A', 0, 0x3d, 0xd8, 0x00, 0xde];
    let detected_utf16 = create(&utf16_le, options);
    assert_eq!(source_bytes(&detected_utf16, 0), utf16_le);
    assert_eq!(formatted_utf8(&detected_utf16, 0), "A😀".as_bytes());

    let malformed_utf8 = [b'a', 0xf0, 0x28, 0x8c, 0x28];
    let detected_latin1 = create(&malformed_utf8, options);
    assert_eq!(source_bytes(&detected_latin1, 0), malformed_utf8);
    assert_eq!(formatted_utf8(&detected_latin1, 0), "að(\u{8c}(".as_bytes());
}

#[test]
fn snapshot_copy_is_revision_tagged_two_pass_and_length_delimited() {
    let source = b"a\0b\r\n";
    let document = create(
        source,
        ViemDocumentOptions {
            file_format: VIEM_FILE_FORMAT_DOS,
            ..ViemDocumentOptions::default()
        },
    );

    let mut required = u64::MAX;
    let mut short = [0xcc; 3];
    // SAFETY: `short` and `required` are distinct writable regions. The
    // deliberately short capacity must be reported without a partial copy.
    let status = unsafe {
        viem_core_copy_source_bytes(
            document.handle,
            document.revision,
            short.as_mut_ptr(),
            short.len() as u64,
            &mut required,
        )
    };
    assert_eq!(status, ViemStatus::BufferTooSmall);
    assert_eq!(required, source.len() as u64);
    assert_eq!(short, [0xcc; 3]);

    assert_eq!(source_bytes(&document, 0), source);
    assert_eq!(formatted_utf8(&document, 0), b"a\0b\n");

    let mut oversized = vec![0xdd; source.len() + 1];
    required = 99;
    // SAFETY: The destination has one byte more capacity than the source and
    // is distinct from the required-length output.
    let status = unsafe {
        viem_core_copy_source_bytes(
            document.handle,
            0,
            oversized.as_mut_ptr(),
            oversized.len() as u64,
            &mut required,
        )
    };
    assert_eq!(status, ViemStatus::Ok);
    assert_eq!(&oversized[..source.len()], source);
    assert_eq!(oversized[source.len()], 0xdd, "the ABI must not append NUL");

    required = 99;
    // SAFETY: This is a valid length query with a deliberately stale
    // revision. The required-length output remains writable.
    let status = unsafe {
        viem_core_copy_formatted_utf8(document.handle, 41, ptr::null_mut(), 0, &mut required)
    };
    assert_eq!(status, ViemStatus::StaleRevision);
    assert_eq!(required, 0);

    // SAFETY: Null with a positive output capacity is intentionally invalid;
    // the required-length pointer is valid and distinct.
    let status = unsafe {
        viem_core_copy_source_bytes(document.handle, 0, ptr::null_mut(), 1, &mut required)
    };
    assert_eq!(status, ViemStatus::NullPointer);

    // SAFETY: This intentionally omits the required-length output, which must
    // be rejected before any output write.
    let status = unsafe {
        viem_core_copy_source_bytes(document.handle, 0, ptr::null_mut(), 0, ptr::null_mut())
    };
    assert_eq!(status, ViemStatus::NullPointer);
}

#[test]
fn latin1_utf16_and_markdown_sources_round_trip_exactly() {
    let latin1_source = b"caf\xe9\r\n";
    let latin1 = create(
        latin1_source,
        ViemDocumentOptions {
            encoding: VIEM_ENCODING_LATIN1,
            file_format: VIEM_FILE_FORMAT_DOS,
            ..ViemDocumentOptions::default()
        },
    );
    assert_eq!(source_bytes(&latin1, 0), latin1_source);
    assert_eq!(formatted_utf8(&latin1, 0), "café\n".as_bytes());

    let utf16_le_source = [
        0xff, 0xfe, // BOM
        b'A', 0x00, // A
        b'\r', 0x00, // Mac logical break
        b'B', 0x00, // B
    ];
    let utf16_le = create(
        &utf16_le_source,
        ViemDocumentOptions {
            encoding: VIEM_ENCODING_UTF16_LE,
            file_format: VIEM_FILE_FORMAT_MAC,
            ..ViemDocumentOptions::default()
        },
    );
    assert_eq!(source_bytes(&utf16_le, 0), utf16_le_source);
    assert_eq!(formatted_utf8(&utf16_le, 0), b"A\nB");

    let markdown = "# **é**\n";
    let mut utf16_be_source = vec![0xfe, 0xff];
    for unit in markdown.encode_utf16() {
        utf16_be_source.extend_from_slice(&unit.to_be_bytes());
    }
    let utf16_be_markdown = create(
        &utf16_be_source,
        ViemDocumentOptions {
            encoding: VIEM_ENCODING_UTF16_BE,
            format: VIEM_FORMAT_MARKDOWN,
            file_format: VIEM_FILE_FORMAT_UNIX,
            ..ViemDocumentOptions::default()
        },
    );
    assert_eq!(source_bytes(&utf16_be_markdown, 0), utf16_be_source);
    assert_eq!(formatted_utf8(&utf16_be_markdown, 0), "é".as_bytes());

    let detected_markdown_source = b"# title\r\nbody\r\n";
    let detected_markdown = create(
        detected_markdown_source,
        ViemDocumentOptions {
            encoding: VIEM_ENCODING_UTF8,
            format: VIEM_FORMAT_MARKDOWN,
            file_format: VIEM_FILE_FORMAT_DETECT,
            ..ViemDocumentOptions::default()
        },
    );
    assert_eq!(
        source_bytes(&detected_markdown, 0),
        detected_markdown_source
    );
    assert_eq!(formatted_utf8(&detected_markdown, 0), b"title\nbody");
}

#[test]
fn malformed_source_utf8_remains_opaque_on_the_core_surface() {
    let source = [b'a', 0xff, b'b'];
    let document = create(&source, ViemDocumentOptions::default());
    assert_eq!(source_bytes(&document, 0), source);
    assert_eq!(formatted_utf8(&document, 0), "a\u{fffd}b".as_bytes());


}

#[test]
fn destroying_a_handle_is_final_and_detected_by_every_operation() {
    let mut document = create(b"text", ViemDocumentOptions::default());
    let handle = document.handle;
    assert_eq!(viem_core_destroy(handle), ViemStatus::Ok);
    document.handle = 0;
    assert_eq!(viem_core_destroy(handle), ViemStatus::InvalidHandle);
    assert_eq!(viem_core_destroy(0), ViemStatus::InvalidHandle);

    let mut revision = 99;
    // SAFETY: The output is valid; the destroyed handle is intentionally
    // invalid and must be rejected without accessing freed document state.
    let status = unsafe { viem_core_revision(handle, &mut revision) };
    assert_eq!(status, ViemStatus::InvalidHandle);
    assert_eq!(revision, 0);

    let mut required = 99;
    // SAFETY: This is a valid length query against an intentionally destroyed
    // handle. The output remains writable.
    let status =
        unsafe { viem_core_copy_source_bytes(handle, 0, ptr::null_mut(), 0, &mut required) };
    assert_eq!(status, ViemStatus::InvalidHandle);
    assert_eq!(required, 0);


}

#[test]
fn explicit_format_constants_remain_disjoint() {
    // These values are serialized in C structs; this test guards accidental
    // aliases even though Rust's domain types are distinct enums.
    assert_ne!(VIEM_FORMAT_PLAIN_TEXT, VIEM_FORMAT_MARKDOWN);
    assert_ne!(VIEM_FILE_FORMAT_UNIX, VIEM_FILE_FORMAT_DOS);
    assert_ne!(VIEM_FILE_FORMAT_DOS, VIEM_FILE_FORMAT_MAC);
}
