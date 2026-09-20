use crate::support::{Recorder, Rng, RunStats};
use viem_core::command::ex_execute::ExExecuteError;
use viem_core::command::{CommandStatus, ExCommandError, InputEvent, Key};
use viem_core::document::HistoryRetentionPolicy;
use viem_core::layout::{BoundaryAffinity, MockTextMeasurementProvider};
use viem_core::{Core, CoreError, CoreEvent, Document, DocumentError, Encoding, Format, ViewId};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::VecDeque;
use unicode_segmentation::UnicodeSegmentation;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
enum Action {
    Init {
        source: String,
        format: String,
        width: f32,
        height: f32,
    },
    Key {
        view: usize,
        key: String,
    },
    Text {
        view: usize,
        text: String,
    },
    Resize {
        view: usize,
        width: f32,
        height: f32,
    },
    Wrap {
        view: usize,
        enabled: bool,
    },
    Flow {
        view: usize,
        enabled: bool,
    },
    Scale {
        view: usize,
        scale: f32,
    },
    Scroll {
        view: usize,
        left: f32,
        top: f32,
    },
    Format {
        view: usize,
        format: String,
    },
    Place {
        view: usize,
        offset: usize,
        extend: bool,
    },
    RecycleView {
        view: usize,
        width: f32,
        height: f32,
    },
    UndoRedo {
        view: usize,
    },
    Reopen,
}

fn format(name: &str) -> Result<Format, String> {
    Ok(match name {
        "plain" => Format::PlainText,
        "markdown" => Format::Markdown,
        "markdown_source" => Format::MarkdownSource,
        "html" => Format::Html,
        "html_source" => Format::HtmlSource,
        "rtf" => Format::Rtf,
        _ => return Err(format!("invalid format {name}")),
    })
}
fn key(name: &str) -> Result<Key, String> {
    Ok(match name {
        "Escape" => Key::Escape,
        "Enter" => Key::Enter,
        "Backspace" => Key::Backspace,
        "Delete" => Key::Delete,
        "Left" => Key::Left,
        "Right" => Key::Right,
        "Up" => Key::Up,
        "Down" => Key::Down,
        "Home" => Key::Home,
        "End" => Key::End,
        "PageUp" => Key::PageUp,
        "PageDown" => Key::PageDown,
        "Ctrl-r" => Key::Ctrl('r'),
        "Ctrl-v" => Key::Ctrl('v'),
        _ => {
            let mut chars = name.chars();
            let c = chars.next().ok_or("empty key")?;
            if chars.next().is_some() {
                return Err(format!("invalid key {name}"));
            }
            Key::Char(c)
        }
    })
}

fn initial(rng: &mut Rng) -> Action {
    let cases=[
        ("plain","First paragraph.\nSecond line.\n\nLast line."),
        ("plain","e\u{301} 👩\u{200d}💻 🇨🇦 العربية עברית 中文\n\tTabs\0NUL\n"),
        ("markdown","# Heading\n\nA **bold** paragraph with _emphasis_.\ncontinued prose.\n\n1. First item\n   continued\n2. Second\n\n```rust\nfn main() {}\n\n```\n\nEnd."),
        ("markdown_source","- first\n  continuation\n- second\n\n> quote\n\nA [link](https://example.test).\n"),
        ("html","<html><body><h1>Heading</h1><p>First <b>bold</b> &amp; prose.</p><ol><li>one</li><li>two<br>continued</li></ol><pre>a &lt; b\n\ncode</pre><p><br></p></body></html>"),
        ("html_source","<!doctype html><p>Prose\ncontinues <em>here</em>.</p>\n<!--keep--><ul><li>last</li></ul>"),
        ("rtf","{\\rtf1\\ansi First {\\b bold} paragraph.\\par second\\line continued\\par }"),
        ("html","<p></p>"),("markdown",""),("plain",""),
    ];
    let (format, source) = cases[rng.usize(cases.len())];
    let source = if rng.chance(1, 12) {
        match rng.usize(3) {
            0 => "Short paragraph.\n".repeat(1200),
            1 => "long wrapped words 👩\u{200d}💻 ".repeat(3500),
            _ => source.repeat(40),
        }
    } else {
        source.into()
    };
    Action::Init {
        source,
        format: format.into(),
        width: [80., 240., 600.][rng.usize(3)],
        height: [64., 160., 400.][rng.usize(3)],
    }
}

