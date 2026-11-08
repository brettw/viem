use super::*;
use crate::command::{InputEvent, Key};
use crate::document::syntax::service::{SyntaxProvider, SyntaxRequest, SyntaxResult, WORKER_COUNT};
use crate::document::{Encoding, FileFormat};
use crate::layout::MockTextMeasurementProvider;
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    mpsc, Condvar, Mutex,
};
use std::time::{Duration, Instant};

mod retention;

struct Suspended {
    gate: Arc<(Mutex<bool>, Condvar)>,
    calls: Arc<AtomicUsize>,
    started: mpsc::Sender<()>,
}
impl SyntaxProvider for Suspended {
    fn analyze(&mut self, request: &SyntaxRequest, _: &AtomicBool) -> SyntaxResult {
        assert!(std::thread::current()
            .name()
            .unwrap_or("")
            .starts_with("viem-syntax-"));
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.started.send(()).unwrap();
        let (lock, cv) = &*self.gate;
        let mut released = lock.lock().unwrap();
        while !*released {
            released = cv.wait(released).unwrap();
        }
        SyntaxResult::missing(request, "Controlled suspended provider")
    }
}
struct Suspension {
    gate: Arc<(Mutex<bool>, Condvar)>,
    services: Vec<SyntaxService>,
    calls: Arc<AtomicUsize>,
}
impl Suspension {
    fn all_workers() -> Self {
        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        let calls = Arc::new(AtomicUsize::new(0));
        let (sender, receiver) = mpsc::channel();
        let mut services = Vec::new();
        for document in 0..WORKER_COUNT {
            let (gate, calls, started) = (gate.clone(), calls.clone(), sender.clone());
            let mut service = SyntaxService::with_factory(Arc::new(move || {
                Box::new(Suspended {
                    gate: gate.clone(),
                    calls: calls.clone(),
                    started: started.clone(),
                })
            }));
            service.request(
                SyntaxInputSnapshot::new(
                    SyntaxInputIdentity {
                        document: 900_000 + document as u64,
                        revision: 0,
                        generation: 1,
                    },
                    crate::document::FormattedTextTree::try_from_text("x").unwrap(),
                ),
                0..1,
            );
            services.push(service);
        }
        let this = Self {
            gate,
            services,
            calls,
        };
        for _ in 0..WORKER_COUNT {
            receiver.recv_timeout(Duration::from_secs(10)).unwrap();
        }
        this
    }
}
impl Drop for Suspension {
    fn drop(&mut self) {
        for service in &mut self.services {
            service.cancel();
        }
        let (lock, cv) = &*self.gate;
        *lock.lock().unwrap_or_else(|e| e.into_inner()) = true;
        cv.notify_all();
    }
}

struct ReadySyntaxGate {
    gate: Arc<(Mutex<usize>, Condvar)>,
    started: mpsc::Receiver<usize>,
}
impl ReadySyntaxGate {
    fn release(&self, phase: usize) {
        let (lock, cv) = &*self.gate;
        *lock.lock().unwrap_or_else(|e| e.into_inner()) = phase;
        cv.notify_all();
    }

    fn wait_for(&self, phase: usize) {
        assert_eq!(
            self.started.recv_timeout(Duration::from_secs(5)).unwrap(),
            phase
        );
    }
}
impl Drop for ReadySyntaxGate {
    fn drop(&mut self) {
        self.release(usize::MAX);
    }
}

