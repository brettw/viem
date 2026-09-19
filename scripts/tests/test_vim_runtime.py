"""Snapshot integrity and replacement checks; no native build required."""

import importlib.util
from pathlib import Path
import tempfile
import unittest


SCRIPT = Path(__file__).resolve().parents[1] / "vim-runtime.py"
SPEC = importlib.util.spec_from_file_location("vim_runtime", SCRIPT)
vim = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(vim)


class RuntimeTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="viem-runtime-test-")
        self.directory = Path(self.temporary.name)
        self.source = self.directory / "source"
        (self.source / "syntax" / "shared").mkdir(parents=True)
        (self.source / "LICENSE").write_bytes(b"original license\r\n")
        (self.source / "syntax" / "vim.vim").write_bytes(b"runtime! syntax/shared/helper.vim\r\n")
        (self.source / "syntax" / "shared" / "helper.vim").write_bytes(b'" legacy encoding: \xe9\r\n')
        self.assets = self.directory / "Viem \u65e5\u672c.app" / "Contents" / "Resources" / "vim"
        self.assets.mkdir(parents=True)

    def tearDown(self):
        self.temporary.cleanup()

    def test_import_preserves_bytes_and_replaces_obsolete_files(self):
        (self.assets / "README.md").write_bytes(b"keep provenance instructions")
        self.assertEqual(vim.import_runtime(self.source, "fixture 1", self.assets), 3)
        copied = self.assets / "runtime" / "syntax" / "shared" / "helper.vim"
        self.assertEqual(copied.read_bytes(), b'" legacy encoding: \xe9\r\n')
        (self.assets / "runtime" / "syntax" / "obsolete.vim").write_bytes(b"old")
        vim.import_runtime(self.source, "fixture 2", self.assets)
        self.assertFalse((self.assets / "runtime" / "syntax" / "obsolete.vim").exists())
        self.assertEqual((self.assets / "README.md").read_bytes(), b"keep provenance instructions")
        self.assertEqual(vim.verify(self.assets), 3)

    def test_verification_rejects_changed_missing_and_unexpected_files(self):
        for change in ["changed", "missing", "unexpected"]:
            with self.subTest(change=change):
                vim.import_runtime(self.source, "fixture", self.assets)
                syntax = self.assets / "runtime" / "syntax"
                if change == "changed":
                    (syntax / "shared" / "helper.vim").write_bytes(b"changed")
                elif change == "missing":
                    (syntax / "shared" / "helper.vim").unlink()
                else:
                    (syntax / "unexpected.vim").write_bytes(b"extra")
                with self.assertRaisesRegex(ValueError, "differs from manifest"):
                    vim.verify(self.assets)

    def test_invalid_import_leaves_previous_snapshot_intact(self):
        vim.import_runtime(self.source, "fixture", self.assets)
        manifest = (self.assets / "manifest.json").read_bytes()
        (self.source / "syntax" / "vim.vim").unlink()
        with self.assertRaisesRegex(ValueError, "must contain"):
            vim.import_runtime(self.source, "invalid", self.assets)
        self.assertEqual((self.assets / "manifest.json").read_bytes(), manifest)
        self.assertEqual(vim.verify(self.assets), 3)

    def test_verification_rejects_symlinked_helpers(self):
        vim.import_runtime(self.source, "fixture", self.assets)
        helper = self.assets / "runtime" / "syntax" / "shared" / "helper.vim"
        helper.unlink()
        try:
            helper.symlink_to(self.source / "syntax" / "shared" / "helper.vim")
        except OSError as error:
            self.skipTest(f"Symlinks unavailable: {error}")
        with self.assertRaisesRegex(ValueError, "symlinks are unsupported"):
            vim.verify(self.assets)


if __name__ == "__main__":
    unittest.main()
