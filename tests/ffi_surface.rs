use evim_core::ffi::{
    evim_core_abi_version, evim_document_copy_formatted_utf8, evim_document_copy_source_bytes,
    evim_document_create, evim_document_destroy, evim_document_replace_formatted_utf8,
    evim_document_revision, EvimDocumentHandle, EvimDocumentOptions, EvimStatus,
    EVIM_CORE_ABI_VERSION, EVIM_DOCUMENT_OPTIONS_SIZE, EVIM_ENCODING_DETECT, EVIM_ENCODING_LATIN1,
    EVIM_ENCODING_UTF16_BE, EVIM_ENCODING_UTF16_LE, EVIM_ENCODING_UTF8, EVIM_FILE_FORMAT_DETECT,
    EVIM_FILE_FORMAT_DOS, EVIM_FILE_FORMAT_MAC, EVIM_FILE_FORMAT_UNIX, EVIM_FORMAT_MARKDOWN,
    EVIM_FORMAT_PLAIN_TEXT,
};
use std::ptr;

struct TestDocument {
    handle: EvimDocumentHandle,
    revision: u64,
}

impl Drop for TestDocument {
    fn drop(&mut self) {
        if self.handle != 0 {
            let _ = evim_document_destroy(self.handle);
        }
    }
}

fn create(source: &[u8], options: EvimDocumentOptions) -> TestDocument {
    let mut handle = 0;
    let mut revision = u64::MAX;
    // SAFETY: Every pointer is derived from a live Rust allocation/value and
    // remains valid for the complete synchronous call. Outputs are distinct.
    let status = unsafe {
        evim_document_create(
            source.as_ptr(),
            source.len() as u64,
            &options,
            &mut handle,
            &mut revision,
        )
    };
    assert_eq!(status, EvimStatus::Ok);
    assert_ne!(handle, 0);
    TestDocument { handle, revision }
}

type CopyFunction =
    unsafe extern "C" fn(EvimDocumentHandle, u64, *mut u8, u64, *mut u64) -> EvimStatus;