fn push_keys(queue: &mut VecDeque<Action>, view: usize, keys: &str) {
    for c in keys.chars() {
        queue.push_back(Action::Key {
            view,
            key: c.to_string(),
        });
    }
}
fn special(queue: &mut VecDeque<Action>, view: usize, key: &str) {
    queue.push_back(Action::Key {
        view,
        key: key.into(),
    });
}

fn generate(
    rng: &mut Rng,
    core: &Core<MockTextMeasurementProvider>,
    views: &[ViewId],
    queue: &mut VecDeque<Action>,
) {
    let view = rng.usize(views.len());
    // Bounded input batches end in Escape. Operations still record each event,
    // exposing intermediate states and keeping replay independent of batching.
    special(queue, view, "Escape");
    match rng.usize(16) {
        0..=4 => {
            let starts = ["i", "a", "A", "o", "O", "R"];
            push_keys(queue, view, starts[rng.usize(starts.len())]);
            let tokens = [
                "-",
                "a",
                " prose ",
                "é",
                "\u{301}",
                "👩\u{200d}💻",
                "🇨🇦",
                "العربية",
                "中",
                "&",
                "<br>",
                "**",
                "1. ",
                "\t",
                "\0",
            ];
            for _ in 0..1 + rng.usize(5) {
                let text = if core.document().source_byte_len() > 128 * 1024 {
                    "x"
                } else {
                    tokens[rng.usize(tokens.len())]
                };
                queue.push_back(Action::Text {
                    view,
                    text: text.into(),
                });
                if rng.chance(1, 4) {
                    special(queue, view, "Enter");
                }
                if rng.chance(1, 6) {
                    special(queue, view, "Backspace");
                }
            }
            special(queue, view, "Escape");
        }
        5..=7 => {
            let commands = [
                "0", "$", "gg", "G", "w", "b", "e", "j", "k", "h", "l", "dd", "dw", "yy", "p", "P",
                "x", "X", "J", ">>", "<<", "u", "~", ".", "vllx", "Vj~", "ciw",
            ];
            let command = commands[rng.usize(commands.len())];
            if rng.chance(1, 5) {
                push_keys(queue, view, &(1 + rng.usize(4)).to_string());
            }
            push_keys(queue, view, command);
            if command == "ciw" {
                queue.push_back(Action::Text {
                    view,
                    text: "changed".into(),
                });
            }
            special(queue, view, "Escape");
        }
        8 => queue.push_back(Action::Resize {
            view,
            width: [24., 80., 135., 240., 800., 1600.][rng.usize(6)],
            height: [32., 80., 240., 700.][rng.usize(4)],
        }),
        9 => queue.push_back(Action::Scroll {
            view,
            left: [0., 20., 500., 20_000.][rng.usize(4)],
            top: [0., 25., 400., 4000., f32::MAX][rng.usize(5)],
        }),
        10 => {
            queue.push_back(Action::Wrap {
                view,
                enabled: rng.chance(1, 2),
            });
            queue.push_back(Action::Flow {
                view,
                enabled: rng.chance(1, 2),
            });
            queue.push_back(Action::Scale {
                view,
                scale: [0.25, 0.67, 1., 1.25, 2., 5.][rng.usize(6)],
            });
        }
        11 => {
            let name = match core.document().format() {
                Format::Markdown => "markdown_source",
                Format::MarkdownSource => "markdown",
                Format::Html => "html_source",
                Format::HtmlSource => "html",
                Format::Rtf => "rtf",
                Format::PlainText => "plain",
                Format::Code => "code",
            };
            queue.push_back(Action::Format {
                view,
                format: name.into(),
            });
        }
        12 => {
            let mut boundaries = core
                .document()
                .text()
                .grapheme_indices(true)
                .map(|(i, _)| i)
                .collect::<Vec<_>>();
            boundaries.push(core.document().text().len());
            queue.push_back(Action::Place {
                view,
                offset: boundaries[rng.usize(boundaries.len())],
                extend: rng.chance(1, 3),
            });
            special(queue, view, "Escape");
        }
        13 => queue.push_back(Action::UndoRedo { view }),
        14 => queue.push_back(Action::RecycleView {
            view,
            width: 80. + rng.usize(800) as f32,
            height: 40. + rng.usize(300) as f32,
        }),
        _ => {
            if rng.chance(1, 2) {
                queue.push_back(Action::Reopen);
            } else {
                push_keys(queue, view, ":sort");
                special(queue, view, "Enter");
                special(queue, view, "Escape");
            }
        }
    }
}

