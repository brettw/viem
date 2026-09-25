//! Popup geometry follows the completed word in the portable layout.

use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use viem_core::command::{InputEvent, Key};
use viem_core::document::BoundaryAffinity;
use viem_core::layout::{
    MeasurementEnvironmentId, MeasurementError, MetricsGeneration, MockTextMeasurementProvider,
    RenderRunPolicy, ShapeRequest, ShapedFragment, TextMeasurementProvider,
};
use viem_core::{CompletionPopupAnchor, Core, CoreEvent, Document, ViewId};

fn key<P: TextMeasurementProvider>(core: &mut Core<P>, view: ViewId, key: Key) {
    core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(key)))
        .unwrap();
}

fn start<P: TextMeasurementProvider>(core: &mut Core<P>, view: ViewId, at: usize) {
    key(core, view, Key::Char('i'));
    core.handle(
        view,
        CoreEvent::PlaceCursor {
            document_revision: core.document().revision(),
            text_offset: at,
            affinity: BoundaryAffinity::Upstream,
            extend_selection: false,
        },
    )
    .unwrap();
    key(core, view, Key::Ctrl('n'));
}

fn anchor<P: TextMeasurementProvider>(core: &Core<P>, view: ViewId) -> CompletionPopupAnchor {
    core.completion_popup_anchor(view)
        .unwrap()
        .expect("the completed word is visible")
}

fn selected<P: TextMeasurementProvider>(core: &Core<P>, view: ViewId) -> Option<&str> {
    let menu = core.completion_presentation(view).unwrap().unwrap();
    menu.selected_index.map(|index| menu.items[index].as_str())
}

#[test]
fn ltr_popup_stays_at_the_word_start_across_longer_shorter_and_original_candidates() {
    let source = "xx al\nalps alphabet albatross\n  tail";
    let mut core = Core::new(Document::new(source));
    let view = core.add_view(MockTextMeasurementProvider::new(), 400.0, 240.0);
    start(&mut core, view, "xx al".len());
    let initial = anchor(&core, view);
    let revision = core.document().revision();
    let history = core.document().history_status();
    assert!(!initial.right_to_left);
    assert_eq!(initial.rect.width, 0.0);
    for word in [
        Some("alps"),
        Some("alphabet"),
        Some("albatross"),
        None,
        Some("alps"),
    ] {
        assert_eq!(selected(&core, view), word);
        let current = anchor(&core, view);
        assert_eq!(
            current.rect, initial.rect,
            "candidate {word:?} moved the popup"
        );
        let snapshot = core.presentation_layout(view).unwrap().snapshot().unwrap();
        let first_letter = snapshot.rows[0]
            .clusters
            .iter()
            .find(|cluster| cluster.text_range.start == "xx ".len())
            .unwrap();
        assert_eq!(current.rect.x, first_letter.x);
        assert_eq!(current.layout_revision, snapshot.revision);
        assert_eq!(core.document().source_bytes(), source.as_bytes());
        assert_eq!(core.document().revision(), revision);
        assert_eq!(core.document().history_status(), history);
        key(&mut core, view, Key::Ctrl('n'));
    }
    key(&mut core, view, Key::Escape);
    assert!(core.completion_popup_anchor(view).unwrap().is_none());
}

#[test]
fn popup_follows_word_rewrapping_and_resize_instead_of_its_moving_end_caret() {
    let source = "xx al\nalps alphabet";
    let mut core = Core::new(Document::new(source));
    let view = core.add_view(MockTextMeasurementProvider::new(), 75.0, 240.0);
    core.handle(view, CoreEvent::SetWrap(true)).unwrap();
    start(&mut core, view, "xx al".len());
    assert_eq!(selected(&core, view), Some("alps"));
    let short = anchor(&core, view);
    key(&mut core, view, Key::Ctrl('n'));
    assert_eq!(selected(&core, view), Some("alphabet"));
    let wrapped = anchor(&core, view);
    assert!(
        wrapped.rect.y > short.rect.y,
        "the longer word must wrap to a later row"
    );
    assert!(
        wrapped.rect.x < short.rect.x,
        "the wrapped word begins at the new row origin"
    );
    assert_ne!(wrapped.layout_revision, short.layout_revision);
    key(&mut core, view, Key::Ctrl('p'));
    assert_eq!(anchor(&core, view).rect, short.rect);

    key(&mut core, view, Key::Ctrl('n'));
    core.handle(
        view,
        CoreEvent::Resize {
            width: 400.0,
            height: 240.0,
        },
    )
    .unwrap();
    let widened = anchor(&core, view);
    assert_eq!(widened.rect, short.rect);
    assert_ne!(widened.layout_revision, wrapped.layout_revision);
    assert_eq!(
        widened.layout_revision,
        core.presentation_layout(view)
            .unwrap()
            .snapshot()
            .unwrap()
            .revision
    );
    assert_eq!(core.document().source_bytes(), source.as_bytes());
}

