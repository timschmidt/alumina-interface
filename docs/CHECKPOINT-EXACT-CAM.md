# Exact CAM development checkpoint

Snapshot: 2026-08-11. This is development evidence against the current sibling
working trees, not a reproducible release qualification.

## Implemented evidence chain

- Hypergraphics `d0952a5f2623a4141bdf0dd89114fea93685a1fd` adds certified exact
  Hypercurve curve/path line adapters.
- Hypergraphics `31811aeb17bd2dc827db5669558f6251e0c2f2aa` adds role-preserving
  certified curved-region boundary adapters.
- `ExactScene` retains the current local CSGRS mesh, an exact line/arc/cubic
  source path, a curved material region with a hole, and their one-way display
  evidence. The application owns no alternate float geometry renderer.
- Direct Hypercurve-to-Hyperpath promotion preserves exact line and explicit
  circular-arc objects. General cubics fail with a typed metric blocker rather
  than using display chords.
- A separate motion compiler certifies source chords, exact chord length,
  nearest machine steps, nearest cumulative timer boundaries, and real sibling
  `alumina-machine-ir::ExecutionSegment<2>` output.

The deterministic compiler fixture reports:

```text
source_curves=3
source_fragments=4
canonical_segments=197
final_steps=960,0
end_tick=1583188
ideal_chord_path_length_mm_display_f64=15.831876712961
source_chord_error_mm=1/1024
curve_to_canonical_chord_bound_mm_display_f64=0.009815397265
timer_boundary_error_seconds=1/2000000
segment_duration_error_seconds=1/1000000
```

The two values marked `display_f64` are explicitly lossy diagnostics. Their
authoritative counterparts remain exact `Real` expressions. For this 80
steps/mm two-axis fixture, the conservative path bound is exactly represented
as `1/1024 + sqrt(2)/160 mm`; the timer-boundary bound is exactly half of one
1 MHz tick and the segment-duration bound is one complete tick.

That bound ends at the canonical interpolated command chords. It is not a
physical following-error claim; discrete step-event, calibration, mechanics,
and control errors remain separate qualification inputs.

## Verification

The following passed against the paths and revisions in `HYPER-BASELINE.md`:

- Hypergraphics: 31 unit tests, one dispatch integration test, two README
  tests, all benchmark smoke runs, all-feature/all-target strict clippy, and
  rustdoc with warnings denied.
- Alumina Interface: all workspace/all-target tests (13 exact-core tests, one
  compile-fail doc test, and two protocol-client tests), native strict clippy,
  `wasm32-unknown-unknown` strict clippy, and workspace rustdoc with warnings
  denied.
- Native/WASM source and permissive-license audit; GPL, LGPL, AGPL, and SSPL
  remain rejected.
- Offline locked Trunk release build, `wasm-tools validate`, all gzip integrity
  checks, and Brotli integrity validation.

Production artifact facts from this checkpoint build:

| Artifact | Bytes | SHA-256 |
| --- | ---: | --- |
| `alumina-interface_bg.wasm` | 3,791,907 | `a02518db33fc3f777bd26025080e29db3910b9062b91cb0a55b67b70854ae6d7` |
| `alumina-interface_bg.wasm.br` | 1,462,064 | compression integrity verified |
| `alumina-interface_bg.wasm.gz` | 1,776,640 | compression integrity verified |
| `alumina-interface.js` | 75,563 | `142202f87e449d354d954f449eaf62a8ddc5b8100897f41b0ccd303bf4ca695a` |
| `index.html` | 1,290 | `aeb69fcf104256c53ddb649aeb7f8f1b3db12d045c565c562c2e3603d1ceb324` |
| `Cargo.lock` | — | `2b5be2b5c541edb50887bc2da66eead19db2c66cef61e3ae78d61f037c0a355a` |

## Qualification limits

- The sibling CSGRS working tree is authoritative. Its local `0.23.0` manifest
  label is not permission to substitute the old published release.
- Hyperphysics had concurrent tracked modifications at capture time. Therefore
  this is not a coherent clean-stack release pin even though the tested build
  passed.
- The compiler currently schedules constant feed along its certified chord
  path. It does not claim an exact general-Bezier arc-length/feed law.
- Calibration, following, control, qualified timer jitter, and hardware-output
  errors are not yet included in the development fixture's physical budget.
- No board was connected or energized for this checkpoint.
