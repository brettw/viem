//! Explicit launch context, never ambient filesystem/editor state.
use super::{Setup, Value, VimPattern, VimRegexLimits};

impl Setup<'_> {
    pub(super) fn environment_builtin(
        &mut self,
        name: &str,
        values: &[Value],
    ) -> Result<Value, String> {
        match (name, values) {
            ("execute", [Value::Text(command)]) => {
                // A syntax package may inspect an existing cluster to choose
                // containment. This is one read-only query, never Ex execution.
                if self.emitted.as_ref().is_some_and(|pending|
                    pending.iter().any(|command| !command.trim().is_empty())) {
                    return Err("setup syntax query after pending generated commands is unsupported".into());
                }
                let mut words = command.split_whitespace();
                let (Some("syn" | "syntax"), Some("list"), Some(cluster), None) =
                    (words.next(), words.next(), words.next(), words.next()) else {
                    return Err("setup execute supports only syntax cluster inspection".into());
                };
                let cluster = cluster.strip_prefix('@').filter(|name|
                    !name.is_empty() && name.len() <= 128 && name.bytes().all(|byte|
                        byte.is_ascii_alphanumeric() || byte == b'_'))
                    .ok_or("invalid syntax inspection cluster")?;
                let Some((name, groups)) = self.syntax_clusters.get(&cluster.to_ascii_lowercase()) else {
                    return Err(format!("E392: No such syntax cluster: {cluster}"));
                };
                Ok(Value::Text(format!("\n--- Syntax items ---\n{name} cluster={} ",
                    if groups.is_empty() { "NONE".into() } else { groups.join(",") })))
            }
            ("expand", [Value::Text(expression)])
                if expression.starts_with('%') || expression.starts_with("<sfile>") =>
            {
                let (value, suffix) = if let Some(suffix) = expression.strip_prefix("<sfile>") {
                    (self.source_file.clone(), suffix)
                } else {
                    (self.filename.clone().unwrap_or_default(), &expression[1..])
                };
                Ok(Value::Text(filename_modifiers(&value, suffix)?))
            }
            ("fnamemodify", [Value::Text(value), Value::Text(modifiers)]) => {
                Ok(Value::Text(filename_modifiers(value, modifiers)?))
            }
            ("printf", [Value::Text(format), values @ ..]) => {
                Ok(Value::Text(super::formatting::format(format, values)?))
            }
            ("hlexists", [Value::Text(name)]) => {
                // Function-generated commands are applied by the loader after
                // the call returns. Reading their effects before that point
                // would choose a branch using stale declaration state.
                if self
                    .emitted
                    .as_ref()
                    .is_some_and(|pending| pending.iter().any(|command| !command.trim().is_empty()))
                {
                    return Err(
                        "setup hlexists after pending generated commands is unsupported".into(),
                    );
                }
                Ok(Value::Number(i64::from(
                    self.highlights.contains(&name.to_ascii_lowercase()),
                )))
            }
            ("bufname", []) | ("bufname", [Value::Number(0)]) => {
                Ok(Value::Text(self.filename.clone().unwrap_or_default()))
            }
            ("bufname", [Value::Text(name)]) if name.is_empty() || name == "%" => {
                Ok(Value::Text(self.filename.clone().unwrap_or_default()))
            }
            ("executable", [Value::Text(_)]) => {
                // This compiler deliberately exposes no external executable
                // capability. Runtime defaults must not depend on the host PATH.
                Ok(Value::Number(0))
            }
            ("getline", [Value::Number(line)]) if *line <= 0 => Ok(Value::Text(String::new())),
            ("line", [Value::Text(position)]) if position == "$" => {
                let input = self
                    .input
                    .as_ref()
                    .ok_or("setup line('$') requires buffer context")?;
                let count = super::super::super::setup_line_count(input);
                self.reads.line_count = Some(count);
                Ok(Value::Number(count as i64))
            }
            ("search", values) => self.prefix_search(values),
            _ => Err(format!(
                "unsupported syntax setup function or arguments: {name}"
            )),
        }
    }

    pub(super) fn buffer_line(&mut self, line: i64) -> Result<String, String> {
        let line = usize::try_from(line).unwrap_or(0);
        let text = super::super::super::setup_line(self.input.as_ref().unwrap(), line);
        if !self.reads.lines.contains_key(&line) {
            if self.reads.lines.len() >= 256
                || self.reads.retained_bytes() + text.as_ref().map_or(0, String::len) > 256 * 1024
            {
                self.reads.exhausted_reads = true;
                return Err("syntax setup queried-line budget exceeded".into());
            }
            self.reads.lines.insert(line, text.clone());
        }
        text
    }

    fn prefix_search(&mut self, values: &[Value]) -> Result<Value, String> {
        let [Value::Text(source), Value::Text(flags), rest @ ..] = values else {
            return Err("setup search requires a pattern and flags".into());
        };
        if !flags.contains('n')
            || flags
                .chars()
                .any(|flag| !matches!(flag, 'c' | 'n' | 'w' | 'W'))
            || rest.len() > 2
        {
            return Err("setup search supports only non-moving cnwW flags".into());
        }
        if self.input.is_some() {
            return self.buffer_search(source, flags, rest);
        }
        let endline = match rest.first() {
            None => 32,
            Some(Value::Text(value)) if value.is_empty() => 32,
            Some(Value::Number(0)) => 32,
            Some(Value::Number(value)) if (1..=32).contains(value) => *value as usize,
            _ => return Err("setup search is confined to the first 32 supplied lines".into()),
        };
        if rest
            .get(1)
            .is_some_and(|value| !matches!(value,Value::Number(n) if *n >= 0))
        {
            return Err("invalid setup search timeout".into());
        }
        let end = self
            .prefix
            .match_indices('\n')
            .nth(endline.saturating_sub(1))
            .map_or(self.prefix.len(), |(at, _)| at);
        let text = &self.prefix[..end];
        let at = if flags.contains('c') {
            0
        } else {
            text.chars().next().map_or(0, char::len_utf8)
        };
        let pattern = VimPattern::compile(source, false, VimRegexLimits::default())?;
        let cancel = self.cancel;
        let mut found = pattern.find_text_with_control(text, at, &mut self.fuel, &mut || {
            cancel.is_some_and(|c| c.load(super::Ordering::Relaxed))
        })?;
        if found.is_none() && flags.contains('w') && !flags.contains('W') && at != 0 {
            found = pattern.find_text_with_control(text, 0, &mut self.fuel, &mut || {
                cancel.is_some_and(|c| c.load(super::Ordering::Relaxed))
            })?;
        }
        Ok(Value::Number(found.map_or(0, |found| {
            1 + text[..found.start].bytes().filter(|b| *b == b'\n').count() as i64
        })))
    }

    fn buffer_search(
        &mut self,
        source: &str,
        flags: &str,
        rest: &[Value],
    ) -> Result<Value, String> {
        use super::super::super::VimRegexProgress;
        use regex_syntax::hir::Look;
        const BYTE_LIMIT: usize = 1024 * 1024;
        const INSTRUCTION_LIMIT: usize = 1_000_000;
        let input = self.input.as_ref().unwrap();
        let stop_line = match rest.first() {
            None | Some(Value::Number(0)) => None,
            Some(Value::Text(value)) if value.is_empty() => None,
            Some(Value::Number(line)) if *line > 0 => Some(*line as usize),
            _ => return Err("invalid syntax setup search stop line".into()),
        };
        let timeout = match rest.get(1) {
            None | Some(Value::Number(0)) => 100,
            Some(Value::Number(ms)) if *ms > 0 => (*ms as u64).min(100),
            _ => return Err("invalid syntax setup search timeout".into()),
        };
        let end = stop_line
            .and_then(|line| input.text_tree().hard_line_end(line - 1).ok())
            .unwrap_or(input.byte_len());
        let pattern = VimPattern::compile(source, false, VimRegexLimits::default())?;
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(timeout);
        let mut at = if flags.contains('c') {
            0
        } else {
            input
                .chunk_at(0)
                .first()
                .map_or(1, |byte| utf8_width(*byte))
        };
        let mut inspected = 0;
        let mut budget = self.fuel.min(INSTRUCTION_LIMIT);
        let initial_budget = budget;
        let wrap_origin =
            flags.contains('w') && !flags.contains('W') && stop_line.is_none() && at != 0;
        let result = (|| {
            for (start, end) in std::iter::once((at, end)).chain(wrap_origin.then_some((0, 0))) {
                at = start;
                while at <= end {
                    if at > BYTE_LIMIT || budget == 0 || std::time::Instant::now() >= deadline {
                        return Err("syntax setup search work budget exceeded".into());
                    }
                    if self
                        .cancel
                        .is_some_and(|cancel| cancel.load(super::Ordering::Relaxed))
                    {
                        return Err("syntax compilation cancelled".into());
                    }
                    if at > 0 && pattern.start_anchor == Some(Look::Start) {
                        break;
                    }
                    let mut continuation = pattern.start(at);
                    loop {
                        let mut fuel = budget.min(2048);
                        let before = fuel;
                        let progress = pattern.resume_with_control(
                            &mut continuation,
                            input,
                            &mut fuel,
                            &mut || {
                                self.cancel
                                    .is_some_and(|cancel| cancel.load(super::Ordering::Relaxed))
                                    || std::time::Instant::now() >= deadline
                            },
                        );
                        budget -= before - fuel;
                        inspected = inspected.max(continuation.inspected_end);
                        if inspected > BYTE_LIMIT
                            || budget == 0
                            || std::time::Instant::now() >= deadline
                        {
                            return Err("syntax setup search work budget exceeded".into());
                        }
                        match progress {
                            VimRegexProgress::Pending => {
                                if self
                                    .cancel
                                    .is_some_and(|cancel| cancel.load(super::Ordering::Relaxed))
                                {
                                    return Err("syntax compilation cancelled".into());
                                }
                            }
                            VimRegexProgress::Failed(error) => return Err(error),
                            VimRegexProgress::Complete(Some(found)) => {
                                return Ok(1 + input
                                    .text_tree()
                                    .hard_line_at_byte(found.start)
                                    .map_err(|e| e.to_string())?);
                            }
                            VimRegexProgress::Complete(None) => {
                                break;
                            }
                        }
                    }
                    if at == input.byte_len() || pattern.start_anchor == Some(Look::Start) {
                        break;
                    }
                    if pattern.start_anchor == Some(Look::StartLF) {
                        let line = input
                            .text_tree()
                            .hard_line_at_byte(at)
                            .map_err(|e| e.to_string())?;
                        let Ok(next) = input.text_tree().hard_line_start(line + 1) else {
                            break;
                        };
                        at = next;
                    } else {
                        at += utf8_width(input.chunk_at(at)[0]);
                    }
                }
                if pattern.start_anchor != Some(Look::Start) {
                    inspected = inspected.max(end);
                }
                if inspected > BYTE_LIMIT {
                    return Err("syntax setup search byte budget exceeded".into());
                }
            }
            Ok(0)
        })();
        self.fuel -= initial_budget - budget;
        // Failed searches retain their actual inspected dependency too, so
        // unrelated body edits cannot repeatedly retry a capped compilation.
        self.reads.search_end = Some(self.reads.search_end.unwrap_or(0).max(inspected));
        let line = result?;
        if self.reads.search_results.len() >= 256 {
            self.reads.exhausted_reads = true;
            return Err("syntax setup search count budget exceeded".into());
        }
        self.reads.search_results.push(line);
        Ok(Value::Number(line as i64))
    }
}

