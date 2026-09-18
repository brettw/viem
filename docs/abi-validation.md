# C ABI validation

The Rust FFI definitions and `include/viem_core.h` are maintained together.
The explicit ABI check compares the C header's struct sizes, alignment, selected
field offsets, constants, and function declarations with the current Rust core.
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
