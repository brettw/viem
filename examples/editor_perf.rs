//! Editor latency probe for large documents. It drives the portable core the
//! way a frontend does: open, attach views, scroll by wheel and page, type,
//! undo, and poll syntax between events. Every operation records wall time so
//! p50/p95/max latencies can be compared across builds.
//!
//! Usage: editor_perf <scenario> [size_bytes] [json_out]
//! Scenarios: markdown, markdown_source, html, code, text.
//! Shaping uses the mock provider; results are core costs, not native frames.
use serde_json::{json, Value};
use std::time::{Duration, Instant};
use viem_core::command::{InputEvent, Key};
use viem_core::document::syntax::detection::LanguageSelection;
use viem_core::document::HistoryRetentionPolicy;
use viem_core::layout::MockTextMeasurementProvider;
use viem_core::{Core, CoreEvent, Document, Encoding, Format, ViewId};

type Editor = Core<MockTextMeasurementProvider>;

const WIDTH: f32 = 800.;
const HEIGHT: f32 = 600.;

#[derive(Default)]
struct Series {
    samples: Vec<f64>,
    /// Calling-thread CPU time per sample; wall time far above it means the
    /// core waited for another thread rather than doing its own work.
    cpu_samples: Vec<f64>,
}

fn thread_cpu_seconds() -> f64 {
    #[repr(C)]
    struct Timespec {
        seconds: i64,
        nanoseconds: i64,
    }
    extern "C" {
        fn clock_gettime(clock: i32, out: *mut Timespec) -> i32;
    }
    const CLOCK_THREAD_CPUTIME_ID: i32 = 3;
    let mut now = Timespec { seconds: 0, nanoseconds: 0 };
    // SAFETY: a valid out-pointer for a supported POSIX clock.
    if unsafe { clock_gettime(CLOCK_THREAD_CPUTIME_ID, &mut now) } != 0 {
        return 0.;
    }
    now.seconds as f64 + now.nanoseconds as f64 * 1e-9
}

impl Series {
    fn record(&mut self, elapsed: Duration) {
        self.samples.push(elapsed.as_secs_f64() * 1000.);
    }
    fn record_cpu(&mut self, cpu_seconds: f64) {
        self.cpu_samples.push(cpu_seconds * 1000.);
    }
    fn percentile(&self, fraction: f64) -> f64 {
        if self.samples.is_empty() {
            return 0.;
        }
        let mut sorted = self.samples.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let index = ((sorted.len() - 1) as f64 * fraction).round() as usize;
        sorted[index]
    }
    fn summary(&self) -> Value {
        let cpu_total: f64 = self.cpu_samples.iter().sum();
        json!({
            "samples": self.samples.len(),
            "p50_ms": self.percentile(0.5),
            "p95_ms": self.percentile(0.95),
            "max_ms": self.samples.iter().cloned().fold(0., f64::max),
            "total_ms": self.samples.iter().sum::<f64>(),
            "thread_cpu_total_ms": cpu_total,
        })
    }
}

fn time<T>(series: &mut Series, operation: impl FnOnce() -> T) -> T {
    let started = Instant::now();
    let cpu_started = thread_cpu_seconds();
    let result = operation();
    let elapsed = started.elapsed();
    series.record_cpu(thread_cpu_seconds() - cpu_started);
    if elapsed.as_millis() >= 100 && std::env::var_os("VIEM_PERF_TRACE").is_some() {
        eprintln!("slow op #{} in series: {:.1} ms", series.samples.len(), elapsed.as_secs_f64() * 1000.);
    }
    series.record(elapsed);
    result
}

fn key(core: &mut Editor, view: ViewId, key: Key) {
    let outcome = core
        .handle_with_layout(view, CoreEvent::Input(InputEvent::Key(key.clone())))
        .unwrap_or_else(|error| panic!("{key:?}: {error:?}"));
    if let Some(command) = outcome.command {
        assert!(
            matches!(
                command.status,
                viem_core::command::CommandStatus::Complete
                    | viem_core::command::CommandStatus::Pending
            ),
            "{:?}",
            command.status
        );
    }
}

fn text(core: &mut Editor, view: ViewId, text: &str) {
    let outcome = core
        .handle_with_layout(view, CoreEvent::Input(InputEvent::text(text)))
        .unwrap();
    assert!(outcome.document_changed, "typing {text:?} changed nothing");
}

