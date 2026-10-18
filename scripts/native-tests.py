#!/usr/bin/env python3
"""Prepare verified native artifacts once; run focused or complete tests repeatedly."""
import argparse
import datetime
import json
import os
import shutil
from pathlib import Path
import subprocess
import sys
import tempfile
import time

from validation_support import (ROOT, build_lock, child_options, digest_files,
                                environment_identity, input_identity, native_preflight)


def split_swift_arguments(arguments):
    """Selection/execution flags do not invalidate compiled test artifacts."""
    build = []
    test = []
    value_flags = {'--filter', '--skip', '--num-workers', '--xunit-output'}
    switches = {'--parallel', '--no-parallel', '--list-tests', '--enable-test-discovery'}
    index = 0
    while index < len(arguments):
        value = arguments[index]
        if value == '--skip-build':
            raise ValueError('Use automatic verified artifact reuse; raw --skip-build is unsupported')
        if value in value_flags:
            if index + 1 == len(arguments): raise ValueError(f'{value} needs a value')
            test += arguments[index:index + 2]; index += 2; continue
        if value in switches or any(value.startswith(flag + '=') for flag in value_flags):
            test.append(value)
        else:
            build.append(value); test.append(value)
        index += 1
    return build, test


def artifact_roots(bin_dir, profile, package, require=True):
    roots = [ROOT / 'target' / profile / 'libviem_core.a', bin_dir / 'ViemPackageTests.xctest']
    roots += [bin_dir / name for name in ('Viem', 'viem-cli', 'blocking-viem')]
    if package: roots.append(ROOT / '.build/Viem.app')
    if require and any(not path.exists() for path in roots):
        raise RuntimeError('native build did not produce required artifacts')
    return [str(path) for path in roots]


def artifact_inventory(roots):
    paths = []
    for name in roots:
        path = Path(name)
        if path.is_dir(): paths += [str(p) for p in path.rglob('*') if p.is_file()]
        elif path.is_file(): paths.append(str(path))
        else: return []
    return sorted(set(paths))


def discard_generated_outputs(roots):
    """Roots come from the current Swift/Cargo build configuration, not the cache.

    A normal incremental build need not notice modified output bytes or remove
    extra bundle resources. Remove these specific generated products before
    rebuilding a damaged artifact set; leave every other build product intact.
    """
    for name in roots:
        path = Path(name)
        if path.is_symlink() or path.is_file(): path.unlink()
        elif path.is_dir(): shutil.rmtree(path)


