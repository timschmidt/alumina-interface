# Canonical global multi-MCU job

The interface uses `alumina-job`'s allocation-free canonical manifest schema.
It does not define a browser-only participant map or JSON execution manifest.

## Owned compilation result

`CanonicalGlobalJob2` retains both sides of the binding:

- every owned `CanonicalMachinePartition2`, including its exact execution
  blocks, storage chunks, identities, limits, and terminal replay facts; and
- the shared `ALMJMF01` manifest object whose participant records name those
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
global_job_sha256=acd6bb77c405c770ef4cc75a7d5423f84bcbab440a39c71707d5701d38714eae
participant_set_sha256=bf496a0af361742e00968605947c1fbde338eaf9db0862206cade1fa49739e4e
global_chunk_manifest_sha256=2d84df13947cfb73f0ded35a559c98107ffd9284ab6a49f00cdb1785bf355311
```

The fixture deliberately supplies participants in reverse discovery order and
requires repeated compilation to produce identical sorted bytes and digests.
Both local partitions contain the same harmless line/arc/cubic motion fixture,
which makes this a synchronization/identity example rather than a useful
machine split.

## Open authority boundaries

- Fixture source/compiler/interface/policy/machine/coordinate/safety/resource/
  error digests are explicit nonzero sentinels. Production compilation must
  derive them from canonical source, authenticated capabilities and active
  configuration, compiler/build identity, and retained evidence objects.
- The compiler does not yet partition axes/resources from one global machine
  graph; the caller currently supplies complete participant packages.
- Browser Wi-Fi transport now strictly authenticates and reconciles each local
  partition before publishing this identical manifest on that MCU. Credential
  UI, live-browser/device qualification, prepare receipts, clock fits,
  commit/confirm/abort orchestration, and recovery UI remain open.
- The manifest proves identity and exact declared duration, not physical clock
  synchronization, start-edge spread, following error, or safe distributed
  stopping. Those require simulation, HIL, and hardwired safety evidence.
