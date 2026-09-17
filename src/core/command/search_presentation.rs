//! Read-only search presentation. Preview never changes editing authority,
//! histories, registers, selections, or the eventual operator's origin.
use super::search_regex::{CompiledRegex, RegexInput, RegexLimits, RegexWork, SearchOptions};
use super::*;

const PREVIEW_WORK_LIMIT: u64 = 1_000_000;
const PREVIEW_NAVIGATION_LIMIT: usize = 1_024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IncrementalSearchPreview {
    pub document: DocumentId,
    pub revision: Revision,
    pub origin: usize,
    pub destination: usize,
    pub matched_range: Range<usize>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SearchPresentation {
    pub pattern: Option<String>,
    pub options: SearchOptions,
    pub highlight_all: bool,
    pub incremental_active: bool,
    pub incremental_match: Option<IncrementalSearchPreview>,
    /// Substitution previews are limited to the explicitly addressed hard lines.
    pub search_range: Option<Range<usize>>,
    /// Preview failures are informational and never replace command status.
    pub diagnostic: Option<String>,
}

/// An opaque, inexpensive key for a host's cached preview. It contains snapshot
/// identity and semantic input only; comparing it never scans document text.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SearchPresentationKey {
    document: DocumentId,
    revision: Revision,
    origin: usize,
    options: SearchOptions,
    suppressed: bool,
    last_search: Option<(SearchDirection, String)>,
    prompt: Option<(
        CommandLineKind,
        String,
        usize,
        Option<PendingOperator>,
        Vec<SearchDirection>,
    )>,
}

struct PreviewQuery {
    pattern: String,
    direction: SearchDirection,
    count: usize,
    options: SearchOptions,
    scope: Option<Range<usize>>,
    navigation: Vec<SearchDirection>,
}

impl CommandInterpreter {
    pub fn search_presentation_key(&self, document: &Document) -> SearchPresentationKey {
        SearchPresentationKey {
            document: document.id(),
            revision: document.revision(),
            origin: self.cursor,
            options: self.search_options,
            suppressed: self.search_highlight_suppressed,
            last_search: self.last_search.clone(),
            prompt: self.command_line_state.as_ref().map(|state| {
                (
                    state.kind,
                    state.buffer.input.clone(),
                    state.count,
                    state.operator,
                    self.incremental_navigation(state).to_vec(),
                )
            }),
        }
    }

