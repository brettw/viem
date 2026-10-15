//! Isolated large-file memory probe. See docs/performance.md.
//! Arguments: plain|code|markdown lines|short|long|markdown|richpaste|richpaste-list SOURCE_BYTES
//! [open|document|views|switch|flat|exercise|search|richpaste] [utf8|utf16le|utf16be|latin1].
//! Supplemental shapes include crlf and invalid (all invalid UTF-8 bytes). The
//! markdown shape repeats rich blocks; `switch` turns Markdown Preview off (Code
//! with the Markdown language) and back on.
//! `richpaste` pastes a whitespace-rich fragment into cold dense Markdown,
//! reports the operation's additional heap/work, then exercises undo and redo.
//! Run each case in a fresh release process; syntax providers are disabled.
use serde_json::{json, Value};
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;
use viem_core::command::{CommandInterpreter, CommandStatus, InputEvent, Key};
use viem_core::command::clipboard::{ClipboardCommandContext, ClipboardContent, ClipboardGeneration, ClipboardSnapshot, ClipboardTarget};
use viem_core::document::syntax::detection::LanguageSelection;
use viem_core::layout::MockTextMeasurementProvider;
use viem_core::{Core, CoreEvent, Document, Encoding, Format};
use viem_core::document::{ClipboardFragment, FileFormat, HistoryRetentionPolicy, measure_document_work};

struct TrackingAllocator;
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
static LIVE_COUNT: AtomicUsize = AtomicUsize::new(0);
static TOTAL_COUNT: AtomicUsize = AtomicUsize::new(0);

fn added(bytes: usize) {
    let live = LIVE.fetch_add(bytes, Ordering::Relaxed) + bytes;
    PEAK.fetch_max(live, Ordering::Relaxed);
}

unsafe impl GlobalAlloc for TrackingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = System.alloc(layout);
        if !ptr.is_null() {
            added(layout.size());
            LIVE_COUNT.fetch_add(1, Ordering::Relaxed);
            TOTAL_COUNT.fetch_add(1, Ordering::Relaxed);
        }
        ptr
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let ptr = System.alloc_zeroed(layout);
        if !ptr.is_null() {
            added(layout.size());
            LIVE_COUNT.fetch_add(1, Ordering::Relaxed);
            TOTAL_COUNT.fetch_add(1, Ordering::Relaxed);
        }
        ptr
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
        LIVE_COUNT.fetch_sub(1, Ordering::Relaxed);
        System.dealloc(ptr, layout);
    }
    unsafe fn realloc(&self, ptr: *mut u8, old: Layout, size: usize) -> *mut u8 {
        let result = System.realloc(ptr, old, size);
        if !result.is_null() {
            TOTAL_COUNT.fetch_add(1, Ordering::Relaxed);
            if size >= old.size() {
                added(size - old.size());
            } else {
                LIVE.fetch_sub(old.size() - size, Ordering::Relaxed);
            }
        }
        result
    }
}

#[global_allocator]
static ALLOCATOR: TrackingAllocator = TrackingAllocator;

#[cfg(target_os = "macos")]
fn process_memory() -> Value {
    // TASK_VM_INFO's prefix is defined by the macOS SDK mach/task_info.h.
    // Use an aligned oversized buffer and check the returned natural_t count.
    unsafe extern "C" {
        static mach_task_self_: u32;
        fn task_info(task: u32, flavor: u32, info: *mut u32, count: *mut u32) -> i32;
    }
    let mut data = [0_u64; 128];
    let mut count = (std::mem::size_of_val(&data) / 4) as u32;
    let result = unsafe { task_info(mach_task_self_, 22, data.as_mut_ptr().cast(), &mut count) };
    if result != 0 || count < 38 {
        return json!({"unavailable": result, "returned_words": count});
    }
    json!({
        "rss_bytes": data[2], "peak_rss_bytes": data[3],
        "compressed_bytes": data[15], "physical_footprint_bytes": data[18],
        "peak_physical_footprint_bytes": if count >= 44 { Some(data[21]) } else { None },
    })
}

