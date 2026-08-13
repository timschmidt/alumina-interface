# Exact machine scheduling V1

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
    -> retained Hypercurve line/arc source
    -> lossless Hyperpath metric promotion
    -> exact-stop lookahead + four-phase jerk replay
    -> certified interpolation onto the firmware V1 segment model
    -> configured step/tick/output lattices
    -> allocation-free production StepperExecutor preflight
    -> independently replayed chained execution blocks
    -> content-addressed SD partition
       ├-> canonical ALMEVD01 evidence transcript
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

The first line/arc scheduler uses no source-curve approximation, so that
allocation may be zero. The full-travel calibration term is intentionally
conservative; a later per-job certificate may tighten the occupied extent but
may never exceed this machine envelope. An unresolved exact comparison or an
insufficient requested total fails before scheduling.

## Exact-stop lookahead and jerk schedule

Supported lines and circular arcs are promoted losslessly from Hypercurve to
Hyperpath. V1 assigns zero retained blend radius and zero feed to every source
join, as well as zero entry and exit feed. Hyperpath and Hypersolve replay every
tangent span, join constraint, and speed node. Zero radius therefore means an
intentional unblended exact stop, not a missing radius or an unchecked divide.

Each retained source element receives a symmetric four-phase, rest-to-rest,
constant-jerk schedule. Its phase distances are `1/12`, `5/12`, `5/12`, and
`1/12` of the exact element length. A common phase duration is rounded upward
to an exact integer device-tick interval after satisfying the feed,
acceleration, and jerk lower bounds. The reconstructed phases are then replayed
by Hyperpath/Hypersolve for length, state continuity, feed, acceleration, and
jerk.

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

Every retained line or arc is evaluated exactly at the resulting path
fractions. Coordinates and cumulative times are then independently rounded to
the configured step and tick lattices. A subdivision may legitimately quantize
to zero steps while consuming time; this is an intentional hold segment, not a
dropped part of the schedule. Collapsed tick boundaries, integer overflow, or
an unresolved interpolation predicate reject the lowering.

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

Canonical `ALMEVD01` V1 binds:

- exact rational line/arc source identity;
- configuration and capability digests;
- partition object and manifest identities and object length;
- timer and output-quantum facts;
- block, point, segment, position, tick, finish, and step-count facts; and
- exact requested total, source, controller, and interpolation budgets.

Replay reconstructs the transcript from the retained program and partition and
requires byte-for-byte equality; externally stored bytes are SHA-256 checked
before reconstruction. V1 evidence serialization accepts only retained
line/arc primitives whose parameters are exact rationals. The schedule can
carry other exact expressions such as the symbolic semicircle length, but a
non-rational source parameter currently fails the evidence boundary.

The event-level simulator report is separate verification evidence; it is not
silently encoded as an `ALMEVD01` certification flag. The transcript's partition
replay flag refers to the independent canonical `MotionStreamValidator` replay
performed during packaging.

## Current limits

- Exactly two Cartesian stepper axes are supported by this scheduler.
- Scheduled source geometry is limited to axis-aligned lines and explicit
  circular arcs; general Bezier, PH, spline, and NURBS scheduling remains
  fail-closed at this boundary.
- Every unblended source join is a full stop. Certified nonzero-radius blends
  and longer-range velocity optimization are not implemented yet.
- Scalar limits use the most conservative axis-wide envelope; direction-aware
  utilization and non-Cartesian kinematics remain future work.
- Firmware V1 follows the certified smooth schedule through bounded
  constant-velocity segments; it does not run an onboard jerk planner.
- Physical timer jitter, TinyBee PCM-short timing, driver behavior, mechanics,
  and safety response still require the documented disconnected-load and HIL
  qualification ladder. The current TinyBee motion package remains non-armable.
