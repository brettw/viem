# Validate event-driven background work on Windows

Validate this change on Windows and fix any regressions you find. Read
`AGENTS.md`, `docs/development-workflow.md`, `docs/abi-validation.md`, and
`src/win/README.md` first. Preserve unrelated working-tree changes. Do not commit
or publish unless asked. Run builds and measurements sequentially; retain the
reports under `target/windows-validation` and `target/validation`.

## Changed behavior

- `CoreDocument` owns one retained native wake registration, shared by all its
  views/windows. A worker callback only queues owner-thread work. The owner
  collects results and arms a one-shot timer for an actual deadline, if any.
- The core preserves the 250 ms pause before complete highlighting of small
  documents and the 400 ms pause before preparing the other Markdown view.
  Worker/projection completions advance work without frontend polling.
- Syntax queue admission and closed-provider cleanup no longer depend on
  another frame or periodic worker wakeups.
- `EditorWindow` has no repeating maintenance timer. Each `CoreView` advances
  search/completion only while pending. Shared-buffer search changes wake the
  other views. Drag autoscroll runs only during an out-of-bounds drag.
- Caret blinking runs only in the active pane of the active window while its
  editor input owns focus. Blinking remains intentional visible UI work.
- Recovery backup scheduling uses a resettable four-second one-shot after an
  actual document revision change. Unchanged documents have no backup timer;
  edits during a write retain a new deadline and invalidate obsolete writes.

The acceptance target is **no recurring maintenance wakeups once work has
settled**. A rounded Task Manager reading of 0% alone does not prove this.
Observe wakeups/stacks as well as process CPU. Exclude active caret blinking,
drag scrolling, animations, and real external-file events from the idle sample.

## Build and automated checks

Use a Windows x64 developer PowerShell with the repository's documented MSVC,
Windows SDK, .NET, Rust, and Python versions. Start from this exact candidate;
record `git rev-parse HEAD`, `git status --short`, and `git diff --stat` with the
results. Do not use a stale DLL or a build from another worktree.

```powershell
python src/win/tools/generate_bindings.py --check
cargo run --locked --example check_c_abi
python scripts/validate.py check
python scripts/validate.py focused --filter code_syntax::
python scripts/validate.py focused --filter code_syntax_ffi::
.\scripts\test-win.ps1 -Suite BackgroundWork,VimRuntime,BackgroundLayout,Reload,MarkdownViewPreference,CodeStyles,StyleFollowing,FileChecks
```

Also run the library scheduler/notification tests:

```powershell
python scripts/validate.py focused --tests lib --filter background
python scripts/validate.py focused --tests lib --filter document::syntax::service::
```

Inspect the selected test/module names in the output. Cover queue saturation beyond
the runnable queue limit, continuation admission, close with a retained wait
handle, callback lifetime, completion racing collection/disarming, and more than 16
distinct visible syntax regions settling without repeated requests. A test
that selected zero cases is not evidence.

`BackgroundWork` exercises the real Rust DLL and native dispatcher. It checks
idle Text documents, shared-view wake coalescing, syntax completion without
calling `ServiceBackgroundWork`, incremental completion without calling `View.Poll`, shared
search highlighting, and queued notifications after close. Its checks for
absence of work use an observation interval after observable readiness.
It also injects a failure after view/layout/provider creation and verifies the
partial view is removed, native resources are released, and the document can
create a replacement view.
`FileChecks` includes recovery tests for idle scheduling, presentation-only
notifications, deadline retirement, close cancellation, and edits during a
blocked backup. Existing failure/retry and durable-replacement tests still run.

Then run the full native suite and the AOT callback/lifetime checks:

```powershell
.\scripts\test-win.ps1
.\scripts\test-win.ps1 -NativeAot -Suite BackgroundWork,VimRuntime,Reload,MarkdownViewPreference,FileChecks
.\scripts\build-win.ps1 -Configuration Release
```

