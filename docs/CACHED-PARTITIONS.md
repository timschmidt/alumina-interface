# Canonical cached-partition boundary

The browser compiler packages canonical integer motion using the real sibling
`aluminafw` schemas. It does not own a second block, storage, manifest, or
`JobPrepare` representation.

## One-way artifact pipeline

```text
certified CanonicalPathProgram2 or CanonicalScheduledProgram2
    -> firmware ExecutionSegment<2>
    -> firmware capacity + horizon partitioning
    -> chained 512-byte ExecutionBlock values
    -> independent MotionStreamValidator replay
    -> SHA-256 MachineJobPartition object
    -> ordered content-addressed storage chunks + manifest digest
    -> UploadPlan / ChunkUploadHeader
    -> boot-local JobDescriptor only when preparing a live device
```

Every execution block repeats the stream, capability, and active-configuration
identities and chains the previous block digest. Block capacity comes from
`alumina_machine_ir::maximum_motion_segments_per_block`; the interface does not
copy the schema's private record-size formula. Partitioning is greedy and
deterministic, constrained by both that record capacity and the caller's
firmware block-horizon limit.

Before returning an artifact, the compiler decodes every emitted block and
replays it through `MotionStreamValidator` with the same independent checks used
by firmware. Terminal tick, chain digest, relative displacement, and absolute
final lattice position must reproduce the canonical program. A too-long block,
bad identity, discontinuity, step limit, counter overflow, or terminal mismatch
fails before storage identities are exposed.

The machine-bound scheduled path adds two gates before this block replay.
Program, partition, canonical configuration, and immutable capability identities
must agree exactly, and the scheduled segments must already have passed the
allocation-free production `StepperExecutor` preflight using the configuration's
timer, pulse, setup/hold, maximum-frequency, and output-quantum facts. Packaging
rechecks its terminal position and tick; it does not infer electrical validity
from the more general machine-block validator.

## Storage and preparation identities

The complete block concatenation is one
`ObjectKind::MachineJobPartition`. `alumina-storage` computes its content ID,
per-chunk IDs, and canonical ordered-manifest ID. Storage chunks deliberately do
not need to align to 512-byte execution blocks; firmware's `PartitionAssembler`
reconstructs exact owned blocks across arbitrary verified chunk boundaries.

The immutable artifact retains:

- exact block bytes and count;
- stream, capability, and configuration identities;
- block and segment admission limits;
- object and ordered-manifest SHA-256 identities;
- resumable `UploadPlan` and each `ChunkUploadHeader`;
- initial/final machine-lattice positions; and
- independently replayed terminal tick, displacement, and chain digest.

The upload transaction ID identifies a resumable storage mutation; it is not
content authority. The boot-local nonzero `prepare_id` is intentionally absent
from cached bytes. `CanonicalMachinePartition2::job_prepare_body` adds it later
and encodes the real `alumina_job::JobDescriptor` after a live device boot is
known.

## Deterministic fixture

The current line/arc/cubic fixture uses the exact-CAM policy documented in
`EXACT-TOOLPATH.md`, a maximum 450,000-tick block horizon, 1,000 steps per
segment, and 700-byte storage chunks. It produces:

```text
partition_blocks=20
partition_bytes=10240
storage_chunks=15
maximum_observed_block_ticks=449740
partition_sha256=49e4292876fbf4d0d2c83b1adbf5f6d8069faf3603fd6243bed06786a4f7401e
chunk_manifest_sha256=6aeb0b3fcf14b087d02683d956fd215488a89bf57b191526f189cd379c00376c
terminal_block_sha256=ec0b6b3c82d61f23180c32f709676e93f7402e03e37c4bf177348d95f91b5285
```

Tests compile the fixture twice and require identical partition/chunk bytes and
identities. They also pass every chunk through the real `UploadCoordinator`
begin/verify/record/finalize/publication lifecycle and through arbitrary-split
block assembly. These are deterministic native development facts; renewed WASM
and production-bundle evidence is recorded separately.

Two owned partition fixtures now feed the shared canonical global manifest.
That later boundary preserves these local object/manifest/terminal identities;
it does not reopen, reinterpret, or concatenate execution records. See
`GLOBAL-JOB-MANIFEST.md`.

The machine-bound line/arc fixture additionally replays the packaged bytes with
`alumina-sim::replay_cached_stepper_partition`. That event-level path uses the
real `JobDescriptor`, first rehashes the complete immutable object, then uses
`RealtimeJob` and `CachedStepperExecutor`, advances to each exact deadline,
acknowledges owned blocks in order, and requires terminal tick, position, step
counts, finish cycle, and block digest to agree. Canonical
`ALMEVD01` evidence then binds the exact-rational source, configuration,
capability, budgets, executor facts, and content-addressed partition. See
`EXACT-MACHINE-SCHEDULING.md`.

## Still outside this boundary

- The older general-curve deterministic fixture still uses explicit sentinel
  identities. The machine-bound scheduled path uses the canonical configuration
  and capability digests, but a live device must still authenticate the
  capability and report the configuration as durably active before execution.
- The window-free compiler remains independent of transport. Its artifacts now
  have a browser Wi-Fi upload/retry/finalize and exact cache-reconciliation
  consumer, plus a headless clock/prepare/install/confirm coordinator. Visible
  workflow wiring and complete worker-owned cached-job driving remain open.
- Two-axis line/arc motion now has exact-stop lookahead, certified jerk
  scheduling, configuration-derived calibration/following bounds, and
  production executor preflight. General curves still use the separate
  constant-feed chord compiler; blends, broader kinematics, and qualified
  hardware timing remain open.
- Passing software replay does not qualify SD media, a board, or physical
  motion. No board is flashed or energized by packaging or its tests.
