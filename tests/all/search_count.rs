use viem_core::command::{InputEvent, Key};
use viem_core::document::{Document, Encoding, Format};
use viem_core::layout::MockTextMeasurementProvider;
use viem_core::{Core, CoreEvent, ViewId};

type TestCore = Core<MockTextMeasurementProvider>;
fn key(core: &mut TestCore, view: ViewId, key: Key) {
    core.handle(view, CoreEvent::Input(InputEvent::Key(key))).unwrap();
}
fn command(core: &mut TestCore, view: ViewId, text: &str) {
    for ch in text.chars() { key(core, view, Key::Char(ch)); }
    key(core, view, Key::Enter);
}
fn count(core: &mut TestCore, view: ViewId) -> Option<(usize, usize)> {
    for _ in 0..2_000 {
        if !core.search_work_pending(view).unwrap() {
            return core.search_match_count(view).map(|count| (count.current, count.total));
        }
        core.poll_search(view).unwrap();
    }
    panic!("count scan did not converge");
}

#[test]
fn search_count_follows_navigation_case_options_and_incremental_cancel() {
    let mut core = Core::new(Document::new("hit HIT hit hat"));
    let view = core.add_view(MockTextMeasurementProvider::default(), 400., 100.);
    assert_eq!(count(&mut core, view), None);
    command(&mut core, view, "/hit");
    assert_eq!(count(&mut core, view), Some((2, 2)));
    key(&mut core, view, Key::Char('n'));
    assert_eq!(count(&mut core, view), Some((1, 2)));
    key(&mut core, view, Key::Char('N'));
    assert_eq!(count(&mut core, view), Some((2, 2)));
    command(&mut core, view, ":set ignorecase");
    assert_eq!(count(&mut core, view), Some((3, 3)));
    for ch in "/hat".chars() { key(&mut core, view, Key::Char(ch)); }
    assert_eq!(count(&mut core, view), Some((1, 1)));
    key(&mut core, view, Key::Escape);
    assert_eq!(count(&mut core, view), Some((3, 3)));
    command(&mut core, view, "/absent");
    assert_eq!(count(&mut core, view), Some((0, 0)));
    assert_eq!(core.document().text(), "hit HIT hit hat");
    assert!(!core.document().is_dirty());
}

#[test]
fn search_count_uses_formatted_text_and_counts_zero_width_and_scoped_matches() {
    let document = Document::from_bytes(b"**hit** *hit*\n\nhit\n\nlast".to_vec(), Encoding::Utf8, Format::Markdown).unwrap();
    let mut core = Core::new(document);
    let view = core.add_view(MockTextMeasurementProvider::default(), 400., 100.);
    command(&mut core, view, "/hit");
    assert_eq!(count(&mut core, view), Some((2, 3)));
    command(&mut core, view, "/^");
    assert_eq!(count(&mut core, view), Some((2, 3)));
    for ch in ":1s/hit".chars() { key(&mut core, view, Key::Char(ch)); }
    assert_eq!(count(&mut core, view), Some((1, 2)));
    key(&mut core, view, Key::Escape);
    assert_eq!(count(&mut core, view), Some((2, 3)));
}

#[test]
fn search_count_yields_and_discards_stale_work_between_shared_view_edits() {
    let mut core = Core::new(Document::new(&"hit\n".repeat(20_000)));
    let view = core.add_view(MockTextMeasurementProvider::default(), 400., 100.);
    let other = core.add_view(MockTextMeasurementProvider::default(), 400., 100.);
    command(&mut core, view, ":set noincsearch");
    command(&mut core, view, "/hit");
    assert!(core.search_work_pending(view).unwrap());
    assert!(core.search_match_count(view).is_none());
    assert_eq!(count(&mut core, view), Some((2, 20_000)));
    for ch in "ggdd".chars() { key(&mut core, other, Key::Char(ch)); }
    assert!(core.search_match_count(view).is_none());
    assert!(core.search_work_pending(view).unwrap());
    assert_eq!(count(&mut core, view).unwrap().1, 19_999);
    command(&mut core, view, "/missing");
    assert!(core.search_match_count(view).is_none());
    assert_eq!(count(&mut core, view), Some((0, 0)));
}
