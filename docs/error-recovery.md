# Error recovery

Source bytes, source transactions and resource identities stay authoritative.
Configuration and rendering are allowed to degrade when a verified document can
still be edited. A warning describes the degradation; it never turns a failed
source edit, save or clipboard transfer into a success.

## Recovery boundaries

| Failure | Behavior |
| --- | --- |
| Missing installed or bundled font | Native font lookup uses the requested fallback families, then the system font. Bundled-font registration failures leave installed fonts available. |
| Native shaper declines a request | Retry that batch once with system typography, preserving Unicode context, direction, language, script, scale, text ranges and render-resource policy. Successful fallback carries a layout warning. |
| Font registration or device generation changes during shaping | Recapture compatible layout work through the existing bounded coordinator retry. A font retry cannot disguise a resource transition. |
| Missing style, broken inheritance, invalid appearance declarations, conflicting named styles or invalid OpenType tags | Presentation capture can use the format's built-in stylesheet. Retain paragraph, list, quote and code owners and exact coordinates. The configured sheet and source remain unchanged. |
| Invalid initial or live theme settings | Keep available validated styles, report diagnostics, and continue applying changes to other documents. Later corrected themes can still apply. Style editing and preference validation remain strict. |
| Invalid optional document defaults, syntax resources, detection setup, startup commands or selection preferences | Keep the available defaults and continue independent setup. Core integrity/ownership failures still propagate. |
| Missing, failed or over-budget syntax provider | Use available highlighting fallback or ordinary text styling. Highlighting never blocks source editing or becomes source content. |
| Completion or whitespace-marker export fails | Omit that optional UI data for the frame and warn; verified text geometry remains usable. |
| Presentation export fails | Keep a previous Mac frame only if core still attests to its complete identity. Otherwise discard geometry and report the error; the buffer and command surface remain available. Windows clears a failed frame. A later refresh can resume. |
| Background pre-layout fails or is superseded | Discard the candidate. Windows stops speculative scheduling for the failed dependencies; foreground interaction remains independent. |
| Initial recovery-backup claim fails | Keep the opened document and report that backups are unavailable. Native recovery writers retain existing valid backups on failed writes. Rebinding a backup during a source replacement retains its stronger preservation requirements. |
| Clipboard access, file read/write or external host effect fails | Report failure and retain the document. Do not replay the operation automatically, mark a failed save as saved, or substitute empty source for a failed read. An effect failure after a committed command retains that command's source/history; refresh the view and report the failed effect. |
| Unsupported source transform, encoding policy, read-only action, stale selection, invalid Unicode range or failed transaction verification | Reject the operation without its success-dependent mutations. Do not flatten formatting, clamp source coordinates or switch encoding to force success. |
| Malformed provider response, invalid native lease, insufficient shaping context, invalid core ownership, panic or exhausted identity | Reject the result/operation. Default fonts cannot repair these contracts. Preserve the last valid source and allow later independent actions where core can still validate them. |

The shared recovery boundary covers visible regional layout, wrapped and unwrapped
layout, composition layout, and direct document layout. It operates on the captured
region, without traversing unrelated document content. Width-count measurements
also use the bounded system-font retry.

Recovered fragments use ordinary cache dependencies and budgets. Changing the
stylesheet, typography, metrics generation or measurement environment retires
incompatible entries. Core continues to validate every successful fallback
response before caching or installation. A failed fallback is a layout failure;
it does not imply that a source edit which already committed should be retried.

Native providers release partially created render resources on failure. Device
replacement prepares its measuring surface before exchanging ownership, and a
failed drawing flush still releases temporary brushes.

## Diagnostics and validation

Layout warnings cross the C ABI as a read-only, two-pass UTF-8 query against an
exact installed snapshot identity. The export examines at most 64 records,
returns at most 16 distinct messages and 8 KiB, and never performs layout work.
Both frontends cache the query per layout and suppress repeated unchanged
warnings. Optional setup diagnostics retain at most eight bounded messages.
Failure to read warnings cannot invalidate an otherwise verified frame.

The fault tests cover successful font recovery, failure of both attempts,
resource-generation changes, insufficient context, malformed fallback responses,
Unicode/bidi identity, empty caret geometry, cache invalidation, regional recovery
in large documents, Markdown structure, exact source/undo and stale diagnostic
exports. macOS tests exercise the real Core Text provider; Windows smoke checks
exercise the real DirectWrite provider. Run the native checks on their target OS:
see [macOS development](macos-development.md) and
[Windows development](../src/win/README.md).
