# Alumina Interface

Alumina Interface is the greenfield browser/WASM authority for Alumina CAD/CAM,
machine configuration, job compilation, control, and diagnostics. The current
checkpoint establishes the exact geometry and protocol foundations; it does not
preserve the routes, graph files, renderer, or device model of the earlier
prototype.

## Current baseline

- `crates/alumina-interface-core` is window-free and owns exact design/CAM
  values, bounded unit-bearing measurements, canonical integer machine values,
  explicit one-way display projections, and the checked Hypercurve-to-Hyperpath
  metric boundary.
- `crates/alumina-interface-client` is a headless native protocol client with a
  deterministic in-memory simulator transport. Browser Wi-Fi transport will use
  the same canonical frame validation.
- The native/browser shell composes `ExactScene` and `ExactCamera` values and
  uploads them through Hypergraphics. It contains no application-owned vertex,
  normal, grid, camera-matrix, or primitive-float geometry pipeline.
- CSGRS and every Hyper dependency resolve to sibling repositories in the
  shared workspace. There is no crates.io CSGRS fallback.
- CSGRS builds native `TriangleMesh` geometry. Hypergraphics performs checked
  mesh expansion and certified Hypercurve chord subdivision into exact scene
  vertices, and owns the only f64/f32 GPU boundary.
- The baseline renders an exact line/arc/cubic source path with retained chord
  evidence. Exact lines and circular arcs promote losslessly to Hyperpath; a
  general cubic fails with a typed metric blocker instead of borrowing its
  display chords. The retained line/semicircle fixture certifies the symbolic
  path length `4 + 2*pi` through Hyperpath and Hypersolve.
- A separate motion-specific compiler certifies source chords, rounds every
  coordinate and cumulative time with Hyperreal's certified integer boundary,
  replays half-lattice/half-tick bounds through Hyperlimit, and emits the real
  `alumina-machine-ir::ExecutionSegment` type. The deterministic fixture uses
  80 steps/mm, 1 MHz ticks, 10 mm/s, and a `1/1024 mm` source-chord budget.
- Canonical segments are deterministically partitioned using the firmware's
  queried record capacity and caller-owned horizon limits. Every chained
  512-byte block is independently replayed before `alumina-storage` creates the
  real resumable upload, chunk-manifest, publication, and later boot-local
  `alumina-job::JobDescriptor` bytes.

The selected local revisions and any uncommitted source state are recorded in
[`docs/HYPER-BASELINE.md`](docs/HYPER-BASELINE.md). A dirty local source tree is
valid for development but cannot qualify a reproducible compiler release.
The current curve and metric contract is in
[`docs/EXACT-TOOLPATH.md`](docs/EXACT-TOOLPATH.md).
The immutable block/cache boundary is in
[`docs/CACHED-PARTITIONS.md`](docs/CACHED-PARTITIONS.md).
The latest verified development evidence is in
[`docs/CHECKPOINT-EXACT-CAM.md`](docs/CHECKPOINT-EXACT-CAM.md).

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

The next interface milestones add certified filled-region meshing, supported
general-curve metric compilation, capability-derived machine/error policy,
global multi-MCU manifests, browser SD upload/reconciliation, direct Wi-Fi
coordination, annotated board photography, bounded telemetry,
oscilloscope/logic-analyzer views, and the typed timed LabVIEW-style graph. Raw
G-code remains an optional exact UI importer, never firmware or canonical job
input.

This repository is MIT licensed. Dependencies are restricted to permissive
licenses accepted by the Alumina project; GPL-family code is excluded.
