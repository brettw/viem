# Vim script compatibility audit

Differential audit of Viem's Vim script support against real Vim, run on
2026-11-14 at commit `5c6b7f0`. About 1,750 probes ran through both
`/usr/bin/vim` (9.1.1752, the repository's recorded oracle) and Viem; roughly
150 behaved differently. The pure language engine matched on 937 of 1,077
probes. The editor-facing builtins matched far less often.

Only outstanding findings remain below. Each item shows Vim's result where it
differs.

## Method

- **Engine probes** ran through the in-memory host with `VIEM_DEBUG_SCRIPT`
  (see [Vim script validation](docs/vimscript.md#validation)).
- **Editor probes** ran through a temporary integration test that sourced a
  script into a real `Core` with a six-line buffer, and through separately
  typed command lines for the items marked "confirmed across command lines".
  That test is not in the tree.
- Each probe was wrapped in `try`/`catch` and printed `string(expr)`, so both
  sides report values and exception text in the same form.

Some error numbers differ only because the oracle is 9.1 and the target is
9.2 (E896 against E1531, for example). Those are not listed.

## Not covered

- About 115 probes in the last editor set never ran in Viem because the script
  aborted at an unsupported `:normal`: `fnamemodify`, `simplify`, file and
  glob functions, `strftime`, `getcompletion`, and the remaining window and
  syntax queries.
- Vim9 script, autocommands, user commands, and options beyond the few used in
  probes were not probed.
- Some differences may be deliberate under the host profile. Only the block
  register width and the `nrformats` default were checked against the docs.
