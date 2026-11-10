#!/usr/bin/env python3
"""Protect the local source/lockfile boundary after the Client import."""
import importlib.util
from pathlib import Path
import shutil
import tempfile
import unittest

spec = importlib.util.spec_from_file_location('source_pins', Path(__file__).with_name('check-owner-source-pins.py'))
pins = importlib.util.module_from_spec(spec)
spec.loader.exec_module(pins)

class SourcePins(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        names = pins.MANIFESTS + ['Cargo.lock', 'crates/owner-dashboard/Cargo.lock', 'crates/hagency-client/Cargo.lock']
        names += [str(p.relative_to(pins.ROOT)) for p in (pins.ROOT / pins.LIBRARY / 'native').glob('hagency*/Cargo.toml')]
        for name in names:
            target = self.root / name
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(pins.ROOT / name, target)

    def test_local_libraries_share_all_three_locks(self):
        self.assertEqual(pins.check(self.root)['lockfiles'], 3)

    def test_sibling_checkout_is_refused(self):
        p = self.root / 'Cargo.toml'
        p.write_text(p.read_text().replace('crates/hagency-client/native/hagency"', '../hagency-client/native/hagency"'))
        with self.assertRaisesRegex(SystemExit, 'in-tree library'):
            pins.check(self.root)

    def test_git_copy_in_dashboard_lock_is_refused(self):
        p = self.root / 'crates/owner-dashboard/Cargo.lock'
        p.write_text(p.read_text().replace('name = "hagency-metering"', 'name = "hagency-metering"\nsource = "git+https://example.test/client"'))
        with self.assertRaisesRegex(SystemExit, 'resolve locally'):
            pins.check(self.root)

    def test_old_browser_build_is_refused(self):
        (self.root / pins.LIBRARY / 'native/hagency/build.rs').touch()
        with self.assertRaisesRegex(SystemExit, 'retired standalone UI'):
            pins.check(self.root)

if __name__ == '__main__':
    unittest.main()
