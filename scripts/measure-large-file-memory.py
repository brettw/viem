#!/usr/bin/env python3
"""Repeat the pinned allocator/process fixtures in fresh serial processes.

Build first: cargo build --release --offline --example large_file_memory
The original JSON is an immutable baseline. Output must use another path.
"""
import argparse
import hashlib
import json
import pathlib
import platform
import subprocess
import time

ROOT = pathlib.Path(__file__).resolve().parent.parent
BASELINE = ROOT / 'docs/large-file-memory-measurements.json'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', required=True, type=pathlib.Path)
    parser.add_argument('--binary', type=pathlib.Path,
                        default=ROOT / 'target/release/examples/large_file_memory')
    parser.add_argument('--supplemental', action='store_true')
    parser.add_argument('--limits', type=pathlib.Path)
    args = parser.parse_args()
    if args.output.resolve() == BASELINE.resolve():
        parser.error('preserve the original baseline; choose a different output')
    original = json.loads(BASELINE.read_text())
    cases = [case['arguments'] for case in original['results']]
    if args.supplemental:
        cases = [
            ['plain', 'lines', 104857600, 'exercise'],
            ['plain', 'lines', 104857600, 'document', 'utf16le'],
            ['plain', 'lines', 104857600, 'document', 'utf16be'],
            ['plain', 'lines', 104857600, 'document', 'latin1'],
            ['plain', 'long', 2097152, 'views'],
            ['plain', 'longword', 2097152, 'views'],
            ['plain', 'short', 2000000, 'exercise'],
            ['plain', 'crlf', 16777216, 'document'],
            ['plain', 'invalid', 1048576, 'open'],
        ]
    # Include new source files as well as tracked edits; HEAD alone does not
    # identify an implementation being measured before its eventual commit.
    source_hash = hashlib.sha256()
    for path in sorted((ROOT / 'src').rglob('*')):
        if path.is_file() and path.suffix in ('.rs', '.swift', '.h', '.c'):
            source_hash.update(str(path.relative_to(ROOT)).encode())
            source_hash.update(b'\0')
            source_hash.update(path.read_bytes())
    result = {
        'fixture_revision': 1,
        'source_revision': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
        'working_source_sha256': source_hash.hexdigest(),
        'binary_sha256': hashlib.sha256(args.binary.read_bytes()).hexdigest(),
        'probe_sha256': hashlib.sha256((ROOT / 'examples/large_file_memory.rs').read_bytes()).hexdigest(),
        'compiler': subprocess.check_output(['rustc', '--version'], text=True).strip(),
        'os': platform.platform(),
        'method': original['method'],
        'supplemental': args.supplemental,
        'results': [],
    }
    limits = json.loads(args.limits.read_text()) if args.limits else None
    failures = []
    for case in cases:
        start = time.monotonic()
        process = subprocess.run([str(args.binary.resolve()), *map(str, case)],
                                 cwd=ROOT, capture_output=True, text=True, timeout=600)
        samples = [json.loads(line) for line in process.stdout.splitlines() if line.strip()]
        entry = {'arguments': case, 'exit_code': process.returncode, 'samples': samples}
        if process.stderr:
            entry['stderr'] = process.stderr
        result['results'].append(entry)
        args.output.write_text(json.dumps(result, indent=2) + '\n')
        print(case, f'{time.monotonic() - start:.2f}s', f'exit={process.returncode}', flush=True)
        if process.returncode:
            failures.append(f'{case}: exit {process.returncode}')
        matched = None
        if limits:
            matched = next((item for item in limits['cases'] if item['arguments'] == case), None)
            if matched is None:
                failures.append(f'{case}: regression ceilings are missing')
            else:
                observed = {sample.get('phase') for sample in samples}
                for phase in matched['phases'].keys() - observed:
                    failures.append(f'{case}: required phase {phase} is missing')
        for sample in samples:
            if 'phase' not in sample:
                continue
            print(' ', sample['phase'],
                  f"live={sample['live_requested_heap_bytes']/1048576:.2f}MiB",
                  f"peak={sample['peak_requested_heap_bytes']/1048576:.2f}MiB",
                  f"undo={sample['can_undo']}", flush=True)
            if matched:
                ceiling = matched['phases'].get(sample['phase'])
                if ceiling:
                    for metric in ('live_requested_heap_bytes', 'peak_requested_heap_bytes'):
                        if sample[metric] > ceiling[metric]:
                            failures.append(f'{case}/{sample["phase"]}: {metric} exceeds {ceiling[metric]}')
                    if ceiling.get('can_undo') is True and sample['can_undo'] is not True:
                        failures.append(f'{case}/{sample["phase"]}: useful undo missing')
    if failures:
        raise SystemExit('\n'.join(failures))


if __name__ == '__main__':
    main()
