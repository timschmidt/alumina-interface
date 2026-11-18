#!/usr/bin/env python3
"""Regression tests for the locked sibling-source audit."""

import importlib.util
from pathlib import Path
import subprocess
import tempfile
import unittest


script = Path(__file__).with_name("audit-sibling-sources.py")
spec = importlib.util.spec_from_file_location("sibling_audit", script)
audit = importlib.util.module_from_spec(spec)
spec.loader.exec_module(audit)


def package(root, name, dependency=""):
    root.mkdir(parents=True)
    (root / "src").mkdir()
    (root / "src/lib.rs").touch()
    (root / "Cargo.toml").write_text(
        f'[package]\nname = "{name}"\nversion = "1.0.0"\nedition = "2021"\n'
        f'[workspace]\n[dependencies]\n{dependency}'
    )


class SiblingSourceTests(unittest.TestCase):
    def test_same_version_from_another_checkout(self):
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory)
            intended = base / "csgrs/crates/hyperinterval"
            alternate = base / "other/csgrs/crates/hyperinterval"
            consumer = base / "consumer"
            package(intended, "hyperinterval")
            package(alternate, "hyperinterval")
            package(consumer, "consumer", 'hyperinterval = { path = "../csgrs/crates/hyperinterval" }')
            subprocess.run(["cargo", "generate-lockfile", "--offline"], cwd=consumer, check=True)

            owners = audit.workspace_packages([consumer, intended])
            required = {"hyperinterval", "consumer"}
            self.assertEqual(audit.check_graph(audit.metadata(consumer), owners, required), 2)
            manifest = consumer / "Cargo.toml"
            manifest.write_text(manifest.read_text().replace("../csgrs/", "../other/csgrs/"))
            with self.assertRaisesRegex(ValueError, "hyperinterval resolved to"):
                audit.check_graph(audit.metadata(consumer), owners, required)

            # The path-only substitution was lock-compatible. A version change
            # requires a lock update, which the audit must refuse to perform.
            lock = consumer / "Cargo.lock"
            original = lock.read_bytes()
            manifest = alternate / "Cargo.toml"
            manifest.write_text(manifest.read_text().replace("1.0.0", "1.0.1"))
            with self.assertRaises(subprocess.CalledProcessError):
                audit.metadata(consumer)
            self.assertEqual(lock.read_bytes(), original)

    def test_every_resolved_copy_and_required_name(self):
        manifest = Path("/workspace/csgrs/crates/hyperinterval/Cargo.toml").resolve()
        owners = {"hyperinterval": (manifest, "1.0.0")}
        correct = {"name": "hyperinterval", "source": None, "manifest_path": str(manifest),
                   "version": "1.0.0"}
        for change in [{"source": "registry+example"}, {"version": "0.9.0"}]:
            with self.subTest(change=change), self.assertRaisesRegex(ValueError, "resolved to"):
                audit.check_graph({"packages": [correct, correct | change]}, owners, set())
        with self.assertRaisesRegex(ValueError, "unrecognized local package"):
            audit.check_graph({"packages": [correct | {"name": "unowned"}]}, owners, set())
        disconnected = {
            "packages": [correct | {"id": "isolated"}],
            "workspace_default_members": ["application"],
            "resolve": {"nodes": [{"id": "application", "deps": []},
                                  {"id": "isolated", "deps": []}]},
        }
        with self.assertRaisesRegex(ValueError, "missing required workspace packages"):
            audit.check_graph(disconnected, owners, {"hyperinterval"})
        registry_only = disconnected | {
            "packages": [correct | {"id": "application", "source": "registry+example"}],
        }
        with self.assertRaisesRegex(ValueError, "missing required workspace packages"):
            audit.check_graph(registry_only, {}, {"hyperinterval"})


if __name__ == "__main__":
    unittest.main()