    pub(super) fn incremental_navigation<'a>(
        &self,
        state: &'a CommandLineState,
    ) -> &'a [SearchDirection] {
        if state.incremental_navigation_input.as_deref() == Some(&state.buffer.input) {
            &state.incremental_navigation
        } else {
            &[]
        }
    }

    pub fn search_presentation(&self, document: &Document) -> SearchPresentation {
        let mut result = SearchPresentation {
            pattern: self
                .last_search
                .as_ref()
                .map(|(_, pattern)| pattern.clone()),
            options: self.search_options,
            highlight_all: self.search_options.hlsearch && !self.search_highlight_suppressed,
            incremental_active: false,
            incremental_match: None,
            search_range: None,
            diagnostic: None,
        };
        if !self.search_options.incsearch {
            return result;
        }
        let Some(state) = &self.command_line_state else {
            return result;
        };
        let search_prompt = state.kind != CommandLineKind::Ex;
        let query = self.preview_query(document, state);
        result.incremental_active = search_prompt || query.is_some();
        let Some(query) = query else {
            if search_prompt && !state.buffer.input.is_empty() {
                result.pattern = None;
                result.highlight_all = false;
            }
            return result;
        };
        result.pattern = Some(query.pattern.clone());
        result.options = query.options;
        result.highlight_all = query.options.hlsearch;
        result.search_range = query.scope.clone();
        match search_match(document, self.cursor, &query, preview_limits()) {
            Ok(Some(matched_range)) => {
                result.incremental_match = Some(IncrementalSearchPreview {
                    document: document.id(),
                    revision: document.revision(),
                    origin: self.cursor,
                    destination: matched_range.start,
                    matched_range,
                });
            }
            Ok(None) => {}
            Err(error) => {
                result.pattern = None;
                result.highlight_all = false;
                result.diagnostic = Some(error);
            }
        }
        result
    }

    fn preview_query(&self, document: &Document, state: &CommandLineState) -> Option<PreviewQuery> {
        if state.buffer.input.is_empty() {
            return None;
        }
        let mut options = self.search_options;
        let (pattern, direction, count, scope) = match state.kind {
            CommandLineKind::SearchForward | CommandLineKind::SearchBackward => (
                state.buffer.input.clone(),
                if state.kind == CommandLineKind::SearchForward {
                    SearchDirection::Forward
                } else {
                    SearchDirection::Backward
                },
                state
                    .operator
                    .map(effective_operator_count)
                    .transpose()
                    .ok()?
                    .unwrap_or(state.count),
                None,
            ),
            CommandLineKind::Ex => {
                let (range, mut pattern, override_case, count) =
                    ex::incremental_substitute(&state.buffer.input)?;
                if pattern.is_empty() {
                    pattern = self.last_search.as_ref()?.1.clone();
                }
                let current = document.hard_line_at_offset(self.cursor)?;
                let mut lines =
                    ex_execute::resolve_range(range.as_ref(), current, document.line_count())
                        .ok()?;
                if let Some(count) = count {
                    let count = usize::try_from(count).ok()?.max(1);
                    lines.start = lines.end;
                    lines.end = lines
                        .start
                        .saturating_add(count - 1)
                        .min(document.line_count() - 1);
                }
                let snapshot = document.hard_line_snapshot();
                let start = snapshot.line(lines.start)?.content_range().start;
                let end = snapshot.line(lines.end)?.content_range().end;
                if let Some(ignorecase) = override_case {
                    options.ignorecase = ignorecase;
                    options.smartcase = false;
                }
                options.wrapscan = false;
                (pattern, SearchDirection::Forward, 1, Some(start..end))
            }
        };
        Some(PreviewQuery {
            pattern,
            direction,
            count,
            options,
            scope,
            navigation: self.incremental_navigation(state).to_vec(),
        })
    }

    pub(super) fn navigate_incremental_search(
        &mut self,
        document: &Document,
        forward: bool,
    ) -> CommandOutput {
        if !self.search_options.incsearch {
            return CommandOutput::unsupported("incremental search is disabled");
        }
        let Some(state) = self.command_line_state.as_ref() else {
            return CommandOutput::pending();
        };
        let Some(mut query) = self.preview_query(document, state) else {
            return CommandOutput::pending();
        };
        if query.navigation.len() >= PREVIEW_NAVIGATION_LIMIT {
            return CommandOutput::pending();
        }
        query.navigation.push(if forward {
            SearchDirection::Forward
        } else {
            SearchDirection::Backward
        });
        // Do not lose a useful current match when a bounded next/previous
        // query cannot find a destination.
        if matches!(
            search_match(document, self.cursor, &query, preview_limits()),
            Ok(Some(_))
        ) {
            let state = self
                .command_line_state
                .as_mut()
                .expect("active search prompt");
            state.incremental_navigation_input = Some(state.buffer.input.clone());
            state.incremental_navigation = query.navigation;
        }
        CommandOutput::pending()
    }

    pub(super) fn invalidate_changed_incremental_navigation(&mut self) {
        if let Some(state) = self.command_line_state.as_mut() {
            if state.incremental_navigation_input.as_deref() != Some(&state.buffer.input) {
                state.incremental_navigation.clear();
                state.incremental_navigation_input = None;
            }
        }
    }
}

fn preview_limits() -> RegexLimits {
    RegexLimits {
        work: PREVIEW_WORK_LIMIT,
        ..RegexLimits::default()
    }
}

