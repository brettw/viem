//! Syntax declarations and their bounded program state. Script parsing and
//! execution belong to the shared Vim script runtime.
use super::*;

#[cfg(test)]
#[path = "review_tests.rs"]
mod review_tests;

impl Loader<'_> {
    pub(super) fn set_keyword(&mut self, value: &str) -> Result<(), String> {
        if value.is_empty() {
            return Ok(());
        }
        if value == "clear" {
            self.keyword = VimKeyword::parse(&self.keyword_option)?;
            self.syntax_keyword_option = None;
        } else {
            self.keyword = VimKeyword::parse(value)?;
            self.syntax_keyword_option = Some(value.to_owned());
        }
        Ok(())
    }
    pub(super) fn set_buffer_keyword(&mut self, value: &str) -> Result<(), String> {
        let keyword = VimKeyword::parse(value)?;
        if self.syntax_keyword_option.is_none() {
            self.keyword = keyword;
        }
        self.keyword_option = value.to_owned();
        self.setup.set_keyword_option(value);
        Ok(())
    }
    pub(super) fn charge_statement_work(&mut self) -> Result<(), String> {
        if self.cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
            return Err("syntax setup cancelled".into());
        }
        self.statement_fuel = self
            .statement_fuel
            .checked_sub(1)
            .ok_or("syntax setup traversal budget exceeded")?;
        Ok(())
    }
}

