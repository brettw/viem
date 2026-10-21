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

## 1. Ex commands

- **Overlapping confirmed substitutions inside `:global` remain unsupported.**
  Confirmation now supports disjoint per-target substitutions, inherited flags,
  nested current-line predicates and Ctrl-E/Ctrl-Y scrolling. All choices stay
  bound to the original revision and commit in one undo unit. A body such as
  `:g/x/%s/x/xx/gc` revisits matches affected by earlier decisions, requiring
  sequential previews against tentative results; it is rejected before asking
  questions. For example, with two original lines containing `x`, accepting the
  first whole-buffer substitution changes both to `xx`; the second selected
  line's invocation must then offer the newly created matches. Merging previews
  computed from the original text cannot reproduce that behavior.

  **Vim behavior (checked 2026-10-20).** `:global` first marks its original
  target lines, then executes the body against the evolving buffer. Deleted
  targets disappear; newly inserted lines do not become extra global targets.
  Each substitute invocation starts a fresh match scan, so its range can include
  replacements made by an earlier invocation. Within one invocation, replacement
  text is not recursively substituted. See [Vim's global contract](https://vimhelp.org/repeat.txt.html#%3Aglobal).

  Confirmation responses belong to the current substitute invocation: `a`
  accepts its remaining matches; `q` and Escape end it while retaining accepted
  replacements; `l` accepts one last match and ends it. The next global target
  can still run and ask again. Ctrl-C interrupts the whole global execution and
  retains completed replacements. Ctrl-E/Ctrl-Y only scroll. The implementation
  restores substitute flags between invocations; a later body with explicit
  `gc` asks again. References: [confirmation keys](https://vimhelp.org/change.txt.html#%3As_c)
  and [Vim 9.2 substitute/global implementation](https://github.com/vim/vim/blob/v9.2.0000/src/ex_cmds.c#L4313-L4498).

  An interactive-mode oracle run on local Vim 9.1.1752, with `-Nu NONE -i NONE
  -n --not-a-term`, confirmed these examples for two lines `x`, `x` and
  `:g/x/%s/x/xx/gc` (responses supplied with `feedkeys(..., 'xt')`):

  | Responses | Final lines | Meaning |
  | --- | --- | --- |
  | `aa` | `xxxx`, `xxxx` | Each invocation accepts all its own matches. |
  | `qa` or Escape then `a` | `xx`, `xx` | First invocation quits; second still runs. |
  | `la` or `yqa` | `xxxx`, `xx` | Second invocation sees the first accepted edit. |
  | `y` then Ctrl-C | `xx`, `x` | Interrupt preserves progress and stops globally. |

  Do not run this oracle in `-es`: Ex-mode confirmation consumes response lines,
  so a string of response keys is not the same experiment. The source check is
  against Vim 9.2; the executable observations above are explicitly Vim 9.1.

  **Recommended implementation path (deferred).**

  1. Replace eager collection of all previews in `global_confirmation.rs` with
     a resumable global execution session: original document/view identities,
     stable target-line identities, body command, next target, accumulated
     diagnostics, and a private document candidate. Share unchanged source and
     projection storage; retain only the current candidate and the original
     publication checkpoint, not a full copy per response.
  2. Resolve each target and its body range against that candidate. Prepare one
     substitute invocation at a time. Track its original match progression
     separately from display offsets so accepted text is visible without being
     recursively matched. Rebase surviving global targets after each verified
     edit; explicitly remove deleted targets instead of collapsing them onto a
     surviving line. Resolve nested current-line predicates at execution time.
  3. Give the invoking view a candidate-specific projection/layout identity for
     highlight, caret and scrolling. Other views continue to display published
     source. Never feed candidate coordinates into original-revision layout.
     Fetch exact visible layout through the existing cancellable scheduler.
  4. Model response scope explicitly: next match, finish this invocation, or
     interrupt the global session. Keep accepted edits on `q`, Escape, `l` and
     Ctrl-C according to the table. Preserve Viem's documented per-target error
     policy: report an individual failure and continue from the last verified
     candidate. A stale external edit cancels the entire unpublished session.
  5. On completion or explicit interruption, atomically publish the verified
     candidate and its metadata as one undo unit, guarded by the original
     revision and relevant view/configuration identities. Update the current
     original-revision confirmation contract in AGENTS.md to distinguish the
     publication checkpoint from the evolving private preview. Enforce shared
     limits on matches, replacement bytes, work and retained memory across all
     invocations; exclude time waiting for user answers from compute budgets.
     Resource/verification failure must discard unpublished work explicitly.
  6. Before removing the rejection, test the response matrix, overlapping
     explicit/count ranges, deleted/split targets, nested predicates, zero-width
     and newly created matches, mixed encodings/endings, Markdown source
     preservation, multiple views, stale cancellation and exact one-step
     undo/redo. Add locality and memory bounds for repeated match growth.

  This is a design recommendation, not implemented support. Keep the current
  early rejection until both candidate execution and candidate presentation are
  available. See [global confirmation](src/core/command/global_confirmation.rs)
  and [confirmation tests](tests/all/substitute_confirmation.rs).
