# Exact machine scheduling

The browser/WASM compiler now has one machine-bound scheduling path for exact
two-axis stepper motion. It begins with the firmware's validated canonical
configuration, retains supported Hypercurve geometry through Hyperpath and
Hypersolve certification, and releases cached machine IR only after replaying
the production stepper executor's electrical contract.

This is deterministic software evidence. It does not qualify a board, motor
driver, machine, output waveform, or physical following-error claim.

## Authority flow

```text
authenticated ALMCAP02 capability + canonical ALMCFG05 configuration
    -> allocation-free ConfigurationDocumentView
    -> exact two-axis MachineDynamicsProfile2
    -> certified MachineResolutionBudget2
    -> retained Hypercurve line/arc/cubic source
    -> lossless or certified/budgeted metric-path construction
    -> native exact source-envelope / usable-travel proof
    -> exact dense-axis projection for an all-affine route
    -> exact acceleration lookahead under explicit node ceilings
    -> stop-separated component-local jerk-feasibility refinement
    -> two- or four-phase jerk replay from selected boundary nodes
    -> certified interpolation onto the firmware V1 segment model
    -> configured step/tick/output lattices
    -> allocation-free production StepperExecutor preflight
    -> independently replayed chained execution blocks
    -> content-addressed SD partition
       ├-> canonical ALMEVD02 evidence transcript
       └-> event-level cached-partition simulator replay (verification)
```

No display chord, GPU coordinate, G-code token, or UI-local machine schema can
enter this path.

## Configuration-derived machine profile

`MachineDynamicsProfile2::from_configuration` accepts only a completely
validated `ConfigurationDocumentView`. V1 requires exactly two dense stepper
axes and no FOC axes. Each axis retains its exact resource bindings, electrical
minimums, commanded travel model, uncertainty intervals, usable travel,
velocity, acceleration, jerk, and following-error facts.

Command density is derived exactly as:

```text
full steps/turn × microsteps × motor turns/output turn × calibration scale
-------------------------------------------------------------------------
                       travel/output turn
```

Independent uncertainty is propagated through the expression. Scheduling uses
the conservative lower or upper endpoint appropriate to each bound. The
effective step frequency is the lesser of the configured resource frequency
and `TimerTickHertz / (pulse_high_cycles + pulse_low_cycles)`. This prevents a
nominal velocity fact from exceeding the electrical pulse-period ceiling.

The configuration also supplies the exact integer device-cycle frequency and
the backend's smallest addressable output interval. Firmware retains these V5
facts and refuses to construct a stepper executor if they differ from the
compiled Embassy clock or selected board backend.

## Exact travel envelope

Before lookahead or interpolation, `CertifiedTravelEnvelope2` asks Hypercurve
for the complete retained path bounds and compares each exact minimum and
maximum with the conservative usable travel interval derived from
configuration uncertainty. Hypercurve evaluates native circular-arc cardinal
extrema, so this proof covers a maximum or minimum between later interpolation
samples. It does not infer bounds from emitted points or renderer chords.

The certificate retains exact source and usable X/Y bounds in millimetres for
inspection. A source coordinate beyond either configured boundary, an
uncertified source bound, or an unresolved exact comparison fails before any
schedule or canonical machine value is released. Regression coverage narrows
the fixture's X travel to 7 mm and proves that its native 8 mm path maximum is
rejected.

Lowering performs a second proof after every exact coordinate is rounded to the
configured step lattice. Each integer command is divided by the same exact
nominal command density and compared with usable travel; an outward half-step
rounding cannot escape merely because the source coordinate was inside. A
regression uses a `1600/3 steps/mm` lattice whose exact 8 mm endpoint rounds to
8.000625 mm and proves rejection against an 8.0003 mm limit.

## Machine-resolution budget

`MachineResolutionBudget2` proves that the requested two-dimensional position
envelope contains all of the following conservative components:

- caller-owned source-curve approximation allocation;
- caller-owned controller interpolation allocation;
- Euclidean nearest-endpoint half-step error;
- Euclidean one-step DDA tracking error within a segment;
- command-density/calibration uncertainty over the complete usable travel;
- configured following error; and
- one complete output quantum of position at the maximum vector velocity.

