.PHONY: debug release run-debug run-release run check-abi clean

# Both configurations share the app bundle; clean must not race a build.
.NOTPARALLEL:

debug release:
	./scripts/build-mac-app.sh $@

run-debug: debug
run-release: release

# Start the newly built executable even if another instance is already running.
run-debug run-release:
	open -n .build/Viem.app

run: run-release

# Explicit native validation; ordinary Rust tests do not invoke a C compiler.
check-abi:
	cargo run --locked --example check_c_abi

# Remove path-dependent Swift/Clang caches as well as Rust build artifacts.
clean:
	rm -rf .build target
