# Canonical global multi-MCU job

The interface uses `alumina-job`'s allocation-free canonical manifest schema.
It does not define a browser-only participant map or JSON execution manifest.

## Owned compilation result

`CanonicalGlobalJob2` retains both sides of the binding:

- every owned `CanonicalMachinePartition2`, including its exact execution
  blocks, storage chunks, identities, limits, and terminal replay facts; and
- the shared `ALMJMF02` manifest object whose participant records name those
  exact facts.

Inputs may arrive in any discovery order. The compiler sorts them by stable
`DeviceId` before calling the firmware schema. Duplicate devices, duplicate
streams, zero identities, malformed partition layout, nonzero unused-axis
slots, and local/global duration disagreement fail before manifest bytes are
returned.

The manifest's global duration is an exact rational
`duration_ticks / global_timebase_hz`. Each participant carries
`local_end_tick / local_timer_hz`; the shared decoder proves equality by checked
`u128` cross multiplication. It does not compare rounded seconds or browser
clock values. Runtime device-cycle epochs remain a later boot-bound scheduling
operation and never enter cached stream ticks.

The canonical object SHA-256 becomes `global_job_digest`. A separate
domain-separated SHA-256 over the complete ordered participant records becomes
`participant_set_digest`. These are the exact fields already consumed by
firmware `JobCommit`; the selected participant record supplies its local
partition digest.

## Exact shared scheduled-job compiler

`compile_shared_scheduled_global_job` is the production-shaped browser
boundary for schedule-derived participants. Its ordering is fixed:

1. validate stable device/board/resource/safety identities;
2. select the smallest common exact timer-lattice factor by complete
   all-participant production preflight;
3. replay each selected stream against its retained exact ideal-time/step-point
   carrier;
4. construct and independently replay every immutable participant partition;
5. build canonical `ALMSYN01` evidence over the exact derivations, complete
   factor-search trace, selected streams, and partition identities; then
6. derive global timer frequency, duration, and `synchronization_digest` from
   those results before constructing `ALMJMF02`.

The caller supplies a `SharedGlobalJobCompilePolicy2` template with those three
derived fields set to zero. Nonzero placeholders reject, so stale timing facts
cannot masquerade as authority. The compiler writes the `ALMSYN01` digest into
the global synchronization field and every participant's timing/error evidence
field. Board, resource, safety, source, build, configuration, and coordinate
identities remain explicit nonzero caller facts and are committed by the final
manifest.

Shared timer-lattice V1 requires one exact cumulative ideal event grid, local
timer frequency, and output quantum across participants. This makes every
selected local terminal tick identical and satisfies the existing manifest's
exact duration proof without adding a float, tolerance, or second wire schema.
Mixed clocks and event grids are rejected pending an explicit synchronization-
marker/idle model.

`ALMSYN01` is a 104-byte compact record containing the streamed
`ALMSRT01` digest and length plus the selected factor/search/timer/terminal
summary. Replay reconstructs all upstream source/metric/approximation/planner/
lowering digests, candidate outcomes, selected ticks/segments/preflights, and
partition identities. Corruption, participant substitution, reordered input,
an inconsistent binary-search trace, a non-timing rejection, or terminal
divergence fails before the global manifest is returned.

## Storage

The same global manifest must be published on every participant. Object bytes
and content identity remain identical, while
`CanonicalGlobalJob2::upload_plan_for` permits a different nonzero resumable
transaction ID per device. Each plan and chunk prefix is still a real
`alumina-storage` type. Tests replay one complete manifest through
`UploadCoordinator` under a second transaction identity.

The current two-participant fixture emits:

```text
global_participants=2
global_manifest_bytes=1312
global_manifest_chunks=2
global_job_sha256=2626778741a7046fd1957371132ed44c54790ce5b7f7b4c145930af4f93ddd9c
participant_set_sha256=d26ddc63881977582a1a3138c676be9e60a95db24fd923480a0706021977217a
global_chunk_manifest_sha256=546d614a553733fb44cdc3af4900b348c7713f030888832d749e6433ae296cf2
```

The fixture deliberately supplies participants in reverse discovery order and
requires repeated compilation to produce identical sorted bytes and digests.
Both local partitions contain the same harmless line/arc/cubic motion fixture,
which makes this a synchronization/identity example rather than a useful
machine split.

The Machine/CAM workspace additionally constructs a two-participant
schedule-derived fixture through the new shared compiler. Both identical
TinyBee profiles accept factor `4096/4096`, so one complete round performs two
production replays and both partitions end at tick 9,639,280. The fixture
retains:

```text
shared_evidence_magic=ALMSYN01
shared_outer_bytes=104
shared_transcript_bytes=218287
participant_partition_bytes=125952,125952
global_manifest_bytes=1312
```

The UI displays the selected factor, replay counts, common terminal tick,
shared evidence/transcript identities, manifest identity, and each canonical
participant package. All are offline artifacts; no network or device operation
is initiated.

## Open authority boundaries

- The older general-motion fixture still uses explicit nonzero sentinel
  identities. The schedule-derived Machine/CAM fixture derives source,
  planner/lowering, configuration, duration, and shared timing identities from
  retained artifacts, while its offline board/resource/safety/build fixture
  identities remain deterministic development values. Live compilation must
  replace them with authenticated board, active-configuration, build, resource,
  coordinate, and safety evidence.
- The compiler does not yet partition axes/resources from one global machine
  graph; the caller currently supplies complete participant packages.
- Browser Wi-Fi transport now strictly authenticates and reconciles each local
  partition before publishing this identical manifest on that MCU. Credential
  UI, live-browser/device qualification, prepare receipts, clock fits,
  commit/confirm/abort orchestration, and recovery UI remain open.
- The manifest proves identity and exact declared duration, not physical clock
  synchronization, start-edge spread, following error, or safe distributed
  stopping. Those require simulation, HIL, and hardwired safety evidence.
