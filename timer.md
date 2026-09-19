# Idle CPU: the 60 Hz syntax poll

Working note for whoever picks this up. Written 2026-09-17 against `main` at
`0385995`. Everything below was measured or read out of the source, not assumed;
where something is inference it says so.

## Status

Two unconditional repeating timers accounted for all idle CPU. One is fixed, one
is not.

- **Fixed.** The 1 Hz `code_style.json` poll in `EVCodeStyleFileMonitor`. It is
  now read once at startup, with an explicit `Format > Style > Reload style
  sheet` command. See the git history for that change; it is not revisited here.
- **Open, and the larger of the two.** A 60 Hz per-document syntax poll,
  [src/mac/Editor/Sources/EVCoreDocument.swift:116](src/mac/Editor/Sources/EVCoreDocument.swift:116).
  Roughly three quarters of idle CPU. This note is about that one.

## Measurement

The app is a plain `NSApplication`; nothing exotic is needed.

```bash
sample $(pgrep -f 'Viem.app/Contents/MacOS/Viem') 10 -file /tmp/viem-sample.txt
```

Baseline before any fix: 6m28s of CPU over 13h01m elapsed, ≈0.83% of a core
sustained, with the app idle and one Code buffer (a Makefile) open.

In a 10s sample, of 8000 main-thread samples, 7897 were blocked in `mach_msg` —
correct idle — and **every** non-idle sample was under `__CFRunLoopRun` →
`__CFRunLoopDoTimers`:

```
77  __CFRunLoopRun + 1816        ← all main-thread CPU
  59  __CFRunLoopDoTimers
      32  __CFRunLoopDoTimer + 980   → callbacks
          29  EVCoreDocument.swift:117 → pollSyntax
           1  EVCodeStyleFileMonitor.swift:32   (since removed)
      21  __CFArmNextTimerInMode → mk_timer_arm  ← pure kernel rearm overhead
```

Two things worth internalising from that shape:

- About **27% of the cost is `mk_timer_arm`** — the kernel re-arming the timer,
  not the callback doing anything. That portion is paid even when `pollSyntax`
  early-returns, so a non-Code buffer is not free either.
- The other threads are clean. `viem-syntax-0/1` sat 7995/8000 samples in
  `__psynch_cvwait`, and `EVFileChangeMonitor`'s kqueue sources cost nothing.
  Do not go looking for a second offender; there isn't one.

## What the timer actually does

The timer is created unconditionally in `EVCoreDocumentBackend.init` and lives
until `deinit`. It is **per document**, so N open buffers is N × 60 Hz. Nothing
gates it on window visibility, app activation, or outstanding work.

[`pollSyntax`](src/mac/Editor/Sources/EVCoreDocument.swift:545) early-returns
unless `sourceFormat == .code`, so for a text buffer the cost is the wakeup and
rearm only. For a Code buffer it calls through to
[`Core::poll_syntax`](src/core/coordinator/syntax.rs:215), which does real work
**before** it can conclude nothing changed. Per tick, per view:

- `syntax_input()` — builds a `SyntaxInputSnapshot` (an `Arc`-ish text-tree
  clone; cheap, but not free)
- `rebase_input(…)` — does early-out on unchanged identity, so this part is fine
- a scan of `snapshot.rows` filtered to the viewport, collected into a **freshly
  allocated `Vec`** ([syntax.rs:267](src/core/coordinator/syntax.rs:267)) — the
  profile catches this mid-`RawVecInner::finish_grow`
- `service.request(input, range)` per distinct range
- `service.poll(…)` — mutex acquire, `ready` is `None`
- `code_style::snapshot()`

and only then reaches
[`if !completed && !style_changed && !input_changed { return false }`](src/core/coordinator/syntax.rs:294).
The cost therefore scales with layout size — it gets worse in big files.

On top, `refreshSyntaxDiagnostics` runs 4×/sec per document, allocating a `Set`
and sorting it. `reportLoadDiagnostics` does at least early-out before posting.

## Why it is a poll and not a push

This is the part to understand before changing anything. The polling is not an
oversight; three separate constraints produce it.

**1. The FFI is pull-only by construction.** There is no core → frontend
notification channel anywhere. The only callbacks crossing the ABI go the other
way (core → text-shaping provider) and are documented "This callback must never
call back into the core"
([src/core/ffi.rs:980](src/core/ffi.rs:980)). AGENTS.md:5415 states
"Lock-protected code does not call user, adapter, frontend, or provider code."
A worker that finishes a parse has no sanctioned way to wake Swift.

**2. Results must be installed on the thread that owns the core.** A worker
cannot install its own result: that means mutating the coordinator
(`replace_presentation_coverage`, then `publish_code_presentation` →
`invalidate_syntax_presentation` on every view), which needs the core lock,
which a worker must not hold while inside a provider. So
[`Pool::run`](src/core/document/syntax/service.rs:203) parks the result in
`slot.ready` and loops back to its condvar.
[`SyntaxService::poll`](src/core/document/syntax/service.rs:486) is the hand-off
**and the validation point** — it rejects results whose input identity,
configuration, or registry generation no longer match. Stale work dies there.