fn exercise_syntax_finishing_during_layout(
    explicit_publication: bool,
    before_install: impl FnOnce(&mut Core<MockTextMeasurementProvider>, ViewId),
) {
    let _registry = crate::document::syntax::treesitter::package_registry_test_guard();
    struct ContinuingProvider {
        gate: Arc<(Mutex<usize>, Condvar)>,
        calls: Arc<AtomicUsize>,
        started: mpsc::Sender<usize>,
    }
    impl SyntaxProvider for ContinuingProvider {
        fn analyze(&mut self, request: &SyntaxRequest, _: &AtomicBool) -> SyntaxResult {
            let phase = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
            self.started.send(phase).unwrap();
            let (lock, cv) = &*self.gate;
            let mut released = lock.lock().unwrap();
            while *released < phase {
                released = cv.wait(released).unwrap();
            }
            let mut result = SyntaxResult::missing(request, "controlled completed syntax");
            result.coverage = crate::document::syntax::Coverage::Exact;
            result.continuation = phase == 1;
            result.runs.push(crate::document::syntax::SyntaxRun {
                range: request.range.start..request.range.start + 2,
                name: crate::document::syntax::SyntaxStyleName("Keyword".into()),
                origin: "controlled test provider".into(),
                priority: 0,
            });
            result.runs.push(crate::document::syntax::SyntaxRun {
                range: request.range.start + 3..request.range.start + 7,
                name: crate::document::syntax::SyntaxStyleName("@provider.undefined".into()),
                origin: "controlled test provider".into(),
                priority: 0,
            });
            result
        }
    }
    let (started, receiver) = mpsc::channel();
    let gate = ReadySyntaxGate {
        gate: Arc::new((Mutex::new(0), Condvar::new())),
        started: receiver,
    };
    let mut core = Core::<MockTextMeasurementProvider>::new(
        Document::from_bytes(b"fn main() {}".to_vec(), Encoding::Utf8, Format::Code).unwrap(),
    );
    core.initialize_code_detection("fixture.rs", false).unwrap();
    let provider_gate = gate.gate.clone();
    let calls = Arc::new(AtomicUsize::new(0));
    core.set_syntax_provider_factory(Arc::new(move || {
        Box::new(ContinuingProvider {
            gate: provider_gate.clone(),
            calls: calls.clone(),
            started: started.clone(),
        })
    }));
    let view = core.add_view(MockTextMeasurementProvider::new(), 400., 120.);
    gate.wait_for(1);
    let cancellation = LayoutCancellationToken::new();
    let request = core
        .prepare_view_layout_job(
            view,
            LayoutJobPriority::ChangedVisibleRows,
            LayoutJobRegion::Viewport(ViewportLayoutRegion::new(0..1, 0., 120.).unwrap()),
            cancellation.clone(),
        )
        .unwrap();
    let configuration = request.configuration_generation();
    let candidate = compute_layout_job(
        &mut core.views.get_mut(&view).unwrap().engine,
        &request,
        LayoutExecutionContext::WorkerPool,
    )
    .unwrap();
    gate.release(1);
    // The scheduler stores a result before starting its continuation. Waiting
    // for phase two proves that phase one's result is ready without polling it
    // into the document, sleeping, or racing the provider's return path.
    gate.wait_for(2);
    assert_eq!(core.syntax_statistics().publications, 0);
    assert!(core.syntax_style_names().is_empty(), "unpublished names stay private");
    before_install(&mut core, view);

    if explicit_publication {
        assert!(core.poll_syntax());
        assert!(cancellation.is_cancelled());
        assert_ne!(core.layout(view).unwrap().configuration_generation(), configuration);
        let before_rejection = core.layout(view).unwrap().clone();
        assert_eq!(
            core.install_view_layout_job(view, candidate),
            Err(CoreError::LayoutInstall(LayoutJobInstallRejection::Cancelled))
        );
        assert_eq!(core.layout(view).unwrap(), &before_rejection);
    } else {
        let installed = core.install_view_layout_job(view, candidate).unwrap();
        assert!(!cancellation.is_cancelled());
        assert_eq!(core.syntax_statistics().publications, 0);
        assert_eq!(core.layout(view).unwrap().configuration_generation(), configuration);
        assert_eq!(
            core.layout(view).unwrap().snapshot().unwrap().revision,
            installed.layout_revision
        );
        assert!(core.poll_syntax(), "the completed syntax remains ready for the next publication");
    }
    assert_eq!(core.syntax_statistics().publications, 1);
    assert_eq!(core.syntax.service.runs(core.syntax_input().identity()).len(), 2);
    assert_eq!(core.syntax_style_names(), ["@provider.undefined", "Keyword"]);
    let sheet = core.document.projection().style_sheet();
    let generated = code_style::resolve_name(sheet, "@provider.undefined")
        .expect("publication generates an implicit definition");
    assert!(sheet.is_implicit_character(generated));
    assert_eq!(core.syntax_statistics().publications, 1, "menu inspection does not publish work");
    core.set_code_language(LanguageSelection::None);
    assert!(core.syntax_style_names().is_empty(), "retired language names are not exposed");
    assert_eq!(core.document.revision(), Revision(0));
    assert!(!core.document.is_dirty());
}