#[cfg(not(target_os = "macos"))]
fn process_memory() -> Value {
    Value::Null
}

fn sample(phase: &str, started: Instant, document: Option<&Document>) {
    // Capture requested heap before allocating the JSON/reporting values.
    let live = LIVE.load(Ordering::Relaxed);
    let peak = PEAK.load(Ordering::Relaxed);
    let live_count = LIVE_COUNT.load(Ordering::Relaxed);
    let total_count = TOTAL_COUNT.load(Ordering::Relaxed);
    let process = process_memory();
    let status = document.map(Document::history_status);
    println!(
        "{}",
        json!({
            "phase": phase, "elapsed_ms": started.elapsed().as_secs_f64() * 1000.,
            "live_requested_heap_bytes": live, "peak_requested_heap_bytes": peak,
            "live_allocations": live_count, "total_allocations_and_reallocations": total_count,
            "process": process,
            "history_estimate_bytes": status.as_ref().map(|s| s.retained_memory_bytes),
            "history_live_state_bytes": status.as_ref().map(|s| s.live_state_memory_bytes),
            "history_additional_bytes": status.as_ref().map(|s| s.additional_history_memory_bytes),
            "history_nodes": status.as_ref().map(|s| s.node_count),
            "can_undo": status.as_ref().map(|s| s.can_undo),
            "hard_lines": document.map(|d| d.projection().hard_line_count()),
        })
    );
}

fn rich_paste(mut document: Document, original: Vec<u8>, list: bool, started: Instant) {
    // Use a source-backed fragment whose leading whitespace needs local
    // protection while the later bold text retains its character treatment.
    let donor_bytes = "line  \r\n  \r\nafter **double** space"
        .encode_utf16().flat_map(u16::to_le_bytes).collect();
    let donor = Document::from_bytes_with_file_format(
        donor_bytes, Encoding::Utf16Le, Format::Markdown, FileFormat::Dos,
    ).unwrap();
    let copied = "  \nafter double spac";
    let fragment = donor.clipboard_fragment(4..4 + copied.len()).unwrap();
    let fragment = ClipboardFragment::from_json(fragment.json(), copied).unwrap();
    let register = viem_core::command::RegisterValue::from_clipboard_fragment(fragment).unwrap();
    let clipboard = ClipboardCommandContext::new().with_read(ClipboardSnapshot::new(
        ClipboardTarget::Clipboard, ClipboardGeneration(1), ClipboardContent::from_register(register),
    ));
    drop(donor);
    let mut commands = CommandInterpreter::new();
    commands.handle(&mut document, InputEvent::key('i')).unwrap();
    let at = if list { 6 } else { 0 };
    assert!(commands.set_cursor(&document, at));
    sample("rich_paste_ready", started, Some(&document));
    // Preserve the cumulative peak used by the ordinary phases, while also
    // measuring the additional heap used by this first cold paste alone.
    let live_before = LIVE.load(Ordering::Relaxed);
    let previous_peak = PEAK.swap(live_before, Ordering::Relaxed);
    let allocations_before = TOTAL_COUNT.load(Ordering::Relaxed);
    let paste_started = Instant::now();
    let (result, work) = measure_document_work(|| commands.handle_with_clipboard_context(
        &mut document, InputEvent::Key(Key::PasteClipboard), &clipboard,
    ));
    let elapsed_ms = paste_started.elapsed().as_secs_f64() * 1000.;
    let live_after = LIVE.load(Ordering::Relaxed);
    let paste_peak = PEAK.load(Ordering::Relaxed);
    let allocations = TOTAL_COUNT.load(Ordering::Relaxed) - allocations_before;
    PEAK.fetch_max(previous_peak, Ordering::Relaxed);
    let output = result.unwrap();
    assert_eq!(output.status, CommandStatus::Complete);
    assert!(output.document_changed);
    sample("rich_pasted", started, Some(&document));
    println!("{}", json!({
        "operation_measurement": "richpaste", "elapsed_ms": elapsed_ms,
        "live_requested_heap_delta_bytes": live_after as i64 - live_before as i64,
        "peak_requested_heap_delta_bytes": paste_peak - live_before,
        "allocations_and_reallocations": allocations,
        "work": {
            "source_full_materializations": work.source_full_materializations,
            "source_full_materialized_bytes": work.source_full_materialized_bytes,
            "source_decoded_bytes": work.source_decoded_bytes,
            "formatted_full_materialized_bytes": work.formatted_full_materialized_bytes,
            "formatted_compatibility_materialized_bytes": work.formatted_compatibility_materialized_bytes,
            "markdown_block_parse_calls": work.markdown_block_parse_calls,
            "markdown_block_parsed_bytes": work.markdown_block_parsed_bytes,
            "full_projection_candidates": work.full_projection_candidates,
            "regional_projection_candidates": work.regional_projection_candidates,
            "clipboard_style_blocks_visited": work.clipboard_style_blocks_visited,
            "clipboard_style_text_bytes": work.clipboard_style_text_bytes,
        },
    }));
    commands.handle(&mut document, InputEvent::Key(Key::Escape)).unwrap();
    assert_eq!(commands.handle(&mut document, InputEvent::key('u')).unwrap().status, CommandStatus::Complete);
    sample("rich_paste_undo", started, Some(&document));
    assert_eq!(document.source_bytes(), original);
    assert_eq!(commands.handle(&mut document, InputEvent::Key(Key::Ctrl('r'))).unwrap().status, CommandStatus::Complete);
    sample("rich_paste_redo", started, Some(&document));
    // Source/text reads follow the measured phases so they cannot warm the
    // projection or introduce a flat cache before the operation under test.
    let unit_bytes = if matches!(document.encoding(), Encoding::Utf16Le | Encoding::Utf16Be) { 2 } else { 1 };
    let tail_start = if list { "- first\r\n\r\n  <span></span>\r\n".len() * unit_bytes } else { 0 };
    let saved = document.source_bytes();
    assert!(saved.ends_with(&original[tail_start..]));
    if list {
        let prefix_end = "- first\r\n\r\n".len() * unit_bytes;
        assert!(saved.starts_with(&original[..prefix_end]));
    }
    drop(saved);
    assert!(document.text()[at..].starts_with(copied));
    assert!(viem_core::layout::DocumentLayoutStyles::semantic_character_at(
        document.projection(), at + copied.find("double").unwrap(), false,
    ).unwrap().bold);
    drop(document);
    drop(commands);
    drop(clipboard);
    drop(original);
    sample("dropped", started, None);
}

