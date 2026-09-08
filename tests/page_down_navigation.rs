use evim_core::command::{layout_motion::LayoutMotionError, CommandStatus, InputEvent, Key};
use evim_core::document::{Encoding, Format};
use evim_core::layout::{
    compute_layout_job, LayoutCancellationToken, LayoutEngine, LayoutExecutionContext,
    LayoutJobPriority, MockTextMeasurementProvider,
};
use evim_core::{Core, CoreEvent, Document};

#[test]
fn page_down_demand_is_atomic_and_resumes_after_exact_layout_is_installed() {
    let source = (0..120)
        .map(|index| format!("## Section {index}\n\nA paragraph with **formatted words** that wraps across several visual rows.\n\n"))
        .collect::<String>();
    for (format, flow) in [
        (Format::Markdown, false),
        (Format::MarkdownSource, false),
        (Format::MarkdownSource, true),
    ] {
        let document =
            Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format).unwrap();
        let mut core = Core::new(document);
        let view = core.add_view(MockTextMeasurementProvider::new(), 320., 160.);
        if format == Format::MarkdownSource {
            core.handle(view, CoreEvent::SetParagraphFlow(flow))
                .unwrap();
        }
        let revision = core.document().revision();
        let mut demand_observed = false;
        for _ in 0..100 {
            let cursor = core.command_state(view).unwrap().cursor();
            let top = core.layout(view).unwrap().viewport_top();
            let result = core
                .handle(view, CoreEvent::Input(InputEvent::Key(Key::PageDown)))
                .unwrap();
            let status = result.command.unwrap().status;
            if let CommandStatus::NeedsMoreLayout(LayoutMotionError::OutsideMaterializedCoverage(
                demand,
            )) = status
            {
                assert_eq!(core.command_state(view).unwrap().cursor(), cursor);
                assert_eq!(core.layout(view).unwrap().viewport_top(), top);
                assert_eq!(core.document().revision(), revision);
                assert_eq!(core.document().source_bytes(), source.as_bytes());
                assert!(cursor < core.document().text().len() / 2);
                let request = core
                    .prepare_view_layout_demand(
                        view,
                        LayoutJobPriority::NewlyExposedRows,
                        &demand,
                        LayoutCancellationToken::new(),
                    )
                    .unwrap();
                let mut worker = LayoutEngine::new(MockTextMeasurementProvider::new());
                let candidate =
                    compute_layout_job(&mut worker, &request, LayoutExecutionContext::WorkerPool)
                        .unwrap();
                core.install_view_layout_job(view, candidate).unwrap();
                let retry = core
                    .handle(view, CoreEvent::Input(InputEvent::Key(Key::PageDown)))
                    .unwrap();
                assert_eq!(retry.command.unwrap().status, CommandStatus::Complete);
                assert!(core.command_state(view).unwrap().cursor() > cursor);
                assert!(core.layout(view).unwrap().viewport_top() > top);
                demand_observed = true;
                break;
            }
            assert_eq!(status, CommandStatus::Complete);
        }
        assert!(demand_observed, "{format:?} flow={flow}");
        assert_eq!(core.document().revision(), revision);
        assert_eq!(core.document().source_bytes(), source.as_bytes());
    }
}