fn search_match(
    document: &Document,
    origin: usize,
    query: &PreviewQuery,
    limits: RegexLimits,
) -> Result<Option<Range<usize>>, String> {
    search_match_sequence(
        &document.hard_line_snapshot(),
        origin,
        query.direction,
        &query.pattern,
        query.count,
        query.options,
        limits,
        &query.navigation,
        query.scope.clone(),
    )
}

pub(super) fn search_destination_with_navigation(
    lines: &HardLineSnapshot,
    origin: usize,
    direction: SearchDirection,
    pattern: &str,
    count: usize,
    options: SearchOptions,
    navigation: &[SearchDirection],
) -> Result<Option<usize>, String> {
    search_match_sequence(
        lines,
        origin,
        direction,
        pattern,
        count,
        options,
        RegexLimits::default(),
        navigation,
        None,
    )
    .map(|matched| matched.map(|range| range.start))
}

#[allow(clippy::too_many_arguments)]
fn search_match_sequence(
    lines: &HardLineSnapshot,
    origin: usize,
    direction: SearchDirection,
    pattern: &str,
    count: usize,
    options: SearchOptions,
    limits: RegexLimits,
    navigation: &[SearchDirection],
    scope: Option<Range<usize>>,
) -> Result<Option<Range<usize>>, String> {
    let regex = CompiledRegex::compile(
        pattern,
        options
            .case_insensitive(pattern)
            .map_err(|e| e.to_string())?,
        limits,
    )
    .map_err(|e| e.to_string())?;
    let input = RegexInput::new(lines);
    let mut work = RegexWork::new(limits);
    let bounds = scope.clone().unwrap_or(0..lines.text_length());
    let mut seek = |cursor: usize,
                    direction: SearchDirection,
                    include_start: bool|
     -> Result<Option<Range<usize>>, String> {
        let mut scan =
            |start: usize, before: Option<usize>| -> Result<Option<Range<usize>>, String> {
                let mut at = start;
                let mut last = None;
                while at <= bounds.end {
                    let Some(matched) = regex
                        .find(&input, at, bounds.end, &mut work)
                        .map_err(|e| e.to_string())?
                    else {
                        break;
                    };
                    let range = matched.range();
                    if before.is_some_and(|limit| range.start >= limit) {
                        break;
                    }
                    if lines.is_grapheme_boundary(range.start) {
                        if before.is_none() {
                            return Ok(Some(range));
                        }
                        last = Some(range.clone());
                    }
                    let Some(next) = lines.next_grapheme_boundary(range.start) else {
                        break;
                    };
                    at = next;
                }
                Ok(last)
            };
        match direction {
            SearchDirection::Forward => {
                let start = if include_start {
                    Some(cursor.max(bounds.start))
                } else {
                    lines
                        .next_grapheme_boundary(cursor)
                        .map(|at| at.max(bounds.start))
                };
                let found = if let Some(start) = start.filter(|start| *start <= bounds.end) {
                    scan(start, None)?
                } else {
                    None
                };
                if found.is_none() && options.wrapscan {
                    scan(bounds.start, None)
                } else {
                    Ok(found)
                }
            }
            SearchDirection::Backward => {
                let found = scan(bounds.start, Some(cursor))?;
                if found.is_none() && options.wrapscan {
                    scan(bounds.start, Some(bounds.end.saturating_add(1)))
                } else {
                    Ok(found)
                }
            }
        }
    };
    let mut cursor = if let Some(scope) = &scope {
        scope.start
    } else {
        origin
    };
    let mut completed = 0;
    let mut seen = HashMap::new();
    let mut result = None;
    while completed < count.max(1) {
        if let Some(previous) = seen.insert(cursor, completed) {
            let cycle = completed - previous;
            let skip = (count.max(1) - completed) / cycle;
            if skip > 0 {
                completed += skip * cycle;
                continue;
            }
        }
        let Some(found) = seek(cursor, direction, scope.is_some() && completed == 0)? else {
            return Ok(None);
        };
        cursor = found.start;
        result = Some(found);
        completed += 1;
    }
    for direction in navigation {
        let Some(found) = seek(cursor, *direction, false)? else {
            return Ok(None);
        };
        cursor = found.start;
        result = Some(found);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup(text: &str, settings: &str) -> (Document, CommandInterpreter) {
        let mut document = Document::new(text);
        let mut commands = CommandInterpreter::new();
        assert!(commands
            .configure_startup(&mut document, settings)
            .is_empty());
        (document, commands)
    }
    fn keys(
        commands: &mut CommandInterpreter,
        document: &mut Document,
        input: &str,
    ) -> CommandOutput {
        let mut output = CommandOutput::complete();
        for key in mappings::parse_key_notation(input).unwrap() {
            output = commands.handle(document, InputEvent::Key(key)).unwrap();
        }
        output
    }
    fn preview(commands: &CommandInterpreter, document: &Document) -> IncrementalSearchPreview {
        commands
            .search_presentation(document)
            .incremental_match
            .unwrap()
    }

    #[test]
    fn options_default_off_and_support_aliases_queries_toggles_resets_and_atomic_errors() {
        let (mut document, mut commands) = setup("one two", "");
        assert!(!commands.search_presentation(&document).options.hlsearch);
        assert!(!commands.search_presentation(&document).options.incsearch);
        assert_eq!(
            keys(&mut commands, &mut document, ":set hls is<CR>").status,
            CommandStatus::Complete
        );
        let query = keys(&mut commands, &mut document, ":set hls? is?<CR>");
        assert_eq!(query.ex_outcome.unwrap().frontend_requests.len(), 2);
        let state = commands.search_presentation(&document);
        assert!(state.options.hlsearch && state.options.incsearch);
        assert!(matches!(
            keys(&mut commands, &mut document, ":set nohls nois nonsense<CR>").status,
            CommandStatus::ExError(_)
        ));
        assert_eq!(
            commands.search_presentation(&document).options,
            state.options
        );
        keys(&mut commands, &mut document, ":setlocal hls! is!<CR>");
        assert!(!commands.search_presentation(&document).options.hlsearch);
        assert!(!commands.search_presentation(&document).options.incsearch);
        keys(
            &mut commands,
            &mut document,
            ":set hls is<CR>:set hls& is&<CR>",
        );
        assert_eq!(commands.search_options, SearchOptions::default());
    }

    #[test]
    fn nohlsearch_suspends_without_changing_option_or_history_and_search_reenables() {
        let (mut document, mut commands) = setup("one two one", "set hls");
        keys(&mut commands, &mut document, "/one<CR>");
        assert!(commands.search_presentation(&document).highlight_all);
        let last = commands.last_search.clone();
        keys(&mut commands, &mut document, ":noh<CR>");
        assert!(commands.search_options.hlsearch);
        assert!(!commands.search_presentation(&document).highlight_all);
        assert_eq!(commands.last_search, last);
        keys(&mut commands, &mut document, ":set hls?<CR>");
        assert!(!commands.search_presentation(&document).highlight_all);
        keys(&mut commands, &mut document, "n");
        assert!(commands.search_presentation(&document).highlight_all);
        keys(&mut commands, &mut document, ":nohlsearch<CR>:set hls<CR>");
        assert!(commands.search_presentation(&document).highlight_all);
        for invalid in ["noh!", "1noh", "noh extra"] {
            assert!(ex::parse_ex(invalid).is_err());
        }
    }

    #[test]
    fn incremental_preview_is_counted_and_read_only_until_commit() {
        let (mut document, mut commands) = setup("zero one one one", "set is hls");
        let revision = document.revision();
        keys(&mut commands, &mut document, "2/on");
        let p = preview(&commands, &document);
        assert_eq!(p.origin, 0);
        assert_eq!(p.destination, 9);
        assert_eq!(p.matched_range, 9..11);
        assert_eq!(commands.cursor(), 0);
        assert!(commands.last_search.is_none());
        assert!(commands.search_history.is_empty());
        assert_eq!(document.revision(), revision);
        keys(&mut commands, &mut document, "e<CR>");
        assert_eq!(commands.cursor(), 9);
        assert_eq!(
            commands.last_search,
            Some((SearchDirection::Forward, "one".into()))
        );
        assert_eq!(commands.search_history, ["one"]);
        assert!(!commands.search_presentation(&document).incremental_active);
    }

    #[test]
    fn cancel_invalid_missing_and_empty_patterns_preserve_cursor_and_saved_search() {
        let (mut document, mut commands) = setup("zero one two", "set is hls");
        keys(&mut commands, &mut document, "/one<CR>:noh<CR>");
        let origin = commands.cursor();
        let last = commands.last_search.clone();
        keys(&mut commands, &mut document, "/two");
        assert_eq!(preview(&commands, &document).destination, 9);
        keys(&mut commands, &mut document, "<Esc>");
        assert_eq!(commands.cursor(), origin);
        assert_eq!(commands.last_search, last);
        assert!(!commands.search_presentation(&document).highlight_all);
        keys(&mut commands, &mut document, "/[");
        let invalid = commands.search_presentation(&document);
        assert!(invalid.incremental_active && invalid.incremental_match.is_none());
        assert!(invalid.diagnostic.is_some());
        assert!(matches!(
            keys(&mut commands, &mut document, "<CR>").status,
            CommandStatus::Error(_)
        ));
        assert_eq!(commands.cursor(), origin);
        assert_eq!(commands.last_search, last);
        keys(&mut commands, &mut document, "/absent");
        assert!(commands
            .search_presentation(&document)
            .incremental_match
            .is_none());
        keys(&mut commands, &mut document, "<C-u>");
        assert!(commands.search_presentation(&document).incremental_active);
        assert!(commands
            .search_presentation(&document)
            .incremental_match
            .is_none());
        keys(&mut commands, &mut document, "<Esc>");
        assert_eq!(commands.cursor(), origin);
    }

    #[test]
    fn navigation_keys_are_preview_only_and_enter_uses_the_selected_match() {
        let (mut document, mut commands) = setup("zero one one one", "set is");
        keys(&mut commands, &mut document, "/one");
        let key = commands.search_presentation_key(&document);
        keys(&mut commands, &mut document, "<C-g>");
        assert_ne!(commands.search_presentation_key(&document), key);
        assert_eq!(preview(&commands, &document).destination, 9);
        keys(&mut commands, &mut document, "<C-g><C-t>");
        assert_eq!(preview(&commands, &document).destination, 9);
        assert_eq!(commands.cursor(), 0);
        keys(&mut commands, &mut document, "<CR>");
        assert_eq!(commands.cursor(), 9);
        keys(&mut commands, &mut document, "?one<C-g><CR>");
        assert_eq!(commands.cursor(), 9);
    }

    #[test]
    fn changed_prompt_and_native_replacement_drop_navigation_without_changing_history() {
        let (mut document, mut commands) = setup("zero one one two", "set is");
        keys(&mut commands, &mut document, "/one<C-g>");
        assert_eq!(preview(&commands, &document).destination, 9);
        keys(&mut commands, &mut document, "x<BS>");
        assert_eq!(preview(&commands, &document).destination, 5);
        let expected = commands.command_line_snapshot().unwrap();
        commands
            .edit_command_line(
                &document,
                CommandLineEditRequest {
                    document: document.id(),
                    revision: document.revision(),
                    expected,
                    action: CommandLineEditAction::Replace {
                        range: 0..3,
                        text: "two".into(),
                    },
                },
            )
            .unwrap();
        assert_eq!(preview(&commands, &document).destination, 13);
        keys(&mut commands, &mut document, "<CR>/one<CR>/<Up>");
        assert_eq!(preview(&commands, &document).destination, 9);
        assert_eq!(commands.search_history, ["two", "one"]);
    }

    #[test]
    fn visual_search_keeps_anchor_and_cursor_until_enter_or_cancel() {
        let (mut document, mut commands) = setup("zero one two", "set is");
        keys(&mut commands, &mut document, "vl/one");
        assert_eq!(commands.visual_anchor(), Some(0));
        assert_eq!(commands.cursor(), 1);
        assert_eq!(preview(&commands, &document).destination, 5);
        keys(&mut commands, &mut document, "<Esc>");
        assert_eq!(commands.mode(), Mode::VisualCharacter);
        assert_eq!(commands.cursor(), 1);
        keys(&mut commands, &mut document, "/one<CR>");
        assert_eq!(commands.visual_anchor(), Some(0));
        assert_eq!(commands.cursor(), 5);
    }

    #[test]
    fn operator_preview_preserves_source_registers_counts_undo_and_navigation_dot_recipe() {
        let (mut document, mut commands) = setup("zero one one one one", "set is");
        let source = document.source_bytes();
        keys(&mut commands, &mut document, "\"a2d/one");
        assert_eq!(preview(&commands, &document).destination, 9);
        assert!(commands.register('a').is_none());
        assert_eq!(document.source_bytes(), source);
        keys(&mut commands, &mut document, "<Esc>");
        keys(&mut commands, &mut document, "\"ad/one<C-g><CR>");
        assert_eq!(document.text(), "one one one");
        assert_eq!(commands.register('a').unwrap().text, "zero one ");
        keys(&mut commands, &mut document, ".");
        assert_eq!(document.text(), "one");
        keys(&mut commands, &mut document, "u");
        assert_eq!(document.text(), "one one one");
    }

    #[test]
    fn substitute_preview_obeys_range_escaping_flags_and_does_not_edit_source() {
        let (mut document, mut commands) = setup("one\nONE\none", "set is hls");
        let source = document.source_bytes();
        keys(&mut commands, &mut document, ":2,3s/on");
        let state = commands.search_presentation(&document);
        assert_eq!(state.search_range, Some(4..11));
        assert_eq!(state.incremental_match.unwrap().matched_range, 8..10);
        keys(&mut commands, &mut document, "e/x/i");
        assert_eq!(preview(&commands, &document).matched_range, 4..7);
        assert_eq!(document.source_bytes(), source);
        assert!(commands.last_search.is_none());
        keys(&mut commands, &mut document, "<CR>");
        assert_eq!(document.text(), "one\nx\nx");
        assert_eq!(commands.last_search.as_ref().unwrap().1, "one");
    }

    #[test]
    fn case_wrap_unicode_and_zero_width_preview_match_committed_search() {
        let (mut document, mut commands) = setup("αβ CAT cat\nnext", "set is ic sc");
        keys(&mut commands, &mut document, "/CAT");
        let at = preview(&commands, &document).destination;
        assert_eq!(at, 5);
        keys(&mut commands, &mut document, "<CR>?cat");
        let at = preview(&commands, &document).destination;
        keys(&mut commands, &mut document, "<CR>");
        assert_eq!(commands.cursor(), at);
        keys(&mut commands, &mut document, "/$");
        let at = preview(&commands, &document).destination;
        keys(&mut commands, &mut document, "<CR>");
        assert_eq!(commands.cursor(), at);
        keys(&mut commands, &mut document, ":set nows<CR>/absent");
        assert!(commands
            .search_presentation(&document)
            .incremental_match
            .is_none());
    }

    #[test]
    fn large_document_preview_work_is_bounded_without_flattening_and_cached_key_is_read_only() {
        let (mut document, mut commands) = setup(&"ordinary text\n".repeat(100_000), "set is hls");
        keys(&mut commands, &mut document, "/notpresent");
        let key = commands.search_presentation_key(&document);
        let state = commands.search_presentation(&document);
        assert!(state.incremental_match.is_none());
        assert!(state
            .diagnostic
            .as_deref()
            .is_some_and(|text| text.contains("RegexResourceLimit")));
        assert!(!document.projection().compatibility_text_is_materialized());
        assert_eq!(commands.search_presentation_key(&document), key);
        keys(&mut commands, &mut document, "<Esc>");
        assert_ne!(commands.search_presentation_key(&document), key);
    }
}
