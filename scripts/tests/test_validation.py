"""Validation correctness without Rust builds or access to user GUI state."""
import importlib.util
import json
import os
import signal
import time
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

SCRIPTS = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(SCRIPTS))
import validation_support as support
import validate
spec = importlib.util.spec_from_file_location('native_tests', SCRIPTS / 'native-tests.py')
native = importlib.util.module_from_spec(spec)
spec.loader.exec_module(native)


class ValidationTests(unittest.TestCase):
    def test_evidence_reuse_requires_success_and_unchanged_complete_key(self):
        key = {'inputs': 'source', 'environment': {'rustc': 'A'}, 'options': {'filter': ['a']}}
        entry = {'outcome': 'passed', 'key': key, 'inputs_after': 'source'}
        self.assertIs(validate.reusable_evidence({'summary': [entry]}, key), entry)
        for changed in [dict(key, inputs='other'), dict(key, environment={'rustc': 'B'}),
                        dict(key, options={'filter': ['b']})]:
            self.assertIsNone(validate.reusable_evidence({'summary': [entry]}, changed))
        for changed in [dict(entry, outcome='failed'), dict(entry, inputs_after='edited')]:
            self.assertIsNone(validate.reusable_evidence({'summary': [changed]}, key))

    def test_native_cache_verifies_artifacts_and_inputs(self):
        with tempfile.TemporaryDirectory() as directory:
            binary = Path(directory) / 'test-binary'
            binary.write_bytes(b'compiled')
            identity = {'source': 'A', 'build_args': [], 'environment': 'toolchain'}
            record = {'identity': identity, 'roots': [directory], 'paths': [str(binary)], 'artifact_digest': support.digest_files([str(binary)])}
            self.assertTrue(native.cache_matches(record, identity)[0])
            extra = Path(directory) / 'unexpected-resource'; extra.write_bytes(b'extra')
            self.assertFalse(native.cache_matches(record, identity)[0])
            extra.unlink()
            self.assertFalse(native.cache_matches(record, dict(identity, source='B'))[0])
            binary.write_bytes(b'changed')
            self.assertFalse(native.cache_matches(record, identity)[0])
            binary.unlink()
            self.assertFalse(native.cache_matches(record, identity)[0])

    def test_damaged_bundle_rebuild_removes_extra_files_but_not_other_outputs(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            bundle = root / 'Tests.xctest'; bundle.mkdir()
            (bundle / 'expected').write_bytes(b'changed')
            (bundle / 'unexpected-resource').write_bytes(b'extra')
            unrelated = root / 'other-product'; unrelated.write_bytes(b'keep')
            native.discard_generated_outputs([str(bundle)])
            self.assertFalse(bundle.exists())
            self.assertEqual(unrelated.read_bytes(), b'keep')

    def test_test_selection_reuses_build_but_build_flags_do_not(self):
        build, test = native.split_swift_arguments(['-c', 'release', '-Xswiftc', '-enable-testing', '--filter', 'Menu', '--parallel'])
        self.assertEqual(build, ['-c', 'release', '-Xswiftc', '-enable-testing'])
        self.assertIn('--filter', test)
        with self.assertRaises(ValueError): native.split_swift_arguments(['--skip-build'])
        with self.assertRaises(ValueError): native.split_swift_arguments(['--filter'])

    def test_input_identity_includes_untracked_sources_and_ignores_notes(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            subprocess.run(['git', 'init', '-q', directory], check=True)
            (root / 'src/core').mkdir(parents=True)
            source = root / 'src/core/new file.rs'; source.write_text('one')
            names = support.input_files('focused', root)
            self.assertIn('src/core/new file.rs', names)
            first = support.digest_files(names, root)
            (root / 'notes.txt').write_text('unrelated')
            self.assertEqual(first, support.digest_files(support.input_files('focused', root), root))
            source.write_text('two')
            self.assertNotEqual(first, support.digest_files(names, root))

    def test_included_markdown_is_an_input_but_unrelated_docs_are_not(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            subprocess.run(['git', 'init', '-q', directory], check=True)
            (root / 'src/core').mkdir(parents=True); (root / 'docs').mkdir()
            (root / 'src/core/demo.rs').write_text('const DEMO: &str = include_str!("../../docs/demo.md");')
            demo = root / 'docs/demo.md'; demo.write_text('# fixture')
            (root / 'docs/workflow.md').write_text('unrelated prose')
            names = support.input_files('focused', root)
            self.assertIn('docs/demo.md', names)
            self.assertNotIn('docs/workflow.md', names)
            first = support.digest_files(names, root)
            demo.write_text('# changed fixture')
            self.assertNotEqual(first, support.digest_files(names, root))

    def test_license_changes_invalidate_native_packages_only(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            subprocess.run(['git', 'init', '-q', directory], check=True)
            (root / 'docs/licenses').mkdir(parents=True)
            (root / 'LICENSE.md').write_text('Viem license index')
            notice = root / 'docs/licenses/dependency.txt'
            notice.write_text('upstream notice')
            (root / 'docs/workflow.md').write_text('unrelated prose')
            names = support.input_files('native', root)
            self.assertIn('LICENSE.md', names)
            self.assertIn('docs/licenses/dependency.txt', names)
            self.assertNotIn('docs/workflow.md', names)
            first = support.digest_files(names, root)
            notice.write_text('updated upstream notice')
            self.assertNotEqual(first, support.digest_files(support.input_files('native', root), root))
            self.assertEqual(support.input_files('focused', root), [])

    @unittest.skipUnless(os.name == 'posix', 'descriptor inheritance is POSIX-specific')
    def test_build_lock_is_exclusive_reentrant_and_released_on_failure(self):
        with tempfile.TemporaryDirectory() as directory, mock.patch.object(support, 'ROOT', Path(directory)):
            with support.build_lock() as acquired:
                self.assertFalse(acquired['inherited'])
                with support.build_lock() as nested:
                    self.assertTrue(nested['inherited'])
                code = ('import sys; from pathlib import Path; '
                        f'sys.path.insert(0, {str(SCRIPTS)!r}); import validation_support as v; '
                        f'v.ROOT=Path({directory!r}); '
                        '\nwith v.build_lock(timeout=.1): print("acquired")')
                child = subprocess.run([sys.executable, '-c', code], capture_output=True, text=True, **support.child_options())
                self.assertEqual(child.returncode, 0, child.stderr)
                env = dict(os.environ); env.pop(support.LOCK_ENV, None)
                blocked = subprocess.run([sys.executable, '-c', code], env=env, capture_output=True, text=True)
                self.assertNotEqual(blocked.returncode, 0)
                self.assertIn('build lock unavailable', blocked.stderr)
            with self.assertRaisesRegex(ValueError, 'intentional'):
                with support.build_lock(): raise ValueError('intentional')
            with support.build_lock(): pass

    def test_preflight_socket_failure_prevents_native_build(self):
        with mock.patch.object(support.sys, 'platform', 'darwin'), \
             mock.patch.object(support.shutil, 'which', return_value='/tool'), \
             mock.patch.object(support.subprocess, 'check_output', return_value='Aqua'), \
             mock.patch.object(support.socket, 'socket', side_effect=PermissionError('socket denied')):
            run = mock.Mock()
            with self.assertRaisesRegex(PermissionError, 'socket denied'):
                support.native_preflight(run)
            run.assert_not_called()

    def test_contract_filters_resolve_to_existing_tests(self):
        contracts = json.loads((SCRIPTS / 'fixtures/markdown-contracts.json').read_text())
        root = SCRIPTS.parent
        for cases in contracts.values():
            for behavior, selected in cases.items():
                module, function = selected.split('::', 1)
                source = (root / 'tests/all' / f'{module}.rs').read_text()
                if function: self.assertIn(f'fn {function}(', source, behavior)

    def test_cargo_artifacts_use_reported_paths_and_distinct_baseline_logs(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            binary = root / 'cargo-reported-output'; binary.write_bytes(b'binary')
            run = mock.Mock(out=root, manifest={})
            def command(stage, name, argv, **kwargs):
                log = root / f'{stage}-{name}.log'
                log.write_text(json.dumps({'reason': 'compiler-artifact',
                    'target': {'name': 'probe', 'kind': ['example']}, 'executable': str(binary)}))
                return 0, log
            run.command.side_effect = command
            for cwd in (validate.ROOT, root):
                self.assertEqual(validate.cargo_artifact(run, 'consistency', [], 'probe', cwd=cwd), str(binary))
            logs = [item['log'] for item in run.manifest['artifacts']]
            self.assertEqual(len(set(logs)), 2)
            self.assertTrue(all(Path(log).exists() for log in logs))

    @unittest.skipUnless(os.name == 'posix', 'process-group interruption is POSIX-specific')
    def test_interruption_stops_descendants_and_records_elapsed_command(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory); ready = root / 'ready'; leaked = root / 'leaked'
            grandchild = f'import time,pathlib; time.sleep(.7); pathlib.Path({str(leaked)!r}).touch()'
            worker = ('import subprocess,sys,time,pathlib; '
                      f'subprocess.Popen([sys.executable,"-c",{grandchild!r}]); '
                      f'pathlib.Path({str(ready)!r}).touch(); time.sleep(20)')
            code = ('import sys,signal; from pathlib import Path; '
                    f'sys.path.insert(0,{str(SCRIPTS)!r}); import validate; '
                    'signal.signal(signal.SIGTERM,validate.interrupt_requested); '
                    f'run=validate.Run(Path({directory!r}),False); '
                    f'run.command("test","worker",[sys.executable,"-c",{worker!r}])')
            coordinator = subprocess.Popen([sys.executable, '-B', '-c', code], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            try:
                deadline = time.monotonic() + 5
                while not ready.exists() and time.monotonic() < deadline: time.sleep(.01)
                self.assertTrue(ready.exists(), 'worker never became ready')
                coordinator.send_signal(signal.SIGTERM); coordinator.wait(timeout=5)
                manifest = json.loads((root / 'manifest.json').read_text())
                record = manifest['stages'][0]
                self.assertTrue(record['interrupted'])
                self.assertGreaterEqual(record['seconds'], 0)
                self.assertIsNotNone(record['exit'])
                time.sleep(.8)
                self.assertFalse(leaked.exists(), 'a descendant survived interruption')
            finally:
                if coordinator.poll() is None:
                    coordinator.kill(); coordinator.wait()

    def test_changed_inputs_cannot_be_recorded_as_success(self):
        # Exercise the actual stage coordinator, not just the reuse predicate.
        with tempfile.TemporaryDirectory() as directory, \
             mock.patch.object(validate, 'source_identity', return_value={}), \
             mock.patch.object(validate, 'environment_identity', return_value={}), \
             mock.patch.object(validate, 'evidence_key', return_value={'inputs': 'before'}), \
             mock.patch.object(validate, 'input_identity', return_value='after'), \
             mock.patch.object(validate, 'stage_check'), \
             mock.patch.object(sys, 'argv', ['validate.py', 'check', '--out', directory]):
            self.assertEqual(validate.main_locked({'seconds': 0}), 1)
            manifest = json.loads((Path(directory) / 'manifest.json').read_text())
            self.assertIn('stale', manifest['summary'][0]['outcome'])
            self.assertIsNone(validate.reusable_evidence(manifest, {'inputs': 'before'}))


if __name__ == '__main__': unittest.main()
