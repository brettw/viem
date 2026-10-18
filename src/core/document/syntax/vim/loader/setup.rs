//! Bounded, immutable document queries for the shared script host.
//! This module owns no expression, function or control-flow evaluator.
use super::{
    AtomicBool, BTreeMap, BTreeSet, Ordering, VimPattern, VimRegexLimits, VimSetupContext,
};
mod environment;
#[derive(Clone, Debug)]
/// Query transport only; shared interpreter values retain all script identity.
pub(super) enum QueryValue {
    Number(i64),
    Text(String),
    List(Vec<QueryValue>),
}
pub(super) struct Setup<'a> {
    prefix: String,
    input: Option<crate::document::syntax::SyntaxInputSnapshot>,
    pub(super) reads: super::super::VimSetupReads,
    filetype: String,
    filename: Option<String>,
    source_file: String,
    keyword_option: String,
    highlights: BTreeSet<String>,
    syntax_clusters: BTreeMap<String, (String, Vec<String>)>,
    fuel: usize,
    storage_reserved: usize,
    cancel: Option<&'a AtomicBool>,
    pub(super) cursor: (usize, usize),
    pub(super) ignore_case: bool,
    pub(super) flags: BTreeMap<String, i64>,
}
impl<'a> Setup<'a> {
    pub(super) fn new(context: &VimSetupContext, cancel: Option<&'a AtomicBool>) -> Self {
        Self {
            prefix: context.prefix.clone(),
            input: context.input.clone(),
            reads: Default::default(),
            filetype: String::new(),
            filename: context.filename.clone(),
            source_file: String::new(),
            keyword_option: "@,48-57,_,192-255".into(),
            highlights: environment::initial_highlights(),
            syntax_clusters: BTreeMap::new(),
            fuel: 8_000_000,
            storage_reserved: 0,
            cancel,
            cursor: (1, 1),
            ignore_case: false,
            flags: BTreeMap::new(),
        }
    }
    pub(super) fn set_filetype(&mut self, name: &str) {
        self.filetype = name.into();
    }
    pub(super) fn context_option(&self, name: &str) -> Result<QueryValue, String> {
        match name {
            "&filetype" => Ok(QueryValue::Text(self.filetype.clone())),
            _ => self
                .flags
                .get(name)
                .map(|n| QueryValue::Number(*n))
                .ok_or_else(|| format!("unknown context field: {name}")),
        }
    }
    fn charge(&mut self, amount: usize) -> Result<(), String> {
        if self.cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
            return Err("syntax compilation cancelled".into());
        }
        self.fuel = self
            .fuel
            .checked_sub(amount.max(1))
            .ok_or("syntax query work budget exceeded")?;
        Ok(())
    }
    fn reserve_storage(&mut self, amount: usize) -> Result<(), String> {
        self.storage_reserved = self.storage_reserved.saturating_add(amount);
        if self.storage_reserved > 4 * 1024 * 1024 {
            return Err("syntax query storage budget exceeded".into());
        }
        Ok(())
    }
    pub(super) fn context_function(
        &mut self,
        name: &str,
        values: Vec<QueryValue>,
    ) -> Result<QueryValue, String> {
        match (name, values.as_slice()) {
            ("getline", [QueryValue::Number(line)]) => self.line_text(*line).map(QueryValue::Text),
            ("getline", [QueryValue::Number(start), QueryValue::Number(end)]) => {
                if end.saturating_sub(*start) > 256 {
                    return Err("syntax queried-line count budget exceeded".into());
                }
                let last = self.input.as_ref().map_or(
                    self.prefix.lines().count().max(1),
                    super::super::setup_line_count,
                ) as i64;
                if *start < 1 || *end < *start {
                    return Ok(QueryValue::List(Vec::new()));
                }
                (*start..=(*end).min(last))
                    .map(|line| self.line_text(line).map(QueryValue::Text))
                    .collect::<Result<Vec<_>, _>>()
                    .map(QueryValue::List)
            }
            _ => self.environment_builtin(name, &values),
        }
    }
    fn line_text(&mut self, line: i64) -> Result<String, String> {
        if line <= 0 {
            return Ok(String::new());
        }
        if self.input.is_some() {
            return self.buffer_line(line);
        }
        if line > 32 {
            return Err("setup getline is confined to the first 32 supplied lines".into());
        }
        Ok(self
            .prefix
            .split('\n')
            .nth(line as usize - 1)
            .unwrap_or("")
            .into())
    }
    pub(super) fn set_cursor(&mut self, line: i64, column: i64) -> Result<i64, String> {
        if line < 0 || column < 0 {
            return Ok(-1);
        }
        let count = self.input.as_ref().map_or(
            self.prefix.split('\n').count().max(1),
            super::super::setup_line_count,
        );
        let line = if line == 0 {
            self.cursor.0
        } else {
            line as usize
        };
        if line > count && self.input.is_some() {
            self.reads.line_count = Some(count);
        }
        let line = line.min(count).max(1);
        let text = self.line_text(line as i64)?;
        let column = if column == 0 {
            self.cursor.1
        } else {
            column as usize
        };
        let mut byte = column.saturating_sub(1).min(text.len().saturating_sub(1));
        while !text.is_char_boundary(byte) {
            byte -= 1;
        }
        self.cursor = (line, byte + 1);
        Ok(0)
    }
    pub(super) fn update_syntax_cluster(
        &mut self,
        name: &str,
        groups: &[String],
    ) -> Result<(), String> {
        let bytes = name.len() + groups.iter().map(String::len).sum::<usize>();
        self.reserve_storage(bytes.saturating_mul(2).saturating_add(128))?;
        self.charge(bytes)?;
        self.syntax_clusters
            .insert(name.to_ascii_lowercase(), (name.into(), groups.to_vec()));
        Ok(())
    }
    pub(super) fn append_syntax_cluster_group(
        &mut self,
        name: &str,
        group: &str,
    ) -> Result<(), String> {
        self.reserve_storage(
            name.len()
                .saturating_add(group.len())
                .saturating_mul(2)
                .saturating_add(128),
        )?;
        self.charge(name.len() + group.len())?;
        self.syntax_clusters
            .entry(name.to_ascii_lowercase())
            .or_insert_with(|| (name.into(), Vec::new()))
            .1
            .push(group.into());
        Ok(())
    }
    pub(super) fn clear_syntax_cluster(&mut self, name: Option<&str>) {
        if let Some(name) = name {
            self.syntax_clusters.remove(&name.to_ascii_lowercase());
        } else {
            self.syntax_clusters.clear();
        }
    }
    pub(super) fn set_keyword_option(&mut self, value: &str) {
        self.keyword_option = value.to_owned();
    }
    pub(super) fn define_highlight(&mut self, name: &str) -> Result<(), String> {
        if name.starts_with('@') {
            return Ok(());
        }
        if name.is_empty() || name.len() > 128 {
            return Err("invalid setup highlight group name".into());
        }
        let name = name.to_ascii_lowercase();
        if !self.highlights.contains(&name) {
            self.reserve_storage(name.len().saturating_add(96))?;
            self.highlights.insert(name);
        }
        Ok(())
    }
    pub(super) fn set_source_file(&mut self, value: &str) -> String {
        std::mem::replace(&mut self.source_file, value.to_owned())
    }
}
