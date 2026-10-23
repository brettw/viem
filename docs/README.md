# Development guides

Keep this directory focused on current procedures, behavior that is easy to
misunderstand, useful examples, and actionable unfinished work. Generated
measurements belong in ignored output directories such as `target/`; patch
summaries and completed validation reports do not need separate documents.

Product and engineering requirements live in [AGENTS.md](../AGENTS.md).
[Markdown compatibility gaps](../MARKDOWN_GAPS.md) track the GFM work.

- [Development workflow](development-workflow.md): staged validation, shadow
  verification, candidate tracing, explicit materialization and measurement
  discipline for changes to shared editing code.
- [Markdown feature contracts](markdown-feature-contracts.md): early integration coverage and parser/owner boundaries.
- [macOS development](macos-development.md): build, run, test and startup setup.
- [Blocking editor](blocking-editor.md): Git integration and document completion.
- [Filename completion](filename-completion.md): Ex menu traversal, acceptance,
  and comparisons with zsh.
- [Registers and marks](vim-registers-and-marks.md): special registers, expression prompts, file marks and their lifetime.
- [Split views](window-panes.md): window commands, resize units, and mouse dragging.
- [Windows development](../src/win/README.md): native build, packaging and tests.
- [Third-party licenses](licenses/README.md): dependency and bundled-asset license
  texts, provenance, and update procedures; indexed in the [root license](../LICENSE.md).
- [C ABI validation](abi-validation.md): matching the Rust core and native headers.
- [Error recovery](error-recovery.md): safe presentation defaults, optional
  resources, diagnostic handling, and failures that must remain strict.
- [Backend fuzzing](fuzzing.md): campaigns, replay, watchdogs, history probes,
  the case-by-case consistency probe, and coverage-guided parser fuzzing of
  untrusted documents and clipboard data.
- [Performance measurement](performance.md): current tools, regression limits
  and measurement scope; no historical results.
- [Native selection](native-selection.md): native/Vim interaction differences
  and remaining platform gaps.
- [Native background layout](windows-background-layout.md): worker ownership,
  cache-only installation, cancellation and native regression checks.
- [Viewport stability](viewport-stability.md): scroll anchoring, minimal caret
  reveal and the exception for delayed syntax geometry.
- [Code formatting](code-formatting.md): reflow flags, language context, overrides, and list prefixes.
- [Indent alignment](indent-alignment.md): optional proportional-font alignment,
  persistent anchor indexing, dependency invalidation and geometry.
- [Vim script support](vimscript.md): startup, legacy and Vim9 languages, mappings, events,
  native option mappings, compatibility limits, engine invariants and oracle tests.
- [Vim theme import](vim-theme-import.md): color scheme evaluation, the base
  preset, and how Vim highlight groups map onto theme styles.
- [Highlighter compatibility](highlighter-compatibility.md): bundled syntax gaps,
  dependency coverage, and load-versus-execution audit requirements.
- [Comment language coverage](comment-language-coverage.md): supported forms, limits, defaults, and pinned Vim provenance.
- [Markdown demo](markdown_demo.md): loadable examples of supported syntax and
  deliberate presentation exceptions.
- [Markdown tables](markdown-tables.md): GFM compatibility, Word-like
  editing, source alignment, and acceptance requirements.

Syntax contracts stay beside their implementations:
[language detection](../src/core/document/syntax/detection/PROFILE.md),
[Vim](../src/core/document/syntax/vim/PROFILE.md), and
[Tree-sitter](../src/core/document/syntax/treesitter/PROFILE.md).
Unicode data provenance and regeneration live with the
[line-break data](../data/unicode/15.0.0/README.md).

[mac_references](mac_references) contains the editor's visual references.
[Word style.png](<Word style.png>) is a composition/density reference for the
style inspector, not a feature or implementation specification.