/// Policy rejections are explicit and counted. In particular verification,
/// stale snapshot, Unicode boundary and layout errors are never ignored.
fn allowed_policy(error: &CoreError) -> bool {
    matches!(
        error,
        CoreError::Document(
            DocumentError::AmbiguousProjection
                | DocumentError::UnsupportedFormatting
                | DocumentError::FormattedPayloadCannotReproject
                | DocumentError::UnrepresentableFormattedCharacter { .. }
        )
    )
}
fn expected_status(status: &CommandStatus) -> bool {
    match status {
        CommandStatus::RegisterReadError(_) => true,
        CommandStatus::Error(message) => {
            matches!(
                message.as_str(),
                "no previous change"
                    | "text object is empty"
                    | "not enough characters on line"
                    | "no line changes to undo"
                    | "no previous visual selection"
                    | "already at the oldest document state"
                    | "already at the newest document state"
            ) || message.starts_with("empty register ")
        }
        CommandStatus::ExError(ExCommandError::Execute(ExExecuteError::Document(error))) => {
            allowed_policy(&CoreError::Document(error.clone()))
        }
        _ => false,
    }
}
fn dispatch(
    core: &mut Core<MockTextMeasurementProvider>,
    view: ViewId,
    event: CoreEvent,
    stats: &mut RunStats,
    recorder: &mut Recorder,
) -> Result<(), String> {
    let before = core.document().source_bytes();
    let revision = core.document().revision();
    let before_context = json!({"source":String::from_utf8_lossy(&before),"text":core.document().text(),
        "format":format!("{:?}",core.document().format()),
        "cursor":core.command_state(view).map(|s|s.cursor()),
        "mode":core.command_state(view).map(|s|format!("{:?}",s.mode()))});
    match core.handle(view, event) {
        Err(error) if allowed_policy(&error) => {
            if core.document().source_bytes() != before || core.document().revision() != revision {
                return Err(format!("rejected operation changed source: {error:?}"));
            }
            stats.expected_rejections += 1;
            recorder.event(json!({"type":"expected_rejection","error":format!("{error:?}")}))?;
        }
        Err(error) => {
            recorder.event(json!({"type":"failure_context","before":before_context,
                "after_source":String::from_utf8_lossy(&core.document().source_bytes()),"error":format!("{error:?}")}))?;
            return Err(format!("unexpected core error: {error:?}"));
        }
        Ok(outcome) => {
            if let Some(command) = outcome.command {
                match command.status {
                    CommandStatus::Complete
                    | CommandStatus::Pending
                    | CommandStatus::Cancelled
                    | CommandStatus::SearchNotFound => {}
                    status if expected_status(&status) => {
                        if core.document().source_bytes() != before
                            || core.document().revision() != revision
                        {
                            return Err(format!("rejected command changed source: {status:?}"));
                        }
                        stats.expected_rejections += 1;
                        recorder.event(
                            json!({"type":"expected_rejection","error":format!("{status:?}")}),
                        )?;
                    }
                    status => {
                        recorder.event(json!({"type":"failure_context","before":before_context,
                        "after_source":String::from_utf8_lossy(&core.document().source_bytes()),"error":format!("{status:?}")}))?;
                        return Err(format!("unexpected command status: {status:?}"));
                    }
                }
            }
        }
    }
    Ok(())
}

