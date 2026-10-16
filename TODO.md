# TODO

Follow-ups from background pre-rendering (`FULL_PRELAYOUT_SOURCE_BYTES`,
[prelayout.rs](src/core/coordinator/prelayout.rs),
[native background layout](docs/windows-background-layout.md)) and the Windows
Markdown Preview performance pass.

## macOS: title-bar Markdown Preview button

AGENTS.md now limits the formatting toolbar to Markdown Preview: Text,
non-Markdown Code and literal Markdown have no toolbar, and with Preview off the
title bar shows a Markdown Preview button in the toolbar toggle's place. Turning
Preview on restores the remembered toolbar visibility and the toolbar toggle.
Windows implements this (`src/win/Shell/FormattingToolbarChrome.cs`, checked by
`src/win/Diagnostics/FormattingToolbarTests.cs`). The macOS frontend still shows
a toolbar holding only the Preview toggle in literal Markdown; update
`src/mac/AppShell/EVFormattingToolbarChrome.swift`, `EVFormattingToolbar.swift`
and their tests. This was not compiled on the Windows machine.

## Preview switch costs shared by both frontends

Measured on Windows with `scripts/test-win.ps1 -Optimized -ProfileDocument
AGENTS.md -ProfileScenario preview` after the font fix:

- Each switch re-applies the theme's style variant for the new format (about
  11–15 ms for the 95 KB AGENTS.md) and refreshes once more. The core could
  select the format's stylesheet variant as part of the switch transaction.
- Switching Preview off waits up to the 100 ms syntax grace period, because the
  Code projection has not been highlighted. The background view-format
  preparation could also request visible Code syntax, or the grace wait could be
  skipped for a projection switch.

## Giant lines in visible layout

A pre-rendered document now prepares hard lines larger than the 64 KiB slice
size whole in the background, so its extent is exact and it reports complete.
Visible layout still lays such a line out in resumable slices (the
`materialize_long_line_*` paths in `src/core/coordinator.rs`) without reusing
the prepared rows, so scrolling into it shapes bounded slices on the input
thread.

## Literal provenance for non-UTF-8 sources

`FormattedDocument::literal_source_in_span` and `literal_text_in_span`
(`src/core/document/projection.rs`) slice and re-encode the span's text for
every query when the source is not UTF-8, so per-grapheme queries over one line
are quadratic in its length. The UTF-8 streaming path in
`source_grapheme_ranges` does not cover these encodings.

## Test isolation for the Code stylesheet

Syntax tests that emit undefined captures materialize implicit definitions in
the process-wide Code stylesheet. A concurrently running Markdown test then
republishes its presentation and reshapes, which made
`retained_tables_reuse_exact_rows_without_shaping` fail under `cargo test --lib
prelayout`. The prelayout tests now pin the sheet
(`syntax.markdown.code_sheet`); other tests that assert no shaping in Markdown
can still flake the same way.

## Windows

- `CoreDocument.IsMarkdownDocument` reads the document modes (a JSON export)
  for literal Code on every toolbar synchronization. Minor; cache it per
  revision and language if it shows up in profiles.
