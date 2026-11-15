//! Read-only script queries over the host's current buffer/window compatibility view.
use super::*;
use crate::vimscript::{Dict, EvalResult, Value, VimError};
use unicode_segmentation::UnicodeSegmentation;

pub(crate) fn script_window_matches(value: Option<&Value>) -> Result<bool, VimError> {
    Ok(match value {
        None => true,
        Some(value) => matches!(value.to_number()?, 0 | 1 | 1000),
    })
}

pub(crate) fn script_buffer_matches(value: Option<&Value>, filename: Option<&str>) -> Result<bool, VimError> {
    Ok(match value {
        None => true,
        Some(Value::Number(number)) => matches!(number, 0 | 1),
        Some(Value::String(text)) => {
            let text = text.to_str_lossy().into_owned();
            matches!(text.as_str(), "" | "%") || filename.is_some_and(|name| name == text
                || std::path::Path::new(name).file_name().is_some_and(|name| name.to_string_lossy() == text))
        }
        Some(value) => matches!(value.to_number()?, 0 | 1),
    })
}

fn history_kind(name: &str) -> Option<u8> {
    match name {
        "cmd" | "c" | ":" => Some(0), "search" | "s" | "/" | "?" => Some(1),
        "expr" | "e" | "=" => Some(2), "input" | "i" | "@" | "debug" | "d" | ">" => Some(3),
        _ => None,
    }
}