def cache_matches(record, identity):
    if record.get('identity') != identity:
        return False, 'source, environment, build options or packaging requirement changed'
    paths = record.get('paths', [])
    if not record.get('roots') or artifact_inventory(record['roots']) != paths:
        return False, 'compiled artifact inventory changed'
    if not paths or any(not Path(path).is_file() for path in paths):
        return False, 'compiled artifact missing'
    if digest_files(paths) != record.get('artifact_digest'):
        return False, 'compiled artifact bytes changed'
    return True, 'source and compiled artifacts verified'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--no-package', action='store_true', help='tests do not require a packaged app; skip packaging')
    parser.add_argument('--report', type=Path, help='native phase manifest (default: timestamped validation directory)')
    options, arguments = parser.parse_known_args()
    build_args, test_args = split_swift_arguments(arguments)
    stamp = datetime.datetime.now().strftime('%Y%m%d-%H%M%S-%f')
    report = options.report or ROOT / 'target/validation' / f'native-{stamp}' / 'manifest.json'
    report.parent.mkdir(parents=True, exist_ok=True)
    manifest = {'started': datetime.datetime.now().isoformat(), 'phases': []}
    def save(): report.write_text(json.dumps(manifest, indent=2) + '\n')
    def run(name, argv):
        started = time.monotonic()
        log = report.parent / f'native-{name}.log'
        print(f'[native/{name}] {" ".join(argv)}', flush=True)
        with log.open('w') as output:
            process = subprocess.Popen(argv, cwd=ROOT, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                                       text=True, **child_options())
            try:
                for line in process.stdout:
                    output.write(line); output.flush(); print(line, end='', flush=True)
                code = process.wait()
            finally:
                if process.poll() is None:
                    process.terminate(); process.wait(timeout=10)
        manifest['phases'].append({'phase': name, 'argv': argv, 'seconds': round(time.monotonic() - started, 3),
                                   'exit': code, 'log': str(log)})
        save()
        if code: raise RuntimeError(f'native {name} failed (exit {code}); see {log}')
        return log.read_text()
    try:
        with build_lock() as lock, tempfile.TemporaryDirectory(prefix='viem-tests-config-') as config:
            manifest['lock'] = lock
            os.environ.update(VIEM_CONFIG_DIR=config, MACOSX_DEPLOYMENT_TARGET='26.0',
                              CLANG_MODULE_CACHE_PATH=str(ROOT / '.build/clang-module-cache'),
                              SWIFTPM_MODULECACHE_OVERRIDE=str(ROOT / '.build/clang-module-cache'))
            profile = os.environ.setdefault('VIEM_RUST_PROFILE', 'native-test')
            native_preflight(run)
            identity = {'source': input_identity('native'), 'environment': environment_identity(True),
                        'build_args': build_args, 'package': not options.no_package}
            manifest['identity'] = identity
            cache_file = ROOT / '.build/validation/native-artifacts.json'
            try: cache = json.loads(cache_file.read_text())
            except (OSError, ValueError): cache = {}
            reusable, reason = cache_matches(cache, identity)
            manifest['artifacts_reused'] = reusable
            manifest['artifact_reason'] = reason
            print(f'[native/artifacts] {reason}', flush=True)
            if not reusable:
                prior_valid, prior_reason = cache_matches(cache, cache.get('identity')) if cache.get('roots') else (True, '')
                if not prior_valid:
                    print(f'[native/artifacts] rebuilding generated outputs: {prior_reason}', flush=True)
                    current_bin = run('locate-damaged-artifacts', ['swift', 'build', '--disable-sandbox', '--show-bin-path', *build_args]).strip()
                    roots = artifact_roots(Path(current_bin), profile, not options.no_package, require=False)
                    discard_generated_outputs(roots)
                run('abi', ['cargo', 'run', '--locked', '--profile', profile, '--example', 'check_c_abi'])
                run('rust', ['cargo', 'build', '--locked', '--profile', profile])
                # SwiftPM tracks Swift/header changes, but not the linked Rust
                # archive. Only that boundary (or damaged output) forces a C
                # dependency invalidation; test-only edits retain native modules.
                archive = ROOT / 'target' / profile / 'libviem_core.a'
                rust_link = {'path': str(archive), 'digest': digest_files([str(archive)])}
                if cache.get('rust_link') != rust_link or reason in ('compiled artifact bytes changed', 'compiled artifact inventory changed'):
                    print('[native/link] Rust archive identity changed or output damaged; invalidating shim', flush=True)
                    (ROOT / 'src/mac/CViemCore/shim.c').touch()
                if not options.no_package:
                    configuration = 'debug'
                    for index, arg in enumerate(build_args):
                        if arg in ('-c', '--configuration') and index + 1 < len(build_args): configuration = build_args[index + 1]
                    run('package', ['scripts/build-mac-app.sh', configuration])
                run('test-build', ['swift', 'build', '--disable-sandbox', '--build-tests', *build_args])
                bin_path = run('locate-artifacts', ['swift', 'build', '--disable-sandbox', '--show-bin-path', *build_args]).strip()
                if input_identity('native') != identity['source']:
                    raise RuntimeError('native inputs changed during build; artifacts were not certified')
                roots = artifact_roots(Path(bin_path), profile, not options.no_package)
                paths = artifact_inventory(roots)
                cache = {'identity': identity, 'roots': roots, 'paths': paths, 'artifact_digest': digest_files(paths), 'rust_link': rust_link}
                cache_file.parent.mkdir(parents=True, exist_ok=True)
                candidate = cache_file.with_suffix('.tmp')
                candidate.write_text(json.dumps(cache, indent=2) + '\n'); candidate.replace(cache_file)
            save()
            run('tests', ['swift', 'test', '--disable-sandbox', '--skip-build', *test_args])
            manifest['source_after'] = input_identity('native')
            if manifest['source_after'] != identity['source']:
                raise RuntimeError('native inputs changed during tests; result is stale')
            valid, reason = cache_matches(cache, identity)
            if not valid: raise RuntimeError(f'native artifacts changed during tests: {reason}')
            manifest['outcome'] = 'passed'
    except (RuntimeError, OSError, subprocess.SubprocessError, ValueError) as error:
        manifest['outcome'] = 'failed'; manifest['error'] = str(error)
        print(str(error), file=sys.stderr)
    finally:
        manifest['finished'] = datetime.datetime.now().isoformat()
        save()
        print(f'native report: {report}', flush=True)
    return 0 if manifest['outcome'] == 'passed' else 1


if __name__ == '__main__':
    sys.exit(main())