fn main() {
    // Shadow verification (enabled for Cargo-run processes) would add complete
    // reprojection to every regional edit and distort the measured allocations.
    std::env::set_var("VIEM_SHADOW_PROJECTION", "off");
    let args: Vec<_> = std::env::args().collect();
    assert!(
        args.len() >= 4,
        "plain|code|markdown lines|short|long|markdown|richpaste|richpaste-list SOURCE_BYTES [open|document|views|switch|flat|richpaste]"
    );
    let format = match args[1].as_str() {
        "plain" => Format::PlainText,
        "code" => Format::Code,
        "markdown" => Format::Markdown,
        _ => panic!("unknown format"),
    };
    let size: usize = args[3].parse().unwrap();
    let operation = args.get(4).map(String::as_str).unwrap_or("document");
    assert!(size > 0 && matches!(operation, "open" | "document" | "views" | "switch" | "flat" | "exercise" | "search" | "richpaste"));
    assert!(operation != "switch" || format.is_markdown() || format.is_code());
    assert!(operation != "richpaste" || (format == Format::Markdown && matches!(args[2].as_str(), "richpaste" | "richpaste-list")));
    let encoding_name = args.get(5).map(String::as_str).unwrap_or("utf8");
    let encoding = match encoding_name {
        "utf8" => Encoding::Utf8, "latin1" => Encoding::Latin1,
        "utf16le" => Encoding::Utf16Le, "utf16be" => Encoding::Utf16Be,
        _ => panic!("unknown encoding"),
    };
    let utf16 = matches!(encoding, Encoding::Utf16Le | Encoding::Utf16Be);
    assert!(!utf16 || size % 2 == 0);

    let started = Instant::now();
    println!(
        "{}",
        json!({"fixture_revision":1,"format":args[1],"shape":args[2],
        "source_bytes":size,"operation":operation,"syntax":"disabled","history_policy":"default",
        "encoding":encoding_name,
        "validation_retains_original_source":operation == "richpaste",
        "scope":"Rust requested heap and macOS self TASK_VM_INFO; mock shaping, no AppKit",
        "type_sizes": {
            "Block": std::mem::size_of::<viem_core::document::Block>(),
            "ProvenanceSpan": std::mem::size_of::<viem_core::document::ProvenanceSpan>(),
            "StyleSpan": std::mem::size_of::<viem_core::document::StyleSpan>(),
            "CharacterProperties": std::mem::size_of::<viem_core::document::CharacterProperties>(),
            "BlockProperties": std::mem::size_of::<viem_core::document::BlockProperties>(),
        }})
    );
    sample("baseline", started, None);
    let mut bytes = vec![b'a'; if utf16 { size / 2 } else { size }];
    match args[2].as_str() {
        "lines" => {
            for (i, byte) in bytes.iter_mut().enumerate() {
                if i % 128 == 127 {
                    *byte = b'\n';
                } else if i % 8 == 7 {
                    *byte = b' ';
                }
            }
        }
        "crlf" => {
            for (i, byte) in bytes.iter_mut().enumerate() {
                if i % 128 == 126 { *byte = b'\r'; }
                else if i % 128 == 127 { *byte = b'\n'; }
                else if i % 8 == 7 { *byte = b' '; }
            }
        }
        "invalid" => {
            assert_eq!(encoding, Encoding::Utf8);
            bytes.fill(0xff);
        }
        "short" => {
            for i in (1..bytes.len()).step_by(2) {
                bytes[i] = b'\n';
            }
        }
        "long" => (),
        "markdown" => {
            assert_eq!(encoding, Encoding::Utf8);
            let mut text = String::with_capacity(bytes.len() + 1024);
            let mut block = 0usize;
            while text.len() < bytes.len() {
                block += 1;
                text.push_str(&format!(
                    "# Heading {block}\n\nProse with **strong**, _emphasis_, `code`, a [link](https://example.com/{block}) \
                     and &amp; entities\ncontinuing on a soft line with plain words.\n\n- item one\n- item *two*\n  \
                     continued\n1. first\n2. second\n\n> quoted text\n> more of the quote\n\n| a | b |\n| - | - |\n\
                     | {block} | x |\n\n```rust\nfn main() {{}}\n```\n\n"));
            }
            text.truncate(bytes.len());
            bytes = text.into_bytes();
        }
        "richpaste" | "richpaste-list" => {
            // A short target paragraph followed by a table about every few
            // hundred bytes; the unrelated suffix must not affect paste work.
            let block = concat!(
                "A paragraph with **bold**, *emphasis*, and `code` kept beside ordinary prose. ",
                "Plain words preserve a stable paragraph boundary. ",
                "Plain words preserve a stable paragraph boundary. ",
                "Plain words preserve a stable paragraph boundary.\r\n\r\n",
                "| first | second |\r\n| --- | --- |\r\n| alpha | beta |\r\n\r\n",
            );
            let mut text = String::with_capacity(bytes.len() + block.len());
            if args[2] == "richpaste-list" {
                // A cold, empty paragraph within the first list item, as
                // authored by splitting that item with Alt/Option-Enter.
                text.push_str("- first\r\n\r\n  <span></span>\r\n- tail\r\n\r\n");
            } else {
                text.push_str("target\r\n\r\n");
            }
            while text.len() < bytes.len() { text.push_str(block); }
            text.truncate(bytes.len());
            bytes = text.into_bytes();
        }
        "longword" => {
            assert!(bytes.len() >= 16);
            bytes[..7].copy_from_slice(b"before ");
            let tail = bytes.len() - 6;
            bytes[tail..].copy_from_slice(b" after");
        }
        _ => panic!("unknown shape"),
    }
    if utf16 {
        bytes = bytes.into_iter().flat_map(|byte| match encoding {
            Encoding::Utf16Le => [byte, 0], _ => [0, byte],
        }).collect();
    }
    sample("source_ready", started, None);
    let original = (operation == "richpaste").then(|| bytes.clone());
    let mut document = if operation == "richpaste" {
        Document::from_bytes_with_file_format(bytes, encoding, format, FileFormat::Dos).unwrap()
    } else {
        Document::from_bytes(bytes, encoding, format).unwrap()
    };
    sample("opened", started, Some(&document));
    if let Some(original) = original {
        rich_paste(document, original, args[2] == "richpaste-list", started);
        return;
    }
    if operation == "open" {
        drop(document);
        sample("dropped", started, None);
        return;
    }
    document.replace(0..0, "x").unwrap();
    sample("one_edit_default_history", started, Some(&document));
    if operation == "flat" {
        std::hint::black_box(document.text());
        sample("flat_text_read", started, Some(&document));
    }
    document.replace(1..1, "\n").unwrap();
    sample("newline_edit_default_history", started, Some(&document));
    if matches!(operation, "exercise" | "search") {
        let mut commands = CommandInterpreter::new();
        for key in [Key::Char('/'), Key::Char('z'), Key::Char('z'), Key::Char('z'), Key::Enter] {
            let _ = std::hint::black_box(commands.handle(&mut document, InputEvent::Key(key)));
        }
        sample("searched_absent_pattern", started, Some(&document));
        drop(commands);
    }
    if operation == "exercise" {
        let len = document.projection().text_tree().byte_len();
        document.begin_edit_group();
        document.replace(10..11, "X").unwrap();
        document.replace(len - 10..len - 9, "Y").unwrap();
        document.end_edit_group();
        sample("distant_group", started, Some(&document));
        document.try_undo().unwrap();
        sample("distant_group_undo", started, Some(&document));
        document.try_redo().unwrap();
        sample("distant_group_redo", started, Some(&document));
        let serialized = document.source_bytes();
        std::hint::black_box(serialized.len());
        drop(serialized);
        document.mark_saved();
        sample("serialized_and_marked_saved", started, Some(&document));
        document.replace(1..len - 1, "").unwrap();
        document.set_history_retention_policy(HistoryRetentionPolicy::new(1, usize::MAX));
        sample("large_delete_history_pruned", started, Some(&document));
    }
    if operation == "switch" {
        let mut core = Core::new(document);
        let view = core.add_view(MockTextMeasurementProvider::new(), 800., 600.);
        sample("first_view", started, Some(core.document()));
        for phase in ["switched", "switched_back"] {
            let document = core.document();
            let mode = if document.format().is_markdown() {
                viem_core::DocumentMode::Code("markdown".into())
            } else {
                viem_core::DocumentMode::Markdown
            };
            core.handle(view, CoreEvent::SetDocumentMode { document: document.id(), revision: document.revision(), mode })
                .unwrap();
            sample(phase, started, Some(core.document()));
        }
        drop(core);
    } else if operation == "views" {
        let mut core = Core::new(document);
        core.set_code_language(LanguageSelection::None);
        let first = core.add_view(MockTextMeasurementProvider::new(), 800., 600.);
        sample("first_view", started, Some(core.document()));
        let second = core.add_view(MockTextMeasurementProvider::new(), 480., 600.);
        sample("second_view", started, Some(core.document()));
        for i in 0..8 {
            for view in [first, second] {
                core.handle(view, CoreEvent::SetWrap(i % 2 == 0)).unwrap();
                core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Char('G'))))
                    .unwrap();
                core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Char('g'))))
                    .unwrap();
                core.handle(view, CoreEvent::Input(InputEvent::Key(Key::Char('g'))))
                    .unwrap();
            }
        }
        sample("scroll_and_wrap_toggles", started, Some(core.document()));
        drop(core);
    } else {
        drop(document);
    }
    sample("dropped", started, None);
}
