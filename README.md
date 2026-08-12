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
- `crates/alumina-interface-client` owns the headless native protocol client,
  deterministic simulator transport, origin-bound HMAC V2 session, strict boot
  challenge decoder, retry-safe content-addressed upload reconciliation, and a
  WASM `fetch` adapter. Its versioned UI/worker contract carries bounded
  commands and redacted clock snapshots; browser and native paths share
  canonical frame and response-proof validation.
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
- Owned per-MCU artifacts are sorted by stable device identity into the real
  canonical global job manifest. Its exact content and participant-set digests
  bind directly to firmware schedule commits, and the same manifest can use an
  independent resumable upload transaction on every MCU.
- Per-participant delivery now binds both exact publications before I/O,
  reconciles the executable partition first, then the identical global
  manifest, and returns to `StorageInspect` after any ambiguous fetch. Browser
  fetch omits ambient credentials, rejects redirects, disables caching, and
  binds request/response proofs to the actual document origin.
- The browser shell creates one dedicated module worker that owns each device's
  HMAC secret, HTTP session, causal clock model, automatic retry cadence, and a
  bounded 64-observation history. The UI can add, probe, and disconnect
  diagnostic sessions and never receives credentials. A headless Chromium
  smoke check reaches the worker-ready state without contacting a device.
- That worker exchanges production-format authenticated heartbeat traffic with
  the deterministic host MCU fixture and recovers from response loss, a finite
  outage, and reboot while conservatively rejecting excessive delay.
- The headless multi-MCU coordinator now retains each authenticated first-output
  observation, rejects later evidence regression, exactly inverts its
  boot-scoped device-cycle interval into conservative browser monotonic bounds,
  and preserves simulator/peripheral/software authority. The repeatable
  two-device flow proves each known simulated edge is contained and exposes
  outer edge-spread and shared-epoch-error bounds in the diagnostic UI.
- The window-free core now owns the first greenfield typed graph document:
  exact unit/type registries, bounded typed literals, explicit clocks and
  HostExact/Service/Realtime domains, opaque versioned nodes, typed ports and
  wires, and a canonical bounded `ALGR` V1 codec. Untrusted loads enforce an
  independent admission policy, rebuild through the validators, require exact
  byte replay, and derive a SHA-256 graph identity. This is structural graph
  authority only; it never becomes an arbitrary firmware graph interpreter.
- A separately bounded node registry resolves opaque kinds only when exact
  port/parameter shapes, allowed domain families, complete current-tick
  feedthrough, and optional read-before-write state are declared against the
  document's exact type/clock context. Iterative port-level analysis accepts
  deliberate delayed feedback and returns exact wire/feedthrough witnesses for
  forbidden combinational cycles; it emits no executable implementation.
- Checked recursive storage analysis proves canonical maximum bytes for every
  literal or runtime payload/sample type and rejects state storage smaller than
  its complete exact value domain. It does not claim a firmware runtime layout.
- Required/optional input contracts distinguish same-owner synchronous slots
  from bounded event/stream queues with explicit full behavior. Exact reports
  include timestamp/sequence envelopes and reject scalar cross-domain sharing,
  stream over-capacity, and per-input/total allocation overflow.
- Cross-clock Stream feedthrough requires an audited latest-at-or-before
  transition. Exact clock resolution proves a shared tick-zero root, the
  smallest rational schedule pattern, minimum input capacity, and separately
  bounded held-sample state; implicit or independent-root transitions reject.
- A separate implementation registry admits nine reviewed `HostExact`
  simulation behaviors: external Stream source, audited latest-at-or-before
  transition, Stream sink, exact add/subtract/scale/clamp, explicit
  read-before-write unit delay, and a fail-safe Boolean permit gate. A visible
  multi-rate discrete PID/interlock fixture composes those primitives without
  hidden controller state. The bounded simulator uses exact rational clock
  time and unit scales, orders every coincident source tick first, and produces
  the same canonical result regardless of caller sample order.
