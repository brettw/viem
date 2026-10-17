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
