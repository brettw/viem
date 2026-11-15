# Background work and idle scheduling

Settled documents have no periodic maintenance callbacks. Syntax workers sleep
until a request or cleanup event. Visible caret blinking, active interaction,
explicit script timers and external-file notifications are separate work.

## Notification and ownership

Each core has one `WorkWake`, shared by its Code service, Markdown code-block
services and alternate-projection preparation. Workers store results before
signalling and release every lock before invoking a receiver. The notification
is a hint to collect; results stay in Rust mailboxes and retain their exact
identity checks. Workers never mutate a document, publish layout or draw.

The C ABI retains one thread-safe callback context per document. Its callback
only queues owner-thread work. Replacing or removing a subscription releases
the old context after in-flight callbacks finish, outside locks and core
leases. A queued native closure independently checks its weak owner and exact
registration/core identity. Closing a document disables delivery; reloading
binds the surviving frontend to the replacement core's registration.

`viem_core_service_background_work` acknowledges readiness **before** collecting
results, so a completion during collection can schedule another turn. Native
delivery uses the same clear-before-service rule. Its result distinguishes:

- Presentation changes, which refresh affected views.
- Diagnostic changes, which refresh diagnostics independently.
- More ready work, which schedules another bounded owner turn.
- The next real deadline, which arms one non-repeating native timer.

A running worker alone is never a reason to schedule another check. The
bounded Markdown collector explicitly reports ready results left for another
turn; it cannot depend on a further completion after all workers have stopped.
Terminal failures settle just like successful coverage. Incompatible prepared
Code services are retired rather than becoming permanently ready work.

Each syntax service also remembers which of its **current** visible requests
have completed, keyed to the exact input and configuration. Evicting an older
coverage-cache entry cannot restart an unchanged view, including its first-paint
wait. Without this small record, more simultaneous view regions than cache
entries could request and evict each other's coverage indefinitely. Removed
requests and changed identities retire the records; no text or runs are copied.

Core FFI turns compare source/projection, format, syntax configuration, style
and view/layout dependency identities. A change signals only after the core
has been returned to its registry. Thus edits, undo/redo, source reloads,
scrolling, resizing, zoom and Preview changes use the same scheduling path.
Package and global style changes notify subscriptions after registry locks
are released. Unchanged dependencies require no text scan or view allocation.

## Delayed work

The existing editing pauses remain:

| Work | Deadline and continuation |
| --- | --- |
| Complete highlighting of eligible small Code documents | 250 ms after the input changes; visible work takes priority, and completions advance bounded chunks |
| Prepare the other Markdown presentation | 400 ms after a revision changes; projection completion starts prepared Code syntax when needed |

The owner arms only its earliest outstanding deadline. Superseding edits
replace the deadline and cancel stale work. Once the deadline has fired, waiting
for a worker or reaching a terminal result does not re-arm it. The bounded
100 ms first-display syntax grace wait remains; it releases the core while
waiting and is not recurring idle work.

Windows search and completion use per-view one-shot callbacks only while work
is pending. Shared search changes notify every affected view. Drag autoscroll
runs only during an out-of-bounds drag; caret blinking only while focused.
Recovery backup delays are armed by changes rather than sampled on an idle
clock.

## Worker scheduling and cleanup

The pool admits at most 128 mailboxes at once. Overflow retains one scheduling
entry per pending service, containing only a weak mailbox and its shared close
flag. It never copies a request or text snapshot. Each service reserves its
single entry before enqueueing outside the mailbox lock. Workers promote older
waiters before admitting continuations, so saturation recovers without another
frontend request.

Scheduling payload is two pointer-sized handles per pending service (16 bytes
on 64-bit hosts), plus deque capacity and one shared atomic close allocation
per service. Overflow storage shrinks after bursts. Admission is amortized
constant time; close removes that service from the scheduling lists in time
proportional to pending registrations. Provider retention still has its existing
eight-session and 512 MiB limits.

An arm64 release comparison on 2026-11-15 against `056b72f` measured 368
additional requested heap bytes for the 4 MiB Code/two-view fixture, after
subtracting each process's startup baseline. The corresponding Markdown
fixture used 4,672 fewer bytes. This small bookkeeping cost is accepted to
eliminate polling; these core probes exclude native UI and actual syntax
providers, so they do not measure whole-app worker memory.

Close sets a shared lifecycle flag and records a persistent cleanup request in
the worker wait predicate. Cleanup does not infer closure from the mailbox's
reference count: a detached syntax wait may still hold it. This prevents a
notification before parking from losing cleanup forever. Native providers are
destroyed on workers outside mailbox, scheduler and cache locks.

## Validation

Core tests cover coalescing, callback context ownership, real completion
notifications, one-shot deadlines, saturation and continuations, fair admission,
cleanup with retained wait handles, stale prepared results, and more current
visible requests than coverage-cache entries. Native tests
cover automatic publication, coalescing, deadlines, reload and close. Windows
validation instructions for this change are in `windows_timer.md` at the
repository root.

For idle profiling, let useful work settle and deactivate the caret. Check
callback counts and worker wake stacks as well as CPU time. Repeat with one and
many Text, Code and Markdown documents, shared views and diff windows; a rounded
0% process reading alone is not proof of an idle scheduler.

The implementation's standard and rich-paste memory checks passed. Two
supplemental document-only deletion/history-pruning ceilings also fail on the
unchanged baseline: approximately 100.31 MiB and 1.97 MiB remain against a
1 MiB limit. Those fixtures call `mark_saved()` before deletion and retain the
saved-source snapshot; their source and limits were unchanged by this work.
