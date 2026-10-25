# Reference link controls

Full (`[text][name]`), collapsed (`[name][]`), and defined shortcut (`[name]`)
links show only their formatted labels in Preview, with Markdown reference
styling. Definitions retain their visible source notation. Their caret popup
uses the existing link surface. Image references keep their existing image
controls; footnotes are outside this feature.

The `[d]` toolbar button follows the caret or selection. At a reference it opens
that reference's settings; in eligible prose it inserts a reference around the
selection, or inserts entered text at the caret. The name picker offers existing
definitions. A new name creates a shared definition at the document end, together
with the use in one undo step. Selected inline formatting is retained. Definition
choices are enumerated only when the editor opens.

Edit changes the occurrence's label or reference. The label field contains its
visible text; changing the reference alone preserves its source spelling and
inline formatting.
Changing a collapsed/shortcut label expands it to a full reference so its target
stays unchanged. The reference picker lists active definitions.

Edit shared definition changes the destination and optional title, with an
explicit count of affected links and images. Rename reference everywhere updates
uses while preserving their labels, including image alternative text. Convert to
inline detaches one occurrence and preserves its title and label markup. Remove
link retains the visible formatted label. Neither action deletes the definition.

Definitions expose the same settings, Find uses, and Rename. Later duplicate
definitions are marked inactive. Editing them does not change the active target.
Deleting an active definition explicitly converts its uses to inline forms in
the same transaction; unused or inactive definitions can be deleted directly.

Go to definition navigates to the source definition and centers its caret row
within the available scroll extent. The button beside Edit shared definition in
the edit form also opens the destination popup in edit mode; the compact popup's
Go to definition action leaves it in its normal mode. Navigation discards any
unapplied draft without changing the source. Open target uses existing
link navigation, including heading fragments. Find uses lists the occurrences.
Document jumps enter the Vim jump list. All actions use exact captured identities;
a changed selection or revision rejects the action. Cancel changes no source.

Explicit unresolved full/collapsed references offer choosing an existing
reference or creating a definition at the document end. Creation explains that
other matching occurrences can become active. It is verified after insertion,
so an unclosed source container cannot silently swallow the new definition.
Unresolved shortcut bracket text stays ordinary text.

The disposable reference index is revision-bound. It reads projected owners,
not a whole-document text copy, and has a 2 MiB / 4096-owner budget plus a
65536-style traversal budget. Incomplete enumeration is indicated and disables
shared mutations. Local unresolved-candidate recognition reads at most 8 KiB.
These are presentation limits, not syntax or document size limits.

## Feature contracts and basic validation

The implementation touches recognition (all spellings and duplicates), formatted
label ownership and source preservation, syntax activation by definitions, native authoring, source
locality, history, and navigation/stale publication. Focused examples are in
`tests/all/markdown_references.rs`; macOS popup checks are in
`EVLinkPopoverTests`. Reference images are covered only as dependents of shared
definition changes.

Full encoding/container/large-document matrices, consistency campaigns, and
platform-wide validation remain separate from the initial manual testing pass.
