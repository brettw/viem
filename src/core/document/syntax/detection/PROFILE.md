# Portable language detection profile

Profile version 2 uses decoded normalized text and never executes Vimscript,
filetype autocommands, shell commands, regular expressions, or package callbacks.
An explicit language (including None) precedes modelines, user filename rules,
bundled filename rules with content disambiguation, shebangs, and finally content
signatures. Modelines retain unknown language names instead of guessing another.

Bundled filename rules match case-sensitive basenames. Vim script recognizes
`*.vim`, `.exrc`, `_exrc`, and `.netrwhist`, followed by the broad `*vimrc*`
fallback for `.vimrc`, `_vimrc`, `vimrc`, their gVim variants, and local variants
such as `.vimrc.local`. Specific recognized extensions take precedence over
this fallback; for example, `vimrc.py` selects Python. User associations and
modelines keep their higher precedence. These rules are portable declarations,
not execution of a configured Vim runtime's `filetype.vim`.

DetectionProfile compiles an ordered declarative list of ContentSignature and
ExtensionDisambiguator values. Its default includes the PHP/XML signatures and
C/C++/Objective-C header and Objective-C/MATLAB disambiguators. An unresolved .h
uses C and an unresolved .m uses Objective-C. Registration inserts a new rule
before existing rules; the most recent matching registration wins within that
stage. User filename associations still precede every extension disambiguator.
A custom extension disambiguator can also recognize an otherwise unknown suffix.

A rule specifies a language, a literal, and one of three match scopes: anywhere
within a sampled complete line, after whitespace in logical line zero, or after
leading whitespace in the first nonempty sampled line. Literals never cross
logical line boundaries. Rules are case-sensitive; language aliases are
canonicalized after selection. No regex dialect or arbitrary native callback is
part of this interface.

The detector reads at most 64 KiB total, from complete first/last-five lines
inside fixed 32 KiB head/tail windows. It uses line indexes and bounded slices;
it skips a giant or truncated line. Modelines, shebangs and registered content
rules reuse these samples. Content rules share one compiled finite automaton
pass; they do not reread or join document ranges. A full-capacity adversarial
fixture verifies at most twice the sample byte count plus 64 output steps for
one line. General scan work is at most twice the sampled bytes plus 64 output
steps per sampled line, with at most ten sampled lines.

Compilation accepts at most 64 rules and 4096 total literal bytes, at most 256
bytes per literal and 128 per language/extension. It rejects empty, multiline or
NUL literals and invalid identifiers. Its trie has at most 4097 states, and
failed registration preserves the prior profile. Filename globs retain their
separate bounded count, pattern size and shared matching fuel.

A caller can use detect_with_profile directly or install an Arc<DetectionProfile>
through Core::set_code_detection_profile. Installation is an explicit policy
change and redetects an initialized buffer without changing source or undo.
Ordinary edits do not rerun detection. The detect wrapper uses the shared bundled
profile. Additional package integrations can supply this portable policy without
adding platform UI or C ABI dependencies.
