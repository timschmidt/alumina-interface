# Certified cubic Bezier motion

Snapshot: 2026-08-13.

The browser/WASM Machine/CAM compiler can retain a native exact Hypercurve
`CubicBezier2` as authoritative source geometry while scheduling a separately
certified line/arc metric path. This is a motion-specific reduction under the
machine resolution budget. It never consumes Hypergraphics vertices, painter
coordinates, or a G-code tessellation.

## Authority boundary

```text
retained exact Hypercurve path
    ├─ native source bounds ───────────────> usable-travel proof
    └─ certify_metric_path
         ├─ line / CircularArc2 ──────────> lossless exact copy
         └─ CubicBezier2
              -> exact de Casteljau subdivision
              -> pointwise degree-elevated chord predicates
              -> bounded binary subdivision
              -> exact-Real LineSeg2 chords
              -> source-index / motion-range / error / depth certificate
                    -> exact diagonal Hyperpath metric carriers
                    -> exact forward/reverse lookahead and jerk refinement
                       under a zero caller ceiling at every cubic chord node
                    -> four-phase jerk schedule per metric element
                    -> bounded controller interpolation
                    -> step/tick lattices and executor preflight
                    -> cached partition + ALMEVD03
```

`CurvePath2` remains in both `CertifiedJerkSchedule2` and
`CanonicalScheduledProgram2`. `CertifiedMetricPath2` is a distinct object. Its
spans map every retained source curve to a contiguous range of motion elements
and record its exact rational certified positional bound and the deepest
subdivision actually used. The bound is conservative; it is not claimed to be
the least or attained deviation. Lines and arcs have a zero source-to-motion
bound. A cubic requires a strictly positive caller-owned allocation.

## Bounded admission and failure

`MetricPathApproximationLimits2` bounds both the complete generated motion
element count and the requested binary subdivision depth. The interactive
policy permits 16,384 motion elements and depth 20. Before calling Hypercurve,
the compiler further caps effective depth so its worst-case leaf count cannot
exceed the remaining motion-element allowance. With room for only one chord,
the effective depth is zero: the unsplit chord must certify or the curve fails.
Candidate vectors use checked counters and fallible reservations.

The reduction fails without releasing a partial path when:

- the source allocation is negative or a cubic receives zero allocation;
- source curves alone exceed the element allowance;
- an exact comparison remains unresolved, or the curve still requires
  subdivision after the selected depth is exhausted;
- a generated prefix exceeds the element allowance;
- a count or allocation cannot be represented; or
- a retained family has no connected metric policy.

For one cubic span, the endpoint chord is elevated to cubic degree. Subtracting
its four controls from the source controls produces the exact Bezier controls
of `source(t) - chord(t)`. If both interior difference-control norms fit the
allocation, convexity proves the same Euclidean bound for every shared
parameter `t`. Otherwise the compiler uses Hypercurve's exact de Casteljau
half-split and repeats the proof. This is stronger than a perpendicular
flatness test: a collinear cubic that travels backward and forward cannot
collapse to its endpoint chord. A dedicated regression retains both excursions.

All representative control points and generated de Casteljau points remain
exact `Real` values, and Hyperlimit decides each squared-distance comparison.
The metric path is reconstructed as native `LineSeg2` objects and revalidated
as a connected `CurvePath2` before Hyperpath sees it.

## Conservative first motion policy

The current scheduler treats every approximated metric join as an exact stop,
including adjacent certified chords from one cubic. Each chord therefore starts
and ends at zero feed and receives an independently replayed symmetric
four-phase constant-jerk profile. This is intentionally slow, but it avoids
claiming that an instantaneous chord-direction change can occur at nonzero
velocity.

