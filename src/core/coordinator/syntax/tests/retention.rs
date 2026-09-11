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