Run `python scripts/validate.py suites` if the shared Rust candidate does not
already have matching suite evidence. The validator's `native` stage is for
macOS; use `test-win.ps1` here. Keep the generated JSON reports and report every
failure, including failures that pass on a second run. Native input tests need
the foreground desktop; do not interact with it during those stages.

## Native behavior checks

Use an isolated profile and copies of documents. Include a small Code file, a
Code file larger than 256 KiB, Markdown with several fenced code blocks, and
plain Text. Exercise each with one pane, split views, multiple windows, and a
diff window with both live and protected base documents.

1. Open a Code file and wait without typing or scrolling: syntax and diagnostics
   must arrive. Scroll into uncached text; resize and zoom. New coverage must
   arrive without another input event. Keep the existing 100 ms first-display
   grace behavior.
2. Edit, undo/redo, change language, change a Code style, and change theme.
   Results must use the current source/configuration. Style changes must not
   leave stale colors in another window showing the same buffer.
3. Edit Markdown repeatedly, then stop. Verify delayed preparation completes
   and Preview switches in both directions work. Switch/edit/reload while a
   projection or syntax job is running; stale work must never install.
4. Exercise incremental `/` search, persistent highlights and `:noh` in split
   views, and insert-mode Ctrl-N/Ctrl-P completion. They must finish when input
   stops, then their timers must stop. Repeat after closing the active pane.
5. Drag ordinary text and row/gutter selections outside every viewport edge.
   Autoscroll must continue while stationary outside, stop on reentry, release,
   lost capture, replacement, or close, and not restart while idle. Include a
   drag beginning on a task checkbox and table selection gestures.
6. Close/reload rapidly while work is pending, including closing one of two
   windows sharing a document and closing a diff base. Check for access to a
   released core, duplicate drains, exceptions crossing callbacks, leaked
   registrations/providers, and shutdown hangs. Repeat under NativeAOT.
7. Verify caret blinking continues when focused and stops when the pane/window
   loses focus, a menu or dialog owns focus, or the OS disables caret blinking.
8. Edit a file-backed document, stop for four seconds, and verify its recovery
   snapshot updates while the source file remains unchanged. Repeat with edits
   during an active backup, then close. There must be no repeated recovery
   timer while idle, and the owned recovery slot must retire safely on close.

## Idle profiling

Use the freshly published Release build and an isolated profile. Wait for
useful syntax, layout, and Markdown preparation to finish. Collect separate
samples with one document and many documents, including Text, Code, Markdown,
shared views, and diff windows. Deactivate the window for the maintenance-only
sample; also test minimized windows. Keep the caret stationary and do not edit
files externally during the interval.

Use Windows Performance Recorder/Analyzer CPU sampling and thread wake/context
switch traces. Record the process CPU delta over at least 30 seconds, sample
duration, document sizes/counts, window state, and whether the caret was active.
Repeat after an edit/scroll settles. Inspect these paths:

- No repeating `EditorWindow` maintenance tick.
- No `CoreDocument.ServiceBackgroundWork` calls after ready results and genuine
  deadlines have drained. In a Debug inspection, the document's
  `BackgroundWorkDrainsForTesting` remains unchanged and
  `BackgroundWorkScheduledForTesting` is false.
- No idle `CoreView` pending-work callbacks or drag-scroll callbacks. The Debug
  pending-work/drag-scroll scheduling properties should be false.
- No periodic recovery checks: `BackupScheduledForTesting` is false after a
  backup settles, and recovery has no one-second wake pattern.
- `viem-syntax-0/1` remain parked until a request or cleanup event; there must
  be no 100 ms timeout wake pattern.
- Closing the final view releases worker-owned resources without a timer or
  another input event. Do not infer release only from process working-set size.

Report the build/candidate identity, commands, selected test counts, failures,
report/trace paths, CPU measurements and wakeup evidence. State separately any
environment limitation that prevented validation. Do not claim Windows passed
based on macOS tests or source review alone.
