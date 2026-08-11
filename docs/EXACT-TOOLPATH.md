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
    ├─ lossless family promotion ─> Hyperpath metric objects ─> Hypersolve replay
    └─ motion-specific certified subdivision ─> exact chord metric
                                                ─> step/timer lattices
                                                ─> alumina-machine-ir
```

The paths are intentionally one-way and disjoint. A display chord remains an
exact-`Real` object and carries a certified geometric error bound, but it is
still an approximation of its source curve. It cannot become authoritative
CAM input, a metric carrier, or machine IR.

## Implemented family boundary

| Hypercurve source | Hyperpath result | Policy |
| --- | --- | --- |
| axis-aligned `LineSeg2` | `LinePathSegment` | exact endpoint copy and strict predicate validation |
| `CircularArc2` | `ExplicitCircularArc` | exact center/endpoints/direction and exact derived radius |
| general quadratic, cubic, rational, spline, or NURBS | typed `UnsupportedMetricCurve` | fail closed; no chord or float fallback |

This limitation is deliberate. Hyperpath already has native cubic and quintic
Pythagorean-hodograph carriers, but a general Hypercurve Bezier is not assumed
to be PH. A later compiler stage must either prove and promote the appropriate
family or retain certified metric approximation evidence designed for motion,
not reuse renderer tessellation.

The current canonical compiler provides that separate approximation boundary:
it invokes Hypercurve subdivision itself with a motion policy, computes the
exact length of the resulting chord path, and schedules constant feed against
that compiled path. This does not claim a lossless general-Bezier metric or
reuse the Hypergraphics output.

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
`ExactScene`; attempting metric promotion of the full fixture is tested to
fail on the cubic.

The scene also retains a curved material loop and a rectangular hole as one
exact `CurveRegion2`. Hypergraphics materializes its exact boundary paths,
retains independently reported path/role certainty, and certifies each loop's
display chords. Material and hole colors come from authoritative roles, never
from line-mesh winding.

## Required next boundaries

- add certified filled-region triangulation without reclassifying display chords;
- promote supported PH curves and replace chord-feed approximation where an
  exact or tighter certified source-curve metric is available;
- add machine capabilities and bounded physical calibration inputs;
- quantify geometric, timing, and actuator error at configured machine
  resolution before emitting `alumina-machine-ir`;
- certify lookahead, acceleration, jerk, and multi-axis scheduling against the
  retained path rather than display geometry.