impl Loader<'_> {
    pub(super) fn highlight(&mut self, rest: &str, forced: bool) -> Result<(), String> {
        let mut words = rest
            .split_whitespace()
            .take_while(|s| !s.starts_with('"'))
            .collect::<Vec<_>>();
        let default = words
            .first()
            .is_some_and(|s| matches!(*s, "def" | "default"));
        if default {
            words.remove(0);
        }
        let Some(&first) = words.first() else {
            return Ok(());
        };
        if first == "clear" {
            if words.len() == 1 {
                self.highlight_definitions.clear();
                self.program.links.clear();
            } else {
                for name in &words[1..] {
                    let name = self.intern_group_name(name);
                    self.highlight_definitions.remove(&name);
                    self.program.links.remove(&name);
                }
            }
            return Ok(());
        }
        if first == "link" {
            if words.len() != 3 {
                return Err("invalid highlight link declaration".into());
            }
            validate_groups(&[words[1].to_owned(), words[2].to_owned()])?;
            let group = self.intern_group_name(words[1]);
            let unlink = words[2] == "NONE";
            let target = if unlink {
                "NONE".to_owned()
            } else {
                self.intern_group_name(words[2])
            };
            if default
                && (self.program.links.contains_key(&group)
                    || self.highlight_definitions.contains(&group))
            {
                return Ok(());
            }
            if !forced && self.highlight_definitions.contains(&group) {
                return Ok(());
            }
            if unlink || group == target {
                self.program.links.remove(&group);
            } else {
                self.program.links.insert(group, target);
            }
            return Ok(());
        }
        validate_groups(&[first.to_owned()])?;
        if words.len() == 1 {
            return Ok(());
        } // read-only query does not create a group
        let first = self.intern_group_name(first);
        let first = first.as_str();
        for attribute in &words[1..] {
            if attribute.eq_ignore_ascii_case("NONE") {
                continue;
            }
            let (key, value) = attribute
                .split_once('=')
                .ok_or("invalid highlight attribute")?;
            if !matches!(
                key.to_ascii_lowercase().as_str(),
                "term"
                    | "cterm"
                    | "gui"
                    | "ctermfg"
                    | "ctermbg"
                    | "ctermul"
                    | "guifg"
                    | "guibg"
                    | "guisp"
                    | "font"
                    | "start"
                    | "stop"
                    | "blend"
            ) || value.is_empty()
            {
                return Err(format!("unsupported highlight attribute: {attribute}"));
            }
        }
        if default
            && (self.program.links.contains_key(first)
                || self.highlight_definitions.contains(first))
        {
            return Ok(());
        }
        // Syntax-owned presentation declarations establish the named group and
        // link precedence. Its visual properties come from the Code stylesheet.
        self.program.links.remove(first);
        self.highlight_definitions.insert(first.to_owned());
        Ok(())
    }
    pub(super) fn synchronization(&mut self, rest: &str) -> Result<(), String> {
        let (kind, arguments) = word(rest)?;
        if kind == "clear" {
            return Ok(());
        }
        if matches!(kind, "match" | "region") {
            let (name, mut tail) = word(arguments)?;
            let mut declaration = format!("{kind} {name}");
            while !tail.is_empty() && !tail.starts_with('"') {
                let (token, next) = word(tail)?;
                if matches!(token, "grouphere" | "groupthere") {
                    let (group, next) = word(next)?;
                    validate_groups(&[group.to_owned()])?;
                    if group != "NONE"
                        && !self.program.rules.iter().any(|r| {
                            r.group.eq_ignore_ascii_case(group)
                                && matches!(r.kind, RuleKind::Region { .. })
                        })
                    {
                        return Err(format!(
                            "syntax sync target is not a defined region: {group}"
                        ));
                    }
                    tail = next;
                } else {
                    // The rest may begin with a quoted pattern. Copy delimited
                    // patterns intact; synchronization directives outside them
                    // are metadata and never become ordinary content rules.
                    let prefix = ["start=", "skip=", "end="]
                        .into_iter()
                        .find(|p| tail.starts_with(p))
                        .unwrap_or("");
                    let pattern = &tail[prefix.len()..];
                    if !prefix.is_empty()
                        || pattern
                            .chars()
                            .next()
                            .is_some_and(|c| !c.is_alphanumeric() && c != '_')
                    {
                        let (_, after) = delimited(pattern)?;
                        let length = tail.len() - after.len();
                        declaration.push(' ');
                        declaration.push_str(&tail[..length]);
                        tail = after.trim_start();
                    } else {
                        declaration.push(' ');
                        declaration.push_str(token);
                        tail = next;
                    }
                }
            }
            // A double quote at the first pattern position is a delimiter.
            if !tail.is_empty() {
                declaration.push(' ');
                declaration.push_str(tail);
            }
            let before = self.program.rules.len();
            let result = self.syntax(&declaration);
            self.program.rules.truncate(before);
            return result;
        }
        if kind == "linecont" {
            let (pattern, rest) = delimited(arguments)?;
            VimPattern::compile_with_keyword(
                &pattern,
                self.case_ignore,
                VimRegexLimits::default(),
                &self.keyword,
            )?;
            if !rest.trim().is_empty() && !rest.trim_start().starts_with('"') {
                return self.synchronization(rest.trim());
            }
            return Ok(());
        }
        let mut tokens = rest
            .split_whitespace()
            .take_while(|s| !s.starts_with('"'))
            .peekable();
        while let Some(token) = tokens.next() {
            if token == "fromstart" {
                self.program.fromstart = true;
                continue;
            }
            if token == "ccomment" {
                if tokens
                    .peek()
                    .is_some_and(|s| !s.contains('=') && *s != "fromstart")
                {
                    validate_groups(&[tokens.next().unwrap().to_owned()])?;
                }
                continue;
            }
            if let Some(v) = token
                .strip_prefix("minlines=")
                .or_else(|| token.strip_prefix("lines="))
            {
                self.program.minlines = v.parse().map_err(|_| "invalid minlines")?;
                continue;
            }
            if let Some(v) = token.strip_prefix("maxlines=") {
                self.program.maxlines = v.parse().map_err(|_| "invalid maxlines")?;
                continue;
            }
            if let Some(v) = token.strip_prefix("linebreaks=") {
                let _: usize = v.parse().map_err(|_| "invalid linebreaks")?;
                continue;
            }
            return Err(format!("unsupported synchronization declaration: {token}"));
        }
        // Recovery hints cannot establish Exact coverage. The scanner continues
        // to recover from validated checkpoints or clearly provisional context.
        Ok(())
    }
}

pub(super) fn decode_source(bytes: &[u8]) -> Result<String, String> {
    if let Ok(text) = std::str::from_utf8(bytes) {
        return Ok(text.to_owned());
    }
    // Legacy runtime attribution comments can contain opaque non-UTF8 bytes.
    // They are not executable declarations; only their physical line and input
    // hash matter. Never substitute invalid bytes in an active command.
    let mut source = String::new();
    for line in bytes.split_inclusive(|&b| b == b'\n') {
        match std::str::from_utf8(line) {
            Ok(text) => source.push_str(text),
            Err(_) => {
                if line.iter().copied().find(|b| !b.is_ascii_whitespace()) != Some(b'"') {
                    return Err("invalid UTF-8 in syntax declaration".into());
                }
                source.push('"');
                if line.ends_with(b"\n") {
                    source.push('\n');
                }
            }
        }
    }
    Ok(source)
}