fn copy_bytes(function: CopyFunction, document: &TestDocument, revision: u64) -> Vec<u8> {
    let mut required = u64::MAX;
    // SAFETY: `out_required` is writable. A null/zero destination is the
    // documented length-query form.
    let query = unsafe { function(document.handle, revision, ptr::null_mut(), 0, &mut required) };
    let expected_query = if required == 0 {
        EvimStatus::Ok
    } else {
        EvimStatus::BufferTooSmall
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
    assert_eq!(status, EvimStatus::Ok);
    assert_eq!(second_required, required);
    result
}

fn source_bytes(document: &TestDocument, revision: u64) -> Vec<u8> {
    copy_bytes(evim_document_copy_source_bytes, document, revision)
}

fn formatted_utf8(document: &TestDocument, revision: u64) -> Vec<u8> {
    copy_bytes(evim_document_copy_formatted_utf8, document, revision)
}

fn replace(
    document: &TestDocument,
    revision: u64,
    range: std::ops::Range<u64>,
    replacement: &[u8],
) -> (EvimStatus, u64) {
    let mut new_revision = u64::MAX;
    // SAFETY: The replacement slice is readable for its declared length and
    // the revision output is a distinct writable value.
    let status = unsafe {
        evim_document_replace_formatted_utf8(
            document.handle,
            revision,
            range.start,
            range.end,
            replacement.as_ptr(),
            replacement.len() as u64,
            &mut new_revision,
        )
    };
    (status, new_revision)
}

#[test]
fn abi_create_validates_pointers_and_every_option_domain() {
    assert_eq!(evim_core_abi_version(), EVIM_CORE_ABI_VERSION);
    assert_eq!(EVIM_DOCUMENT_OPTIONS_SIZE, 16);

    let options = EvimDocumentOptions::default();
    let mut handle = 99;
    let mut revision = 99;
    // SAFETY: Options and outputs are valid; null with nonzero input length is
    // intentional and must be rejected before dereferencing.
    let status =
        unsafe { evim_document_create(ptr::null(), 1, &options, &mut handle, &mut revision) };
    assert_eq!(status, EvimStatus::NullPointer);
    assert_eq!((handle, revision), (0, 0));

    // SAFETY: Outputs are live; the null options pointer is intentionally
    // invalid and must be checked before it is read.
    let status =
        unsafe { evim_document_create(ptr::null(), 0, ptr::null(), &mut handle, &mut revision) };
    assert_eq!(status, EvimStatus::NullPointer);

    for (mut invalid, expected) in [
        (
            EvimDocumentOptions {
                struct_size: EVIM_DOCUMENT_OPTIONS_SIZE - 1,
                ..options
            },
            EvimStatus::InvalidArgument,
        ),
        (
            EvimDocumentOptions {
                encoding: 99,
                ..options
            },
            EvimStatus::InvalidEncoding,
        ),
        (
            EvimDocumentOptions {
                format: 99,
                ..options
            },
            EvimStatus::InvalidFormat,
        ),
        (
            EvimDocumentOptions {
                file_format: 99,
                ..options
            },
            EvimStatus::InvalidFileFormat,
        ),
    ] {
        handle = 99;
        revision = 99;
        // Keep the value mutable to demonstrate that the implementation only
        // reads the versioned options prefix.
        let options_pointer = &mut invalid as *mut EvimDocumentOptions;
        // SAFETY: All pointers refer to live values. Each options value is
        // structurally readable but contains one intentionally invalid field.
        let status = unsafe {
            evim_document_create(ptr::null(), 0, options_pointer, &mut handle, &mut revision)
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
    let options = EvimDocumentOptions {
        encoding: EVIM_ENCODING_DETECT,
        file_format: EVIM_FILE_FORMAT_UNIX,
        ..EvimDocumentOptions::default()
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
        EvimDocumentOptions {
            file_format: EVIM_FILE_FORMAT_DOS,
            ..EvimDocumentOptions::default()
        },
    );

    let mut required = u64::MAX;
    let mut short = [0xcc; 3];
    // SAFETY: `short` and `required` are distinct writable regions. The
    // deliberately short capacity must be reported without a partial copy.
    let status = unsafe {
        evim_document_copy_source_bytes(
            document.handle,
            document.revision,
            short.as_mut_ptr(),
            short.len() as u64,
            &mut required,
        )
    };
    assert_eq!(status, EvimStatus::BufferTooSmall);
    assert_eq!(required, source.len() as u64);
    assert_eq!(short, [0xcc; 3]);

    assert_eq!(source_bytes(&document, 0), source);
    assert_eq!(formatted_utf8(&document, 0), b"a\0b\n");

    let mut oversized = vec![0xdd; source.len() + 1];
    required = 99;
    // SAFETY: The destination has one byte more capacity than the source and
    // is distinct from the required-length output.
    let status = unsafe {
        evim_document_copy_source_bytes(
            document.handle,
            0,
            oversized.as_mut_ptr(),
            oversized.len() as u64,
            &mut required,
        )
    };
    assert_eq!(status, EvimStatus::Ok);
    assert_eq!(&oversized[..source.len()], source);
    assert_eq!(oversized[source.len()], 0xdd, "the ABI must not append NUL");

    required = 99;
    // SAFETY: This is a valid length query with a deliberately stale
    // revision. The required-length output remains writable.
    let status = unsafe {
        evim_document_copy_formatted_utf8(document.handle, 41, ptr::null_mut(), 0, &mut required)
    };
    assert_eq!(status, EvimStatus::StaleRevision);
    assert_eq!(required, 0);

    // SAFETY: Null with a positive output capacity is intentionally invalid;
    // the required-length pointer is valid and distinct.
    let status = unsafe {
        evim_document_copy_source_bytes(document.handle, 0, ptr::null_mut(), 1, &mut required)
    };
    assert_eq!(status, EvimStatus::NullPointer);

    // SAFETY: This intentionally omits the required-length output, which must
    // be rejected before any output write.
    let status = unsafe {
        evim_document_copy_source_bytes(document.handle, 0, ptr::null_mut(), 0, ptr::null_mut())
    };
    assert_eq!(status, EvimStatus::NullPointer);
}

#[test]
fn replacement_rejects_stale_revisions_invalid_utf8_and_grapheme_splits() {
    let document = create(b"ae\xcc\x81z", EvimDocumentOptions::default());
    assert_eq!(formatted_utf8(&document, 0), "ae\u{301}z".as_bytes());

    let (status, revision) = replace(&document, 0, 2..2, b"x");
    assert_eq!(status, EvimStatus::NotGraphemeBoundary);
    assert_eq!(revision, 0);

    let inverted_start = 4;
    let inverted_end = 1;
    let (status, revision) = replace(&document, 0, inverted_start..inverted_end, b"x");
    assert_eq!(status, EvimStatus::InvalidRange);
    assert_eq!(revision, 0);

    let (status, revision) = replace(&document, 0, 1..4, &[0xff]);
    assert_eq!(status, EvimStatus::InvalidUtf8);
    assert_eq!(revision, 0);

    let mut null_input_revision = 99;
    // SAFETY: Null with a nonzero replacement length is intentionally invalid;
    // the revision output itself is live and writable.
    let status = unsafe {
        evim_document_replace_formatted_utf8(
            document.handle,
            0,
            1,
            4,
            ptr::null(),
            1,
            &mut null_input_revision,
        )
    };
    assert_eq!(status, EvimStatus::NullPointer);
    assert_eq!(null_input_revision, 0);

    let (status, revision) = replace(&document, 0, 1..4, "ø".as_bytes());
    assert_eq!(status, EvimStatus::Ok);
    assert_eq!(revision, 1);
    assert_eq!(source_bytes(&document, revision), "aøz".as_bytes());

    let (status, returned_revision) = replace(&document, 0, 1..1, b"stale");
    assert_eq!(status, EvimStatus::StaleRevision);
    assert_eq!(returned_revision, 0);
    assert_eq!(source_bytes(&document, revision), "aøz".as_bytes());

    let mut queried_revision = 99;
    // SAFETY: The output points to a live writable value.
    let status = unsafe { evim_document_revision(document.handle, &mut queried_revision) };
    assert_eq!(status, EvimStatus::Ok);
    assert_eq!(queried_revision, revision);
}

#[test]
fn latin1_utf16_and_markdown_sources_round_trip_exactly() {
    let latin1_source = b"caf\xe9\r\n";
    let latin1 = create(
        latin1_source,
        EvimDocumentOptions {
            encoding: EVIM_ENCODING_LATIN1,
            file_format: EVIM_FILE_FORMAT_DOS,
            ..EvimDocumentOptions::default()
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
        EvimDocumentOptions {
            encoding: EVIM_ENCODING_UTF16_LE,
            file_format: EVIM_FILE_FORMAT_MAC,
            ..EvimDocumentOptions::default()
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
        EvimDocumentOptions {
            encoding: EVIM_ENCODING_UTF16_BE,
            format: EVIM_FORMAT_MARKDOWN,
            file_format: EVIM_FILE_FORMAT_UNIX,
            ..EvimDocumentOptions::default()
        },
    );
    assert_eq!(source_bytes(&utf16_be_markdown, 0), utf16_be_source);
    assert_eq!(formatted_utf8(&utf16_be_markdown, 0), "é".as_bytes());

    let detected_markdown_source = b"# title\r\nbody\r\n";
    let detected_markdown = create(
        detected_markdown_source,
        EvimDocumentOptions {
            encoding: EVIM_ENCODING_UTF8,
            format: EVIM_FORMAT_MARKDOWN,
            file_format: EVIM_FILE_FORMAT_DETECT,
            ..EvimDocumentOptions::default()
        },
    );
    assert_eq!(
        source_bytes(&detected_markdown, 0),
        detected_markdown_source
    );
    assert_eq!(formatted_utf8(&detected_markdown, 0), b"title\nbody");
}

#[test]
fn malformed_source_utf8_remains_opaque_while_replacement_utf8_is_strict() {
    let source = [b'a', 0xff, b'b'];
    let document = create(&source, EvimDocumentOptions::default());
    assert_eq!(source_bytes(&document, 0), source);
    assert_eq!(formatted_utf8(&document, 0), "a\u{fffd}b".as_bytes());

    let (status, revision) = replace(&document, 0, 1..4, &[0xff]);
    assert_eq!(status, EvimStatus::InvalidUtf8);
    assert_eq!(revision, 0);
    assert_eq!(source_bytes(&document, 0), source);
}

#[test]
fn destroying_a_handle_is_final_and_detected_by_every_operation() {
    let mut document = create(b"text", EvimDocumentOptions::default());
    let handle = document.handle;
    assert_eq!(evim_document_destroy(handle), EvimStatus::Ok);
    document.handle = 0;
    assert_eq!(evim_document_destroy(handle), EvimStatus::InvalidHandle);
    assert_eq!(evim_document_destroy(0), EvimStatus::InvalidHandle);

    let mut revision = 99;
    // SAFETY: The output is valid; the destroyed handle is intentionally
    // invalid and must be rejected without accessing freed document state.
    let status = unsafe { evim_document_revision(handle, &mut revision) };
    assert_eq!(status, EvimStatus::InvalidHandle);
    assert_eq!(revision, 0);

    let mut required = 99;
    // SAFETY: This is a valid length query against an intentionally destroyed
    // handle. The output remains writable.
    let status =
        unsafe { evim_document_copy_source_bytes(handle, 0, ptr::null_mut(), 0, &mut required) };
    assert_eq!(status, EvimStatus::InvalidHandle);
    assert_eq!(required, 0);

    let mut new_revision = 99;
    // SAFETY: Empty replacement input requires no readable pointer. The
    // revision output is live; only the handle is intentionally invalid.
    let status = unsafe {
        evim_document_replace_formatted_utf8(handle, 0, 0, 0, ptr::null(), 0, &mut new_revision)
    };
    assert_eq!(status, EvimStatus::InvalidHandle);
    assert_eq!(new_revision, 0);
}

#[test]
fn concurrent_writers_get_one_serial_turn_without_holding_a_model_lock() {
    let mut document = create(b"x", EvimDocumentOptions::default());
    let handle = document.handle;
    let first = std::thread::spawn(move || {
        let mut revision = 99;
        // SAFETY: The static replacement byte and stack output remain valid
        // for the synchronous call. `handle` is a copyable opaque token.
        let status = unsafe {
            evim_document_replace_formatted_utf8(handle, 0, 0, 1, b"a".as_ptr(), 1, &mut revision)
        };
        (status, revision)
    });
    let second = std::thread::spawn(move || {
        let mut revision = 99;
        // SAFETY: The static replacement byte and stack output remain valid
        // for the synchronous call. `handle` is a copyable opaque token.
        let status = unsafe {
            evim_document_replace_formatted_utf8(handle, 0, 0, 1, b"b".as_ptr(), 1, &mut revision)
        };
        (status, revision)
    });

    let outcomes = [first.join().unwrap(), second.join().unwrap()];
    assert_eq!(
        outcomes
            .iter()
            .filter(|(status, _)| *status == EvimStatus::Ok)
            .count(),
        1
    );
    assert_eq!(
        outcomes
            .iter()
            .filter(|(status, revision)| {
                matches!(status, EvimStatus::StaleRevision | EvimStatus::DocumentBusy)
                    && *revision == 0
            })
            .count(),
        1
    );

    document.revision = 1;
    let content = formatted_utf8(&document, 1);
    assert!(content == b"a" || content == b"b");
}

#[test]
fn explicit_format_constants_remain_disjoint() {
    // These values are serialized in C structs; this test guards accidental
    // aliases even though Rust's domain types are distinct enums.
    assert_ne!(EVIM_FORMAT_PLAIN_TEXT, EVIM_FORMAT_MARKDOWN);
    assert_ne!(EVIM_FILE_FORMAT_UNIX, EVIM_FILE_FORMAT_DOS);
    assert_ne!(EVIM_FILE_FORMAT_DOS, EVIM_FILE_FORMAT_MAC);
}
