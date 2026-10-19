"""Shared build ownership, content identities and native capability checks."""
import contextlib
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import shutil
import socket
import subprocess
import sys
import tempfile
import time

ROOT = Path(__file__).resolve().parent.parent
LOCK_ENV = 'VIEM_BUILD_LOCK_FD'


def inherited_lock_fds():
    """Only inherit a descriptor for this checkout's lock, never an arbitrary FD."""
    try:
        fd = int(os.environ[LOCK_ENV])
        if os.name == 'posix' and (os.fstat(fd).st_dev, os.fstat(fd).st_ino) == ((ROOT / 'target/validation/build.lock').stat().st_dev, (ROOT / 'target/validation/build.lock').stat().st_ino):
            return (fd,)
    except (KeyError, ValueError, OSError):
        pass
    return ()


def child_options():
    return {'pass_fds': inherited_lock_fds()} if os.name == 'posix' else {}


@contextlib.contextmanager
def build_lock(timeout=60):
    """One owner across validation and native scripts; descendants share its FD."""
    if inherited_lock_fds():
        yield {'seconds': 0.0, 'inherited': True}
        return
    path = ROOT / 'target/validation/build.lock'
    path.parent.mkdir(parents=True, exist_ok=True)
    started = time.monotonic()
    with path.open('a+b') as lock:
        if os.name == 'posix':
            import fcntl
            acquire = lambda: fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        else:
            import msvcrt
            if path.stat().st_size == 0:
                lock.write(b' '); lock.flush()
            def acquire():
                lock.seek(0)
                msvcrt.locking(lock.fileno(), msvcrt.LK_NBLCK, 1)
        announced = False
        while True:
            try:
                acquire()
                break
            except (BlockingIOError, PermissionError):
                if not announced:
                    print(f'Waiting for build owner: {path.read_text(errors="replace").strip()}', flush=True)
                    announced = True
                if time.monotonic() - started >= timeout:
                    raise RuntimeError(f'build lock unavailable after {timeout}s: {path}')
                time.sleep(.1)
        lock.seek(0); lock.truncate()
        lock.write(json.dumps({'pid': os.getpid(), 'started': time.time()}).encode()); lock.flush()
        old = os.environ.get(LOCK_ENV)
        if os.name == 'posix':
            os.environ[LOCK_ENV] = str(lock.fileno())
        try:
            yield {'seconds': round(time.monotonic() - started, 3), 'inherited': False}
        finally:
            if old is None: os.environ.pop(LOCK_ENV, None)
            else: os.environ[LOCK_ENV] = old
            # Closing the descriptor releases ownership, including on exceptions.


def digest_files(paths, root=ROOT):
    digest = hashlib.sha256()
    for name in sorted(set(str(p) for p in paths)):
        path = root / name
        digest.update(name.encode()); digest.update(b'\0')
        if path.is_file():
            digest.update(str(path.stat().st_mode).encode()); digest.update(b'\0')
            digest.update(path.read_bytes())
        else:
            digest.update(b'<missing>')
        digest.update(b'\0')
    return digest.hexdigest()


def input_files(stage, root=ROOT):
    names = subprocess.check_output(['git', 'ls-files', '-z', '--cached', '--others', '--exclude-standard'], cwd=root).decode().split('\0')
    prefixes = ['src/core/', '.cargo/', 'scripts/', 'assets/', 'data/']
    if stage != 'native': prefixes += ['tests/', 'examples/']
    exact = {'Cargo.toml', 'Cargo.lock', 'build.rs'}
    if stage == 'native':
        prefixes += ['src/mac/', 'include/', 'docs/licenses/']
        exact.update({'examples/check_c_abi.rs', 'Package.resolved', 'LICENSE.md'})
        exact.add('Package.swift')
    selected = {name for name in names if name and (name in exact or any(name.startswith(p) for p in prefixes))}
    # Repository Markdown and other files are also executable fixtures when
    # included by Rust. Follow literal includes instead of excluding prose by
    # extension or making every documentation edit invalidate all evidence.
    for name in list(selected):
        path = root / name
        if path.suffix == '.rs' and path.is_file():
            for included in re.findall(r'\binclude_(?:str|bytes)!\s*\(\s*"([^"\n]+)"', path.read_text()):
                dependency = (path.parent / included).resolve()
                try: selected.add(str(dependency.relative_to(root.resolve())))
                except ValueError: raise RuntimeError(f'validation input escapes checkout: {dependency}')
    return sorted(selected)


