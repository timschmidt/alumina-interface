# Alumina Interface

Alumina Interface is the greenfield browser/WASM authority for Alumina CAD/CAM,
machine configuration, job compilation, control, and diagnostics. The current
checkpoint establishes the exact geometry and protocol foundations; it does not
preserve the routes, graph files, renderer, or device model of the earlier
prototype.

## Current baseline

- `crates/alumina-interface-core` is window-free and owns exact design/CAM
  values, bounded unit-bearing measurements, canonical integer machine values,
  and explicit one-way display projections.
- `crates/alumina-interface-client` is a headless native protocol client with a
  deterministic in-memory simulator transport. Browser Wi-Fi transport will use
  the same canonical frame validation.
- The native/browser shell composes `ExactScene` and `ExactCamera` values and
  uploads them through Hypergraphics. It contains no application-owned vertex,
  normal, grid, camera-matrix, or primitive-float geometry pipeline.
- CSGRS and every Hyper dependency resolve to sibling repositories in the
  shared workspace. There is no crates.io CSGRS fallback.
- CSGRS builds native `TriangleMesh` geometry. Hypergraphics performs the
  checked expansion into exact scene vertices and owns the only f64/f32 GPU
  boundary.

The selected local revisions and any uncommitted source state are recorded in
[`docs/HYPER-BASELINE.md`](docs/HYPER-BASELINE.md). A dirty local source tree is
valid for development but cannot qualify a reproducible compiler release.

## Value domains

The core intentionally keeps four domains structurally separate:

1. exact `hyperreal::Real` CAD/CAM values with compile-time units;
2. bounded measured values expressed as exact rational closed intervals;
3. canonical firmware values expressed as integer counts/ticks and
   `alumina-machine-ir` records; and
4. finite lossy display values produced only by named projection functions.

There is no conversion from a renderer value into an exact or canonical value.
A compile-fail documentation test enforces that boundary. The complete policy
is in [`docs/VALUE-BOUNDARIES.md`](docs/VALUE-BOUNDARIES.md).

## Build and test

The normal verification path is offline once dependencies are present:

```sh
cargo test --workspace --offline
cargo clippy --workspace --all-targets --no-deps --offline -- -D warnings
cargo check --workspace --target wasm32-unknown-unknown --offline
scripts/audit-source-policy.sh
env -u NO_COLOR trunk build --release --locked --offline
```

Run the desktop shell with:

```sh
cargo run --offline
```

Run a browser development server with:

```sh
trunk serve --release --offline
```

The production bundle is written to ignored `dist/` and includes compressed
assets suitable for later embedding in `aluminafw`.

## Scope after this checkpoint

The next interface milestones add current Hypercurve/Hyperpath/Hypersolve CAM,
capability-generated board configuration, immutable SD job workflows, direct
Wi-Fi multi-MCU coordination, annotated board photography, bounded telemetry,
oscilloscope/logic-analyzer views, and the typed timed LabVIEW-style graph. Raw
G-code remains an optional exact UI importer, never firmware or canonical job
input.

This repository is MIT licensed. Dependencies are restricted to permissive
licenses accepted by the Alumina project; GPL-family code is excluded.
