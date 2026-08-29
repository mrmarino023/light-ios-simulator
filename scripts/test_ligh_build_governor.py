#!/usr/bin/env python3
"""Unit tests for BuildGovernor — no Xcode required."""

from __future__ import annotations

import os
import tempfile
import unittest

from ligh_build_governor import (
    cache_key,
    classify_exit,
    run_governed,
    source_stamp,
)


class TestBuildGovernor(unittest.TestCase):
    def test_classify_oom(self) -> None:
        self.assertEqual(classify_exit(-9, ""), "infra_oom")
        self.assertEqual(classify_exit(137, ""), "infra_oom")
        self.assertEqual(classify_exit(1, "error: build failed"), "build_failed")

    def test_source_stamp_stable(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            p = os.path.join(td, "A.swift")
            open(p, "w").write("let x = 1\n")
            a = source_stamp([td])
            b = source_stamp([td])
            self.assertEqual(a, b)
            open(p, "w").write("let x = 2\n")
            c = source_stamp([td])
            self.assertNotEqual(a, c)

    def test_cache_key_changes_with_stamp(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            open(os.path.join(td, "A.swift"), "w").write("a\n")
            k1 = cache_key(["echo"], cwd=td, stamp_roots=[td])
            open(os.path.join(td, "A.swift"), "w").write("b\n")
            k2 = cache_key(["echo"], cwd=td, stamp_roots=[td])
            self.assertNotEqual(k1, k2)

    def test_serialize_and_cache_hit(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            lock = os.path.join(td, "build.lock")
            cache = os.path.join(td, "cache")
            art = os.path.join(td, "Demo.app")
            stamp = os.path.join(td, "src")
            os.makedirs(stamp)
            open(os.path.join(stamp, "App.swift"), "w").write("struct App {}\n")
            # Fake build: create the .app bundle
            script = os.path.join(td, "build.sh")
            open(script, "w").write(
                "#!/bin/bash\nmkdir -p \"$1\"\necho ok > \"$1/Marker\"\n"
            )
            os.chmod(script, 0o755)

            r1 = run_governed(
                [script, art],
                cwd=td,
                stamp_roots=[stamp],
                artifact=art,
                cache_dir=cache,
                lock_path=lock,
                min_free_mb=0,  # skip pressure in CI/dev
                pressure_wait_s=1,
                label="unit",
            )
            self.assertTrue(r1.ok, r1)
            self.assertFalse(r1.cache_hit)
            self.assertTrue(os.path.isfile(os.path.join(art, "Marker")))

            # Remove artifact; next run should restore from cache without re-exec needing content
            import shutil

            shutil.rmtree(art)
            r2 = run_governed(
                [script, art],
                cwd=td,
                stamp_roots=[stamp],
                artifact=art,
                cache_dir=cache,
                lock_path=lock,
                min_free_mb=0,
                pressure_wait_s=1,
                label="unit",
            )
            self.assertTrue(r2.ok, r2)
            self.assertTrue(r2.cache_hit)
            self.assertEqual(r2.fault, "cache_hit")
            self.assertTrue(os.path.isfile(os.path.join(art, "Marker")))

    def test_build_failure(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            r = run_governed(
                ["false"],
                cwd=td,
                lock_path=os.path.join(td, "lock"),
                cache_dir=os.path.join(td, "c"),
                use_cache=False,
                min_free_mb=0,
                pressure_wait_s=1,
            )
            self.assertFalse(r.ok)
            self.assertEqual(r.fault, "build_failed")


if __name__ == "__main__":
    unittest.main()