def input_identity(stage):
    return digest_files(input_files(stage))


def environment_identity(native=False):
    names = ['PATH', 'RUSTFLAGS', 'RUSTDOCFLAGS', 'RUSTC', 'RUSTC_WRAPPER', 'RUSTC_WORKSPACE_WRAPPER', 'RUSTUP_TOOLCHAIN', 'CARGO_TARGET_DIR', 'CARGO_BUILD_TARGET',
             'VIEM_SHADOW_PROJECTION', 'VIEM_SHADOW_PROJECTION_LIMIT', 'CC', 'CFLAGS', 'SDKROOT',
             'DEVELOPER_DIR', 'MACOSX_DEPLOYMENT_TARGET', 'VIEM_RUST_PROFILE',
             'CLANG_MODULE_CACHE_PATH', 'SWIFTPM_MODULECACHE_OVERRIDE']
    tools = ['rustc', 'cargo'] + (['swift', 'xcrun'] if native else [])
    versions = {}
    for tool in tools:
        location = shutil.which(tool)
        if location:
            result = subprocess.run([location, '--version'], capture_output=True, text=True, timeout=15)
            versions[tool] = {'path': location, 'version': (result.stdout + result.stderr).strip()}
        else:
            versions[tool] = None
    return {'workspace': str(ROOT), 'platform': platform.platform(), 'machine': platform.machine(), 'tools': versions,
            'variables': {name: os.environ.get(name) for name in names}}


def native_preflight(run):
    if sys.platform != 'darwin':
        raise RuntimeError('native tests require macOS')
    for tool in ('swift', 'xcrun', 'caffeinate'):
        if not shutil.which(tool): raise RuntimeError(f'{tool} is not on PATH')
    session = subprocess.check_output(['launchctl', 'managername'], text=True).strip()
    if session != 'Aqua': raise RuntimeError(f'no GUI login session: {session}')
    # Check restricted facilities before any native build. Do not touch the
    # general clipboard or user settings; the Swift probe uses a private board.
    with tempfile.TemporaryDirectory(prefix='viem-preflight-') as directory:
        with socket.socket(socket.AF_UNIX) as endpoint:
            endpoint.bind(str(Path(directory) / 'completion.sock'))
            endpoint.listen(1)
        target = Path(directory) / 'write-test'
        target.write_bytes(b'probe')
        if target.read_bytes() != b'probe': raise RuntimeError('temporary file round trip failed')
    source = ROOT / 'scripts/native-preflight.swift'
    cache = ROOT / '.build/validation'
    cache.mkdir(parents=True, exist_ok=True)
    key = hashlib.sha256((source.read_text() + json.dumps(environment_identity(True), sort_keys=True)).encode()).hexdigest()[:20]
    executable = cache / f'preflight-{key}'
    if not executable.exists():
        run('preflight-build', ['xcrun', 'swiftc', str(source), '-o', str(executable)])
    run('preflight-capabilities', [str(executable)])


if __name__ == '__main__':
    # Shell build entry points use this wrapper. Reentrancy is validated by FD.
    if sys.argv[1:] == ['--check-lock']:
        raise SystemExit(0 if inherited_lock_fds() else 1)
    command = sys.argv[1:]
    if command[:1] == ['--']: command = command[1:]
    if not command: raise SystemExit('usage: validation_support.py -- COMMAND [ARGS]')
    try:
        with build_lock() as lock:
            print(f'Build lock acquired in {lock["seconds"]:.3f}s', flush=True)
            raise SystemExit(subprocess.call(command, **child_options()))
    except RuntimeError as error:
        raise SystemExit(str(error))