fn utf8_width(first: u8) -> usize {
    match first {
        0..=0x7f => 1,
        0xc0..=0xdf => 2,
        0xe0..=0xef => 3,
        _ => 4,
    }
}

fn filename_modifiers(value: &str, suffix: &str) -> Result<String, String> {
    let mut value = value.to_owned();
    let mut modifiers = suffix.chars();
    let mut extension_source = None;
    let mut extension_count = 0;
    while let Some(colon) = modifiers.next() {
        if colon != ':' {
            return Err("unsupported setup filename expansion".into());
        }
        let modifier = modifiers.next().ok_or("missing setup filename modifier")?;
        if modifier == 'e' {
            let original = extension_source.get_or_insert_with(|| value.clone());
            extension_count += 1;
            let basename = original.rsplit(['/', '\\']).next().unwrap_or(original);
            let at = basename
                .match_indices('.')
                .filter(|(at, _)| *at != 0)
                .rev()
                .take(extension_count)
                .last()
                .map(|(at, _)| at + 1);
            value = at.map_or("", |at| &basename[at..]).to_owned();
        } else {
            extension_source = None;
            extension_count = 0;
            value = filename_modifier(&value, modifier)?;
        }
    }
    Ok(value)
}

fn filename_modifier(value: &str, modifier: char) -> Result<String, String> {
    let last_separator = value.rfind(['/', '\\']);
    let basename = &value[last_separator.map_or(0, |at| at + 1)..];
    Ok(match modifier {
        't' => basename.to_owned(),
        'h' => last_separator
            .map_or(".", |at| if at == 0 { &value[..1] } else { &value[..at] })
            .to_owned(),
        'r' => basename
            .rfind('.')
            .filter(|at| *at != 0)
            .map_or(value, |at| {
                &value[..last_separator.map_or(0, |at| at + 1) + at]
            })
            .to_owned(),
        'p' if value.starts_with('/') || value.as_bytes().get(1) == Some(&b':') => value.to_owned(),
        _ => {
            return Err(
                "setup filename modifier needs unavailable filesystem or current-directory context"
                    .into(),
            )
        }
    })
}