#[test]
fn popup_direction_and_leading_edge_follow_the_word_inside_mixed_direction_text() {
    for (source, before, prefix, word, rtl) in [
        ("English אב\nאבגד אבגדה", "English ", "אב", "אבגד", true),
        (
            "עברית al\nalphabet alpine",
            "עברית ",
            "al",
            "alphabet",
            false,
        ),
    ] {
        let mut core = Core::new(Document::new(source));
        let view = core.add_view(MockTextMeasurementProvider::new(), 400.0, 240.0);
        start(&mut core, view, before.len() + prefix.len());
        assert_eq!(selected(&core, view), Some(word));
        let popup = anchor(&core, view);
        assert_eq!(
            popup.right_to_left, rtl,
            "word direction overrides preceding context"
        );
        let snapshot = core.presentation_layout(view).unwrap().snapshot().unwrap();
        let word_range = before.len()..before.len() + word.len();
        let word_clusters: Vec<_> = snapshot.rows[0]
            .clusters
            .iter()
            .filter(|cluster| word_range.contains(&cluster.text_range.start))
            .collect();
        assert!(!word_clusters.is_empty());
        let leading_edge = if rtl {
            word_clusters
                .iter()
                .map(|cluster| cluster.x + cluster.advance)
                .fold(f32::NEG_INFINITY, f32::max)
        } else {
            word_clusters
                .iter()
                .map(|cluster| cluster.x)
                .fold(f32::INFINITY, f32::min)
        };
        assert_eq!(
            popup.rect.x, leading_edge,
            "popup must align to the word's visual leading edge"
        );
        assert_eq!(core.document().source_bytes(), source.as_bytes());
    }
}

#[test]
fn empty_prefix_anchor_remains_before_the_inserted_candidate() {
    let source = "before \nalphabet alpine";
    let at = "before ".len();
    let mut core = Core::new(Document::new(source));
    let view = core.add_view(MockTextMeasurementProvider::new(), 400.0, 240.0);
    let original = core
        .presentation_layout(view)
        .unwrap()
        .snapshot()
        .unwrap()
        .logical_endpoint_geometry(at, BoundaryAffinity::Upstream)
        .unwrap()
        .rect;
    start(&mut core, view, at);
    for word in ["alphabet", "alpine"] {
        assert_eq!(selected(&core, view), Some(word));
        let popup = anchor(&core, view);
        assert_eq!(popup.rect, original);
        let snapshot = core.presentation_layout(view).unwrap().snapshot().unwrap();
        let end = snapshot
            .logical_endpoint_geometry(at + word.len(), BoundaryAffinity::Upstream)
            .unwrap();
        assert!(
            popup.rect.x < end.rect.x,
            "the popup must not follow the inserted suffix end"
        );
        key(&mut core, view, Key::Ctrl('n'));
    }
    assert_eq!(core.document().source_bytes(), source.as_bytes());
}

struct CountedProvider {
    mock: MockTextMeasurementProvider,
    bytes: Arc<AtomicUsize>,
}
impl TextMeasurementProvider for CountedProvider {
    fn measurement_environment_id(&self) -> MeasurementEnvironmentId {
        self.mock.measurement_environment_id()
    }
    fn metrics_generation(&self) -> MetricsGeneration {
        self.mock.metrics_generation()
    }
    fn render_run_policy(&self) -> Option<RenderRunPolicy> {
        self.mock.render_run_policy()
    }
    fn shape_batch(
        &mut self,
        requests: &[ShapeRequest<'_>],
    ) -> Result<Vec<ShapedFragment>, MeasurementError> {
        self.bytes.fetch_add(
            requests.iter().map(|request| request.text.len()).sum(),
            Ordering::Relaxed,
        );
        self.mock.shape_batch(requests)
    }
}

#[test]
fn repeated_popup_queries_in_a_large_document_reuse_exact_layout_without_shaping() {
    let source = format!(
        "xx al\nalps alphabet\n{}",
        "unchanged writing\n".repeat(20_000)
    );
    let bytes = Arc::new(AtomicUsize::new(0));
    let mut core = Core::new(Document::new(&source));
    let view = core.add_view(
        CountedProvider {
            mock: MockTextMeasurementProvider::new(),
            bytes: bytes.clone(),
        },
        400.0,
        120.0,
    );
    bytes.store(0, Ordering::Relaxed);
    start(&mut core, view, "xx al".len());
    assert!(
        core.completion_presentation(view)
            .unwrap()
            .unwrap()
            .searching
    );
    assert!(
        bytes.load(Ordering::Relaxed) < 20_000,
        "preview layout must stay local"
    );
    let initial = anchor(&core, view);
    bytes.store(0, Ordering::Relaxed);
    for _ in 0..20 {
        assert_eq!(anchor(&core, view), initial);
    }
    assert_eq!(
        bytes.load(Ordering::Relaxed),
        0,
        "reading an anchor must not start new shaping"
    );
    assert_eq!(core.document().source_bytes(), source.as_bytes());
    assert!(!core.document().is_dirty());
}