/// Publish completed syntax the way a frontend's idle poll does, bounded so a
/// stalled provider cannot hang the probe.
fn settle_syntax(core: &mut Editor, series: &mut Series) -> u64 {
    let started = Instant::now();
    let before = core.syntax_statistics().publications;
    let mut idle_polls = 0;
    while started.elapsed() < Duration::from_secs(20) {
        let published = time(series, || core.poll_syntax());
        if published {
            idle_polls = 0;
        } else {
            idle_polls += 1;
            if idle_polls > 2000 {
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    core.syntax_statistics().publications - before
}

fn markdown_fixture(size: usize) -> Vec<u8> {
    let base = std::fs::read_to_string("AGENTS.md").expect("run from the repository root");
    let mut out = String::with_capacity(size + base.len());
    let mut chapter = 0;
    while out.len() < size {
        chapter += 1;
        out.push_str(&format!("\n# Chapter {chapter}\n\n"));
        out.push_str(&base);
    }
    out.into_bytes()
}

fn html_fixture(size: usize) -> Vec<u8> {
    let mut out = String::from("<!DOCTYPE html><html><head><meta charset='utf-8'><title>Probe</title></head><body>\n");
    let mut n = 0;
    while out.len() < size {
        n += 1;
        out.push_str(&format!("<h2 id='s{n}'>Section {n} with <em>emphasis</em></h2>\n"));
        for p in 0..6 {
            out.push_str(&format!(
                "<p>Paragraph {p} of section {n}: <b>bold words</b> and <i>italic prose</i> with a \
                 <a href='https://example.test/{n}/{p}'>link to somewhere</a>, some <code>inline_code()</code>, \
                 <span style='color:#123456;font-size:15px'>styled span text</span>, an entity &amp; more, \
                 <sup>superscript</sup> and ordinary trailing prose that wraps at the window width.</p>\n"
            ));
        }
        out.push_str("<ul><li>first <b>item</b></li><li>second item<ul><li>nested</li></ul></li><li>third</li></ul>\n");
        out.push_str("<blockquote><p>A quoted paragraph with <i>italic</i> text.</p></blockquote>\n");
        out.push_str("<pre>fn code() {\n    let x = 1;\n}\n</pre>\n");
    }
    out.push_str("</body></html>\n");
    out.into_bytes()
}

fn code_fixture(size: usize) -> Vec<u8> {
    let mut files = Vec::new();
    fn walk(dir: &std::path::Path, files: &mut Vec<std::path::PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(&path, files);
            } else if path.extension().is_some_and(|e| e == "rs") {
                files.push(path);
            }
        }
    }
    walk(std::path::Path::new("src/core"), &mut files);
    files.sort();
    let mut out = Vec::new();
    while out.len() < size {
        for file in &files {
            out.extend_from_slice(&std::fs::read(file).unwrap());
            out.push(b'\n');
            if out.len() >= size {
                break;
            }
        }
    }
    out
}

fn text_fixture(lines: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(lines * 40);
    for i in 0..lines {
        out.extend_from_slice(format!("line {i} with a few words of prose\n").as_bytes());
    }
    out
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let scenario = args.get(1).map(String::as_str).unwrap_or("markdown");
    let size: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(4 * 1024 * 1024);
    let (bytes, format, filename) = match scenario {
        "markdown" => (markdown_fixture(size), Format::Markdown, "probe.md"),
        "markdown_source" => (markdown_fixture(size), Format::MarkdownSource, "probe.md"),
        "html" => (html_fixture(size), Format::Html, "probe.html"),
        "code" => (code_fixture(size), Format::Code, "probe.rs"),
        "text" => (text_fixture(size / 37), Format::PlainText, "probe.txt"),
        other => panic!("unknown scenario {other}"),
    };
    let source_bytes = bytes.len();
    let syntax = scenario == "code";
    let mut report = serde_json::Map::new();
    let started = Instant::now();

    let mut document = Document::from_bytes(bytes, Encoding::Utf8, format).unwrap();
    document.set_history_retention_policy(HistoryRetentionPolicy::unlimited());
    let open_ms = started.elapsed().as_secs_f64() * 1000.;
    let hard_lines = document.projection().text_tree().hard_line_count();
    let mut core = Core::new(document);
    core.initialize_code_detection(filename, false).unwrap();
    if !syntax {
        core.set_code_language(LanguageSelection::None);
    }
    let attach = Instant::now();
    let view = core.add_view(MockTextMeasurementProvider::new(), WIDTH, HEIGHT);
    let first_view_ms = attach.elapsed().as_secs_f64() * 1000.;
    let mut syntax_polls = Series::default();
    let initial = if syntax {
        let start = Instant::now();
        let publications = settle_syntax(&mut core, &mut syntax_polls);
        json!({"publications": publications, "settle_ms": start.elapsed().as_secs_f64() * 1000.})
    } else {
        Value::Null
    };
    report.insert("open_ms".into(), json!(open_ms));
    report.insert("first_view_ms".into(), json!(first_view_ms));
    report.insert("initial_syntax".into(), initial);

    // Wheel scrolling from the top: 300 steps of 60 units.
    let mut wheel = Series::default();
    let mut top = 0.0f32;
    for _ in 0..300 {
        top += 60.;
        time(&mut wheel, || {
            core.handle(view, CoreEvent::SetViewportOrigin { left: 0., top: Some(top) }).unwrap()
        });
        if syntax {
            time(&mut syntax_polls, || core.poll_syntax());
        }
    }
    report.insert("wheel_scroll".into(), wheel.summary());

    // Page Down through the document, then jump to the end and back.
    let mut page = Series::default();
    for _ in 0..60 {
        time(&mut page, || key(&mut core, view, Key::Ctrl('f')));
        if syntax {
            time(&mut syntax_polls, || core.poll_syntax());
        }
    }
    report.insert("page_down".into(), page.summary());
    let mut jumps = Series::default();
    for _ in 0..5 {
        time(&mut jumps, || key(&mut core, view, Key::Char('G')));
        time(&mut jumps, || {
            key(&mut core, view, Key::Char('g'));
            key(&mut core, view, Key::Char('g'));
        });
        // Jump to a middle line by Ex address.
        let middle = hard_lines / 2;
        time(&mut jumps, || {
            key(&mut core, view, Key::Char(':'));
            text_prompt(&mut core, view, &middle.to_string());
            key(&mut core, view, Key::Enter);
        });
        if syntax {
            settle_syntax(&mut core, &mut syntax_polls);
        }
    }
    report.insert("jumps".into(), jumps.summary());

    // Typing in the middle of the document: characters, breaks, then undo.
    let mut typing = Series::default();
    let mut breaks = Series::default();
    let mut undo = Series::default();
    let mut edit_highlight = Series::default();
    for _round in 0..4 {
        key(&mut core, view, Key::Char('A'));
        for character in "The quick brown fox jumps over the lazy dog ".chars() {
            let buffer = character.to_string();
            time(&mut typing, || text(&mut core, view, &buffer));
            if syntax {
                time(&mut syntax_polls, || core.poll_syntax());
            }
        }
        time(&mut breaks, || key(&mut core, view, Key::Enter));
        if syntax {
            let start = Instant::now();
            settle_syntax(&mut core, &mut syntax_polls);
            edit_highlight.record(start.elapsed());
        }
        key(&mut core, view, Key::Escape);
    }
    for _ in 0..4 {
        time(&mut undo, || key(&mut core, view, Key::Char('u')));
    }
    report.insert("type_char".into(), typing.summary());
    report.insert("type_enter".into(), breaks.summary());
    report.insert("undo".into(), undo.summary());
    report.insert("edit_highlight_settle".into(), edit_highlight.summary());

    // A second, narrower view of the same buffer: wrap toggles and distant scrolls.
    let second = core.add_view(MockTextMeasurementProvider::new(), 480., HEIGHT);
    let mut two_view = Series::default();
    for i in 0..10 {
        time(&mut two_view, || {
            core.handle(second, CoreEvent::SetWrap(i % 2 == 0)).unwrap();
            key(&mut core, second, if i % 2 == 0 { Key::Char('G') } else { Key::Ctrl('u') });
        });
    }
    report.insert("second_view_wrap_scroll".into(), two_view.summary());
    // Typing in the first view now has to keep the second view coherent.
    let mut typing_two = Series::default();
    key(&mut core, view, Key::Char('i'));
    for character in "edit with two views open ".chars() {
        let buffer = character.to_string();
        time(&mut typing_two, || text(&mut core, view, &buffer));
    }
    key(&mut core, view, Key::Escape);
    report.insert("type_char_two_views".into(), typing_two.summary());
    if syntax {
        report.insert("syntax_poll".into(), syntax_polls.summary());
        let statistics = core.syntax_statistics();
        report.insert(
            "syntax_statistics".into(),
            json!({"requests": statistics.requests, "publications": statistics.publications,
                   "cache_hits": statistics.cache_hits, "stale_rejections": statistics.stale_rejections}),
        );
    }

    let mut summary = serde_json::Map::new();
    summary.insert("scenario".into(), json!(scenario));
    summary.insert("source_bytes".into(), json!(source_bytes));
    summary.insert("hard_lines".into(), json!(hard_lines));
    summary.insert("total_ms".into(), json!(started.elapsed().as_secs_f64() * 1000.));
    summary.insert("results".into(), Value::Object(report));
    let output = serde_json::to_string_pretty(&Value::Object(summary)).unwrap();
    if let Some(path) = args.get(3) {
        std::fs::write(path, &output).unwrap();
    }
    println!("{output}");
}

fn text_prompt(core: &mut Editor, view: ViewId, text: &str) {
    core.handle_with_layout(view, CoreEvent::Input(InputEvent::text(text))).unwrap();
}