impl CommandInterpreter {
    pub(super) fn script_buffer_query(&mut self, document: &Document, filename: Option<&str>, name: &str, args: &[Value]) -> Option<EvalResult<Value>> {
        if !matches!(name, "bufnr" | "bufexists" | "buflisted" | "bufloaded" | "bufname" | "expand_file"
            | "winnr" | "tabpagenr" | "winbufnr" | "win_id2win" | "bufwinnr" | "win_getid" | "bufwinid" | "tabpagewinnr"
            | "winheight" | "winwidth" | "winline" | "wincol" | "getbufline" | "getbufoneline" | "wordcount"
            | "histnr" | "histget" | "histadd" | "histdel" | "getmarklist" | "getjumplist" | "getbufinfo" | "getwininfo" | "gettabinfo" | "undotree") { return None; }
        let result = (|| {
            let buffer = || script_buffer_matches(args.first(), filename);
            let window = || script_window_matches(args.first());
            Ok(match name {
                "bufnr" => if buffer()? { 1.into() } else { (-1).into() },
                "bufexists" | "buflisted" | "bufloaded" => {
                    let exists = match args.first() {
                        Some(Value::String(text)) if text.as_bytes().is_empty() => false,
                        Some(Value::String(text)) if text.as_bytes() == b"%" => filename.is_some_and(|name| std::path::Path::new(name).file_name().is_some_and(|name| name == "%")),
                        _ => buffer()?,
                    };
                    i64::from(exists).into()
                },
                "bufname" | "expand_file" => if buffer()? { filename.unwrap_or("").into() } else { "".into() },
                "winnr" | "tabpagenr" => 1.into(),
                "winbufnr" => if window()? { 1.into() } else { (-1).into() },
                "win_id2win" => i64::from(args.first().map(Value::to_number).transpose()?.unwrap_or(0) == 1000).into(),
                "bufwinnr" => if buffer()? { 1.into() } else { (-1).into() },
                "win_getid" => if matches!(args.first().map(Value::to_number).transpose()?.unwrap_or(0), 0 | 1) { 1000.into() } else { 0.into() },
                "bufwinid" => if buffer()? { 1000.into() } else { (-1).into() },
                "tabpagewinnr" => if window()? { 1.into() } else { 0.into() },
                "winheight" | "winwidth" => if window()? { (if name == "winheight" { 40 } else { 80 }).into() } else { (-1).into() },
                "winline" | "wincol" => 1.into(),
                "getbufline" | "getbufoneline" => {
                    let line = |index: usize, default| -> Result<i64, VimError> {
                        args.get(index).map(|value| match value {
                            Value::String(text) if text.as_bytes() == b"$" => Ok(document.physical_source_line_number_count() as i64),
                            _ => value.to_number(),
                        }).unwrap_or(Ok(default))
                    };
                    let first = line(1, 1)?; let last = line(2, first)?;
                    let mut lines = Vec::new();
                    if buffer()? && first >= 1 {
                        for index in first..=last.min(document.physical_source_line_number_count() as i64) {
                            let physical = document.physical_line(index as usize - 1).map_err(|error| VimError::plain(error.to_string()))?;
                            lines.push(Value::from(super::scripting::script_line_value(physical.content_text(), document.file_format())));
                        }
                    }
                    if name == "getbufoneline" { lines.into_iter().next().unwrap_or_else(|| "".into()) } else { Value::list(lines) }
                }
                "wordcount" => {
                    let cursor_line = self.script_cursor_line(document).max(1) as usize;
                    let cursor_column = self.script_cursor_column(document).max(1) as usize - 1;
                    let mut total = [0i64; 3]; let mut cursor = [0i64; 3];
                    for index in 0..document.physical_source_line_number_count() {
                        let physical = document.physical_line(index).map_err(|error| VimError::plain(error.to_string()))?;
                        let text = physical.content_text();
                        let count = |text: &str| [text.split_whitespace().count() as i64, text.chars().count() as i64, text.len() as i64];
                        let counts = count(text);
                        for kind in 0..3 { total[kind] += counts[kind] + i64::from(kind != 0); }
                        if index + 1 < cursor_line {
                            for kind in 0..3 { cursor[kind] += counts[kind] + i64::from(kind != 0); }
                        } else if index + 1 == cursor_line {
                            let end = text.grapheme_indices(true).find(|(offset, _)| *offset >= cursor_column).map_or(text.len(), |(offset, value)| offset + value.len());
                            let counts = count(&text[..end]);
                            for kind in 0..3 { cursor[kind] += counts[kind]; }
                        }
                    }
                    Value::Dict(Dict::from_pairs([
                        ("words".into(), total[0].into()), ("chars".into(), total[1].into()), ("bytes".into(), total[2].into()),
                        ("cursor_words".into(), cursor[0].into()), ("cursor_chars".into(), cursor[1].into()), ("cursor_bytes".into(), cursor[2].into()),
                    ]))
                }
                "histnr" | "histget" | "histadd" | "histdel" => {
                    let kind = args.first().map(Value::to_text).transpose()?.and_then(|name| history_kind(&name));
                    let Some(kind) = kind else {
                        return Ok(if name == "histnr" { (-1).into() } else if name == "histget" { "".into() } else { 0.into() });
                    };
                    if kind == 3 { return Ok(if name == "histnr" { (-1).into() } else if name == "histget" { "".into() } else { 0.into() }); }
                    let history = match kind { 1 => &mut self.search_history, 2 => &mut self.expression_history, _ => &mut self.ex_history };
                    match name {
                        "histnr" => if history.is_empty() { (-1).into() } else { (history.len() as i64).into() },
                        "histget" => {
                            let index = args.get(1).map(Value::to_number).transpose()?.unwrap_or(-1);
                            let index = if index < 0 { history.len().checked_sub(index.unsigned_abs() as usize) } else { (index as usize).checked_sub(1) };
                            index.and_then(|index| history.get(index)).cloned().unwrap_or_default().into()
                        }
                        "histadd" => {
                            let entry = args.get(1).map(Value::to_text).transpose()?.unwrap_or_default();
                            push_history(history, entry, self.vim_options.number("history") as usize); 1.into()
                        }
                        _ => { history.clear(); 1.into() }
                    }
                }
                "undotree" => self.script_undo_tree(document),
                "getmarklist" => {
                    if args.first().is_some() && !buffer()? { return Ok(Value::list(Vec::new())); }
                    let local = args.first().is_some();
                    let mut items = Vec::new();
                    for (&mark, &offset) in &self.marks {
                        if mark.is_ascii_uppercase() == local || mark == '`' { continue; }
                        if let Some(pos) = self.script_query_position(document, offset) {
                            let mut pairs = vec![("mark".into(), format!("'{mark}").into()), ("pos".into(), pos)];
                            if !local { pairs.push(("file".into(), filename.unwrap_or("").into())); }
                            items.push(Value::Dict(Dict::from_pairs(pairs)));
                        }
                    }
                    Value::list(items)
                }
                "getjumplist" => {
                    if !window()? { return Ok(Value::list(vec![Value::list(Vec::new()), 0.into()])); }
                    let items = if window()? { self.jumps.iter().filter_map(|offset| {
                        let physical = document.physical_line_at_text(*offset).ok()?;
                        let source = document.projection().source_insertion_point(*offset, true)?;
                        let text = document.physical_source_text(physical.content_range.start..source).ok()?;
                        Some(Value::Dict(Dict::from_pairs([
                            ("bufnr".into(), 1.into()), ("lnum".into(), (physical.index as i64 + 1).into()),
                            ("col".into(), (text.len() as i64).into()), ("coladd".into(), 0.into()),
                        ])))
                    }).collect() } else { Vec::new() };
                    Value::list(vec![Value::list(items), (self.jump_index as i64).into()])
                }
                "getbufinfo" => {
                    if !matches!(args.first(), Some(Value::Dict(_))) && !buffer()? { return Ok(Value::list(Vec::new())); }
                    Value::list(vec![Value::Dict(Dict::from_pairs([
                        ("bufnr".into(), 1.into()), ("name".into(), filename.unwrap_or("").into()),
                        ("lnum".into(), self.script_cursor_line(document).into()), ("linecount".into(), (document.physical_source_line_number_count() as i64).into()),
                        ("loaded".into(), 1.into()), ("listed".into(), 1.into()), ("hidden".into(), 0.into()),
                        ("changed".into(), i64::from(document.history_status().is_dirty).into()),
                        ("variables".into(), Value::Dict(self.script.buffer_vars().clone())),
                        ("windows".into(), Value::list(vec![1000.into()])),
                    ]))])
                }
                "getwininfo" | "gettabinfo" => {
                    let number = args.first().map(Value::to_number).transpose()?;
                    let valid = if name == "getwininfo" { matches!(number, None | Some(0 | 1000)) }
                        else { matches!(number, None | Some(1)) };
                    if !valid { return Ok(Value::list(Vec::new())); }
                    if name == "gettabinfo" {
                        Value::list(vec![Value::Dict(Dict::from_pairs([
                            ("tabnr".into(), 1.into()), ("windows".into(), Value::list(vec![1000.into()])),
                            ("variables".into(), Value::Dict(self.script.tab_vars().clone())),
                        ]))])
                    } else {
                        Value::list(vec![Value::Dict(Dict::from_pairs([
                            ("winid".into(), 1000.into()), ("winnr".into(), 1.into()), ("tabnr".into(), 1.into()), ("bufnr".into(), 1.into()),
                            ("width".into(), 80.into()), ("height".into(), 40.into()), ("winrow".into(), 1.into()), ("wincol".into(), 1.into()),
                            ("topline".into(), 1.into()), ("botline".into(), (document.physical_source_line_number_count() as i64).into()),
                            ("quickfix".into(), 0.into()), ("loclist".into(), 0.into()), ("terminal".into(), 0.into()), ("textoff".into(), 0.into()),
                        ]))])
                    }
                }
                _ => unreachable!("query name checked above"),
            })
        })();
        Some(result)
    }

