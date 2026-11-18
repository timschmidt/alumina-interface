#!/usr/bin/env python3
"""Check the package identities Cargo resolved against the checked-out workspaces."""

import argparse
import json
from pathlib import Path
import subprocess
import sys
import tomllib


ROOT = Path(__file__).resolve().parents[1]
TARGETS = ("x86_64-unknown-linux-gnu", "wasm32-unknown-unknown")
# These packages must still participate in the browser graph. Other members
# (including newly adopted Hyper crates) are checked whenever they resolve.
REQUIRED_BROWSER = {
    "csgrs", "hypercurve", "hypergraphics", "hyperinterval", "hyperlattice",
    "hyperlimit", "hypermesh", "hyperpath", "hyperphysics", "hyperreal",
    "hypersolve", "hypertri", "alumina-board", "alumina-capability",
    "alumina-clock", "alumina-config", "alumina-foc", "alumina-graph-ir",
    "alumina-job", "alumina-machine-ir", "alumina-motion", "alumina-net",
    "alumina-protocol", "alumina-runtime", "alumina-safety", "alumina-service",
    "alumina-shift-register", "alumina-sim", "alumina-storage",
}


def metadata(root, *options):
    return json.loads(subprocess.check_output([
        "cargo", "metadata", "--format-version", "1", "--locked", "--offline",
        "--manifest-path", str(root / "Cargo.toml"), *options,
    ], cwd=root))


def workspace_packages(roots):
    packages = {}
    for root in roots:
        graph = metadata(root, "--no-deps")
        members = set(graph["workspace_members"])
        for pkg in graph["packages"]:
            if pkg["id"] not in members:
                continue
            name = pkg["name"]
            path = Path(pkg["manifest_path"]).resolve()
            if not path.is_relative_to(root.resolve()):
                raise ValueError(f"{name} workspace member lies outside {root}: {path}")
            if name in packages:
                raise ValueError(f"ambiguous owner for {name}: {packages[name][0]} and {path}")
            packages[name] = (path, pkg["version"])
    return packages


def local_patches(root, owners):
    owned_paths = {path for path, _ in owners.values()}
    with (root / "Cargo.toml").open("rb") as manifest:
        patches = tomllib.load(manifest).get("patch", {}).get("crates-io", {})
    packages = {}
    for entry in patches.values():
        if "path" not in entry:
            continue
        path = (root / entry["path"] / "Cargo.toml").resolve()
        if path in owned_paths:
            continue
        if not path.is_relative_to(root.resolve()):
            raise ValueError(f"vendored patch outside {root}: {path}")
        with path.open("rb") as manifest:
            package = tomllib.load(manifest)["package"]
        packages[package["name"]] = (path, package["version"])
    return packages


def check_graph(graph, owners, required):
    seen = set()
    for pkg in graph["packages"]:
        name = pkg["name"]
        if name not in owners:
            if pkg["source"] is None:
                raise ValueError(f"unrecognized local package {name}: {pkg['manifest_path']}")
            continue
        expected_path, expected_version = owners[name]
        actual_path = Path(pkg["manifest_path"]).resolve()
        if (pkg["source"] is not None or actual_path != expected_path
                or pkg["version"] != expected_version):
            raise ValueError(
                f"{name} resolved to {actual_path} v{pkg['version']} "
                f"({pkg['source'] or 'path'}), expected {expected_path} v{expected_version}"
            )
        seen.add(name)
    if not required:
        return len(seen)
    # A disconnected workspace member cannot satisfy a consumer requirement.
    roots = set(graph["workspace_default_members"])
    edges = {node["id"]: node["deps"] for node in graph["resolve"]["nodes"]}
    reachable = set()
    pending = list(roots)
    while pending:
        package = pending.pop()
        if package in reachable:
            continue
        reachable.add(package)
        for dep in edges[package]:
            if package in roots or any(k["kind"] != "dev" for k in dep["dep_kinds"]):
                pending.append(dep["pkg"])
    reachable_names = {p["name"] for p in graph["packages"] if p["id"] in reachable}
    missing = required - (seen & reachable_names)
    if missing:
        raise ValueError("missing required workspace packages: " + ", ".join(sorted(missing)))
    return len(seen)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--coordinated", action="store_true",
                        help="also check the firmware, fuzz and standalone interval clients")
    parser.add_argument("--fetch", action="store_true",
                        help="populate the locked target dependencies before the offline audit")
    args = parser.parse_args()
    csgrs = ROOT.parent / "csgrs"
    firmware = ROOT.parent / "alumina-firmware"
    roots = [ROOT, csgrs, firmware]
    native, wasm = TARGETS
    checks = [(ROOT, native, set()), (ROOT, wasm, REQUIRED_BROWSER)]
    if args.coordinated:
        foc = csgrs / "crates/hyperlattice-interval/conformance/alumina-foc"
        measurements = csgrs / "crates/hyperinterval/conformance/alumina-measurements"
        foc_packages = {"alumina-foc", "hyperinterval", "hyperlattice-interval"}
        measurement_packages = {"alumina-interface-core", "hyperinterval", "hyperreal"}
        checks.extend([
            (firmware, native, {"alumina-foc", "hyperinterval"}),
            (ROOT / "fuzz", native, {"alumina-interface-core", "hyperinterval"}),
            (foc, native, foc_packages),
            (foc, "thumbv7em-none-eabihf", foc_packages),
            (foc / "wasm", wasm, foc_packages | {"interval-foc-conformance"}),
            (measurements, native, measurement_packages),
            (measurements / "wasm", wasm, measurement_packages),
        ])
        roots.extend([ROOT / "fuzz", foc, foc / "wasm", measurements, measurements / "wasm"])
    owners = workspace_packages(roots)
    for root, target, required in checks:
        if args.fetch:
            subprocess.run(["cargo", "fetch", "--locked", "--manifest-path", str(root / "Cargo.toml"),
                            "--target", target], cwd=root, check=True)
        graph = metadata(root, "--filter-platform", target)
        try:
            # Cargo applies patches only from the resolving workspace root.
            patches = local_patches(root, owners)
            if patches.keys() & owners.keys():
                raise ValueError("a registry patch shadows an owned workspace member")
            count = check_graph(graph, owners | patches, required)
        except ValueError as error:
            raise ValueError(f"{root} ({target}): {error}") from error
        print(f"{root.relative_to(ROOT.parent)} ({target}): "
              f"verified {count} local package paths and versions")


if __name__ == "__main__":
    try:
        main()
    except (ValueError, OSError, subprocess.CalledProcessError) as error:
        sys.exit(str(error))
