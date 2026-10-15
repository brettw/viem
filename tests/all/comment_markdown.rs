//! Simplified Markdown in Code comments: the buffer option and typed fences.
use std::time::{Duration, Instant};
use viem_core::command::{InputEvent, Key};
use viem_core::document::{Encoding, Format, ResolvedCharacterStyle};
use viem_core::layout::{DocumentLayoutStyles, MockTextMeasurementProvider};
use viem_core::{Core, CoreEvent, Document, ViewId};

type TestCore = Core<MockTextMeasurementProvider>;

fn code(source: &str) -> (TestCore, ViewId) {
    code_in_view(source, 400.0)
}

/// Recognition covers the visible comments, so a tall view shows a whole file.
fn code_in_view(source: &str, height: f32) -> (TestCore, ViewId) {
    let document =
        Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, Format::Code).unwrap();
    let mut core = Core::new(document);
    core.initialize_code_detection("fixture.rs", false).unwrap();
    let view = core.add_view(MockTextMeasurementProvider::new(), 600.0, height);
    (core, view)
}

fn keys(core: &mut TestCore, view: ViewId, text: &str) {
    for character in text.chars() {
        let key = match character {
            '\n' => Key::Enter,
            '\u{1b}' => Key::Escape,
            other => Key::Char(other),
        };
        core.handle_with_layout(view, CoreEvent::Input(InputEvent::Key(key))).unwrap();
    }
}

fn style(core: &TestCore, needle: &str, nth: usize) -> ResolvedCharacterStyle {
    let text = core.document().text();
    let at = text.match_indices(needle).nth(nth).unwrap_or_else(|| panic!("{needle:?} in {text:?}")).0;
    DocumentLayoutStyles::character_at(core.document().projection(), at, false).unwrap()
}

fn settle(core: &mut TestCore, done: impl Fn(&TestCore) -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        core.poll_syntax();
        if done(core) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    false
}

#[test]
fn option_is_a_buffer_override_of_the_application_default() {
    let (mut core, view) = code("// **a**\nfn f() {}\n");
    assert!(core.comment_markdown().effective(), "on by default");
    core.set_comment_markdown_default(false);
    assert!(!core.comment_markdown().effective());
    keys(&mut core, view, ":set commentmarkdown\n");
    assert_eq!(core.comment_markdown().local_override(), Some(true));
    assert!(core.comment_markdown().effective());
    keys(&mut core, view, ":set cmd!\n");
    assert_eq!(core.comment_markdown().local_override(), Some(false));
    keys(&mut core, view, ":set cmd&\n");
    assert_eq!(core.comment_markdown().local_override(), None);
    assert!(!core.comment_markdown().effective());
    core.set_comment_markdown_default(true);
    assert!(core.comment_markdown().effective());
    let second = core.add_view(MockTextMeasurementProvider::new(), 600.0, 400.0);
    keys(&mut core, second, ":set nocommentmarkdown\n");
    assert!(!core.comment_markdown().effective(), "every view shares the buffer value");
    assert!(!core.command_state(view).unwrap().comment_markdown().effective());
    assert!(!core.document().is_dirty());
    assert!(!core.document().history_status().can_undo, "options never enter undo");
}

#[test]
fn typed_strong_and_fences_present_without_changing_source() {
    let (mut core, view) = code("fn main() {}\n");
    keys(&mut core, view, ":set commentmarkdown\n");
    keys(&mut core, view, "O// Some **bold\n// text** here\u{1b}");
    core.poll_syntax();
    assert!(style(&core, "bold", 0).bold, "recognition is synchronous");
    assert!(style(&core, "text", 0).bold, "strong continues across the leader");
    assert!(!style(&core, "// text", 0).bold);
    assert!(!style(&core, "here", 0).bold);

    // Three backticks and an optional language open a fence that the end of
    // the comment closes; typing three more backticks closes it explicitly.
    keys(&mut core, view, "o// ```python\n// let x = 1\u{1b}");
    assert!(settle(&mut core, |core| style(core, "//", 0).foreground != style(core, "fn", 0).foreground),
        "the Rust provider highlights comments");
    let comment = style(&core, "//", 0);
    assert!(settle(&mut core, |core| style(core, "let", 0).foreground != comment.foreground),
        "fenced code is carved out of the comment presentation");
    assert_eq!(style(&core, "```", 0).foreground, comment.foreground);
    assert_eq!(style(&core, "python", 0).foreground, comment.foreground);
    assert!(style(&core, "fn", 0).foreground != comment.foreground || style(&core, "fn", 0).foreground_is_default);
    keys(&mut core, view, "o// ```\n// after `x`\u{1b}");
    core.poll_syntax();
    assert_eq!(style(&core, "after", 0).foreground, comment.foreground, "the fence closed");
    assert_eq!(
        core.document().text(),
        "// Some **bold\n// text** here\n// ```python\n// let x = 1\n// ```\n// after `x`\nfn main() {}\n",
        "presentation never changes source"
    );

    keys(&mut core, view, ":set nocommentmarkdown\n");
    assert!(settle(&mut core, |core| !style(core, "bold", 0).bold));
    assert_eq!(style(&core, "let", 0).foreground, comment.foreground);
}

#[test]
fn the_demo_example_presents_what_its_comments_describe() {
    let (mut core, _view) = code_in_view(include_str!("../../examples/comment_markdown_demo.rs"), 20_000.0);
    core.poll_syntax();
    let at = |needle: &str| style(&core, needle, 0);
    for strong in ["**strong**", "continue across", "several lines**", "**double asterisks**",
        "__double underscores__", "strong text can", "continue** across", "**Trailing**", "outer comment**"] {
        assert!(at(strong).bold, "{strong}");
    }
    for italic in ["_emphasis_,", "*single asterisks*", "_underscores_", "*emphasis*,"] {
        assert!(at(italic).slant != viem_core::document::FontSlant::Upright, "{italic}");
    }
    assert!(at("~~strikethrough~~.").strikethrough);
    assert!(at("https://doc.rust-lang.org/book/").underline, "bare URLs are links");
    assert!(at("https://www.rust-lang.org>").underline, "autolinks are links");
    for plain in ["does not", "close** across", "snake_case_name", "\\*asterisks", "# not a heading",
        "- not a list", "# Comment Markdown demo", "**not a comment**", "**also not a comment**", "*plain*"] {
        let resolved = at(plain);
        assert!(!resolved.bold && resolved.slant == viem_core::document::FontSlant::Upright, "{plain}");
    }
    let comment = |core: &TestCore| style(core, "// Plain", 0).foreground;
    assert!(settle(&mut core, |core| comment(core) != style(core, "fn main", 0).foreground));
    for code in ["**not bold**", "\"language\"", "def new", "let answer", "cargo run --example comment_markdown_demo\n"] {
        assert!(settle(&mut core, |core| style(core, code, 0).foreground != comment(core)), "{code} is code");
        assert!(!style(&core, code, 0).bold, "{code}");
    }
    assert_eq!(style(&core, "python", 0).foreground, comment(&core), "fence languages are delimiters");
}
