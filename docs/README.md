# Development guides

Keep this directory focused on current procedures, behavior that is easy to
misunderstand, useful examples, and actionable unfinished work. Generated
measurements belong in ignored output directories such as `target/`; patch
summaries and completed validation reports do not need separate documents.

Product and engineering requirements live in [AGENTS.md](../AGENTS.md).
[Markdown compatibility gaps](../MARKDOWN_GAPS.md) track the GFM work.

- [macOS development](macos-development.md): build, run, test and startup setup.
- [Split views](window-panes.md): window commands, resize units, and mouse dragging.
- [Windows development](../src/win/README.md): native build, packaging and tests.
- [C ABI validation](abi-validation.md): matching the Rust core and native headers.
- [Error recovery](error-recovery.md): safe presentation defaults, optional
  resources, diagnostic handling, and failures that must remain strict.
- [Backend fuzzing](fuzzing.md): campaigns, replay, watchdogs and history probes.
- [Performance measurement](performance.md): current tools, regression limits
  and measurement scope; no historical results.
- [Native selection](native-selection.md): native/Vim interaction differences
  and remaining platform gaps.
- [macOS background layout follow-up](windows-background-layout.md): the
  unfinished native scheduler connection and its acceptance checks.
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