#[test]
fn completed_syntax_does_not_invalidate_its_own_layout_installation() {
    exercise_syntax_finishing_during_layout(false, |_, _| {});
}

#[test]
fn explicit_syntax_publication_still_rejects_prepared_layout() {
    exercise_syntax_finishing_during_layout(true, |_, _| {});
}

#[test]
fn speculative_prelayout_does_not_publish_completed_syntax() {
    exercise_syntax_finishing_during_layout(false, |core, view| {
        let before = core.layout(view).unwrap().clone();
        let publications = core.syntax_statistics().publications;

        let _ = core.prepare_view_prelayout(view, true).unwrap();

        assert_eq!(core.syntax_statistics().publications, publications);
        assert_eq!(core.layout(view).unwrap(), &before);
        assert!(core.layout(view).unwrap().snapshot().is_some());
    });
}

fn event(core: &mut Core<MockTextMeasurementProvider>, view: ViewId, key: Key) {
    let outcome = core
        .handle(view, CoreEvent::Input(InputEvent::Key(key)))
        .unwrap();
    let status = outcome.command.as_ref().map(|c| &c.status);
    assert!(
        matches!(
            status,
            Some(crate::command::CommandStatus::Complete | crate::command::CommandStatus::Pending)
        ),
        "{key:?}: {status:?}"
    );
}

fn exercise(bytes: Vec<u8>, encoding: Encoding, syntax: bool, iterations: usize) -> serde_json::Value {
    let size = bytes.len();
    let began = Instant::now();
    let mut document =
        Document::from_bytes_with_file_format(bytes, encoding, Format::Code, FileFormat::Dos)
            .unwrap();
    // The product's 256 MiB history target counts the complete live document,
    // and may evict undo for an already larger input. Give this undo benchmark
    // finite headroom above its measured live-state cost; record the override.
    let history_budget = document
        .history_status()
        .retained_memory_bytes
        .saturating_add(64 * 1024 * 1024);
    document.set_history_retention_policy(crate::document::HistoryRetentionPolicy::new(
        128,
        history_budget,
    ));
    let work = document.open_work_statistics();
    assert_eq!(work.source_decode_passes(), 1);
    assert_eq!(work.decoded_source_bytes(), size);
    let mut core = Core::new(document);
    core.initialize_code_detection("fixture.rs", false).unwrap();
    if !syntax {
        core.set_code_language(LanguageSelection::None);
    }
    assert!(
        core.code_language_detection().unwrap().bytes_inspected <= detection::DETECTION_BYTE_LIMIT
    );
    let first = core.add_view(MockTextMeasurementProvider::new(), 400., 120.);
    let second = core.add_view(MockTextMeasurementProvider::new(), 400., 120.);
    assert!(core.layout(first).unwrap().snapshot().is_some());
    let first_display = began.elapsed().as_secs_f64() * 1000.;
    let mut inputs = Vec::new();
    let mut scrolls = Vec::new();
    for iteration in 0..iterations {
        let start = Instant::now();
        event(&mut core, first, Key::Char('i'));
        let typed = core
            .handle(first, CoreEvent::Input(InputEvent::text("x\n")))
            .unwrap();
        assert!(typed.document_changed, "typing failed: {:?}", typed.command);
        event(&mut core, first, Key::Escape);
        assert!(
            !core
                .document
                .projection()
                .compatibility_text_is_materialized(),
            "input must not flatten the changed Code document"
        );
        event(&mut core, first, Key::Char('u'));
        inputs.push(start.elapsed().as_secs_f64() * 1000.);
        let start = Instant::now();
        for view in [first, second] {
            core.handle(view, CoreEvent::SetWrap(iteration % 2 == 0))
                .unwrap();
            if iteration % 2 == 0 {
                event(&mut core, view, Key::Char('G'));
            } else {
                event(&mut core, view, Key::Char('g'));
                event(&mut core, view, Key::Char('g'));
            }
            assert!(core.layout(view).unwrap().snapshot().is_some());
        }
        scrolls.push(start.elapsed().as_secs_f64() * 1000.);
        assert!(core.syntax_statistics().maximum_pending <= 1);
        assert_eq!(
            core.syntax_statistics().publications,
            0,
            "providers remain suspended"
        );
    }
    assert!(
        !core.document().is_dirty(),
        "all trial edits were undone: {encoding:?}, {size} bytes, syntax {syntax}, history {:?}",
        core.document().history_status()
    );
    let input = core.syntax_input();
    for at in [0, input.byte_len() / 2, input.byte_len().saturating_sub(1)] {
        assert!(input.chunk_at(at).len() <= 4096);
    }
    serde_json::json!({"encoding":format!("{encoding:?}"),"source_bytes":size,"hard_lines":work.projected_hard_lines(),"syntax_enabled":syntax,"first_display_ms":first_display,"input_insert_newline_undo":percentiles(inputs),"two_view_distant_scroll_wrap_toggle":percentiles(scrolls),"provider_publications":0,"maximum_pending":core.syntax_statistics().maximum_pending,"history_budget_bytes":history_budget})
}
fn percentiles(mut values: Vec<f64>) -> serde_json::Value {
    values.sort_by(f64::total_cmp);
    let at = |p: usize| values[(values.len() - 1) * p / 100];
    serde_json::json!({"samples":values.len(),"p50_ms":at(50),"p95_ms":at(95),"p99_ms":at(99),"maximum_ms":values.last()})
}