The last component bounds the strictly smaller grid-only padding introduced by
one-sided interval ceiling. A selected exact time-dilation factor changes the
canonical reference schedule intentionally and is reported as time policy, not
silently charged to a spatial approximation budget.

An all-line/arc path uses no source-curve approximation, so its actual bound is
zero even when the machine policy reserves a positive allocation. A general
cubic can consume that allocation only through an exact pointwise
degree-elevated-chord certificate over Hypercurve de Casteljau subcurves and
caller-owned element/depth limits. The
full-travel calibration term is intentionally conservative; a later per-job
certificate may tighten the occupied extent but may never exceed this machine
envelope. An unresolved exact comparison or an insufficient requested total
fails before scheduling.

## Exact affine dense-axis projection

For an affine path span parameterized by scalar distance `s`, each dense
machine coordinate has one constant exact derivative `dq_i/ds`. Hyperpath now
accepts any number of axes and spans and selects the route-wide limits from

```text
|dq_i/ds| × path feed         <= axis velocity limit
|dq_i/ds| × path acceleration <= axis acceleration limit
|dq_i/ds| × path jerk         <= axis jerk limit.
```

Every structurally positive derivative contributes the exact quotient
`axis_limit / |dq_i/ds|`; a zero derivative contributes no restriction. The
planner selects the exact minimum separately for all three orders, retains the
first span/axis bottleneck deterministically, and asks Hypersolve to replay
every inequality and all three limiting equalities. Empty axis sets,
shape-mismatched projections, negative absolute derivatives, stationary spans,
unresolved comparisons, or a failed replay reject the result.

The current Alumina compiler constructs this certificate only when every
metric carrier is a line. Its exact Cartesian derivatives are
`(|dx|/sqrt(dx²+dy²), |dy|/sqrt(dx²+dy²))`. A 3-4-5 diagonal therefore permits
the scalar limit of two equal X/Y axes to rise by exactly `5/4`, rather than
discarding usable vector capacity through the old axis minimum. The dedicated
two-line G1 regression retains that exact ratio, independently replays four
span/axis rows, and lowers to terminal steps `[9600, 12800]` from the exact
continuous electrical ceiling through the replay-proved timer-lattice factor
`4158/4096`.

This affine formula is deliberately not applied to arcs or nonlinear
kinematics. Those cases also contain higher derivatives of the coordinate map.
A mixed curved route keeps the conservative direction-independent axis minimum
and the existing curvature budgets until those extra acceleration and jerk
terms have their own certificate.

## Exact acceleration lookahead and jerk feasibility

Lines and circular arcs are promoted losslessly from Hypercurve to Hyperpath.
A polynomial cubic is first reduced to exact `LineSeg2` chords under that
pointwise certificate; renderer output is not accepted. Hyperpath's mixed
feed carrier retains the exact Euclidean length of diagonal chords. V1 assigns
zero entry/exit ceilings and zero retained radius everywhere. Caller join
ceilings are positive only when both adjacent metric elements are lossless
exact source lines; arcs and every approximated cubic element receive zero.
Hyperpath then classifies the exact tangent join. A true corner with zero radius
and every reversal remain stops, so a positive selected join is possible only
for exact line-to-line G1 continuity.

Hyperpath combines caller, global-feed, reversal, and retained-radius ceilings,
propagates `sqrt(v² + 2*a*L)` through an exact forward acceleration pass and
then through an exact reverse deceleration pass. It independently replays every
caller ceiling and every global, geometric, reversal, and span reachability row
with Hypersolve. Zero radius at a true corner means an intentional unblended
stop, not a missing value or unchecked divide.

Acceleration reachability alone does not establish jerk feasibility. The next
layer partitions its selected node vector into maximal positive components
separated by exact zero stops. It tries the exact monotonic transition on every
span touching one component. If any transition exceeds acceleration or jerk,
all nodes in that component are divided by two and the complete component is
replayed. The maximum is 64 exact halvings. Exhaustion fails closed; a zero node
is never raised, components on opposite sides of a stop do not affect one
another, and relative feeds inside one component are retained. Final caller,
lookahead, and per-span jerk reports are reconstructed independently.

