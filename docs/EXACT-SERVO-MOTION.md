# Exact servo motion boundary

The browser/WASM compiler is authoritative for servo path planning. Firmware
does not accept geometry, G-code, floating-point splines, or a second planning
language. It accepts only bounded `ALMBLK03` kind-3 recurrences that have
already been projected onto the configured axis and feed-forward lattices.

This is a numerical and software boundary, not a claim that a motor, encoder,
power stage, or machine has been qualified.

## Authority flow

```text
exact CAD / Hyperpath / Hypersolve result
    -> exact Hyperreal cubic Newton-forward recurrences
    -> active FOC configuration-derived cadence and physical limits
    -> certified Q31.32 position projection
    -> certified Q2.30 velocity and q-current projection
    -> discrete extrema + ownership-horizon splitting
    -> independent firmware-record validation
    -> chained 512-byte ALMBLK03 kind-3 blocks
    -> independent complete-stream replay
    -> content-addressed SD partition + ALMJOBD4 descriptor
```

`ExactServoAxisRecurrence` carries four exact coefficients for each of position,
velocity feed-forward, and quadrature-current feed-forward. The four entries
are initial value, first forward difference, second forward difference, and
third forward difference. Source spans share the exact cadence derived from the
complete active FOC axis profiles; a caller cannot choose a different update
grid at packaging time.

## Projection and evidence

`lower_exact_servo_recurrences` asks Hyperreal for a certified dyadic enclosure
of each scaled coefficient at a caller-selected precision. Both enclosure ends
must select the same ties-to-even integer. Position uses Q31.32. Both normalized
feed-forward signals use Q2.30. An unresolved enclosure, integer overflow, or
caller refinement limit is a typed failure.

Every `ServoCoefficientProjection` retains:

- the exact source `Real`;
- its closed scaled rational enclosure;
- the selected integer and fractional width;
- a conservative unscaled error bound; and
- whether exact encoded continuity, rather than nearest rounding, selected an
  initial coefficient.

For later source spans, the compiler makes the prior encoded terminal state the
only initial state. Its distance from the new exact source initial value is
included in the error certificate. It does not insert a compatibility shim or
hide a discontinuity. The complete recurrence error bound is formed exactly
from coefficient errors and the Newton binomial weights through the terminal
update. Position and feed-forward budgets are explicit caller policy.

## Exact splitting and terminal semantics

One source cubic may cross a discrete extremum or exceed a configured ownership
horizon. The browser examines integer recurrence transitions—not display
samples or floating derivatives—and creates the smallest forward sequence of
records under these rules:

- no position, velocity-feed-forward, or q-current-feed-forward signal reverses
  direction inside one record;
- record update count, segment ticks, and block ticks remain within the active
  profile;
- every position transition and record displacement remains within its exact
  per-axis bound; and
- both feed-forward signals remain inside their symmetric configured authority.

Splitting shifts the encoded Newton recurrence with checked integer arithmetic,
so it introduces no second approximation. Every emitted record is then replayed
through `ServoFiniteDifferenceSegment::validate` using the same limits as the
firmware.

Records are half-open. A record emits updates `0..update_count`; its state at
`update_count` is continuation authority. The next record emits that shared
state at update zero. The complete stream must begin with zero feed-forward and
return both feed-forward vectors exactly to zero. Core 1 emits one additional
terminal at-rest hold at the exclusive stream end. Dense update totals must
therefore remain strictly below `u32::MAX`, preserving nonzero contiguous
command identities for every update and the final hold.

## Cache and descriptor binding

`package_canonical_servo_program` supports one through four simultaneous axes.
It calls the firmware's capacity query and kind-3 encoder, chains block digests,
then decodes and replays every block through
`ServoFiniteDifferenceStreamValidator`. Terminal tick, Q31.32/Q2.30 state,
update total, and chain digest must reproduce the browser program before a
storage identity is returned.

The `MachinePartitionPolicy2` limits must exactly match the FOC-derived servo
profile:

- maximum block ticks;
- maximum segment ticks;
- aggregate Q31.32 position-delta authority;
- exact update period; and
- maximum updates per record.

The resulting V4 job descriptor selects `ServoFiniteDifference`, repeats the
last two dense-grid facts, and retains absolute Q31.32 initial positions. The
firmware's typed `prepare_servo` path requires the independently reconstructed
active profile again; the generic job path rejects kind 3.

## Bounded failures and current scope

The compiler separately bounds source spans, output records, examined dense
updates, coefficient precision, position error, and feed-forward error. It uses
fallible reservations and checked counters. A source that cannot fit these
bounds fails before immutable bytes exist.

Current code establishes the exact recurrence and cache boundary. Production
CAM still needs to derive these source recurrences from general Hyperpath
schedules and machine kinematics, and the browser UI still needs a servo job
inspector/editor. The portable firmware runner and complete FOC-axis simulator
already consume the same kind-3 blocks. Target-specific power-stage activation
and physical qualification remain later hardware milestones.

Because Hypercurve is an actively edited sibling, verification copies the
interface, firmware, CSGRS, and Hyper repositories into a content-checked
temporary sibling layout and builds that snapshot. It never formats, resets,
pins, or compiles the live Hypercurve worktree as a side effect of this path.