fn validate(core: &Core<MockTextMeasurementProvider>, views: &[ViewId]) -> Result<(), String> {
    let doc = core.document();
    for &view in views {
        let cursor = core
            .command_state(view)
            .ok_or("missing view command state")?
            .cursor();
        doc.text_point(cursor)
            .map_err(|e| format!("invalid live cursor {cursor}: {e:?}"))?;
        let layout = core.layout(view).ok_or("missing view layout")?;
        if !layout.viewport_top().is_finite() || !layout.viewport_left().is_finite() {
            return Err("nonfinite viewport".into());
        }
        if let Some(snapshot) = layout
            .snapshot()
            .filter(|s| s.document_revision == doc.revision())
        {
            let mut previous = -f32::MAX;
            for row in snapshot.rows.iter() {
                if !row.y.is_finite()
                    || row.y < previous
                    || !row.height().is_finite()
                    || row.height() <= 0.
                {
                    return Err(format!(
                        "invalid row geometry at {}: y={} height={}",
                        row.hard_line_index,
                        row.y,
                        row.height()
                    ));
                }
                previous = row.y;
                for caret in &row.carets {
                    if !caret.x.is_finite() {
                        return Err("nonfinite caret".into());
                    }
                    doc.text_point(caret.point.text_offset)
                        .map_err(|e| format!("invalid layout caret: {e:?}"))?;
                }
                for cluster in &row.clusters {
                    if !cluster.x.is_finite()
                        || !cluster.advance.is_finite()
                        || cluster.advance < 0.
                        || cluster.text_range.start > cluster.text_range.end
                        || cluster.text_range.end > doc.text().len()
                    {
                        return Err(format!("invalid layout cluster {cluster:?}"));
                    }
                }
            }
            if let Some(max) = layout.maximum_viewport_left() {
                if !max.is_finite() || max < 0. || layout.viewport_left() > max + 0.1 {
                    return Err("horizontal viewport exceeds visible range".into());
                }
            }
        }
    }
    Ok(())
}

