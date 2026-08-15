# Authenticated Wi-Fi cache delivery

The browser client uses the firmware's native protocol and storage schemas
directly. It does not introduce JSON job commands, G-code transport, a second
manifest format, or compatibility routes.

## Session boundary

The WASM adapter first performs a cache-disabled, credential-omitting,
redirect-rejecting CORS GET of `/api/v1/auth`. The response decoder rejects an
unknown field, authentication scheme, missing origin binding, changed replay or
rate policy, changed proof-header names, zero nonce, and any nonce that is not
exactly 32 lowercase hexadecimal characters.

In a secure browser context, each fetch dictionary is annotated with
`targetAddressSpace: "local"` so current Chromium Local Network Access can ask
the user for LAN permission and relax mixed-content handling for an HTTP device.
An HTTP UI served by a device is not a secure context, so the adapter does not
pretend that it can request this permission there. Cross-origin local-to-local
behavior and a production secure UI origin remain explicit browser-qualification
items rather than firmware safety assumptions.

Discovery is a fixed 260-byte schema with a canonical `Content-Length`; the
browser rejects a missing or changed declaration before allocating the response
buffer, and the decoder independently requires the exact received length.

`AuthenticatedHttpSession` binds the resulting boot nonce, active configuration
digest, and exact canonical `window.location.origin`. Every native request is
framed before I/O and signed with the firmware's shared HMAC V2 implementation.
The HTTP counter, native sequence, and correlation are spent when the request is
constructed. A response becomes authoritative only after its counter, status,
media, exact bytes, origin-bound response HMAC, frame sequence, operation,
correlation, and configuration digest all validate.

Browser JavaScript cannot set `Origin`; immediately before each fetch the
adapter compares the browser-generated document origin with the origin retained
by the session. The device target origin is a separate validated HTTP(S) URL.
This distinction permits a workspace served by one MCU to address peers without
mistaking the peer URL for the calling security origin.

The live application now places these sessions in a dedicated module worker.
Its bounded commands, credential ownership, retry policy, and redacted clock
snapshots are specified in
[`LIVE-CONTROL-WORKER.md`](LIVE-CONTROL-WORKER.md). Cache-delivery and
distributed-schedule machines are now driven by the same worker. The UI hands
over one independently validated immutable request; it never receives the HMAC
session or sends raw storage/job operations.

## Retry-safe storage protocol

`CacheUploadMachine` starts every transaction with `StorageInspect` for the
exact `(StoredObject, chunk-manifest)` publication. A miss proceeds through the
real firmware operations:

```text
StorageInspect -> StorageBeginUpload -> StoragePutChunk* -> StorageFinalize
```

Begin/resume accepts only exact durable prefix progress. Each local chunk header,
length, upload identity, index, and SHA-256 is checked before framing. A lost or
ambiguous fetch spends the HMAC/native counters, abandons the pending action, and
returns the upload state to `StorageInspect`; it never repeats an assumed-safe
mutation. Consequently a lost chunk response resumes after the device's durable
prefix, while a lost finalize response resolves by observing the published
content identity without sending a second finalize.

`ParticipantCacheDelivery` binds both publications for one sorted global-job
participant before network I/O. It reconciles that MCU's executable partition
to completion, then reconciles the identical global manifest using an
independent device-local upload ID. `prepare_global_cache_delivery` validates
every participant machine before returning any, so a malformed participant or
storage policy cannot yield a partially constructed UI readiness table.

## Live worker lifecycle

Worker schema V7 carries one bounded cached-job owner. A staged request carries the
complete canonical global manifest and each sorted participant's descriptor,
partition upload plan/bytes, and independent manifest upload plan. Before any
I/O, the worker reconstructs all content/publication identities, manifest
records, participant order, network policy, boot bindings, capability digests,
and active configuration digests.

