# 0001: macOS build and package layout

- Status: Accepted
- Date: 2026-09-05

## Context

The Viem specification requires an AppKit frontend under `src/mac`, a narrow C
ABI to the Rust core, and a recorded numeric deployment target. It had left the
concrete build and package layout open.

## Decision

The macOS frontend uses one root Swift Package with small dependency-directed
targets:

- `CViemCore` exposes the normative `include/viem_core.h` header to Swift and
  links the Cargo-produced Rust static library;
- `ViemCoreTextProvider` owns Core Text measurement and render resources;
- `ViemAppShell` owns document/window lifecycle, menus, and application chrome;
- `ViemEditor` composes the core, provider, and custom editor surface; and
- `Viem` is the executable composition root.

Cargo remains responsible for building `viem-core`. `scripts/build-mac-app.sh`
builds both language halves and packages the executable and `Info.plist` as
`.build/Viem.app`. Linking the Rust static library avoids a relocatable dynamic
library dependency in the development bundle. The package and bundle declare a
macOS 26.0 deployment target, matching the latest major SDK available when this
frontend was introduced (macOS SDK 26.5).

The packaging script copies the shared `assets/vim` syntax snapshot into
`Contents/Resources/vim` before signing. It verifies source and destination
against the SHA-256 manifest and replaces the previous owned subtree to remove
obsolete files. The native configuration store resolves the installed bundle
path and always uses these resources, without a configurable directory or
persisting a location-dependent path. The same snapshot is available to Windows
packaging, tracked in `docs/windows-vim-runtime-followup.md`.

## Consequences

The dependency graph keeps AppKit and Core Text out of Rust and permits unit
tests for the shell, editor bridge, and provider independently. A release build
must build the matching Cargo profile before SwiftPM; the packaging script does
this automatically. Distribution signing, notarization, icons, and an Xcode
archive workflow remain later release-engineering work and do not change the
source-module boundaries.
