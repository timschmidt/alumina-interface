# Exact toolpath boundary

The browser/WASM compiler treats the checked-out sibling Hyper stack as one
coherent implementation. Hypercurve owns authored curve geometry; Hyperpath
owns retained metric and scheduling objects; Hypersolve certifies proposed
kinematic values. No published CSGRS package substitutes for the current local
working tree.

## Current pipeline

```text
Hypercurve source path
    ├─ certified subdivision ─> Hypergraphics exact display chords ─> GPU
    │                           (presentation evidence only)
    ├─ lossless or motion-certified metric construction
    │   ─> Hyperpath metric objects ─> Hypersolve replay
    │   └─ machine-bound acceleration/jerk-feasible schedule
    │       ─> certified V1 interpolation ─> configured step/timer lattices
    │       ─> production executor preflight ─> cached partition + evidence
    └─ general-curve certified subdivision ─> exact chord metric
                                              ─> step/timer lattices
                                              ─> development machine IR
```

The paths are intentionally one-way and disjoint. A display chord remains an
exact-`Real` object and carries a certified geometric error bound, but it is
still an approximation of its source curve. It cannot become authoritative
CAM input, a metric carrier, or machine IR.

## Implemented family boundary

| Hypercurve source | Hyperpath result | Policy |
| --- | --- | --- |
| nondegenerate `LineSeg2` | `LinePathSegment` | exact endpoint copy, strict predicate validation, and exact Euclidean feed length |
| `CircularArc2` | `ExplicitCircularArc` | exact center/endpoints/direction and exact derived radius |
| polynomial `CubicBezier2` | exact `LineSeg2` motion path | bounded pointwise degree-elevated-chord certificate over exact Hypercurve de Casteljau spans |
| general quadratic, rational, spline, or NURBS | typed `UnsupportedMetricCurve` | fail closed; no renderer-chord or float fallback |

This boundary is deliberate. Hyperpath already has native cubic and quintic
Pythagorean-hodograph carriers, but a general Hypercurve Bezier is not assumed
to be PH. The current compiler therefore retains a distinct metric path and
source-to-motion certificate designed for motion. It never reuses renderer
tessellation. Every generated cubic chord boundary is an exact stop until a
native or curvature-certified feed policy replaces it.

The general-curve compiler retains a separate approximation boundary: it invokes
Hypercurve subdivision itself with a motion policy, computes the exact length
of the resulting chord path, and schedules constant feed against that compiled
path. This remains useful bounded geometry evidence, but it does not claim a
lossless general-Bezier metric or a certified jerk schedule and never reuses
Hypergraphics output.

The machine-bound V1 path accepts lossless lines/arcs and certified polynomial
cubics. It derives dynamics, electrical timing, physical uncertainty, step
density, timer frequency, and output quantum from canonical Configuration V6
rather than a second UI schema. Hyperpath/Hypersolve certify zero-radius
acceleration lookahead, stop-separated component-local jerk refinement, and
two- or four-phase schedules from the selected boundary nodes. Only lossless
line-to-line G1 joins can remain moving; all approximated cubic and
curvature-bearing joins stop. A separate proof bounds the schedule's firmware
constant-velocity approximation, after which the production stepper executor
is replayed before cache packaging. See `EXACT-MACHINE-SCHEDULING.md` and
`CERTIFIED-BEZIER-MOTION.md`.

After canonical quantization, `partition.rs` queries the firmware schema's exact
per-block record capacity, applies the caller's firmware horizon bound, and
encodes real chained `ExecutionBlock` values. It independently replays the
complete stream before constructing real `alumina-storage` upload/chunk/manifest
identities and a later boot-local `alumina-job::JobDescriptor`. Storage chunks
may cross block boundaries; neither storage hashing nor packaging reconstructs
geometry or changes the exact-CAM error report. See `CACHED-PARTITIONS.md`.

Owned local artifacts then enter the shared `alumina-job` global manifest
schema. Stable-device sorting, participant-set hashing, exact rational duration
agreement, and independent decode bind each named partition to the same source,
compiler, policy, machine, coordinate, safety, and synchronization identities.
See `GLOBAL-JOB-MANIFEST.md`.

## Deterministic fixture

The window-free test fixture is a four-unit axis-aligned line followed by a
radius-two half circle. Its exact total length is `4 + 2*pi`. At one unit of
feed per unit of time, Hyperpath and Hypersolve certify the same symbolic value
as target time. Native and WASM builds compile the same code and local path
dependencies.

The canonical fixture extends through the cubic and uses 80 steps/mm on both
axes, a 1 MHz timer, 10 mm/s chord feed, a `1/1024 mm` source-chord budget, and
depth 24. Coordinates and cumulative times use certified nearest-integer
rounding. Every axis endpoint is at most half a command-lattice unit from its
exact chord endpoint; every cumulative timer boundary is at most half a tick
from ideal, and every segment duration is at most one tick from ideal. The
retained conservative source-curve-to-canonical-chord bound is the source chord
budget plus the Euclidean two-axis endpoint bound, namely
`1/1024 + sqrt(2)/160 mm` for this fixture. Firmware records are the sibling
`alumina-machine-ir` types, not a UI copy.

The display fixture extends that path with a general cubic Bezier. Hypercurve
certifies its presentation chords to `1/1024` model unit with a maximum depth
of 24. The exact source path and the display certificate remain available in
`ExactScene`. Lossless metric promotion of the full fixture still fails on the
cubic, while the separate machine-specific certificate admits it under an
explicit source allocation.

The scene also retains a curved material loop and a rectangular hole as one
exact `CurveRegion2`. Hypergraphics materializes its exact boundary paths,
retains independently reported path/role certainty, and certifies each loop's
display chords. Material and hole colors come from authoritative roles, never
from line-mesh winding.

The machine-bound fixture uses a real canonical two-axis TinyBee configuration,
including exact 1 MHz device time, one-cycle output quantum, 1,600 nominal
steps/mm, electrical pulse constraints, dynamics, calibration uncertainty, and
following error. It schedules the retained four-unit line, radius-two
semicircle, and a cubic arch, stops exactly at every unblended/certified metric
join, lowers the result under a `1/1000 mm` controller-interpolation bound, and
finishes at `[19200, 0]` steps. The source-to-motion allocation is `1/100 mm`.
The same fixture passes production electrical preflight, real cache packaging,
event-level simulator replay, deterministic `ALMEVD03` reconstruction,
identity substitution rejection, and evidence-tamper rejection.

## Required next boundaries

- add certified filled-region triangulation without reclassifying display chords;
- promote supported PH/native curves and replace full-stop chord feed where an
  exact or tighter certified source-curve metric is available;
- add certified nonzero-radius blends and longer-range velocity optimization;
- add direction-aware axis envelopes, broader kinematics, and more than two axes;
- extend canonical source evidence beyond exact-rational lines, arcs, and
  polynomial cubics; and
- qualify physical timing, calibration, following, and safety behavior on each
  board/machine combination without weakening the exact software envelope.
