//! Explicit launch context, never ambient filesystem/editor state.
use super::{Setup, Value, VimPattern, VimRegexLimits};

impl Setup<'_> {
    pub(super) fn environment_builtin(
        &mut self,
        name: &str,
        values: &[Value],
    ) -> Result<Value, String> {
        match (name, values) {
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
            ("search", values) => self.prefix_search(values),
            _ => Err(format!(
                "unsupported syntax setup function or arguments: {name}"
            )),
        }
    }

    fn prefix_search(&mut self, values: &[Value]) -> Result<Value, String> {
        let [Value::Text(source), Value::Text(flags), rest @ ..] = values else {
            return Err("setup search requires a pattern and flags".into());
        };
        if !flags.contains('n')
            || flags.chars().any(|flag| !matches!(flag, 'c' | 'n' | 'W'))
            || rest.len() > 2
        {
            return Err(
                "setup search supports only non-moving cnW flags over the supplied prefix".into(),
            );
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
        let found = pattern.find_text_with_control(text, at, &mut self.fuel, &mut || {
            cancel.is_some_and(|c| c.load(super::Ordering::Relaxed))
        })?;
        Ok(Value::Number(found.map_or(0, |found| {
            1 + text[..found.start].bytes().filter(|b| *b == b'\n').count() as i64
        })))
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