Those zeros are not inferred from the chord angles. Alumina identifies the
approximated source span and supplies zero caller-owned ceilings at every
boundary before Hyperpath planning. Hyperpath combines them with tangent class,
global feed, retained radius, and exact span length; performs squared-speed
forward and reverse reachability; and releases its candidate only after
independent Hypersolve replay. Its later jerk-feasibility pass cannot raise a
zero node. This keeps the cubic policy invariant even though lossless exact
line-to-line G1 joins elsewhere may now remain moving.

Hyperpath now computes an exact Euclidean length for diagonal
`LinePathSegment` values. Axis-ordering and axis-specific APIs remain strict;
only the mixed feed carrier accepts a general nonzero line. This lets certified
cubic chords retain `sqrt(dx² + dy²)` symbolically through lookahead and jerk
replay. When the complete metric route is line-only, the machine layer also
forms exact `|dx|/length` and `|dy|/length` derivatives and uses Hyperpath's
dense-axis projection to replay every velocity, acceleration, and jerk limit.
The cubic's stop-at-every-chord rule remains unchanged.

A future native Bezier, PH, or curvature-certified feed carrier may remove
internal stops. It must replace this certificate explicitly; renderer
tessellation cannot be promoted by convenience.

The phase selector reads exact lookahead boundary nodes. Every cubic chord sees
zero/zero and follows the same rest-to-rest path. Hyperpath's two-phase
monotonic transition is enabled only on lossless exact line chains; it is not
eligible at an approximated chord even if two adjacent chord tangents happen to
be collinear. A future native Bezier or curvature-certified carrier must replace
this explicit source-to-motion policy to move through the curve.

## Error composition

For the current path, the conservative curve-to-canonical command bound is:

```text
certified source-to-motion error
  + selected controller interpolation error
  + Euclidean endpoint half-step error
  + Euclidean within-segment DDA tracking error.
```

The machine-wide budget also includes calibration, following-error, and timer
position components. Scheduling receives the resolution certificate before
source reduction, and lowering refuses a certificate whose actual
source-to-motion bound exceeds the supplied budget. The default TinyBee
fixture allocates `1/100 mm` to source reduction, `1/100 mm` to controller
interpolation, and requests a `1/10 mm` complete machine envelope. Its actual
controller interpolation request remains `1/1000 mm`.

## Canonical evidence

`ALMEVD03` domain-separates and hashes five reconstructed transcripts:

- the retained exact source path, including all four exact-rational cubic
  control points;
- the exact line/arc metric path actually presented to Hyperpath; and
- the source-to-motion spans, family tags, motion ranges, exact error bounds,
  and subdivision depths;
- exact planner policy, tangent/travel/limit facts, affine projection,
  lookahead and component refinement, selected phases, and every retained
  Hypersolve certification row; and
- complete resolution/interpolation/lattice evidence, timer-search policy and
  rejections, exact scheduled points, canonical segments, and executor
  preflight.

The outer transcript binds all five digests, and the planner/lowering byte
lengths, alongside machine identities, partition identities, terminal executor
facts, machine allocations, the certified source-to-motion bound, and the
controller interpolation request. Replay rebuilds all transcripts from the
live schedule/program/partition and requires byte-for-byte identity. Raw CNC
text remains separate provenance and does not enter evidence.

## Present limits

- Source reduction supports exact lines, explicit circular arcs, and polynomial
  cubic Beziers. Other exact families fail closed.
- The metric schedule is two-axis Cartesian stepper motion.
- A complete line-only metric route uses exact affine dense-axis projection;
  mixed curved routes retain conservative direction-independent limits.
- Every certified chord boundary is a full stop. Exact acceleration lookahead,
  component-local jerk refinement, and monotonic positive-boundary phases are
  active for eligible lossless line chains, but no nonzero-feed policy exists
  across an approximated cubic.
- The certificate bounds positional deviation, not tool/process clearance.
  Native source bounds still gate configured travel, while fixtures and tools
  need later job-specific clearance evidence.
- This is software evidence. It does not qualify physical TinyBee timing,
  drivers, motors, mechanics, or safety response.