#[test]
fn vimrc_opens_as_code_without_changing_source_and_respects_explicit_text() {
    let source = b"\xef\xbb\xbf\" Personal settings\r\nset number\r\nlet g:enabled = 1\r\n";
    for filename in [".vimrc", "_vimrc", "vimrc", ".gvimrc", "settings.vim", ".exrc"] {
        let mut core = Core::<MockTextMeasurementProvider>::new(
            Document::from_bytes(source.to_vec(), Encoding::Utf8, Format::PlainText).unwrap(),
        );
        let revision = core.document.revision();
        core.initialize_code_detection(filename, true).unwrap();
        assert_eq!(core.document.format(), Format::Code, "{filename}");
        assert_eq!(
            core.code_language_detection().unwrap().language.as_deref(),
            Some("vim")
        );
        assert_eq!(core.document.source_bytes(), source, "{filename}");
        assert_eq!(core.document.revision(), revision, "{filename}");
        assert!(!core.document.is_dirty(), "{filename}");
    }
    let mut core = Core::<MockTextMeasurementProvider>::new(Document::new("set number\n"));
    core.initialize_code_detection(".vimrc", false).unwrap();
    assert_eq!(core.document.format(), Format::PlainText);
    assert_eq!(
        core.code_language_detection().unwrap().language.as_deref(),
        Some("vim")
    );
    let mut core = Core::<MockTextMeasurementProvider>::new(Document::new("set number\n"));
    core.set_code_language(LanguageSelection::None);
    core.initialize_code_detection(".vimrc", true).unwrap();
    assert_eq!(core.document.format(), Format::PlainText);
    assert_eq!(core.code_language_detection().unwrap().language, None);
}

#[test]
fn language_detection_is_load_time_state_with_explicit_redetection_and_overrides() {
    let mut core = Core::<MockTextMeasurementProvider>::new(Document::new("ordinary text"));
    core.initialize_code_detection("notes", true).unwrap();
    assert_eq!(core.document.format(), Format::PlainText);
    core.document.insert(0, "// vim: ft=rust\n").unwrap();
    core.document = Document::from_bytes(core.document.source_bytes(), Encoding::Utf8, Format::Code).unwrap();
    core.poll_syntax();
    assert_eq!(
        core.code_language_detection().unwrap().language.as_deref(),
        Some("rust")
    );
    core.document.insert(0, "// vim: ft=swift\n").unwrap();
    core.poll_syntax();
    assert_eq!(
        core.code_language_detection().unwrap().language.as_deref(),
        Some("rust")
    );
    core.set_code_language(LanguageSelection::None);
    core.redetect_code_language(Some("x.py"));
    assert_eq!(core.code_language_detection().unwrap().language, None);
    core.set_code_language(LanguageSelection::Automatic);
    assert_eq!(
        core.code_language_detection().unwrap().language.as_deref(),
        Some("rust"),
        "last valid marker wins"
    );
    let mut core = Core::<MockTextMeasurementProvider>::new(Document::new("x"));
    core.set_code_filename_associations(vec![detection::FilenameAssociation {
        pattern: "*.h".into(),
        language: "cpp".into(),
    }])
    .unwrap();
    core.initialize_code_detection("x.h", true).unwrap();
    assert_eq!(core.document.format(), Format::Code);
    assert_eq!(
        core.code_language_detection().unwrap().language.as_deref(),
        Some("cpp")
    );
}

