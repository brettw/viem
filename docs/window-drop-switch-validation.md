# Window closing, file drops, and Markdown switching

## Behavior

- Closing an untitled dirty window reviews unsaved content once. Delete closes
  it, Cancel retains the draft, and another close after Cancel reviews again.
  Different dirty documents in stacked panes are reviewed separately; another
  window displaying the same document keeps it alive.
- A Finder file drop replaces the receiving clean pane. Dirty targets are
  preserved and the file opens in a new window. Additional files open additional
  windows. Opening rechecks pane/document identity, revision, and dirty state;
  an input-method composition is committed before the initial dirty check.
- Markdown mode changes preserve source bytes, cursor provenance, styles, and
  history while avoiding repeated scans through every formatting change.

## Performance diagnosis

The 221,657-byte repository AGENTS.md took approximately 28 seconds per core
switch in the unoptimized debug build. Most time was spent compiling position
maps: each interval endpoint rescanned all earlier formatting splices. Further
costs came from per-grapheme provenance queries, repeated splitting and counting
of rope leaves, and scanning every edit for each paragraph identity.

The replacements traverse ordered changes and provenance in batches and retain
unchanged rope subtrees. The native command path also reused its verified
candidate after closing an undo group; previously it performed the complete
projection twice. Reuse still checks source revision, history identity,
allocation generations, and group state before publishing.

On the same 221,657-byte input, the optimized release core now switches in
69–80ms, versus 1.42–1.47s in an isolated release baseline containing the
earlier paragraph changes. Debug core switches fell from about 28.5s to
0.72–0.84s. These are core timings; the native measurements also include
viewport layout and presentation.

The complete native release status-menu action on AGENTS.md measured 133ms
for the first Source-to-WYSIWYG switch, then 72ms, 81ms, and 72ms for subsequent
alternating switches. The final bundled app is built in release configuration.

The standalone manual Rust benchmark is:

```sh
cargo test --release --test format_switch_performance -- --ignored --nocapture
```

The native regression measures the complete status-menu action, checks one
presentation refresh and viewport-only text export, and preserves exact source.
It normally uses a deterministic large Markdown fixture. Set
`VIEM_PROFILE_MARKDOWN_PATH` to profile another local file.

## Validation

Computer use drove real untitled Delete and Cancel sheets in the isolated
`com.viem.todo-validation` app. Delete closed immediately without a second
sheet; Cancel retained the exact draft and a later close prompted again.
The final release app was also driven between Markdown Source and WYSIWYG at
the beginning and near the end of the large AGENTS.md copy. Screenshots
confirmed source delimiters versus flowed formatted prose, correct caret
content, and refreshed layout without error dialogs.

Finder cross-application drag automation did not deliver a drop to the editor.
The destination is covered by visible-window native integration tests using
file URL pasteboards and the complete NSDraggingInfo callback sequence, through
the real document host and Rust backend. These verify clean replacement,
dirty-target new-window routing, retained draft/undo, and unchanged disk bytes.
Focused tests also cover multiple files, inactive panes, aliases, failed opens,
asynchronous edits and replacements, unsupported payloads, and marked text.

Core regressions compare all anchor policies against the prior mapping
algorithm, compare batched provenance against individual queries, preserve
Unicode/grapheme boundaries, and bound work on tens of thousands of edits.
Large-document tests verify source identity, reprojection after edits, undo,
redo, style equivalence, and unchanged subtree identities.

Final checks passed: 1,418 Rust tests (one manual benchmark intentionally
ignored in the regular suite), 316 native tests, the additional release
AGENTS.md benchmark, `cargo fmt --all -- --check`, and `git diff --check`.
The validation app used an isolated configuration directory; the user's
running editor windows and preferences were retained.
