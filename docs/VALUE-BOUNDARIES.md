# Value-boundary contract

The interface compiler must distinguish mathematical exactness from physical
knowledge, firmware representation, and presentation. A shared scalar type or
an implicit numeric conversion would let evidence disappear, so I0 establishes
four disjoint Rust domains.

| Domain | I0 carrier | Meaning | May decide CAM/topology? |
| --- | --- | --- | --- |
| Exact design/CAM | `ExactValue<U>` and native CSGRS/Hyper values | Source intent and derived exact/certified mathematics | yes |
| Bounded measurement | `BoundedMeasurement<U>` | Closed rational physical interval with an explicit unit | only through a named conservative policy |
| Canonical firmware | `CanonicalStep`, `CanonicalCycle`, and `alumina-machine-ir` | Already quantized integer/fixed-point values accepted by bounded firmware validation | execution only; never reconstructs source intent |
| Display/GPU | `DisplayScalar`, Hypergraphics `RenderVertex64`/`Projection64`, and GPU `f32` | Finite lossy presentation | no |

The allowed information flow is:

```text
exact CAD/CAM ───────┐
bounded measurements ├─ checked compiler + error evidence ─ canonical machine IR
capabilities/config ─┘

exact / measured / canonical ─ named one-way projection ─ display / GPU
```

There is no display-to-exact, display-to-measurement, or display-to-machine
conversion. User-entered decimal geometry is parsed directly as an exact
decimal rational; it is not parsed as `f32`/`f64` and then widened. Camera input
may import finite pointer deltas as exact dyadics because camera state is a
presentation concern and cannot satisfy a CAM input type.

## Exact design/CAM

`ExactValue<U>` owns a `hyperreal::Real` and uses a unit marker such as
`Millimetres` or `Seconds`. It accepts exact decimal text, a `Rational`, or an
already exact `Real`. Geometry remains in the native CSGRS/Hyper carriers. The
core uses Hypercurve as source-path authority and promotes only supported exact
line and circular-arc objects into Hyperpath metric carriers. General cubics
currently return an explicit metric blocker; their certified presentation
chords are not a feed-planning substitute. Hypersolve replays the retained
symbolic path metric, so later CAM work extends the current implementations
instead of introducing a float planner.

An undecided predicate or failed exact operation becomes a compiler diagnostic.
It must not turn into empty geometry, false, zero, or a renderer tolerance.

## Bounded measurements

A physical measurement is not exact source intent even when its bounds are
represented exactly. `BoundedMeasurement<U>` stores a closed rational interval
constructed from `nominal ± uncertainty` and rejects negative uncertainty.
Calibration, following error, electrical tolerances, clock fits, and measured
machine limits will use this domain or a certified interval extension of it.

## Canonical firmware values

Firmware receives bounded integer/fixed-point artifacts. I0's
`canonical_motion_segment` accepts only `CanonicalCycle` and `CanonicalStep`
values and produces the real `alumina-machine-ir::ExecutionSegment`; it does not
duplicate that wire schema. Quantization must produce these values along
with conservative geometry, lattice, timing, calibration, and control-error
evidence. Constructing an integer carrier is not itself proof that quantization
was acceptable.

The first window-free compiler now exercises that boundary. Hypercurve emits a
motion-specific certified chord path independently of display subdivision.
Each coordinate is multiplied by an exact steps/mm value and rounded by
`Real::round_certified`; each cumulative ideal chord time is similarly rounded
onto an exact integer timer lattice. Hyperlimit replays every half-step and
half-tick bound before canonical machine IR is constructed. The report retains
source-chord, per-axis, Euclidean endpoint, combined canonical-chord, and
timer-boundary budgets separately. Discrete step-event, calibration, following,
and control errors remain future inputs and are not silently treated as zero in
a release certificate.

Canonical segments then move through another one-way boundary without becoming
geometry again. The interface queries the real machine-IR record capacity,
encodes chained fixed-size blocks, and independently replays their identities,
limits, timing, displacement, and terminal digest. The same bytes become a real
content-addressed `alumina-storage` object and later a real
`alumina-job::JobDescriptor`. GPU values cannot enter this path, and cached
machine bytes cannot be promoted back into design intent.

## Display and GPU

`project_for_display` is explicitly lossy and returns only a finite
`DisplayScalar`. Hypergraphics similarly owns exact scene construction and the
checked `Real` → `f64` → GPU `f32` narrowing. Its native Hypermesh adapter checks
all triangle indices before producing exact scene vertices. Its Hypercurve
adapter emits exact-`Real` chord endpoints only after certified subdivision and
retains the maximum source-chord error, segment count, depth, and source span
count. That certificate authorizes presentation only. The app merely composes
meshes, camera interactions, and draw callbacks.

The compile-fail test attached to `DisplayScalar` proves that a display result
cannot satisfy `ExactValue<Millimetres>`. Runtime tests separately prove exact
decimal retention, bounded-measurement interval properties, canonical integer
segment construction, and finite scene export.