#[test]
fn platform_provider_factory_runs_on_workers_and_publishes_through_the_shared_core() {
    let _registry = crate::document::syntax::treesitter::package_registry_test_guard();
    struct PlatformProvider;
    impl SyntaxProvider for PlatformProvider {
        fn analyze(&mut self, request: &SyntaxRequest, _: &AtomicBool) -> SyntaxResult {
            assert!(std::thread::current()
                .name()
                .unwrap()
                .starts_with("viem-syntax-"));
            assert_eq!(request.configuration.language.as_deref(), Some("rust"));
            let mut result = SyntaxResult::missing(request, "platform test provider");
            result.coverage = crate::document::syntax::Coverage::Exact;
            result
        }
    }
    let mut core = Core::<MockTextMeasurementProvider>::new(
        Document::from_bytes(b"let x = 1;".to_vec(), Encoding::Utf8, Format::Code).unwrap(),
    );
    core.initialize_code_detection("custom.rs", false).unwrap();
    let revision = core.document.revision();
    core.set_syntax_provider_factory(Arc::new(|| {
        assert!(std::thread::current()
            .name()
            .unwrap()
            .starts_with("viem-syntax-"));
        Box::new(PlatformProvider)
    }));
    core.add_view(MockTextMeasurementProvider::new(), 400., 120.);
    let deadline = Instant::now() + Duration::from_secs(5);
    while !core.syntax_diagnostics().contains("platform test provider") {
        core.poll_syntax();
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    assert_eq!(core.document.revision(), revision);
    assert!(!core.document.is_dirty());
    assert!(core.document.projection().style_spans().is_empty());
}

#[test]
fn pinned_code_pipeline_performance() {
    let _registry = crate::document::syntax::treesitter::package_registry_test_guard();
    let suspended = Suspension::all_workers();
    // The assertions are size-independent; only the timing report needs the
    // million-line and 100 MiB fixtures and repeated samples. Two iterations
    // cover both wrap states and scroll directions.
    let report = std::env::var_os("VIEM_CODE_PIPELINE_REPORT");
    let (lines, iterations) = if report.is_some() { (1_000_000, 20) } else { (100_000, 2) };
    let mut rows = Vec::new();
    for syntax in [false, true] {
        rows.push(exercise(b"x\n".repeat(lines), Encoding::Utf8, syntax, iterations));
        for encoding in [Encoding::Utf8, Encoding::Utf16Le, Encoding::Latin1] {
            let cycle = (0..32)
                .map(|i| {
                    format!(
                        "let x = 'é'; // {}{}",
                        "x".repeat(32 + i * 8),
                        if i % 2 == 0 { "\r\n" } else { "\n" }
                    )
                })
                .collect::<String>();
            let encoded = match encoding {
                Encoding::Utf16Le => cycle
                    .encode_utf16()
                    .flat_map(u16::to_le_bytes)
                    .collect::<Vec<_>>(),
                Encoding::Latin1 => cycle
                    .chars()
                    .map(|c| u8::try_from(c as u32).unwrap())
                    .collect(),
                _ => cycle.into_bytes(),
            };
            let target = if report.is_some() { 100 * 1024 } else { 256 } * 1024;
            let mut bytes = encoded.repeat(target / encoded.len());
            while bytes.len() < target {
                bytes.push(b' ');
                if encoding == Encoding::Utf16Le {
                    bytes.push(0);
                }
            }
            rows.push(exercise(bytes, encoding, syntax, iterations));
        }
    }
    assert_eq!(suspended.calls.load(Ordering::SeqCst), WORKER_COUNT);
    let Some(path) = report else {
        return;
    };
    let command = |name: &str, args: &[&str]| {
        std::process::Command::new(name)
            .args(args)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
            .unwrap_or_else(|| "unavailable".into())
    };
    let result = serde_json::json!({"fixture_revision":1,"hardware":command("sysctl",&["-n","hw.memsize","hw.model","machdep.cpu.brand_string"]),"compiler":command("rustc",&["--version"]),"provider_condition":"all worker callbacks controllably suspended; no foreground provider execution or waits","results":rows});
    std::fs::write(path, serde_json::to_vec_pretty(&result).unwrap()).unwrap();
}

/// Records every analyzed range and reports exact, empty coverage.
struct RecordingProvider(Arc<Mutex<Vec<std::ops::Range<usize>>>>);
impl SyntaxProvider for RecordingProvider {
    fn analyze(&mut self, request: &SyntaxRequest, _: &AtomicBool) -> SyntaxResult {
        self.0.lock().unwrap().push(request.range.clone());
        let mut result = SyntaxResult::missing(request, "recording");
        result.coverage = crate::document::syntax::Coverage::Exact;
        result
    }
}

#[test]
fn complete_highlighting_is_requested_in_bounded_chunks_once_each() {
    let text: String = (0..3000).map(|line| format!("fn function_{line}() {{}}\n")).collect();
    let len = text.len();
    assert!(len > 4 * COMPLETE_SYNTAX_CHUNK_BYTES);
    let mut core = Core::<MockTextMeasurementProvider>::new(
        Document::from_bytes(text.into_bytes(), Encoding::Utf8, Format::Code).unwrap(),
    );
    core.initialize_code_detection("fixture.rs", false).unwrap();
    let ranges = Arc::new(Mutex::new(Vec::new()));
    let recorded = ranges.clone();
    core.set_syntax_provider_factory(Arc::new(move || Box::new(RecordingProvider(recorded.clone()))));
    let view = core.add_view(MockTextMeasurementProvider::new(), 400., 200.);
    let covered = |ranges: &[std::ops::Range<usize>]| {
        let mut sorted = ranges.to_vec();
        sorted.sort_by_key(|range| range.start);
        let mut end = 0;
        for range in sorted {
            if range.start > end { return false; }
            end = end.max(range.end);
        }
        end >= len
    };
    let deadline = Instant::now() + Duration::from_secs(10);
    while !covered(&ranges.lock().unwrap()) {
        core.poll_syntax();
        let _ = core.prepare_view_syntax_wait(view, Instant::now());
        assert!(Instant::now() < deadline, "complete highlighting did not finish: {:?}", ranges.lock().unwrap());
        std::thread::sleep(Duration::from_millis(5));
    }
    let ranges = ranges.lock().unwrap().clone();
    let longest_line = 32;
    assert!(ranges.iter().all(|range| range.len() <= COMPLETE_SYNTAX_CHUNK_BYTES + longest_line),
        "every request fits one chunk: {ranges:?}");
    let mut unique = ranges.clone();
    unique.sort_by_key(|range| (range.start, range.end));
    unique.dedup();
    assert_eq!(unique.len(), ranges.len(), "no chunk is analyzed twice: {ranges:?}");
    // Polling after completion requests nothing further.
    let count = ranges.len();
    for _ in 0..20 {
        core.poll_syntax();
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(core.syntax.service.statistics.requests as usize, count);
}

#[test]
fn highlighting_settles_when_a_collapsed_diff_spans_background_chunks() {
    let _registry = crate::document::syntax::treesitter::package_registry_test_guard();
    let text: String = (0..160).map(|i| format!("Paragraph {i:03} {}\n", "x".repeat(60))).collect();
    let modified = text.replace("Paragraph 010", "Changed 010").replace("Paragraph 145", "Changed 145");
    let side = |source: &str| {
        let mut core = Core::new(Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Code).unwrap());
        core.initialize_code_detection("fixture.md", false).unwrap();
        let recorded = Arc::new(Mutex::new(Vec::new()));
        core.set_syntax_provider_factory(Arc::new(move || Box::new(RecordingProvider(recorded.clone()))));
        let view = core.add_view(MockTextMeasurementProvider::new(), 1000., 500.);
        (core, view)
    };
    let (mut base, left) = side(&text);
    let (mut live, right) = side(&modified);
    let mut diff = DiffSession::new(base.document(), live.document(), true);
    diff.set_collapsed(true);
    diff.sync(&mut base, left, &mut live, right).unwrap();
    for (core, view) in [(&mut base, left), (&mut live, right)] {
        let input = core.syntax_input();
        let chunk_end = complete_syntax_chunk_end(&input, 0);
        let visible = core.visible_syntax_range(&input, &core.views[&view]);
        assert!(0 < visible.start && visible.start < chunk_end && chunk_end < visible.end);
        // Enter the idle phase without waiting for the edit-pause timer.
        core.syntax.input_changed = Some((input.identity(), Instant::now() - COMPLETE_SYNTAX_EDIT_PAUSE));
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let (_, wait) = core.prepare_view_syntax_wait(view, deadline).unwrap();
            if let Some(wait) = wait { wait.wait_until(deadline); }
            if core.syntax.complete_progress == Some((input.identity(), input.byte_len()))
                && !core.syntax.service.has_pending_work() { break; }
            assert!(core.syntax_statistics().requests <= 6, "idle highlighting repeatedly requested the same bytes");
            assert!(Instant::now() < deadline, "background highlighting did not finish");
            std::thread::yield_now();
        }
        assert!(!core.poll_syntax());
        assert!(!core.syntax.service.has_pending_work(), "another frame must stay idle");
    }
}

