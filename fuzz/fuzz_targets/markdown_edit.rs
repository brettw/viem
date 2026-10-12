//! Open untrusted bytes, attach a laid-out view, and replay a short sequence
//! of Vim/native editing input through the full command/transaction path.
//! Layout: [format][op count][ops...][document bytes...].
#![no_main]
use libfuzzer_sys::fuzz_target;
use viem_core::command::{InputEvent, Key};
use viem_core::document::{Document, Format};
use viem_core::layout::MockTextMeasurementProvider;
use viem_core::{Core, CoreEvent};

const TEXTS: &[&str] = &[
    "a", " ", "*", "**", "_", "`", "```", "#", "# ", ">", "> ", "- ", "1. ", "[", "]", "(", ")",
    "|", "<", "</", "&", "&amp;", "\\", "!", "~~", "<b>", "</b>", "<br>", "\t", "é", "e\u{301}",
    "👍🏽", "\u{202e}", "\0", "[x](y)", "![i](a.png)", "| a | b |\n|---|---|\n",
];

fn event(code: u8) -> Vec<InputEvent> {
    let key = |key| vec![InputEvent::Key(key)];
    let keys = |text: &str| text.chars().map(InputEvent::key).collect::<Vec<_>>();
    match code {
        0..=39 => vec![InputEvent::text(TEXTS[code as usize % TEXTS.len()])],
        40 => key(Key::Escape),
        41 => key(Key::Enter),
        42 => key(Key::ShiftEnter),
        43 => key(Key::Tab),
        44 => key(Key::BackTab),
        45 => key(Key::Backspace),
        46 => key(Key::Delete),
        47 => key(Key::Left),
        48 => key(Key::Right),
        49 => key(Key::Up),
        50 => key(Key::Down),
        51 => key(Key::Home),
        52 => key(Key::End),
        53 => key(Key::DocumentStart),
        54 => key(Key::DocumentEnd),
        55 => key(Key::SelectAll),
        56 => key(Key::PageDown),
        57 => key(Key::PageUp),
        58 => key(Key::WordRight),
        59 => key(Key::ParagraphEnd),
        60 => keys("i"),
        61 => keys("a"),
        62 => keys("A"),
        63 => keys("o"),
        64 => keys("O"),
        65 => keys("x"),
        66 => keys("dd"),
        67 => keys("dw"),
        68 => keys("D"),
        69 => keys("J"),
        70 => keys("u"),
        71 => key(Key::Ctrl('r')),
        72 => keys("p"),
        73 => keys("P"),
        74 => keys("yy"),
        75 => keys("v"),
        76 => keys("V"),
        77 => key(Key::Ctrl('v')),
        78 => keys("w"),
        79 => keys("b"),
        80 => keys("e"),
        81 => keys("G"),
        82 => keys("gg"),
        83 => keys("}"),
        84 => keys("{"),
        85 => keys(">>"),
        86 => keys("<<"),
        87 => keys("~"),
        88 => keys("."),
        89 => keys("r!"),
        90 => keys("R"),
        91 => keys("cw"),
        92 => keys("ciw"),
        93 => keys("dap"),
        94 => keys("j"),
        95 => keys("k"),
        96 => keys("l"),
        97 => keys("h"),
        98 => keys("$"),
        99 => keys("0"),
        100 => keys("gj"),
        101 => keys("gk"),
        102 => keys("*"),
        103 => keys("n"),
        104 => keys("3x"),
        105 => keys("gUU"),
        106 => keys("guu"),
        107 => keys("=="),
        108 => keys("gqq"),
        109 => keys(":s/a/b/g\n"),
        110 => keys(":%s/\\w/x/g\n"),
        111 => keys(":sort\n"),
        112 => keys(":g/a/d\n"),
        113 => keys("/a\n"),
        114 => keys(":2,4d\n"),
        115 => keys("yyP"),
        116 => keys("ddp"),
        117 => keys("xp"),
        118 => keys("vjd"),
        119 => keys("Vjd"),
        120 => keys("vjp"),
        121 => keys("vj>"),
        122 => keys("vjU"),
        123 => keys("vjJ"),
        124 => keys("vj~"),
        125 => keys("vjc"),
        126 => key(Key::Ctrl('w')),
        127 => key(Key::Ctrl('u')),
        128 => key(Key::Ctrl('t')),
        129 => key(Key::Ctrl('d')),
        130 => key(Key::Ctrl('n')),
        131 => key(Key::Ctrl('e')),
        132 => key(Key::Ctrl('y')),
        133 => key(Key::Ctrl('f')),
        134 => key(Key::Ctrl('b')),
        135 => keys("q"),
        _ => vec![InputEvent::key(char::from(b'a' + code % 26))],
    }
}

fuzz_target!(|data: &[u8]| {
    let [selector, count, rest @ ..] = data else {
        return;
    };
    let count = usize::from(*count % 24).min(rest.len());
    let (ops, source) = rest.split_at(count);
    let format = match selector & 3 {
        0 => Format::Markdown,
        1 => Format::MarkdownSource,
        2 => Format::Code,
        _ => Format::PlainText,
    };
    let Ok(document) = Document::from_bytes_detect_encoding(source.to_vec(), format) else {
        return;
    };
    let mut core = Core::new(document);
    let width = [300., 40., 1200., 5.][usize::from(selector >> 2 & 3)];
    let view = core.add_view(MockTextMeasurementProvider::new(), width, 200.);
    if selector & 0x10 != 0 {
        let _ = core.handle(view, CoreEvent::Resize { width: width / 3. + 1., height: 90. });
    }
    for &op in ops {
        if op >= 250 {
            let document = core.document();
            if document.format().is_markdown() {
                let event = CoreEvent::SetMarkdownSource {
                    document: document.id(),
                    revision: document.revision(),
                    source: !document.format().is_source_view(),
                };
                let _ = core.handle(view, event);
            }
            continue;
        }
        for input in event(op) {
            let _ = core.handle(view, CoreEvent::Input(input));
        }
    }
    // Every committed edit must remain reproducible from its saved bytes.
    // Opt-in: this checks editing fidelity rather than input safety.
    if std::env::var_os("VIEM_FUZZ_CONSISTENCY").is_none() {
        return;
    }
    let saved = core.document().source_bytes();
    let reopened = Document::from_bytes_with_file_format(
        saved.clone(),
        core.document().encoding(),
        core.document().format(),
        core.document().file_format(),
    );
    if let Ok(reopened) = reopened {
        if core.document().text() != reopened.text() {
            panic!(
                "saved source does not reproduce the edited projection\nsource: {:?}\nedited: {:?}\nreopened: {:?}",
                String::from_utf8_lossy(&saved),
                core.document().text(),
                reopened.text()
            );
        }
    }
});
