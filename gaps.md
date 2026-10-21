# Viem coverage and interface gaps

Reaudited on 2026-10-19 against the current checkout at `1a83642`. Findings
below were checked against command dispatch, native callers, the ABI and tests.
This is an unresolved backlog; historical line counts and port estimates are
omitted. Windows already has a WinUI/DirectWrite frontend.

This audit covers the existing command and interface backlog and adjacent
defects; it is not a complete Vim conformance inventory. Normal `U` and the
Viem Regex v2 dialect are documented product differences. Shell commands/filters,
`:source!`, terminal jobs and automatic plugin/runtime loading remain outside
the script host profile. See [AGENTS.md](AGENTS.md),
[Vim script support](docs/vimscript.md) and [split views](docs/window-panes.md)
for the current commitments.
