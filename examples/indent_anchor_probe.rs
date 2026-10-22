//! Isolated requested-allocation and work probe for indentation summaries.
//!
//! Build/run through the validation owner's Cargo artifact discovery, in a
//! fresh process and without concurrent builds or measurements. Optional first
//! argument is the line count (default 1,000,000). JSON is emitted only after
//! every allocation interval has been captured. Allocation sizes are requested
//! Rust allocation bytes, not resident memory or native rendering latency.

use std::alloc::{GlobalAlloc, Layout, System};
use std::hint::black_box;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;
use viem_core::layout::{IndentAnchorIndex, IndentAnchorWork, LineIndentation};

struct CountingAllocator;

static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);
static DEALLOCATIONS: AtomicUsize = AtomicUsize::new(0);
static ALLOCATED: AtomicUsize = AtomicUsize::new(0);
static DEALLOCATED: AtomicUsize = AtomicUsize::new(0);

fn allocated(bytes: usize) {
    ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
    ALLOCATED.fetch_add(bytes, Ordering::Relaxed);
    let live = LIVE.fetch_add(bytes, Ordering::Relaxed) + bytes;
    PEAK.fetch_max(live, Ordering::Relaxed);
}

fn deallocated(bytes: usize) {
    DEALLOCATIONS.fetch_add(1, Ordering::Relaxed);
    DEALLOCATED.fetch_add(bytes, Ordering::Relaxed);
    LIVE.fetch_sub(bytes, Ordering::Relaxed);
}

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() {
            allocated(layout.size());
        }
        pointer
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc_zeroed(layout) };
        if !pointer.is_null() {
            allocated(layout.size());
        }
        pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) };
        deallocated(layout.size());
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let next = unsafe { System.realloc(pointer, layout, new_size) };
        if !next.is_null() {
            // Record old requested ownership ending and new ownership starting.
            // This does not claim to observe System's transient realloc peak.
            deallocated(layout.size());
            allocated(new_size);
        }
        next
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

#[derive(Clone, Copy)]
struct Counters {
    live: usize,
    allocations: usize,
    deallocations: usize,
    allocated: usize,
    deallocated: usize,
}

impl Counters {
    fn read() -> Self {
        Self {
            live: LIVE.load(Ordering::Relaxed),
            allocations: ALLOCATIONS.load(Ordering::Relaxed),
            deallocations: DEALLOCATIONS.load(Ordering::Relaxed),
            allocated: ALLOCATED.load(Ordering::Relaxed),
            deallocated: DEALLOCATED.load(Ordering::Relaxed),
        }
    }
}

#[derive(serde::Serialize)]
struct Measurement {
    name: &'static str,
    elapsed_ns: u64,
    allocations: usize,
    deallocations: usize,
    requested_allocated_bytes: usize,
    requested_deallocated_bytes: usize,
    requested_live_bytes_before: usize,
    requested_live_bytes_after: usize,
    requested_live_bytes_delta: i128,
    requested_peak_bytes_above_before: usize,
}

fn measure<T>(name: &'static str, operation: impl FnOnce() -> T) -> (T, Measurement) {
    let before = Counters::read();
    PEAK.store(before.live, Ordering::Relaxed);
    let started = Instant::now();
    let result = black_box(operation());
    let elapsed_ns = started.elapsed().as_nanos().try_into().unwrap();
    let after = Counters::read();
    let peak = PEAK.load(Ordering::Relaxed);
    (
        result,
        Measurement {
            name,
            elapsed_ns,
            allocations: after.allocations - before.allocations,
            deallocations: after.deallocations - before.deallocations,
            requested_allocated_bytes: after.allocated - before.allocated,
            requested_deallocated_bytes: after.deallocated - before.deallocated,
            requested_live_bytes_before: before.live,
            requested_live_bytes_after: after.live,
            requested_live_bytes_delta: after.live as i128 - before.live as i128,
            requested_peak_bytes_above_before: peak.saturating_sub(before.live),
        },
    )
}

fn work_json(work: IndentAnchorWork) -> serde_json::Value {
    serde_json::json!({
        "nodes_visited": work.nodes_visited,
        "nodes_created": work.nodes_created,
        "lines_examined": work.lines_examined,
        "lines_copied": work.lines_copied,
    })
}

