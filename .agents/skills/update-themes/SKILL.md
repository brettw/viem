---
name: update-themes
description: Copy the current user's saved Viem themes over the corresponding bundled theme files in this repository to make them the defaults for new installs. Use when asked to update bundled themes from local edits; an optional theme name selects only that theme, otherwise update all bundled themes.
---

# Update Themes

Promote saved local themes to the repository's bundled defaults. A request to
run this skill authorizes the matching file replacements; proceed without an
extra confirmation unless the user imposed a restriction or there is a conflict
with someone else's work. Creating or editing this skill does not run it.

## Select the files

- Work in the Viem checkout containing this skill. Read its applicable
  `AGENTS.md` instructions and inspect Git status before changing anything.
- Resolve the source profile for the current user: use an explicitly requested
  profile directory, otherwise `VIEM_CONFIG_DIR` when set, otherwise the current
  user's home directory plus `.viem`. Append `themes` to that profile path.
  This policy is shared by macOS and Windows; do not hard-code a username or use
  Application Support/AppData. If an override is unusable, report it instead of
  silently choosing another profile.
- Enumerate the tracked JSON files directly in `assets/themes/` to determine
  the bundled set. Currently these are Paper, Midnight, Midnight Mono,
  Midnight Proportional, and Typewriter. Use the checkout's files as authority
  if the set changes.
- If the invocation supplies a theme name, select only that bundled filename
  stem. Treat a multiword name as one name; an optional `.json` suffix is fine.
  Match the exact filename, not a style name, display label, substring, or
  guessed alias. An unknown or ambiguous name is an error: list the available
  bundled names and make no changes. Never fall back to all themes after a
  failed match.
- If no name is supplied, select every bundled theme. For each selected
  destination, use the identically named JSON file in the source directory.
  Ignore additional local themes. Do not copy `config.json`, `startup.viem`,
  obsolete standalone style files, or the entire profile directory.

Examples: `$update-themes` updates all bundled themes; `$update-themes Paper`
updates only Paper; `$update-themes Midnight Proportional` updates only
Midnight Proportional.

## Copy and verify

1. Preflight the entire selected set before writing: require every source and
   destination to exist as a readable file, read the source bytes, and verify
   that each source parses as a JSON object. Report missing or invalid files
   and leave every destination unchanged. Keep the original destination bytes
   and the source snapshots until verification finishes.
2. Inspect existing changes to the selected destinations. Unrelated dirty
   files do not block this task. If a selected destination has conflicting
   edits from another task, explain the specific conflict instead of silently
   replacing that work. Recheck the selected files against the snapshots
   immediately before writing; if they changed during preparation, stop and
   report the conflict rather than copying stale data.
3. Copy only changed files, preserving the complete local file bytes. Use
   atomic replacement for each destination and retain its file permissions.
   Do not merge, reformat, normalize, regenerate, or edit individual theme
   properties. Report any partial failure accurately; do not reset unrelated
   working-tree changes.
4. Confirm every selected destination matches its captured source bytes, then
   review the diff restricted to those files. Run the focused core checks from
   the repository root when files changed:

   ```sh
   cargo test --locked --lib document::theme::tests::
   ```

   These include validation of every packaged preset and the full Midnight
   versus compiled Default comparison. JSON parsing alone does not validate
   Viem's theme schema. Report validation failures or build blockers explicitly;
   do not change source code, fixtures, or tests to force a copied theme to pass.
5. Report the resolved source directory, updated and unchanged theme names,
   and validation result. Leave the changes available for review; do not stage,
   commit, push, build an app bundle, or modify unrelated files unless asked.

## Viem-specific behavior

`assets/themes/` is shared by both native frontends. Default embeds
`assets/themes/Midnight.json` at compile time, so updating Midnight also updates
Default in the next build; there is no separate Default file to copy. Never
replace local edits using `examples/theme_defaults.rs` or generated presets.

Named-theme edits save automatically. The special Default theme is editable
only in memory and has no save path; do not guess its values from `config.json`.
If the user asks to promote unsaved Default edits, explain that they must first
save them as a named theme and explicitly identify the intended bundled target.

These copies change defaults shipped by future builds and new profiles.
Existing profiles retain their local customizations; do not rewrite the user's
source theme files or trigger Revert/Reload in the running app.
