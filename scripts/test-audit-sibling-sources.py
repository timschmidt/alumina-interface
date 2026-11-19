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
            self.assertEqual(audit.check_target(consumer, audit.TARGETS[1], owners, required), 2)
            manifest = consumer / "Cargo.toml"
            manifest.write_text(manifest.read_text().replace("../csgrs/", "../other/csgrs/"))
            with self.assertRaisesRegex(ValueError, "hyperinterval resolved to"):
                audit.check_target(consumer, audit.TARGETS[1], owners, required)

            # The path-only substitution was lock-compatible. A version change
            # requires a lock update, which the audit must refuse to perform.
            lock = consumer / "Cargo.lock"
            original = lock.read_bytes()
            manifest = alternate / "Cargo.toml"
            manifest.write_text(manifest.read_text().replace("1.0.0", "1.0.1"))
            with self.assertRaises(subprocess.CalledProcessError):
                audit.metadata(consumer)
            self.assertEqual(lock.read_bytes(), original)

    def test_required_consumer_dependencies(self):
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory)
            library, helper, consumer = (base / name for name in ("hyperinterval", "helper", "app"))
            package(library, "hyperinterval")
            package(helper, "helper", 'hyperinterval = { path = "../hyperinterval", optional = true }')
            package(consumer, "app")
            manifest = consumer / "Cargo.toml"
            prefix = manifest.read_text()
            renamed = 'interval = { package = "hyperinterval", path = "../hyperinterval" }'
            forward = 'helper = { path = "../helper", features = ["hyperinterval"] }'
            owners = audit.workspace_packages([consumer, library, helper])
            native, wasm = audit.TARGETS
            wasm_dependency = """[target.'cfg(target_arch = "wasm32")'.dependencies]\n""" + renamed
            feature_dependency = ('[features]\nfuzzing = ["helper/hyperinterval"]\n'
                                  '[dependencies.helper]\npath = "../helper"')
            cases = [
                (renamed, wasm, (), True),
                ("[dev-dependencies]\n" + renamed, wasm, (), False),
                ("[build-dependencies]\n" + renamed, wasm, (), False),
                (forward, wasm, (), True),
                ('helper = { path = "../helper" }\n[dev-dependencies]\n' + forward, wasm, (), False),
                (wasm_dependency, native, (), False),
                (wasm_dependency, wasm, (), True),
                (feature_dependency, native, (), False),
                (feature_dependency, native, ("fuzzing",), True),
            ]
            for deps, target, features, required_present in cases:
                with self.subTest(deps=deps, target=target):
                    manifest.write_text(prefix + deps + "\n")
                    subprocess.run(["cargo", "generate-lockfile", "--offline"], cwd=consumer, check=True)
                    if required_present:
                        audit.check_target(consumer, target, owners, {"hyperinterval"}, features)
                    else:
                        with self.assertRaisesRegex(ValueError, "missing required workspace packages: hyperinterval"):
                            audit.check_target(consumer, target, owners, {"hyperinterval"}, features)

    def test_proc_macro_and_disconnected_member_are_not_target_dependencies(self):
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory)
            app, library, derive = (base / name for name in ("app", "hyperinterval", "derive-helper"))
            package(app, "app", 'derive-helper = { path = "../derive-helper" }')
            package(library, "hyperinterval")
            package(derive, "derive-helper", 'hyperinterval = { path = "../hyperinterval" }')
            isolated = base / "isolated"
            package(isolated, "isolated")
            with (derive / "Cargo.toml").open("a") as manifest:
                manifest.write("\n[lib]\nproc-macro = true\n")
            # Include the library as a workspace member so its presence in
            # resolved metadata cannot by itself make it a target dependency.
            for root in [app, library, derive, isolated]:
                manifest = root / "Cargo.toml"
                manifest.write_text(manifest.read_text().replace("[workspace]\n", ""))
            (base / "Cargo.toml").write_text(
                '[workspace]\nresolver = "2"\nmembers = ["app", "hyperinterval", "derive-helper", "isolated"]\n'
                'default-members = ["app"]\n'
            )
            subprocess.run(["cargo", "generate-lockfile", "--offline"], cwd=base, check=True)
            owners = audit.workspace_packages([base])
            for name in ["derive-helper", "hyperinterval", "isolated"]:
                with self.subTest(name=name), self.assertRaisesRegex(ValueError, "missing required workspace packages"):
                    audit.check_target(base, audit.TARGETS[1], owners, {name})

    def test_every_resolved_copy_and_required_name(self):
        manifest = Path("/workspace/csgrs/crates/hyperinterval/Cargo.toml").resolve()
        owners = {"hyperinterval": (manifest, "1.0.0")}
        correct = {"name": "hyperinterval", "source": None, "manifest_path": str(manifest),
                   "version": "1.0.0"}
        for change in [{"source": "registry+example"}, {"version": "0.9.0"}]:
            with self.subTest(change=change), self.assertRaisesRegex(ValueError, "resolved to"):
                audit.check_graph({"packages": [correct, correct | change]}, owners, set(), set())
        with self.assertRaisesRegex(ValueError, "unrecognized local package"):
            audit.check_graph({"packages": [correct | {"name": "unowned"}]}, owners, set(), set())
        registry_only = {"packages": [correct | {"source": "registry+example"}]}
        with self.assertRaisesRegex(ValueError, "missing required workspace packages"):
            audit.check_graph(registry_only, {}, {"hyperinterval"}, {"hyperinterval"})


if __name__ == "__main__":
    unittest.main()