fn main() {
    let lines = std::env::args().nth(1).map_or(1_000_000, |argument| {
        argument
            .parse::<usize>()
            .expect("line count must be an integer")
    });
    assert!(lines >= 3, "line count must be at least three");
    let _ = Instant::now();

    let (empty, empty_measurement) = measure("empty_index", IndentAnchorIndex::default);
    let empty_statistics = empty.statistics();
    let (mut index, construction) = measure("construct_index", || {
        IndentAnchorIndex::from_lines(
            (0..lines).map(|line| LineIndentation::new(if line == 0 { 0 } else { 8 })),
        )
    });
    let initial_statistics = index.statistics();
    let (snapshot, clone_measurement) = measure("clone_retained_snapshot", || index.clone());

    let (edit_work, edit_measurement) = measure("reindent_root_with_snapshot_retained", || {
        let mut work = IndentAnchorWork::default();
        index
            .splice_with_work(0..1, [LineIndentation::new(16)], &mut work)
            .unwrap();
        work
    });
    assert_eq!(snapshot.preceding_with_max(lines, 0), Some(0));
    assert_eq!(index.preceding_with_max(lines, 0), None);
    let middle = lines / 2;
    let (insert_work, insert_measurement) = measure("insert_three_lines", || {
        let mut work = IndentAnchorWork::default();
        index
            .splice_with_work(
                middle..middle,
                [3, 5, 7].map(LineIndentation::new),
                &mut work,
            )
            .unwrap();
        work
    });
    let (delete_work, delete_measurement) = measure("delete_three_lines", || {
        let mut work = IndentAnchorWork::default();
        index
            .splice_with_work(middle..middle + 3, [], &mut work)
            .unwrap();
        work
    });
    let final_statistics = index.statistics();

    const QUERY_PAIRS: usize = 10_000;
    let ((lookup_work, lookup_checksum), lookup_measurement) =
        measure("anchor_and_distant_threshold_queries", || {
            let mut work = IndentAnchorWork::default();
            let mut checksum = 0usize;
            for sample in 0..QUERY_PAIRS {
                let line = 1 + (sample.wrapping_mul(104_729) % (lines - 1));
                checksum ^= black_box(snapshot.anchor_with_work(line, &mut work)).unwrap();
                checksum ^=
                    black_box(snapshot.preceding_with_max_and_work(line, 0, &mut work)).unwrap();
            }
            (work, checksum)
        });
    let (dependency_span, span_measurement) = measure("transitive_dependency_span", || {
        snapshot.dependent_range(0).unwrap()
    });
    assert_eq!(dependency_span, 1..lines);

    // A separate descending fixture gives the root many immediate children;
    // the plateau fixture above intentionally gives it a long transitive chain.
    let enumeration_lines = lines.min(16_384);
    let (enumeration_index, enumeration_construction) =
        measure("construct_dependent_enumeration_fixture", || {
            IndentAnchorIndex::from_lines((0..enumeration_lines).map(|line| {
                LineIndentation::new(if line == 0 {
                    0
                } else {
                    enumeration_lines - line
                })
            }))
        });
    let ((dependent_count, dependent_checksum), enumeration_measurement) =
        measure("enumerate_direct_dependents", || {
            let mut count = 0usize;
            let mut checksum = 0usize;
            for child in enumeration_index.direct_dependents(0) {
                checksum ^= black_box(child);
                count += 1;
            }
            (count, checksum)
        });
    assert_eq!(dependent_count, enumeration_lines - 1);

    // The public iterator deliberately exposes no measurement state. Replay
    // its threshold walk separately to report search work without changing
    // production APIs or mixing instrumentation into the iterator's timing.
    let ((enumeration_work, replay_count), enumeration_work_measurement) =
        measure("instrumented_dependent_threshold_walk", || {
            let mut work = IndentAnchorWork::default();
            let mut from = 1;
            let mut maximum = usize::MAX;
            let mut count = 0usize;
            while let Some(child) =
                enumeration_index.following_with_max_and_work(from, maximum, &mut work)
            {
                count += 1;
                from = child + 1;
                maximum = enumeration_index.line(child).unwrap().columns - 1;
            }
            (work, count)
        });
    assert_eq!(replay_count, dependent_count);

    let (_, release_measurement) = measure("release_all_indexes_and_retained_snapshot", || {
        drop(enumeration_index);
        drop(snapshot);
        drop(index);
        drop(empty);
    });
    let final_live = Counters::read().live;

    // Reporting allocations happen strictly after all measured intervals.
    let report = serde_json::json!({
        "schema": "viem-indent-anchor-probe-v1",
        "profile": if cfg!(debug_assertions) { "debug" } else { "release" },
        "line_count": lines,
        "allocator_scope": "requested Rust allocation bytes, including Arc headers; excludes allocator metadata, RSS and native rendering",
        "initial_fixture": "line zero indent 0, all remaining lines indent 8",
        "empty_index": { "line_count": empty_statistics.line_count, "payload_bytes": empty_statistics.payload_bytes },
        "initial_index": {
            "line_count": initial_statistics.line_count,
            "block_count": initial_statistics.block_count,
            "tree_height": initial_statistics.tree_height,
            "payload_bytes": initial_statistics.payload_bytes,
            "retained_requested_bytes_per_line": construction.requested_live_bytes_delta as f64 / lines as f64,
        },
        "final_index": { "block_count": final_statistics.block_count, "tree_height": final_statistics.tree_height },
        "lookup": { "query_pairs": QUERY_PAIRS, "checksum": lookup_checksum, "work": work_json(lookup_work) },
        "edit_work": work_json(edit_work),
        "insert_work": work_json(insert_work),
        "delete_work": work_json(delete_work),
        "dependency_span": { "start": dependency_span.start, "end": dependency_span.end },
        "enumeration": {
            "fixture_lines": enumeration_lines,
            "direct_dependents": dependent_count,
            "checksum": dependent_checksum,
            "threshold_walk_work": work_json(enumeration_work),
            "work_scope": "separate equivalent threshold-search replay; excludes uninstrumented line metadata lookups",
        },
        "final_requested_live_bytes": final_live,
        "returned_to_empty_baseline": final_live == empty_measurement.requested_live_bytes_after,
        "measurements": [
            empty_measurement, construction, clone_measurement, edit_measurement,
            insert_measurement, delete_measurement, lookup_measurement, span_measurement,
            enumeration_construction, enumeration_measurement, enumeration_work_measurement,
            release_measurement,
        ],
    });
    println!("{}", serde_json::to_string_pretty(&report).unwrap());
}
