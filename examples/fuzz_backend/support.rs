use serde::Serialize;
use serde_json::{json, Value};
use std::alloc::{GlobalAlloc, Layout, System};
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Track requested live heap separately from RSS (which includes allocator
/// retention and shared mappings). No allocation occurs inside these hooks.
pub struct TrackingAllocator;
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

fn added(bytes: usize) {
    let live = LIVE.fetch_add(bytes, Ordering::Relaxed) + bytes;
    PEAK.fetch_max(live, Ordering::Relaxed);
}

unsafe impl GlobalAlloc for TrackingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = System.alloc(layout);
        if !ptr.is_null() {
            added(layout.size());
        }
        ptr
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let ptr = System.alloc_zeroed(layout);
        if !ptr.is_null() {
            added(layout.size());
        }
        ptr
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
        System.dealloc(ptr, layout);
    }
    unsafe fn realloc(&self, ptr: *mut u8, old: Layout, size: usize) -> *mut u8 {
        let result = System.realloc(ptr, old, size);
        if !result.is_null() {
            if size >= old.size() {
                added(size - old.size());
            } else {
                LIVE.fetch_sub(old.size() - size, Ordering::Relaxed);
            }
        }
        result
    }
}

pub fn memory() -> Value {
    let live = LIVE.load(Ordering::Relaxed);
    let peak = PEAK.load(Ordering::Relaxed);
    json!({"live_allocated_bytes":live,"peak_allocated_bytes":peak})
}

#[derive(Debug, Default, Serialize)]
pub struct RunStats {
    pub actions: usize,
    pub expected_rejections: usize,
    pub max_source_bytes: usize,
}

/// SplitMix64 has an explicitly fixed algorithm, independent of library or
/// platform RNG versions. The trace also records actual actions, not just seed.
pub struct Rng(u64);
impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }
    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e3779b97f4a7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
        z ^ (z >> 31)
    }
    pub fn usize(&mut self, upper: usize) -> usize {
        if upper == 0 {
            0
        } else {
            (self.next() % upper as u64) as usize
        }
    }
    pub fn chance(&mut self, numerator: u64, denominator: u64) -> bool {
        assert!(denominator > 0 && numerator <= denominator);
        self.next() % denominator < numerator
    }
}

pub struct Recorder {
    writer: BufWriter<File>,
    session: usize,
    index: usize,
}
impl Recorder {
    pub fn new(path: &Path) -> Result<Self, String> {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        Ok(Self {
            writer: BufWriter::new(File::create(path).map_err(|e| e.to_string())?),
            session: 0,
            index: 0,
        })
    }
    pub fn event(&mut self, value: Value) -> Result<(), String> {
        serde_json::to_writer(&mut self.writer, &value).map_err(|e| e.to_string())?;
        self.writer.write_all(b"\n").map_err(|e| e.to_string())?;
        // The action reaches the OS before it can crash, abort, hang or OOM.
        self.writer.flush().map_err(|e| e.to_string())
    }
    pub fn session(&mut self, index: usize, seed: u64) -> Result<(), String> {
        self.session = index;
        self.index = 0;
        self.event(json!({"type":"session","index":index,"seed":seed,"memory":memory()}))
    }
    pub fn record<T: Serialize>(&mut self, action: &T) -> Result<(), String> {
        self.event(
            json!({"type":"action","session":self.session,"index":self.index,"action":action}),
        )?;
        self.index += 1;
        if self.index % 100 == 0 {
            self.event(json!({"type":"memory","session":self.session,"index":self.index,"memory":memory()}))?;
        }
        Ok(())
    }
}