- The native and browser shells construct that same fallible core fixture and
  open a bounded workspace by default: semantic current-tick layers, explicit
  feedback routes, typed ports, exact parameters/state, and four control traces.
  Canonical `ALGW` V1 embeds the unchanged `ALGR` plus integer canvas positions
  and monotonic ID cursors. Its 11-entry fixed-schema palette supports
  monotonic node creation, atomic node/incident-wire deletion, node moves,
  typed wire edits, and bounded exact scalar parameter editing. Every edit is
  transactional; any graph edit detaches the graph-bound reference trace.
  Plot coordinates come only from certified `f64` enclosures, and the cursor
  retains the exact rational sample. This is an in-memory editing foundation,
  not deployment or firmware authority. See
  [`docs/GRAPH-WORKSPACE-V1.md`](docs/GRAPH-WORKSPACE-V1.md).
- Canonical `ALGT` V1 traces bind the graph digest, semantic/implementation
  registry digest, and inclusive root-clock horizon. Replay decodes only the
  external authority, reruns the fixed simulator, and requires every regenerated
  byte to match; it grants no firmware or deployment authority.
- A separate deployment registry lowers one fixed Boolean Stream subset into
  the sibling firmware's canonical 4 KiB `ALGRIR02` package. Production limits
  are derived from the complete authenticated target capability document, not
  guessed defaults. The compiler binds its identity, exact split arenas,
  opcode/resource palettes, audited semantics, fixed implementations/WCET,
  graph, target MCU, and configuration; proves integer device-cycle periods,
  executor reserve, topology, and exact arena use; then requires the
  allocation-free firmware decoder to replay the package.
- A native cross-repository fixture sends those exact compiler bytes directly
  into the sibling portable firmware runtime. Exact identity/capacity admission,
  safety-gated Service tick-zero priming, unique Service/Realtime owners, and
  1 kHz→500 Hz queue/latest/sink execution reproduce the expected Boolean
  samples without a graph-document interpreter or physical side effect.
- A second cross-repository fixture lowers a typed TinyBee GPIO33 resource
  handle into the first physical opcode and runs it through the firmware actor
  types. GPIO34 and a mismatched target capability digest fail before package
  authority; runtime admission rechecks the same exact opcode/class/access/
  selector palette. This is host functional evidence, not physical input HIL.
- The headless and WASM clients publish that fixed package, reconcile independent
  dual-core installation, and drive exact future start/stop epochs. Running is
  reported only after both permanent actors and the shared bridge agree; the
  first execution fault is retained while an exact stop is reconciled.

The selected local revisions and any uncommitted source state are recorded in
[`docs/HYPER-BASELINE.md`](docs/HYPER-BASELINE.md). A dirty local source tree is
valid for development but cannot qualify a reproducible compiler release.
The current curve and metric contract is in
[`docs/EXACT-TOOLPATH.md`](docs/EXACT-TOOLPATH.md).
The immutable block/cache boundary is in
[`docs/CACHED-PARTITIONS.md`](docs/CACHED-PARTITIONS.md).
The global participant/manifest boundary is in
[`docs/GLOBAL-JOB-MANIFEST.md`](docs/GLOBAL-JOB-MANIFEST.md).
The authenticated browser/cache boundary is in
[`docs/WIFI-CACHE-DELIVERY.md`](docs/WIFI-CACHE-DELIVERY.md).
The dedicated control-worker boundary is in
[`docs/LIVE-CONTROL-WORKER.md`](docs/LIVE-CONTROL-WORKER.md).
The first typed graph and canonical replay boundary is in
[`docs/TYPED-GRAPH-V1.md`](docs/TYPED-GRAPH-V1.md).
The exact-CAM development evidence is in
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

The next interface milestones add supported general-curve metric compilation,
capability-derived machine/error policy, device identity/capability discovery,
physical-browser/radio qualification, full worker-owned cached-job driving,
annotated board photography, bounded telemetry, oscilloscope/logic-analyzer
views, groups/components/front panels, composite/identity-bearing parameter
editing, label/domain editing, workspace persistence and history, broader
deterministic host graph behaviors, and fixed-memory authenticated
Service/Realtime upload/core transfer and task
composition, additional resource opcodes and capability-generated graph nodes,
and physical input/timing qualification. Raw G-code remains an optional exact
UI importer, never firmware or canonical job input.

This repository is MIT licensed. Dependencies are restricted to permissive
licenses accepted by the Alumina project; GPL-family code is excluded.