A zero/zero element uses the existing four-phase rest-to-rest profile. Its
phase distances are `1/12`, `5/12`, `5/12`, and `1/12` of exact length; a
common phase duration is rounded upward to an exact integer device-tick
interval after satisfying feed, acceleration, and jerk lower bounds. An element
with at least one positive boundary uses a conservative two-phase monotonic
transition. For length `L` and feeds `v0`, `v1`, both phases have
`T = L/(v0+v1)`, their shared feed is `(v0+v1)/2`, and acceleration returns to
zero at both element boundaries. Hyperpath independently replays requested
boundaries, monotonic shared feed, exact length sum, continuity, kinematics,
and all dynamic limits.

This is enabled only for lossless straight-line G1 joins. G1 alone would not be
enough across a curvature discontinuity because normal acceleration can jump;
therefore line/arc, arc/arc, all approximated cubic, and all true-corner joins
remain stopped. The planner is conservative rather than time-optimal and does
not construct a retained blend or solve a general N-axis S-curve.

The complete bounded reduction, exact-stop rationale, error composition, and
source/metric evidence contract are documented in
[`CERTIFIED-BEZIER-MOTION.md`](CERTIFIED-BEZIER-MOTION.md).

For a route containing an arc, the scalar tangential envelopes are reduced and
the feed ceiling is additionally constrained by radius-dependent centripetal,
mixed-jerk, and curvature-jerk bounds. These conservative bounds preserve a
full spatial acceleration envelope for the later interpolation proof.

## Lowering to firmware V1 segments

The smooth certified schedule is not misrepresented as native jerk execution.
Current firmware IR contains constant-velocity integer segments, so each jerk
phase is subdivided into the smallest exact integer count that proves

```text
maximum_spatial_acceleration × interval_time² / 8
    <= controller interpolation allocation.
```

Every certified metric line or arc is evaluated exactly at the resulting path
fractions. The selected point retains both its original source index and its
motion-element index. Coordinates are rounded to the configured step lattice.
Timer lowering treats each retained ideal interval independently: after an
exact rational dilation factor is applied, its duration is ceiled to an integer
multiple of the configured output quantum. Consequently no emitted interval is
shorter than its retained ideal interval. A subdivision may legitimately
quantize to zero steps while consuming time; this is an intentional hold
segment, not a dropped part of the schedule. Integer overflow or an unresolved
interpolation/timer predicate rejects the lowering.

`ScheduledLoweringLimits` supplies a caller-owned retained-point budget and a
`TimerDilationPolicy`. The lowerer checks the complete post-phase point count
with checked arithmetic and uses fallible bounded reservations before it
appends points or allocates candidate vectors. Thus an imported dynamics
profile that implies an extreme exact subdivision count fails before browser
memory growth. The interactive policy permits at most 131,072 retained points
and factors in exact increments of `1/4096` through a maximum factor of 16.
These resource/search limits are distinct from the physical interpolation
certificate.

Before the program is exposed, `alumina-motion::preflight_stepper_segments`
feeds every segment through the production `StepperExecutor` validation path.
That allocation-free replay checks continuity, exact event rate, pulse high and
low time, direction and driver-control setup/hold, output-quantum alignment,
overflow, terminal position, terminal tick, emitted step count, and earliest
legal finish. The compiler uses zero allowed lateness; runtime lateness remains
a separate board-qualified policy.

The projected continuous feed may equal an axis's exact electrical pulse-rate
ceiling. Coordinate rounding and centered output-grid edge placement can then
leave no realizable pulse boundary even though the continuous inequality is
equal. The lowerer first replays factor one. Only timing-pressure failures
classified by `alumina-motion` may enter dilation; structural, identity, grid,
overflow, state, and deadline errors return immediately. Candidate interval
durations, centered start offsets, terminal gaps, rate, pulse-low, setup, and
hold inequalities are monotone in the factor, so an exact binary search selects
the smallest numerator on the caller's factor lattice. The accepted complete
stream and its immediately smaller candidate are both replayed through the
unchanged production executor.

