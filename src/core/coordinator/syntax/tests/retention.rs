use super::*;
use crate::document::syntax::{Coverage, SyntaxRun, SyntaxStyleName};
use crate::document::StyleApplication;

#[test]
fn edits_keep_mapped_highlighting_while_superseded_workers_are_blocked_until_empty_replacement() {
    let _registry = crate::document::syntax::treesitter::package_registry_test_guard();
    struct DelayedReplacement {
        gate: Arc<(Mutex<usize>, Condvar)>,
        calls: Arc<AtomicUsize>,
        started: mpsc::Sender<usize>,
    }
    impl SyntaxProvider for DelayedReplacement {
        fn analyze(&mut self, request: &SyntaxRequest, _: &AtomicBool) -> SyntaxResult {
            let phase = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
            self.started.send(phase).unwrap();
            let (lock, cv) = &*self.gate;
            let mut released = lock.lock().unwrap();
            while *released < phase {
                released = cv.wait(released).unwrap();
            }
            let mut result = SyntaxResult::missing(request, "delayed replacement");
            result.coverage = Coverage::Exact;
            if phase == 1 {
                result.runs = vec![
                    SyntaxRun {
                        range: 0..10,
                        name: SyntaxStyleName("Comment".into()),
                        origin: "controlled comment".into(),
                        priority: 0,
                    },
                    SyntaxRun {
                        range: 11..13,
                        name: SyntaxStyleName("Keyword".into()),
                        origin: "controlled keyword".into(),
                        priority: 0,
                    },
                ];
            }
            result
        }
    }
    let (started, receiver) = mpsc::channel();
    let gate = ReadySyntaxGate {
        gate: Arc::new((Mutex::new(0), Condvar::new())),
        started: receiver,
    };
    let calls = Arc::new(AtomicUsize::new(0));
    let mut core = Core::<MockTextMeasurementProvider>::new(
        Document::from_bytes(
            b"// comment\nfn main() {}\n".to_vec(),
            Encoding::Utf8,
            Format::Code,
        )
        .unwrap(),
    );
    core.initialize_code_detection("fixture.rs", false).unwrap();
    let provider_gate = gate.gate.clone();
    core.set_syntax_provider_factory(Arc::new(move || {
        Box::new(DelayedReplacement {
            gate: provider_gate.clone(),
            calls: calls.clone(),
            started: started.clone(),
        })
    }));
    core.poll_syntax();
    let input = core.syntax_input();
    core.syntax
        .service
        .request(input.clone(), 0..input.byte_len());
    gate.wait_for(1);
    gate.release(1);
    let deadline = Instant::now() + Duration::from_secs(5);
    while core.syntax_statistics().publications == 0 {
        core.poll_syntax();
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    let view = core.add_view(MockTextMeasurementProvider::new(), 400., 120.);
    let spans = |core: &Core<MockTextMeasurementProvider>| {
        core.document
            .projection()
            .style_spans()
            .iter()
            .filter_map(|span| {
                if let StyleApplication::Automatic(id) = &span.application {
                    Some((span.range.clone(), id.0.clone()))
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(
        spans(&core),
        [
            (0..10, "syntax:Comment".into()),
            (11..13, "syntax:Keyword".into())
        ]
    );
    event(&mut core, view, Key::Char('i'));
    core.handle(view, CoreEvent::Input(InputEvent::text("XX")))
        .unwrap();
    gate.wait_for(2);
    assert_eq!(
        spans(&core),
        [
            (2..12, "syntax:Comment".into()),
            (13..15, "syntax:Keyword".into())
        ],
        "The first edited frame retains mapped styles before any new analysis completes"
    );
    assert_eq!(core.syntax_style_names(), ["Comment", "Keyword"]);
    event(&mut core, view, Key::Backspace);
    event(&mut core, view, Key::Backspace);
    assert_eq!(
        spans(&core),
        [
            (0..10, "syntax:Comment".into()),
            (11..13, "syntax:Keyword".into())
        ]
    );
    gate.release(2);
    gate.wait_for(3);
    assert_eq!(
        spans(&core).len(),
        2,
        "A cancelled old-input result cannot clear current provisional colors"
    );
    let history = core.document.history_status();
    let revision = core.document.revision();
    gate.release(3);
    let deadline = Instant::now() + Duration::from_secs(5);
    while !spans(&core).is_empty() {
        core.poll_syntax();
        assert!(
            Instant::now() < deadline,
            "Current completed empty coverage did not replace retained colors"
        );
        std::thread::yield_now();
    }
    assert!(core.syntax_style_names().is_empty());
    assert_eq!(core.document.revision(), revision);
    assert_eq!(core.document.history_status(), history);
}

/// The first result is accepted, then each replacement waits on a test gate.
/// This exercises the actual service -> projection -> immediate layout path,
/// rather than injecting already-published style spans into the document.
fn cpp_with_blocked_replacement(
    source: &str,
    initial: Vec<SyntaxRun>,
) -> (Core<MockTextMeasurementProvider>, ViewId, ReadySyntaxGate) {
    struct BlockedReplacement {
        gate: Arc<(Mutex<usize>, Condvar)>,
        calls: Arc<AtomicUsize>,
        started: mpsc::Sender<usize>,
        initial: Vec<SyntaxRun>,
    }
    impl SyntaxProvider for BlockedReplacement {
        fn analyze(&mut self, request: &SyntaxRequest, _: &AtomicBool) -> SyntaxResult {
            let phase = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
            self.started.send(phase).unwrap();
            let (lock, cv) = &*self.gate;
            let mut released = lock.lock().unwrap();
            while *released < phase { released = cv.wait(released).unwrap(); }
            let mut result = SyntaxResult::missing(request, "controlled C++ replacement");
            result.coverage = Coverage::Exact;
            if phase == 1 { result.runs = self.initial.clone(); }
            result
        }
    }
    let (started, receiver) = mpsc::channel();
    let gate = ReadySyntaxGate {
        gate: Arc::new((Mutex::new(0), Condvar::new())),
        started: receiver,
    };
    let mut core = Core::<MockTextMeasurementProvider>::new(
        Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Code).unwrap(),
    );
    core.initialize_code_detection("fixture.cc", false).unwrap();
    let provider_gate = gate.gate.clone();
    let calls = Arc::new(AtomicUsize::new(0));
    core.set_syntax_provider_factory(Arc::new(move || Box::new(BlockedReplacement {
        gate: provider_gate.clone(), calls: calls.clone(), started: started.clone(),
        initial: initial.clone(),
    })));
    core.poll_syntax();
    let input = core.syntax_input();
    core.syntax.service.request(input.clone(), 0..input.byte_len());
    gate.wait_for(1);
    gate.release(1);
    let deadline = Instant::now() + Duration::from_secs(5);
    while core.syntax_statistics().publications == 0 {
        core.poll_syntax();
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    let view = core.add_view(MockTextMeasurementProvider::new(), 400., 120.);
    (core, view, gate)
}

fn accepted_run(range: std::ops::Range<usize>, name: &str) -> SyntaxRun {
    SyntaxRun {
        range, name: SyntaxStyleName(name.into()),
        origin: "controlled C++ capture".into(), priority: 0,
    }
}

fn automatic_style_at(core: &Core<MockTextMeasurementProvider>, offset: usize) -> Option<String> {
    core.document.projection().style_spans().iter().find_map(|span| {
        if !span.range.contains(&offset) { return None; }
        match &span.application {
            StyleApplication::Automatic(id) => Some(id.0.clone()),
            _ => None,
        }
    })
}

fn layout_paint_at(core: &Core<MockTextMeasurementProvider>, view: ViewId, offset: usize) -> crate::layout::ResolvedTextPaint {
    let layout = core.layout(view).unwrap().snapshot().unwrap();
    layout.paint_runs.iter().find(|run| run.text_range.contains(&offset))
        .map_or(&layout.default_paint, |run| &run.paint).clone()
}

fn insert_at(core: &mut Core<MockTextMeasurementProvider>, view: ViewId, at: usize, text: &str) {
    event(core, view, Key::Char('i'));
    core.handle(view, CoreEvent::PlaceCursor {
        document_revision: core.document.revision(), text_offset: at,
        affinity: BoundaryAffinity::Downstream, extend_selection: false,
    }).unwrap();
    core.handle(view, CoreEvent::Input(InputEvent::text(text))).unwrap();
}

#[test]
fn cpp_typing_inherits_preceding_style_in_first_frame_and_exact_empty_result_corrects_it() {
    let _registry = crate::document::syntax::treesitter::package_registry_test_guard();
    let (mut core, view, gate) = cpp_with_blocked_replacement(
        "int value = 1;\n", vec![accepted_run(0..3, "Keyword")],
    );
    let other = core.add_view(MockTextMeasurementProvider::new(), 260., 100.);
    let original_paint = layout_paint_at(&core, view, 2);
    assert_ne!(original_paint, core.layout(view).unwrap().snapshot().unwrap().default_paint);
    insert_at(&mut core, view, 3, "é");
    gate.wait_for(2);
    for id in [view, other] {
        assert_eq!(layout_paint_at(&core, id, 3), original_paint,
            "the very first edited frame in every view must style inserted text without a provider result");
    }
    assert_eq!(automatic_style_at(&core, 3).as_deref(), Some("syntax:Keyword"));
    core.handle(view, CoreEvent::Input(InputEvent::text("界"))).unwrap();
    assert_eq!(core.document.text(), "inté界 value = 1;\n");
    assert_eq!(automatic_style_at(&core, 5).as_deref(), Some("syntax:Keyword"),
        "typing after an optimistic run inherits its appearance too");
    assert_eq!(layout_paint_at(&core, view, 5), original_paint);
    assert_eq!(core.syntax_statistics().publications, 1, "both provider replacements remain blocked");

    gate.release(2);
    gate.wait_for(3);
    core.poll_syntax();
    assert_eq!(automatic_style_at(&core, 5).as_deref(), Some("syntax:Keyword"),
        "a stale result cannot clear provisional styling for newer input");
    let history = core.document.history_status();
    let revision = core.document.revision();
    gate.release(3);
    let deadline = Instant::now() + Duration::from_secs(5);
    while automatic_style_at(&core, 5).is_some() {
        core.poll_syntax();
        assert!(Instant::now() < deadline, "exact empty coverage must clear optimistic syntax");
        std::thread::yield_now();
    }
    core.materialize_immediate_viewport(view, ImmediateLayoutIntent::PreserveViewport).unwrap();
    assert_eq!(layout_paint_at(&core, view, 5), core.layout(view).unwrap().snapshot().unwrap().default_paint);
    assert_eq!(core.document.revision(), revision);
    assert_eq!(core.document.history_status(), history);
}

#[test]
fn cpp_typing_inherits_only_the_immediately_preceding_character_at_unicode_and_style_edges() {
    let _registry = crate::document::syntax::treesitter::package_registry_test_guard();
    for (source, range, at, inserted, expected) in [
        ("int value;\n", 0..3, 0, "x", None),
        ("int value;\n", 0..3, 1, "β", Some("syntax:Keyword")),
        ("int value;\n", 0..3, 3, "x", Some("syntax:Keyword")),
        ("int value;\n", 0..3, 4, "x", None),
        ("int value;\n", 0..3, 3, "\u{301}", Some("syntax:Keyword")),
        ("// café\n", 0..8, 8, "👩‍💻", Some("syntax:Keyword")),
    ] {
        let (mut core, view, _gate) = cpp_with_blocked_replacement(source, vec![accepted_run(range, "Keyword")]);
        let expected_paint = if expected.is_some() {
            layout_paint_at(&core, view, at - 1)
        } else {
            core.layout(view).unwrap().snapshot().unwrap().default_paint.clone()
        };
        insert_at(&mut core, view, at, inserted);
        assert_eq!(automatic_style_at(&core, at).as_deref(), expected,
            "source={source:?}, insertion={inserted:?} at {at}");
        assert_eq!(layout_paint_at(&core, view, at), expected_paint,
            "source={source:?}, insertion={inserted:?} at {at}");
        assert_eq!(core.syntax_statistics().publications, 1);
    }
}