pub fn run(
    seed: u64,
    steps: usize,
    replay: Option<&[Value]>,
    recorder: &mut Recorder,
) -> Result<RunStats, String> {
    let mut rng = Rng::new(seed);
    let mut queue = VecDeque::new();
    let mut core: Option<Core<MockTextMeasurementProvider>> = None;
    let mut views = Vec::new();
    let mut stats = RunStats::default();
    let count = replay.map_or(steps, |r| r.len());
    for index in 0..count {
        let action = if let Some(actions) = replay {
            serde_json::from_value(actions[index].clone())
                .map_err(|e| format!("action {index}: {e}"))?
        } else if index == 0 {
            initial(&mut rng)
        } else {
            if queue.is_empty() {
                generate(&mut rng, core.as_ref().unwrap(), &views, &mut queue);
            }
            queue.pop_front().unwrap()
        };
        recorder.record(&action)?;
        stats.actions += 1;
        if let Action::Init {
            source,
            format: kind,
            width,
            height,
        } = &action
        {
            let mut document =
                Document::from_bytes(source.as_bytes().to_vec(), Encoding::Utf8, format(kind)?)
                    .map_err(|e| format!("open: {e:?}"))?;
            document
                .set_history_retention_policy(HistoryRetentionPolicy::new(128, 16 * 1024 * 1024));
            let mut next = Core::new(document);
            views = vec![
                next.try_add_view(MockTextMeasurementProvider::new(), *width, *height)
                    .map_err(|e| format!("view: {e:?}"))?,
                next.try_add_view(
                    MockTextMeasurementProvider::new(),
                    *width * 1.4,
                    *height * 0.7,
                )
                .map_err(|e| format!("second view: {e:?}"))?,
            ];
            core = Some(next);
        } else {
            let core = core.as_mut().ok_or("action before Init")?;
            let before = core.document().source_bytes();
            let (slot, event, pure) = match &action {
                Action::Key { view, key: name } => (
                    *view,
                    Some(CoreEvent::Input(InputEvent::Key(key(name)?))),
                    false,
                ),
                Action::Text { view, text } => {
                    (*view, Some(CoreEvent::Input(InputEvent::text(text))), false)
                }
                Action::Resize {
                    view,
                    width,
                    height,
                } => (
                    *view,
                    Some(CoreEvent::Resize {
                        width: *width,
                        height: *height,
                    }),
                    true,
                ),
                Action::Wrap { view, enabled } => (*view, Some(CoreEvent::SetWrap(*enabled)), true),
                Action::Flow { view, enabled } => {
                    (*view, Some(CoreEvent::SetParagraphFlow(*enabled)), true)
                }
                Action::Scale { view, scale } => (*view, Some(CoreEvent::SetScale(*scale)), true),
                Action::Scroll { view, left, top } => (
                    *view,
                    Some(CoreEvent::SetViewportOrigin {
                        left: *left,
                        top: Some(*top),
                    }),
                    true,
                ),
                Action::Format { view, format: kind } => (
                    *view,
                    Some(CoreEvent::SetFormat {
                        operation: viem_core::FormatOperation::Reinterpret,
                        document: core.document().id(),
                        revision: core.document().revision(),
                        target: format(kind)?,
                    }),
                    true,
                ),
                Action::Place {
                    view,
                    offset,
                    extend,
                } => (
                    *view,
                    Some(CoreEvent::PlaceCursor {
                        document_revision: core.document().revision(),
                        text_offset: *offset,
                        affinity: BoundaryAffinity::Downstream,
                        extend_selection: *extend,
                    }),
                    true,
                ),
                Action::RecycleView {
                    view,
                    width,
                    height,
                } => {
                    let id = *views.get(*view).ok_or("invalid view slot")?;
                    core.remove_view(id)
                        .map_err(|e| format!("remove view: {e:?}"))?;
                    views[*view] = core
                        .try_add_view(MockTextMeasurementProvider::new(), *width, *height)
                        .map_err(|e| format!("recreate view: {e:?}"))?;
                    (*view, None, true)
                }
                Action::UndoRedo { view } => {
                    let id = *views.get(*view).ok_or("invalid view slot")?;
                    if core.document().history_status().can_undo {
                        let original = core.document().text().to_string();
                        dispatch(
                            core,
                            id,
                            CoreEvent::Input(InputEvent::key('u')),
                            &mut stats,
                            recorder,
                        )?;
                        dispatch(
                            core,
                            id,
                            CoreEvent::Input(InputEvent::Key(Key::Ctrl('r'))),
                            &mut stats,
                            recorder,
                        )?;
                        if core.document().source_bytes() != before
                            || core.document().text() != original
                        {
                            return Err("undo/redo did not restore exact source and text".into());
                        }
                    }
                    (*view, None, true)
                }
                Action::Reopen => {
                    let doc = core.document();
                    let reopened = Document::from_bytes_with_file_format(
                        before.clone(),
                        doc.encoding(),
                        doc.format(),
                        doc.file_format(),
                    )
                    .map_err(|e| format!("reopen: {e:?}"))?;
                    if reopened.source_bytes() != before || reopened.text() != doc.text() {
                        return Err(format!(
                            "fresh projection differs: current={:?} fresh={:?}",
                            doc.text(),
                            reopened.text()
                        ));
                    }
                    (0, None, true)
                }
                Action::Init { .. } => unreachable!(),
            };
            let id = *views.get(slot).ok_or("invalid view slot")?;
            if let Some(event) = event {
                dispatch(core, id, event, &mut stats, recorder)?;
            }
            if pure && core.document().source_bytes() != before {
                return Err(format!("non-edit action changed source: {action:?}"));
            }
        }
        let core = core.as_ref().unwrap();
        stats.max_source_bytes = stats
            .max_source_bytes
            .max(core.document().source_byte_len());
        validate(core, &views).map_err(|e| format!("after action {index} {action:?}: {e}"))?;
    }
    Ok(stats)
}
