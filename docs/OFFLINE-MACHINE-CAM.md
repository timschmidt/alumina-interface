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
6. an exact dense-axis projection with retained span/axis bottlenecks when the
   complete metric route is affine, or an explicit conservative curved-route
   fallback;
7. a bounded exact pointwise certificate over Hypercurve de Casteljau spans,
   followed by Hyperpath's exact forward/reverse node planner, independent
   Hypersolve replay, stop-separated exact jerk-feasibility refinement, and
   phase selection from the resulting boundary feeds; the default fixture has
   no eligible lossless line-to-line G1 join and therefore retains four
   constant-jerk phases per metric element;
8. exact interpolation under a 131,072-point browser budget, followed by
   configured step/output-lattice lowering and the smallest admitted rational
   timer-dilation factor on the caller's bounded search grid;
9. production `StepperExecutor` electrical and terminal preflight, including
   retained factor-one and immediate-predecessor rejection evidence;
10. chained canonical blocks, independently hashed upload chunks, and the
   immutable SD-cache publication;
11. deterministic `RealtimeJob` plus `CachedStepperExecutor` event replay; and
12. a reconstructed canonical `ALMEVD03` transcript binding source, metric
    path, approximation, exact planner policy/certification, complete lowering,
    timer-search, and canonical execution identities.

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
- either every exact affine span/axis derivative, machine limit, replay result,
  and selected bottleneck or an explicit curved-route fallback statement;
- every effective lookahead node ceiling, forward-pass node, final
  jerk-feasible node, positive component/refinement count, and
  caller/geometric/reachability replay result;
- every source-to-motion span, exact error bound, and subdivision depth;
- every two- or four-phase endpoint state for each retained metric element;
- an exact selected metric point with source/motion provenance beside its
  canonical step/tick coordinate;
- exact timer-dilation factor, replay count, factor-one/predecessor failures,
  cumulative delay, segment extension, and output-grid padding;
- a selected firmware segment and complete executor-preflight terminal facts;
- partition object/manifest/chunk identities and cache horizons;
- event replay step counts, output transaction count, terminal state, and
  finish cycle; and
- evidence, exact-source, exact-metric, source-approximation, planner, and
  lowering SHA-256 identities, with canonical planner/lowering transcript byte
  lengths.

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

- the sibling Hyperpath suite passed 2 unit, 436 integration/property, and 2
  README tests; strict all-target Clippy and rustdoc also passed;
- all 28 application tests passed, including a complete headless egui frame,
  deterministic configuration/source-to-event replay, and transactional
  rejection;
- all 117 exact-core tests plus the compile-fail value-boundary test passed,
  including bounded cubic source reduction, exact diagonal metric length,
  jerk-feasible exact-line G1 motion, curvature-bearing G1/reversal stops,
  native source-envelope travel rejection, post-rounding containment,
  caller-bounded interpolation allocation, and five exact CNC importer cases;
- all 37 protocol-client tests and the exact-control integration test passed;
- native and `wasm32-unknown-unknown` strict Clippy passed for the complete
  workspace, and every workspace test target linked for WASM;
- strict rustdoc and the local-source/license policy audit passed;
- the optimized 5,447,452-byte WASM validated with `wasm-tools`; its
  2,443,730-byte gzip and 1,955,126-byte Brotli forms passed integrity checks,
  and the uncompressed artifact has SHA-256
  `58729ee1661c226fc8b15239a72e5a6b128631bbbce5e4f6f3688a1b429034ff`;
- headless Chromium loaded the bundle and dedicated worker over loopback with
  software WebGL, then visibly rendered the complete default Machine/CAM
  inspector, line/arc/cubic motion plot, source-to-motion certificate, and the
  exact acceleration/jerk-feasible status for 35 nodes, 33 joins, and 34 spans,
  zero eligible positive components in the default line/arc/cubic fixture, and
  the active lossless-line G1-only policy. The same view reported selected
  timer factor `1`, the complete `1/4096` through `65536/4096` policy lattice,
  one complete preflight replay, no factor-one or predecessor rejection, and
  the independent `ALMPLN01`/`ALMLOW01` identities and lengths committed by
  `ALMEVD03`;
  and
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
