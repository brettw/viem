# C ABI validation

The Rust FFI definitions and `include/viem_core.h` are maintained together.
The ABI check compares every C header struct size, alignment and field offset
with the current Rust core. `build.rs` generates that inventory from both public
headers on every header change; new structures and fields cannot be omitted from
the comparison. Additional explicit assertions cover constants and function
declarations.
It compiles a temporary C source containing the assertions, without linking or
executing a C program. A missing compiler or failed assertion exits nonzero and
reports the compiler's diagnostics.

Run from the repository root:

```sh
cargo run --locked --example check_c_abi
```

Ordinary `cargo test`, including `cargo test --all-targets`, does not execute
this check. The Rust FFI behavior tests remain in the normal suite. Building
the Rust dependencies can still require a C/C++ compiler for Tree-sitter.

## macOS and other Unix hosts

Install the platform C compiler so that `cc` is available on `PATH`. On macOS,
the Xcode command-line tools provide it. `make check-abi` runs the same command,
and `scripts/test-mac.sh` runs it before the native application and Swift tests.

## Windows with MSVC

Install Visual Studio's C++ build tools and a Windows SDK. Run the command from
a Developer PowerShell or Developer Command Prompt configured for the same
architecture as the Rust toolchain. The check uses `cl` with C11 syntax checks
and treats warnings as errors.

For example, initialize an x64 Visual Studio 2026 Community developer environment
from PowerShell, then run the check:

```powershell
& 'C:\Program Files\Microsoft Visual Studio\18\Community\Common7\Tools\Launch-VsDevShell.ps1' -Arch amd64 -HostArch amd64 -SkipAutomaticLocation
cargo run --locked --example check_c_abi
```

Adjust the installation path for another Visual Studio version or edition.

## Native CI validation

Run `cargo run --locked --example check_c_abi` as an explicit step after setting
up the platform compiler, or use `scripts/test-mac.sh` for macOS validation.
Use native Rust and C toolchains targeting the same platform and architecture;
the generated assertions describe the Rust executable that is running them.
The repository does not currently define a CI workflow.

This checks agreement between the current core and header. It does not enforce
backward ABI compatibility, verify exported symbols by linking, or exercise
frontend marshaling; native integration tests cover the actual language boundary.

## Shared frontend policy APIs

`viem_core_view_native_actions` reports available selection/search intentions;
`viem_core_view_native_action` executes them through the shared controller.
`viem_core_style_parent_candidates` returns stable style IDs for an exact
stylesheet identity, using model role and inheritance constraints.

`viem_application_config` validates and fills the shared `config.json` schema.
Both native stores use its normalized in-memory result and diagnostics. Missing
fields receive portable defaults; unknown fields and explicit sparse style
values survive. Recent-document lists have at most ten absolute paths on both
platforms. Native code owns profile discovery, migration, file identity and
atomic I/O. Invalid files remain untouched; normalization alone never saves.

`viem_effect_batch_read` returns one descriptor of immutable arrays owned by the
batch. Copy needed data before releasing the handle; never release concurrently
with a borrowed read. The export is built once per batch and released with it,
avoiding repeated formatting between size queries and reads. `display_text`
contains the core-formatted Ex output; typed payloads remain available for host
actions. All command-error families, including unsupported commands, produce
message effects. Presentation exports include recording register, bounded UTF-8
pending-command text and active count.

## Native text decoration geometry

Both frontends draw filled rectangles one layout unit thick across the cluster's
full typographic width, using its resolved foreground. Let `baseline` be the row
baseline minus the scaled paint baseline offset, `ascent = max(0, baseline -
cluster.top)` and `descent = max(0, cluster.bottom - baseline)`. The underline's
top edge is `floor(baseline + max(1, descent * 0.35))`; the strikethrough's is
`floor(baseline - ascent * 0.32)`. Subtract the viewport origin after rounding.
These are the shared placement rules for
`EVEditorView.textDecorationsForDrawing` and `DrawingCache.DrawTextForeground`;
platform drawing APIs do not choose independent offsets or center a stroked line
on that top edge.