**3. The poll is mostly the *request* side, not the collect side.** This is the
non-obvious one. `poll_syntax` walks every view each tick, derives the visible
byte range from the layout snapshot, and issues `service.request(…)`. **Scrolling
never pushes a request** — the next poll notices the viewport moved. The poll is
the "what is on screen right now" sampler, which is why it sits on a display
clock. Cooperative slicing reinforces this: providers yield and return
`continuation: true`, and AGENTS.md:6095 targets "roughly 2-4 ms cooperative
syntax slices and publication of ready visible results within one display frame."

## The trap

There is a fourth dependency on recurrence that is easy to miss and will bite a
naive fix. In [`SyntaxService::request`](src/core/document/syntax/service.rs:459):

```rust
if slot.pending.as_ref().is_some_and(|r| r.same(&request)) {
    // A full shared queue leaves the coalesced request in its mailbox.
    // A later frame must retry admission instead of stranding it.
    if !slot.running && !slot.queued {
        slot.queued = pool().enqueue(&self.mailbox);
    }
    return;
}
```

When the shared pool queue is full, `enqueue` fails and the request sits
**un-queued** in its mailbox. Nothing re-enqueues it. The design relies on a
later frame calling `request` again to retry admission — the recurring timer is
the load-shedding recovery path.

Consequence: a "work outstanding" predicate must be **`pending.is_some() ||
running`**, never `queued`. Keying off `queued` would stop the timer exactly
when a request has been shed, and it would never be admitted.

## Why it is wasteful anyway

The design puts three independently-triggered jobs on one unconditional clock:

| Job | Real trigger | Needs a clock? |
|---|---|---|
| Collect a finished result | a worker completed | no — rare, event-driven |
| Notice the viewport moved | scroll, resize, relayout | no — all are events |
| Re-drive a slice / retry admission | a continuation or shed request exists | only while one is outstanding |

Each has a knowable trigger. When none has fired, the tick is pure overhead. The
timer exists because it is simpler than wiring the triggers, and it is
unconditional because nothing tracks whether any of the three is live.

## Direction

There is already a working precedent in this codebase for exactly this shape.
`synchronizeSearchPolling`
([EVEditorSurfaceController.swift:382](src/mac/Editor/Sources/EVEditorSurfaceController.swift:382))
and `synchronizeCompletionPolling`
([:417](src/mac/Editor/Sources/EVEditorSurfaceController.swift:417)) both run at
60 Hz *while work is pending* and invalidate when it drains, driven by
`session.searchWorkPending()`
([EVCoreSearch.swift:14](src/mac/Editor/Sources/EVCoreSearch.swift:14)). Both
cost nothing in the idle profile. Mirror that.

Concretely:

1. Have `viem_core_poll_syntax`
   ([ffi.rs:13475](src/core/ffi.rs:13475)) report a **work-outstanding** flag
   alongside `changed`, computed as `pending.is_some() || running` per the trap
   above. Stop the timer when it goes false.
2. Make viewport changes an **explicit trigger** that restarts the timer.
   Nothing else will notice a scroll, since today only the sampler does. This is
   the substantive part of the work — scroll/resize/relayout paths have to call
   in. Do not skip it; without it, scrolling a Code buffer stops highlighting.
3. Also restart on edits, style-sheet revision changes, and code-preference
   changes. The `.viemGlobalCodeStyleDidChange` observer at
   [EVCoreDocument.swift:105](src/mac/Editor/Sources/EVCoreDocument.swift:105)
   already re-polls; it just needs to restart the timer too.
4. Independently, put a cheap guard at the top of `Core::poll_syntax` on
   (document revision, per-view viewport ranges, sheet revision) **before** the
   row scan and the `request` calls. This is worth doing on its own merits even
   if the timer stays: it removes the per-tick allocation.

Cheaper partial mitigations, if the full thing is too invasive right now: skip
scheduling entirely for non-Code documents, and suspend on
`NSWindow.occlusionState` / `NSApplication.didResignActiveNotification`. Both are
strictly less good than (1)–(3) but neither risks correctness.

## Secondary

The workers park on `wait_timeout(100ms)`
([service.rs:211](src/core/document/syntax/service.rs:211)) rather than an
indefinite wait, so both threads wake 10×/sec forever. Negligible against the
main-thread timer (they were 7995/8000 samples parked), but it is the same
reflex and worth fixing in passing if you are already in that file. Check why
the timeout is there first — it may be covering the same
stranded-request/retire-closed recovery that the frontend timer covers, in which
case it has the same non-obvious dependency.

## Unrelated, do not be confused by it

`EVCodeStyleMenuTests.testCodeMenuListsEveryCharacterDefinitionAndEditsTheGlobalTarget`
fails on clean `main` (verified by stashing). It expects the internal
`"* Incremental match"` built-in to appear in the user-facing style menu. It has
nothing to do with timers. Full suite is otherwise 746 passing.