// Native Vim -Nu NONE baseline identities; syntax categories are created by declarations.
pub(super) fn initial_highlights() -> std::collections::BTreeSet<String> {
    [
        "Added",
        "ColorColumn",
        "ComplMatchIns",
        "Conceal",
        "CurSearch",
        "Cursor",
        "CursorColumn",
        "CursorLine",
        "CursorLineFold",
        "CursorLineNr",
        "CursorLineSign",
        "DiffAdd",
        "DiffChange",
        "DiffDelete",
        "DiffText",
        "DiffTextAdd",
        "Directory",
        "EndOfBuffer",
        "ErrorMsg",
        "FoldColumn",
        "Folded",
        "IncSearch",
        "LineNr",
        "LineNrAbove",
        "LineNrBelow",
        "MatchParen",
        "MessageWindow",
        "ModeMsg",
        "MoreMsg",
        "MsgArea",
        "NonText",
        "Normal",
        "Pmenu",
        "PmenuBorder",
        "PmenuExtra",
        "PmenuExtraSel",
        "PmenuKind",
        "PmenuKindSel",
        "PmenuMatch",
        "PmenuMatchSel",
        "PmenuSbar",
        "PmenuSel",
        "PmenuShadow",
        "PmenuThumb",
        "PopupNotification",
        "PopupSelected",
        "PreInsert",
        "Question",
        "QuickFixLine",
        "Search",
        "SignColumn",
        "SpecialKey",
        "SpellBad",
        "SpellCap",
        "SpellLocal",
        "SpellRare",
        "StatusLine",
        "StatusLineNC",
        "StatusLineTerm",
        "StatusLineTermNC",
        "TabLine",
        "TabLineFill",
        "TabLineSel",
        "TabPanel",
        "TabPanelFill",
        "TabPanelSel",
        "Title",
        "ToolbarButton",
        "ToolbarLine",
        "VertSplit",
        "Visual",
        "VisualNOS",
        "WarningMsg",
        "WildMenu",
        "lCursor",
    ]
    .into_iter()
    .map(str::to_ascii_lowercase)
    .collect()
}
