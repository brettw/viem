# Idle maintenance polling

The repeating syntax maintenance timers have been replaced with event-driven
notifications on macOS and Windows. The syntax pool now waits indefinitely when
idle, retries saturated queue admission itself and receives explicit cleanup
requests. Delayed highlighting and Markdown preparation use one-shot deadlines.

The maintained contract is [background work](docs/background-work.md). See
[Windows validation instructions](windows_timer.md) for the platform handoff.

The earlier 2026-09-17 note measured approximately 0.83% of one core with one
idle Code buffer before the polling fixes. That historical measurement is not a
measurement of this implementation. The separate 1 Hz Code stylesheet poll was
already removed; stylesheet reload remains explicit.