#[test]
fn complete_highlighting_chunks_end_at_line_starts() {
    let text: String = (0..2000).map(|line| format!("line {line}\n")).collect();
    let input = SyntaxInputSnapshot::new(
        SyntaxInputIdentity { document: 1, revision: 0, generation: 1 },
        crate::document::FormattedTextTree::try_from_text(text.as_str()).unwrap(),
    );
    let mut start = 0;
    let mut chunks = 0;
    while start < text.len() {
        let end = super::complete_syntax_chunk_end(&input, start);
        assert!(end > start);
        assert!(end == text.len() || text.as_bytes()[end - 1] == b'\n', "chunk ends after a line delimiter");
        assert!(end - start <= COMPLETE_SYNTAX_CHUNK_BYTES + 12);
        start = end;
        chunks += 1;
    }
    assert!(chunks <= text.len() / COMPLETE_SYNTAX_CHUNK_BYTES + 1);
    assert!(chunks >= text.len() / (COMPLETE_SYNTAX_CHUNK_BYTES + 12));
}

#[test]
fn complete_highlighting_of_a_large_rust_file_keeps_tree_sitter_styles() {
    use crate::document::syntax::{service::BackendProvider, treesitter::TreeSitterBudget};
    // Style correctness must be reproducible under parallel test load. Use
    // the real provider with its normal work limits and a generous time slice.
    struct Provider(BackendProvider);
    impl SyntaxProvider for Provider {
        fn analyze(&mut self, request: &SyntaxRequest, cancelled: &AtomicBool) -> SyntaxResult {
            let budget = TreeSitterBudget { slice_duration: Duration::from_secs(5), ..Default::default() };
            self.0.analyze_with_budget(request, cancelled, &budget)
        }
    }
    let _registry = crate::document::syntax::treesitter::package_registry_test_guard();
    let mut text = String::from("//! Module documentation.\n//! More of it.\n\nuse std::fmt;\n\n");
    for index in 0..900 {
        text.push_str(&format!("/// Doc for item {index}.\npub fn item_{index}(value: u32) -> u32 {{ value + {index} }}\n"));
    }
    assert!(text.len() > 40 * 1024);
    let len = text.len();
    let mut core = Core::<MockTextMeasurementProvider>::new(
        Document::from_bytes(text.into_bytes(), Encoding::Utf8, Format::Code).unwrap(),
    );
    core.initialize_code_detection("fixture.rs", false).unwrap();
    core.set_syntax_provider_factory(Arc::new(|| Box::new(Provider(BackendProvider::default()))));
    let view = core.add_view(MockTextMeasurementProvider::new(), 600., 300.);
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        core.poll_syntax();
        let _ = core.prepare_view_syntax_wait(view, Instant::now());
        let identity = core.syntax_input().identity();
        if core.syntax.complete_progress == Some((identity, len)) && !core.syntax.service.has_pending_work() {
            break;
        }
        assert!(Instant::now() < deadline, "complete highlighting did not finish");
        std::thread::sleep(Duration::from_millis(5));
    }
    core.poll_syntax();
    let runs = core.syntax.service.runs(core.syntax_input().identity());
    let first = runs.iter().find(|run| run.range.start == 0).expect("the module comment is highlighted");
    assert!(first.name.0.starts_with("Comment"), "Tree-sitter capture, not a Vim group: {:?}", first.name);
    let last_comment = core.document().text().rfind("/// Doc for item").unwrap();
    assert!(runs.iter().any(|run| run.range.contains(&last_comment) && run.name.0.starts_with("Comment")),
        "the last background chunk also retains its Tree-sitter comment style");
    assert!(!core.syntax_diagnostics().contains("slice deadline"), "{}", core.syntax_diagnostics());
}
