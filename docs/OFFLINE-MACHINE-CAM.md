# Offline machine/CAM inspector checkpoint

Snapshot: 2026-08-13. This checkpoint advances work that does not require
changing the workstation's Wi-Fi association. The connected bare MKS TinyBee
V1.0 was not contacted, reset, flashed, or driven.

## Visible authority chain

The application opens the Machine/CAM workspace by default. It owns no second
machine model: its input is a canonical firmware-schema `ALMCFG05` document
validated against the primary 8 MiB `mks-tinybee-v1` package. From those exact
bytes it reconstructs:

1. the capability/configuration identity and exact two-axis resource profile;
2. conservative command-density, electrical-rate, travel, dynamics,
   calibration, following-error, and timer facts;
3. a complete machine-resolution budget;
4. the retained exact line/native-semicircle/cubic Hypercurve path;
5. a native-extrema travel-envelope certificate;
6. a bounded exact pointwise certificate over Hypercurve de Casteljau spans, followed by
   Hyperpath/Hypersolve exact-stop lookahead and four constant-jerk phases per
   metric element;
7. exact interpolation under a 131,072-point browser budget, followed by
   configured step/tick lattice lowering;
8. production `StepperExecutor` electrical and terminal preflight;
9. chained canonical blocks, independently hashed upload chunks, and the
   immutable SD-cache publication;
10. deterministic `RealtimeJob` plus `CachedStepperExecutor` event replay; and
11. a reconstructed canonical `ALMEVD02` transcript binding source, metric
    path, and approximation identities.

The default fixture declares its facts as declared—not measured—and includes
the cached-autonomous policy bit. The board package remains non-armable because
its physical visual, polarity, and timing HIL gates are still open.

Travel is checked twice: the complete native source envelope must fit before
scheduling, and every rounded integer command must still fit after division by
the exact command density. This separates between-sample curve extrema from an
outward step-lattice rounding and closes both paths independently.

## Presentation boundary

Exact values remain visible as exact text. The path plot receives only a
one-way finite projection through the core's named display boundary and then a
second explicit conversion into egui's `f32` coordinate domain. The plot is a
diagnostic view of exact scheduled samples; it is never an input to geometry,
scheduling, or machine IR.

The inspector exposes:

- exact nominal/lower/upper axis facts and resource bindings;
- source and usable travel envelopes;
- every component of the machine-wide error budget;
- exact aggregate length, time, feed, acceleration, and jerk limits;
- every source-to-motion span, exact error bound, and subdivision depth;
- all four phase endpoint states for each retained metric element;
- an exact selected metric point with source/motion provenance beside its
  canonical step/tick coordinate;
- a selected firmware segment and complete executor-preflight terminal facts;
- partition object/manifest/chunk identities and cache horizons;
- event replay step counts, output transaction count, terminal state, and
  finish cycle; and
- evidence, exact-source, exact-metric, and source-approximation SHA-256
  identities.

## Transactional file and source exchange

The prior `ALGW`-specific platform bridge is now a generic bounded byte bridge
while parsing authority remains in each owning workspace. Native paths and
browser file selection/download support `.algw`, `.almcfg`, `.almevd`, and
UI-only `.nc` source without treating an extension as evidence or source text as
canonical.

An imported configuration replaces visible state only after complete board
validation, exact derivation, travel certification, scheduling, lowering,
executor preflight, packaging, event replay, evidence construction, and
evidence replay all succeed. A corrupt or semantically inadmissible candidate
leaves the current machine state unchanged. Imported evidence is SHA-256
checked against the current evidence identity and must equal the transcript
freshly reconstructed from the current program and partition.

The optional CNC importer retains exact raw-source SHA-256 and per-curve modal
provenance separately from canonical exact-geometry identity. It accepts only a
bounded explicit line/IJ-arc geometry subset and replaces source state only
after the same travel, schedule, lowering, cache, simulator, and evidence chain
succeeds. See `EXACT-CNC-GEOMETRY-IMPORT.md`.

Lowering checks the caller-owned point budget before each phase and uses
fallible reservations for points and canonical segments. A semantically valid
configuration that would demand pathological interpolation therefore fails
transactionally before large browser allocation, rather than relying on the
later 4 MiB cache admission limit.

## Verification

At checkpoint implementation time:

- all 28 application tests passed, including a complete headless egui frame,
  deterministic configuration/source-to-event replay, and transactional
  rejection;
- all 112 exact-core tests plus the compile-fail value-boundary test passed,
  including bounded cubic source reduction, exact diagonal metric length,
  native source-envelope travel rejection, post-rounding containment,
  caller-bounded interpolation allocation, and five exact CNC importer cases;
- all 37 protocol-client tests and the exact-control integration test passed;
- native and `wasm32-unknown-unknown` strict Clippy passed for the complete
  workspace, and every workspace test target linked for WASM;
- strict rustdoc and the local-source/license policy audit passed;
- the optimized 5,356,204-byte WASM validated with `wasm-tools`; its
  2,413,446-byte gzip and 1,934,812-byte Brotli forms both decompress to SHA-256
  `a2ed14cab2473b175722a670970c3af4975f40842c9a08b6c04090364c710a7f`;
- headless Chromium loaded the bundle and dedicated worker over loopback with
  software WebGL, then visibly rendered the complete default Machine/CAM
  inspector, line/arc/cubic motion plot, source-to-motion certificate, and
  exact-stop schedule; and
- no WLAN or physical board operation occurred.

This is development evidence against the current sibling working trees, not a
physical qualification or reproducible release pin. The exact sibling revision
and dirty-tree digests are recorded in `HYPER-BASELINE.md`.

## Deferred physical work

The browser worker, authenticated Wi-Fi protocols, loopback fixtures, and AP
firmware remain in the repository, but this checkpoint does not associate the
workstation with the Alumina AP. Physical AP/HTTP and SLogic validation resume
only when a separate Internet path is available. The board photograph/hotspot
gate likewise remains open until an operator-owned orthographic image of the
actual fixture is added under a compatible license and reconciled to hardware.