`TimerLatticeScheduleReport2` retains the exact selected factor, replay count,
factor-one rejection, predecessor rejection, ideal and canonical total times,
maximum cumulative delay, maximum segment extension, and strictly
sub-quantum grid padding. The exact 3-4-5 ceiling regression retains
`PulseBoundary { axis: 1 }` at factor one, `Rate { axis: 1 }` at `4157/4096`,
and selects `4158/4096` before ending at `[9600, 12800]`. A policy capped at one
fails closed. No floating safety factor or weakened firmware check exists.

## Cache, simulation, and evidence

`package_canonical_scheduled_program` requires the program's configuration and
capability digests to equal the target partition policy. It checks the executor
preflight terminal facts, constructs real chained 512-byte execution blocks,
replays them with `MotionStreamValidator`, and lets `alumina-storage` produce
the immutable object, chunk, and manifest identities.

`ALMEVD02` binds those resulting canonical bytes, terminal timing, source,
metric path, source approximation, machine identities, and selected error
allocations. It does not yet serialize the affine-projection, lookahead,
component-refinement, jerk, or timer-factor search transcripts themselves. The
in-memory program retains and displays them, but a greenfield evidence V3 must
bind their policies and rows explicitly; final output identity is not a
substitute for planner-decision identity.

`alumina-sim::replay_cached_stepper_partition` consumes the resulting bytes and
real `JobDescriptor`. It first checks the complete byte length, object kind, and
SHA-256 content identity, then performs independent block admission through
`RealtimeJob`, executes through `CachedStepperExecutor`, advances an exact
deadline event loop, acknowledges ownership in production order, and requires
the terminal block digest, tick, position, step counts, and finish cycle to
agree. It uses no target backend and produces no output.

Canonical `ALMEVD02` binds:

- exact-rational line/arc/cubic source identity;
- the exact line/arc metric-path identity;
- source-to-motion spans, family tags, motion ranges, exact error bounds, and
  subdivision depths;
- configuration and capability digests;
- partition object and manifest identities and object length;
- timer and output-quantum facts;
- block, point, segment, position, tick, finish, and step-count facts; and
- exact requested total, source, controller, certified source-to-motion, and
  interpolation budgets.

Replay reconstructs the transcript from the retained program and partition and
requires byte-for-byte equality; externally stored bytes are SHA-256 checked
before reconstruction. Source, metric, and approximation transcripts are
domain-separated and independently hashed. Evidence serialization requires
exact-rational primitive parameters. The schedule can carry other exact
expressions such as the symbolic semicircle length, but a non-rational source
parameter currently fails the evidence boundary.

The event-level simulator report is separate verification evidence; it is not
silently encoded as an `ALMEVD02` certification flag. The transcript's partition
replay flag refers to the independent canonical `MotionStreamValidator` replay
performed during packaging.

## Current limits

- Exactly two Cartesian stepper axes are supported by this scheduler.
- Scheduled source geometry supports exact lines, explicit circular arcs, and
  certified polynomial cubic reduction. Other Bezier, PH, spline, and NURBS
  policies remain fail-closed at this boundary.
- Positive feed is supported only across lossless exact line-to-line G1 joins.
  Every generated cubic chord boundary, curvature-bearing join, corner, and
  reversal remains a stop. Component-local jerk refinement is implemented;
  retained blends, curvature/normal-jerk continuity, globally time-optimal node
  selection, and general nonzero-boundary S-curves are not.
- All-affine two-axis routes use exact dense-axis projection. Mixed curved
  routes retain the most conservative axis-wide envelope; curved derivative
  projection, span-local limits, and non-Cartesian kinematics remain future
  work.
- Exact output-quantum headroom is selected on a caller-owned rational factor
  lattice and complete production replay remains authoritative. Global
  multi-MCU selection of one shared retiming factor and direct native jerk IR
  remain open.
- Firmware V1 follows the certified smooth schedule through bounded
  constant-velocity segments; it does not run an onboard jerk planner.
- Configured source/command travel containment does not by itself prove tool,
  fixture, or physical following-error clearance at a machine boundary. Those
  margins must be present in configured usable travel or added by a later
  job-specific clearance certificate.
- Physical timer jitter, TinyBee PCM-short timing, driver behavior, mechanics,
  and safety response still require the documented disconnected-load and HIL
  qualification ladder. The current TinyBee motion package remains non-armable.