    fn script_undo_tree(&self, document: &Document) -> Value {
        use std::collections::{BTreeMap, HashMap};
        let entries = document.history_timeline_entries();
        let status = document.history_status();
        let latest = document.history_last_change_number();
        let nodes: HashMap<_, _> = entries.iter().map(|entry| (entry.location.change.as_u64(), entry)).collect();
        let mut children: BTreeMap<u64, Vec<u64>> = BTreeMap::new();
        for entry in &entries {
            if let Some(parent) = entry.parent { children.entry(parent.change.as_u64()).or_default().push(entry.location.change.as_u64()); }
        }
        for children in children.values_mut() {
            children.sort_by_key(|sequence| (nodes[sequence].preferred, *sequence));
        }
        let root = entries.iter().find(|entry| entry.parent.is_none()).map_or(0, |entry| entry.location.change.as_u64());
        let output = crate::vimscript::List::new(Vec::new());
        let mut pending = vec![(children.get(&root).cloned().unwrap_or_default(), output.clone())];
        while let Some((mut siblings, list)) = pending.pop() {
            let Some(sequence) = siblings.pop() else { continue };
            let node = nodes[&sequence];
            let dict = Dict::from_pairs([("seq".into(), (sequence as i64).into()), ("time".into(), (node.timestamp as i64).into())]);
            if sequence == latest { dict.insert("newhead", 1.into()); }
            if status.preferred_redo == Some(node.location) { dict.insert("curhead", 1.into()); }
            if let Some(write) = node.saved_write { dict.insert("save", (write as i64).into()); }
            if !siblings.is_empty() {
                let alt = crate::vimscript::List::new(Vec::new());
                dict.insert("alt", Value::List(alt.clone())); pending.push((siblings, alt));
            }
            list.push(Value::Dict(dict));
            if let Some(children) = children.get(&sequence) { pending.push((children.clone(), list)); }
        }
        let current = status.current.change.as_u64();
        let save_last = entries.iter().filter_map(|entry| entry.saved_write).max().unwrap_or(0);
        let mut save_cur = 0; let mut cursor = Some(current);
        while let Some(sequence) = cursor {
            let Some(node) = nodes.get(&sequence) else { break };
            if let Some(write) = node.saved_write { save_cur = write; break; }
            cursor = node.parent.map(|parent| parent.change.as_u64());
        }
        Value::Dict(Dict::from_pairs([
            ("seq_last".into(), (latest as i64).into()), ("seq_cur".into(), (current as i64).into()),
            ("time_cur".into(), (if latest == 0 { 0 } else { nodes.get(&current).map_or(0, |node| node.timestamp) as i64 + 1 }).into()),
            ("save_last".into(), (save_last as i64).into()), ("save_cur".into(), (save_cur as i64).into()),
            ("synced".into(), i64::from(!document.history_group_is_active()).into()), ("entries".into(), Value::List(output)),
        ]))
    }

    fn script_query_position(&self, document: &Document, offset: usize) -> Option<Value> {
        let physical = document.physical_line_at_text(offset).ok()?;
        let source = document.projection().source_insertion_point(offset, true)?;
        let text = document.physical_source_text(physical.content_range.start..source).ok()?;
        Some(Value::list(vec![1.into(), (physical.index as i64 + 1).into(), (text.len() as i64 + 1).into(), 0.into()]))
    }
}