Each participant progresses through partition then global-manifest
reconciliation. Only after every publication is authoritatively observed does
the worker prepare all actors. A user start request is retained until every
bound session is idle, still clock-qualified, and still matches its compiled
device, boot, capability, and job-authorized configuration. The shared future
browser epoch is selected only then. Exact affine clock models produce each
local commit; all installs precede all confirmations. Status rounds retain the
abort guard, priming, observed start, and completion evidence.

Passive heartbeat, health, configuration, capability, and telemetry work can
continue during cache transfer. Cached-job work receives first choice at each
100 ms worker tick so coincident passive deadlines cannot starve it. While a
start intent waits for a transiently busy session, passive work for its bound
participants pauses; this guarantees progress without using a stale early
start epoch.

The worker exposes complete replacement snapshots with cache artifact/phase,
accepted/total bytes, next missing chunk, participant schedule phase, local
start cycle, global phase, shared epoch, and an isolated bounded failure. The
rendering realm validates every snapshot again. Hardware mode additionally
requires a non-simulated armable capability and device-stored production
credential; the representative browser fixture deliberately requests
simulation-only authority.

Only one attempt is owned at a time. After a valid terminal snapshot, a strict
`clear_cached_job` releases the rendering/worker owner and emits `job_removed`;
it does not erase device evidence or storage. A subsequent request uses a new
prepare ID, may reuse identical content-addressed publications, and selects a
fresh future epoch. Firmware/simulator replacement remains transactional and
closed until the previous actor is terminal and quiescent. Exact descriptor
repetition continues to mean idempotent retry, while candidate validation
failure preserves the prior terminal report.

## Present evidence boundary

Native tests exercise HMAC interoperability, strict challenge decoding,
counter loss, chunk/finalize loss, corrupt progress/chunks, schedule ambiguity,
collapsed but monotone status polling, and terminal worker snapshot mutation.
An optimized Chromium run against two independent loopback simulator processes
uploaded 127,264 bytes per MCU, prepared, installed, confirmed, primed, observed
both simulated latches, and reached completion with zero failures. The complete
record is in sibling
`aluminafw/docs/evidence/M10-BROWSER-CACHED-JOB-E2E.md`.

A second qualification ran two distinct attempts without rebooting or
reconnecting those participant sessions. The second attempt reused all cached
bytes and completed the full lifecycle at a new synchronized epoch. See sibling
`aluminafw/docs/evidence/M10-REPEATED-CACHED-JOBS.md`.

An operation-specific fault qualification then discarded one successful
applied chunk response and one successful applied schedule-commit response on
different participants. The live worker exposed both transient failures,
reconciled them through storage inspection and schedule status, and reached a
valid two-participant terminal snapshot; a fresh no-fault regression also
passed. See sibling
`aluminafw/docs/evidence/M10-BROWSER-CACHED-JOB-RECOVERY.md`.

Both participants were then qualified with their first successful applied
`JobConfirm` response discarded. A dedicated expectation required exactly two
confirmation failures and local confirmed counts `0 -> 1` before recovery and
terminal completion. Fresh-actor no-fault behavior also passed. That run exposed
the then-open fresh-owner terminal identity seam; see sibling
`aluminafw/docs/evidence/M10-BROWSER-CACHED-JOB-CONFIRM-RECOVERY.md`.

A later schema-V7 qualification completed one ordinary attempt, replaced the
browser worker without restarting either simulated MCU, and restaged the
identical compiled request. An initial read-only status round matched each
retained boot/descriptor token and returned `retained_complete` in seven
snapshots with the original local cycles, no UI epoch, no failures, and no new
start command. Native tests additionally prove only status operations are
emitted and a complete/empty participant split faults before mutation. See
sibling
`aluminafw/docs/evidence/M10-BROWSER-CACHED-JOB-REATTACHMENT.md`.

Still open are hardened credential persistence, physical browser-to-ESP Wi-Fi,
real SD media, background-tab qualification, lost abort, nonterminal/crash
reattachment and durable browser job persistence, broader packet
disorder/outage cases, live TinyBee/T-Deck Pro cached starts, electrical
simultaneity measurement, and every physical motion/safety qualification claim.
