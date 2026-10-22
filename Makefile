.PHONY: debug release run-debug run-release run check-abi clean

# Both configurations share the app bundle; clean must not race a build.
.NOTPARALLEL:

debug release:
	./scripts/build-mac-app.sh $@

# Inherit make's working directory; Launch Services does not preserve it.
run-debug:
	./scripts/run-mac-app.sh debug

run-release:
	./scripts/run-mac-app.sh release

run: run-release

# Explicit native validation; ordinary Rust tests do not invoke a C compiler.
check-abi:
	cargo run --locked --example check_c_abi

# Remove every build output: path-dependent Swift/Clang caches, Rust builds,
# test output and consistency baselines (all under target/), and fuzz builds.
# scripts/clean-win.ps1 does the same on Windows.
clean:
	rm -rf .build target
	fuzz/clean.sh
	git worktree prune
