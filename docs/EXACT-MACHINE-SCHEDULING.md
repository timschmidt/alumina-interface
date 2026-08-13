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
    -> exact two-pass lookahead under explicit node ceilings
    -> four-phase jerk replay
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
- half a device tick of position at the maximum vector velocity.

An all-line/arc path uses no source-curve approximation, so its actual bound is
zero even when the machine policy reserves a positive allocation. A general
cubic can consume that allocation only through an exact pointwise
degree-elevated-chord certificate over Hypercurve de Casteljau subcurves and
caller-owned element/depth limits. The
full-travel calibration term is intentionally conservative; a later per-job
certificate may tighten the occupied extent but may never exceed this machine
envelope. An unresolved exact comparison or an insufficient requested total
fails before scheduling.

## Exact two-pass lookahead and jerk schedule

Lines and circular arcs are promoted losslessly from Hypercurve to Hyperpath.
A polynomial cubic is first reduced to exact `LineSeg2` chords under that
pointwise certificate; renderer output is not accepted. Hyperpath's mixed
feed carrier retains the exact Euclidean length of diagonal chords. V1 assigns
zero retained blend radius and a caller-owned zero ceiling to every metric
join, as well as zero entry and exit ceilings. Hyperpath classifies every exact
tangent join, combines caller, global-feed, reversal, and retained-radius
ceilings, propagates `sqrt(v² + 2*a*L)` through an exact forward acceleration
pass, and then through an exact reverse deceleration pass. It independently
replays every caller ceiling and every global, geometric, reversal, and span
reachability row with Hypersolve before releasing the schedule. Zero radius at
a true corner therefore means an intentional unblended exact stop, not a
missing radius or an unchecked divide. A G1 join does not require a radius, but
Alumina's explicit zero caller ceiling still stops there.

This introduces the bounded path-wide node planner without changing current
machine output: every selected speed remains exactly zero. The planner's
positive-radius input is only meaningful when the metric path contains the
corresponding retained blend. Alumina does not yet construct such blends or
permit nonzero node feeds.

Phase selection nevertheless consumes those selected nodes rather than
assuming zero implicitly. A zero/zero element uses the existing four-phase
rest-to-rest profile. If a later internal policy supplies at least one positive
boundary feed, the dormant branch asks Hyperpath for a conservative two-phase
monotonic transition. For exact length `L` and boundary feeds `v0`, `v1`, both
phases have `T = L/(v0+v1)`, the shared feed is `(v0+v1)/2`, acceleration
returns to zero at both element boundaries, and no feed overshoot is permitted.
Hyperpath independently replays the requested boundaries, monotonic shared
feed, local kinematics, length sum, phase continuity, feed, acceleration, and
jerk limits. Both-zero input remains owned by the rest-to-rest proposer.

This branch is groundwork, not enabled blending. It cannot invent retained
blend geometry, alter the all-zero caller policy, or make an infeasible short
span pass; independent replay rejects the latter. It is not a general
time-optimal S-curve or a jerk-aware replacement for the acceleration-only
lookahead node selection.

Each retained metric element receives a symmetric four-phase, rest-to-rest,
constant-jerk schedule. Its phase distances are `1/12`, `5/12`, `5/12`, and
`1/12` of the exact element length. A common phase duration is rounded upward
to an exact integer device-tick interval after satisfying the feed,
acceleration, and jerk lower bounds. The reconstructed phases are then replayed
by Hyperpath/Hypersolve for length, state continuity, feed, acceleration, and
jerk.

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
motion-element index. Coordinates and cumulative times are then independently
rounded to the configured step and tick lattices. A subdivision may
legitimately quantize to zero steps while consuming time; this is an
intentional hold segment, not a dropped part of the schedule. Collapsed tick
boundaries, integer overflow, or an unresolved interpolation predicate reject
the lowering.

`ScheduledLoweringLimits` supplies a caller-owned retained-point budget. The
lowerer checks the complete post-phase point count with checked arithmetic and
uses fallible bounded reservations before it appends points or allocates the
canonical segment vector. Thus an imported dynamics profile that implies an
extreme exact subdivision count fails before browser memory growth. The
interactive policy currently permits at most 131,072 retained points,
including the initial point; this resource limit is distinct from the physical
interpolation-error certificate.

Before the program is exposed, `alumina-motion::preflight_stepper_segments`
feeds every segment through the production `StepperExecutor` validation path.
That allocation-free replay checks continuity, exact event rate, pulse high and
low time, direction and driver-control setup/hold, output-quantum alignment,
overflow, terminal position, terminal tick, emitted step count, and earliest
legal finish. The compiler uses zero allowed lateness; runtime lateness remains
a separate board-qualified policy.

## Cache, simulation, and evidence

`package_canonical_scheduled_program` requires the program's configuration and
capability digests to equal the target partition policy. It checks the executor
preflight terminal facts, constructs real chained 512-byte execution blocks,
replays them with `MotionStreamValidator`, and lets `alumina-storage` produce
the immutable object, chunk, and manifest identities.

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
- Every metric join—including every generated cubic chord boundary—is still a
  full stop. Exact forward/reverse acceleration reachability is implemented,
  and a monotonic zero-acceleration-boundary transition exists behind the
  disabled positive-node branch. Retained nonzero-radius blend construction,
  jerk-aware node selection, and general time-optimal nonzero-boundary S-curves
  are not implemented yet.
- Scalar limits use the most conservative axis-wide envelope; direction-aware
  utilization and non-Cartesian kinematics remain future work.
- Firmware V1 follows the certified smooth schedule through bounded
  constant-velocity segments; it does not run an onboard jerk planner.
- Configured source/command travel containment does not by itself prove tool,
  fixture, or physical following-error clearance at a machine boundary. Those
  margins must be present in configured usable travel or added by a later
  job-specific clearance certificate.
- Physical timer jitter, TinyBee PCM-short timing, driver behavior, mechanics,
  and safety response still require the documented disconnected-load and HIL
  qualification ladder. The current TinyBee motion package remains non-armable.
