# Dedicated browser control worker

The browser application now creates a module worker from the same optimized
WASM binary as the rendering realm. The worker has an explicit synchronous WASM
entry; it does not depend on the asynchronous eframe start hook to infer its
realm. A versioned, strict JSON envelope is used only across the browser's
UI/worker boundary. Device traffic remains the canonical authenticated binary
Alumina protocol.

## Ownership and redaction

For each UI-local connection identity, the worker exclusively owns:

- the canonical path-free HTTP(S) device origin;
- the HMAC/AP secret;
- the boot-nonce-bound HTTP/native-protocol session;
- the exact causal `DeviceClockModel`;
- a separate monotonic `RuntimeHealthModel`;
- a passive exact active-configuration observation model;
- one bounded public device identity, retained only after strict schema decode;
- a bounded contiguous `CapabilityDownloadMachine` and one-time publication
  state;
- one capability-selected `TelemetrySubscriptionMachine`, its exact canonical
  request, newest accepted event progress, and bounded publication state;
- at most one capability-selected `WaveformCaptureMachine`, its exact canonical
  configuration, and one-time retained-record publication state;
- the explicit Wi-Fi sampling/error policy; and
- at most 64 accepted causal heartbeat records.

Across all connections, the worker also exclusively owns at most one bounded
`LiveCachedJob`: immutable partition/manifest sources, retry-safe cache
machines, exact compiled session bindings, the distributed schedule
coordinator, pending start intent, and the latest terminal or recoverable
failure state.

Connection commands are bounded before I/O. Custom `Debug` output redacts the
secret, every Rust-owned secret buffer is overwritten when its request or live
session is dropped, and event schemas contain no credential field. Browser
structured cloning and JavaScript strings can still create implementation-owned
copies that Rust cannot prove erased, so this is credential minimization rather
than a claim of hardened secret storage.

The rendering realm receives complete replacement snapshots: connection label
and origin, generation, lifecycle, public boot ID, accepted/rejected counts,
consecutive failures, the latest conservative cycle interval, bounded causal
history, queue/deadline facts, and a redacted error. It never mutates a worker
clock model or treats GPU/render time as a machine timestamp. Schema v2 also
carries exact command/work/telemetry occupancies, service and real-time stack
epochs, sample counters, headroom, freshness, and a separate bounded health
failure diagnostic. Schema v3 adds capability phase, contiguous byte count,
stable complete identity, and a separate bounded capability failure diagnostic.
Schema v4 adds stable device/board/credential-provenance facts and the complete
one-shot waveform lifecycle, capture identity, exact range progress, and
separate bounded capture failure state. Schema v5 adds telemetry lifecycle,
subscription reference, newest accepted sequence/drop progress, isolated
failure state, and complete canonical telemetry-document events. The rendering
realm reconstructs the native AHLT/ASWM, capability, identity, telemetry, and
capture relationships and rejects an invalid snapshot before inserting it.
Schema V6 adds passive canonical active-configuration status and the complete
worker-owned cached-job lifecycle. Schema V7 retains that strict lifecycle and
adds `retained_complete`: a replacement owner may expose exact all-participant
terminal descriptor evidence and local start cycles only with no original UI
epoch and no new start authority. It does not retain a compatibility decoder
for prior schemas.

## Lifecycle and recovery

The worker opens authentication discovery with configuration digest zero,
strictly fetches the same origin's bounded public identity, immediately begins
sampling, and then probes automatically at the declared interval. The public
identity is descriptive rather than arm authority: its board ID and capability
identity must agree with the authenticated capability bytes, and its device ID,
boot ID, capability identity, configuration digest, and frequency must all agree
with a later canonical capture before the rendering realm admits it. One
operation per device may be in flight. Atomic replacement and disconnect use a
generation marker: a late asynchronous result from an erased/replaced session
cannot restore itself.

Transport ambiguity spends the native sequence, correlation, HMAC counter, and
clock probe identity. Exact-estimator rejections remain visible and retry with a
bounded exponential delay. Authentication metadata failures reopen discovery;
a validated changed boot resets the model and history before another sample can
be admitted.

After a successful heartbeat, the worker polls runtime health only when its
explicit 1–60 second minimum interval is due. Manual clock probes cannot bypass
that bound. A health transport or semantic failure retains the last valid
health evidence and its own consecutive-failure/error state without changing
clock phase, clock estimate, deadline state, or safety authority. A fresh
authenticated session clears health evidence because its boot provenance is not
yet known, while preserving the diagnostic that caused reauthentication; a
validated boot change clears both evidence and prior-boot health failures.

After each successful heartbeat, the worker may acquire at most four canonical
capability ranges while the health deadline remains independently bounded. Each
range is at most 240 bytes. Discovery uses a zero expected digest; every later
range uses the first accepted identity and exact contiguous offset. A transport
failure abandons only the pending side-effect-free range, so the next attempt
repeats its digest, offset, and byte bound. Capability failure state never
changes clock qualification or retained health evidence. A validated boot
change clears the old capability bytes and one-time publication state.

Once public identity, boot/clock evidence, and the complete capability agree,
the worker selects the first four sorted resources whose exact graph access is
`StableBooleanInput`. It constructs a latest-only, zero-configuration telemetry
subscription with a ten-hertz requested minimum period. Each successful
heartbeat advances at most one subscribe/status/event-poll operation through
the same authenticated session. A poll acknowledges only the newest complete
event already admitted by the worker; ambiguity repeats that exact request.
Sequence zero makes no acknowledgement claim, so a replacement worker can
reattach to the same exact subscription and receive retained evidence. A
telemetry failure changes only its own bounded error state unless authentication
must be reopened. A boot, stable identity, or capability change clears the
subscription and unpublished UI event.

The UI may request one immediate, input-only capture over one through four
strictly ordered resources. The worker, not the UI, reconstructs each resource
selector and requires `StableBooleanInput` authority in the complete capability
document. It then builds the canonical zero-configuration request with the
current device, boot, capability, and clock identities, a two-second maximum
duration, 64-transition retention, 168-byte ranges, and a generation/sequence
capture ID. Configure, explicit diagnostic arm, status reconciliation, and
side-effect-free exact range reads share the worker's authenticated session.
Ambiguous operations follow the transport-independent capture state machine;
capture failures do not change clock, health, capability, machine-arm, lease, or
output authority. Firmware retains a completed record until its exact
configuration reference is stopped. A repeat request therefore asks the old
machine to stop, retains at most one bounded pending request, reconciles an
ambiguous stop through status, and constructs the replacement only after the
old machine reaches `Stopped`. A changed boot or stable device identity clears
both active and pending capture state.

Firmware's replay window is global for one boot, so starting every replacement
browser session at counter one is invalid. Browser sessions instead seed the
authenticated counter with:

```text
floor(Date.now() milliseconds) * 1,000,000 + random subfield
```

This makes ordinary reload/reconnect counters advance while preserving exact
integer request counters and HMAC coverage. It is not a proof against host wall
clock rollback or simultaneous-session collision. A later multi-controller
browser qualification must either prove the deployment policy sufficient or
replace it with a bounded server-issued session/counter lease.

## Cached-job ownership and deterministic start

The rendering realm may stage one strict `WorkerCachedJobRequest` produced by
authoritative CAM. The worker independently decodes all manifest, descriptor,
upload-plan, content, participant, boot, capability, and configuration
relationships before retaining artifact bytes. Staging additionally requires
each live connection generation and stable device identity to match, a
clock-qualified session, an active job-authorized configuration, and an
explicit simulation-versus-hardware authority match.

Cache operations receive first choice on each 100 ms worker tick. Passive
heartbeats, health, configuration, capability, and telemetry still run on an
idle participant, but cannot starve the job. Partition publication always
precedes publication of the identical global manifest on each MCU; all cache
owners complete before preparation begins.

A start command records intent rather than assuming every participant happens
to be idle in that UI callback. Bound passive work pauses until all sessions are
idle and freshly qualified. Only then does the worker choose the shared future
UI epoch, map it through every exact affine clock model, bind distinct local
commit identities, and begin all-install-before-any-confirm orchestration.
Read-only status rounds carry the job through confirmation, abort-guard closure,
priming, observed local start, and exact completion. A polling interval may
legitimately observe `Primed` followed directly by `Complete`; the client admits
that reachable later state while continuing to reject identity substitution,
observation erasure, and true regression.

The rendering realm receives complete replacement job snapshots only. It can
request start, safe stop/abort/cancel, or terminal clear, but it cannot issue a
raw native storage or schedule operation. Hardware mode remains fail-closed
behind a non-simulated armable capability plus device-stored production
credential; the committed browser fixture is simulation-only.

Terminal clear is deliberately local to the worker/UI owner. The rendering
realm first retains and revalidates the complete terminal snapshot, sends the
strict versioned clear command, waits for the matching `job_removed` event, and
only then may stage another request. The next logical execution must carry a
different nonzero prepare ID; exact descriptor repetition remains an idempotent
firmware retry. Immutable partition and manifest content may be reused. The
device accepts the distinct descriptor only after its previous service,
realtime executor, and schedule are terminal and quiescent, so neither worker
clear nor Wi-Fi ordering becomes replacement authority.

## Operator boundary

The right-hand panel can add a labeled origin/passphrase, show worker readiness,
request an immediate probe, disconnect, and inspect clock/queue/deadline history.
The same panel shows exact queue depth/capacity/free counts, executor allocation,
low exclusion, painted/unpainted bytes, observed maximum use, minimum headroom,
scan/sweep counts, sample age, and RT freshness. It explicitly identifies the
partial boot epoch and incremental convergence. After capability completion it
also shows exact board/revision/chip/qualification, core assignment, memory,
resource ownership/hazards/graph exposure, aliases, licensed visual/hotspot
counts, HIL requirements, and the package's armable claim. It is deliberately
labeled diagnostic-only. For boards with capability-admitted Boolean inputs it
shows low-rate live lifecycle/reference/sequence/drop state, each sample's
value/provenance/quality/captured cycle/age, and up to 64 sampled logic lanes.
It can also request repeated 2 ms captures, show exact integer cycle/range
progress, render an edge trace, and inspect each channel level at a hover cycle.
All labels come from the same board capability used for authorization. The
machine workspace can now compile, stage, start, stop, and clear one cached job
at a time, then stage a distinct attempt without rebooting the devices, with
explicit simulation-only versus hardware authority and participant/cache/
schedule progress. It still provides no direct pin write, process-energy,
safety-reset, or bypass control. Neither a clock-qualified snapshot, stack
watermark, immutable capability document, simulated job, nor simulated trace
proves safety-chain freshness, physical timing, transient stack depth, or
sufficient production sizing.

## Browser smoke evidence

The release Trunk bundle contains `alumina-worker.js`, the generated JS module,
and one shared WASM artifact. On 2026-08-11, the bundle was served from a
localhost static server and loaded in headless Chromium with software WebGL.
The rendered operator panel reported:

```text
Control worker: ready (http://127.0.0.1:8097)
```

The server observed requests for the document, generated JS, WASM, and worker
module, and Chromium reported no application JavaScript/WASM error. This proves
module loading, explicit worker entry, ready-event decoding, and rendering-realm
supervision in that browser run. It did not connect to simulated HTTP firmware
or a physical MCU, and it does not qualify background throttling, AP loss,
reboot, latency spikes, or authenticated heartbeat traffic.

`tests/browser/worker-clock-harness.html` is the authenticated integration
seam. Served from the repository root, it creates the production worker without
eframe,
connects it to `alumina-sim-http` on localhost, and marks the DOM `passed` only
after at least five authenticated observations yield a qualified exact interval,
a schema-V6 health snapshot reports the exact expected queues, executor domains,
samples, and fresh RT witness, and exactly one complete capability-document
event matches both public and signed identity. The extra post-completion
heartbeat makes a duplicate document event observable. A waveform expectation
then requests GPIO22/32/33/35, requires one complete `ALMDIG01` event, exact
range completion, matching capture/generation/context, and zero waveform
failures. The `waveform-repeat` expectation then releases that retained record
through the production state machine, requests a second duration, and requires
exactly two distinct capture IDs and canonical 2,000- then 3,000-cycle headers;
malformed, excess, stale, or mismatched events fail the run. Separate
expectations retain a clock-qualified snapshot with an
isolated health error after a deliberately lost health response, or a clean
clock/health snapshot with an isolated capability error after a deliberately
lost first range, then require bounded recovery. This makes headless-browser
evidence inspectable without reading pixels from the canvas.

The telemetry expectation requires two complete canonical documents and exact
snapshot progress. It checks 176-byte `ALMTLS01` requests, 432-byte `ALMTEV01`
events, matching subscription IDs, four overview samples, and monotone event,
drop, and snapshot-cycle facts. On 2026-08-15, a fresh Chromium 147 run reached
events 1 and 2 with zero drops/failures. A second browser worker against the
same unchanged simulator boot first received the retained exact event 2 and
then advanced to event 3. Complete hashes, commands, and closed claims are in
the sibling `aluminafw/docs/evidence/M10-AUTHENTICATED-LIVE-TELEMETRY.md`.

The `cached-job` expectation configures two standalone simulator processes with
distinct stable device IDs. It waits for both exact capabilities, active
job-authorized configurations, and qualified clocks; injects a request compiled
through the production representative CAM path; and requires cache, prepare,
install, confirm, prime, observed start, and completion on both participants.
On 2026-08-15 the optimized bundle published 127,264 bytes per MCU and reached
global `complete` after 432 validated job snapshots with zero failures. The
shared UI epoch mapped to local cycles `80,884,539` and `79,719,862`. Complete
authority facts and closed physical claims are recorded in sibling
`aluminafw/docs/evidence/M10-BROWSER-CACHED-JOB-E2E.md`.

The `cached-job-repeat` expectation accepts exactly two strict requests. On
2026-08-15 it ran job IDs `2047934465` and `2047934466` consecutively on the same
two simulator boots and worker generations. The first attempt uploaded 127,264
bytes per participant and completed in 432 snapshots. After terminal capture
and an acknowledged worker clear, the second attempt reused the complete cache,
selected a fresh shared epoch, traversed every lifecycle phase, and completed in
60 snapshots without a reload, reconnect, simulator restart, or failure. A
following ordinary single-job run against the same simulator actors also passed
in 60 snapshots. The exact cycles, commands, and closed claims are in sibling
`aluminafw/docs/evidence/M10-REPEATED-CACHED-JOBS.md`.

The `cached-job-recovery` expectation accepts one strict request and requires at
least two independently observed transient failure states, later zero-failure
recovery, and the same fully valid terminal participant facts. On 2026-08-15,
one simulator discarded a successful applied `StoragePutChunk` response and the
other discarded a successful applied `JobCommit` response. The production
worker recovered through storage inspection and schedule status, respectively,
then completed both participants after 434 snapshots with zero terminal
failures. A fresh ordinary no-fault run passed afterward in 432 snapshots. The
exact epochs, cycles, selector contract, and closed claims are in sibling
`aluminafw/docs/evidence/M10-BROWSER-CACHED-JOB-RECOVERY.md`.

The stricter `cached-job-confirm-recovery` expectation requires exactly two
`confirming`-phase transport failures after both caches and commits are
installed. It accepts only locally confirmed-participant counts zero and one in
that order, followed by zero-failure recovery and complete terminal facts. On
2026-08-15 both simulator actors applied `JobConfirm` and discarded the
successful response. The worker reconciled each through `JobStatus`, completed
in 432 snapshots at a shared future epoch, and then passed a fresh-actor
ordinary no-fault regression. A separate same-prepare-ID attempt from a fresh
worker exposed the then-missing terminal identity seam. Exact results and the
original closed boundary are in sibling
`aluminafw/docs/evidence/M10-BROWSER-CACHED-JOB-CONFIRM-RECOVERY.md`.

The `cached-job-reattach` expectation replaces that unbounded boundary with a
strict schema-V7 terminal result. After an ordinary two-participant completion,
it installs the identical compiled request into a replacement worker while both
simulator actors and boots remain unchanged. The fresh owner performs an
all-participant read-only schedule-status round and may pass only as
`retained_complete`, with exact descriptor-token matches, complete cache and
schedule facts, retained local start cycles, `target_ui_ns = null`, no failures,
and no worker start command. On 2026-08-15 it passed in seven snapshots and
retained cycles `165,504,989` and `165,424,194` from the preceding 432-snapshot
ordinary run. Native request-level tests prove exact terminal discovery emits
only `JobStatus` and mixed complete/empty actors fault before mutation. See
`aluminafw/docs/evidence/M10-BROWSER-CACHED-JOB-REATTACHMENT.md`.

The `cached-job-abort-recovery` expectation waits for both future commits to be
globally `installed`, then sends one strict stop command before confirmation.
Both simulator actors apply `JobAbort` and discard the successful response. The
expectation requires exactly two one-failure `aborting` observations, locally
aborted counts `0 -> 1`, status reconciliation before each next mutation, and a
zero-failure all-participant `aborted` terminal. On 2026-08-15 it passed in 389
snapshots with local cycles `102,654,525` and `102,579,983`; a fresh-actor
ordinary completion regression passed in 419 snapshots. See
`aluminafw/docs/evidence/M10-BROWSER-CACHED-JOB-ABORT-RECOVERY.md`.

The distinct `cached-job-confirmed-abort-recovery` expectation waits until both
participants have granted future start authority and reported `confirmed`
before requesting stop. The same two applied abort responses are discarded and
must reconcile revoked-authority counts `0 -> 1 -> 2` before the guard. It
passed in 391 snapshots at local cycles `123,253,557` and `123,219,326`, ending
with both participants `aborted`, zero failure, and no error.

The fixture can deterministically add clock drift and request/response delay,
drop one selected control request, drop an initial run of control requests, or
reboot before a selected control request. It can also discard the first
successful response for one named canonical storage or schedule operation only
after the fixture has applied it. The harness has separate qualified and
conservative-rejection expectations so an excessive causal interval must remain
unusable instead of being mistaken for successful recovery.

At `alumina-interface` commit `0e0a53e` and sibling `aluminafw` simulator commit
`ba888db`, Chromium runs passed nominal, selected-response-loss, finite-outage,
reboot, and bounded-delay qualification cases. A deliberately excessive-delay
case passed only by retaining zero accepted samples and no estimate after three
round-trip rejections. The complete commands, observations, artifact hashes,
and deliberately closed claims are recorded in the sibling
`aluminafw/docs/evidence/M7-BROWSER-AUTH-HTTP-SIM.md` evidence file.

On 2026-08-14 the finalized release bundle and standalone simulator passed the
`expect=waveform-repeat` case in Chromium 147 over loopback only. The worker
reported seven accepted clock samples, the exact 3,531-byte TinyBee capability,
public device ID `ALUM-SIM:TINYBEE`, and two distinct 544-byte four-channel
captures with 2,000- then 3,000-cycle requested durations, 16 simulated
transitions each, exact stop/release/reconfigure progress, and zero capture
failures. Complete verification, artifact hashes, and closed physical claims
are recorded in sibling
`aluminafw/docs/evidence/M10-CAPABILITY-BOUND-WAVEFORM-WORKER-UI.md`.

## Runtime-health client seam

The window-free client now has a separate session-scoped
`RuntimeHealthModel` for firmware's authenticated, bodyless
`HealthSnapshot` operation. It independently decodes the fixed 124-byte body,
exposes exact command/work/telemetry occupancy and the two stack observations,
and distinguishes absent, present-stale, and present-fresh real-time reports.
It rejects response-cycle regression and any service or real-time epoch,
layout, counter, sample-cycle, or observed-headroom regression. A temporarily
absent real-time report does not erase the last monotonic witness used to check
the next present report.

The browser adapter supplies both window and worker fetch functions. They
require the same zero-configuration HTTP session already opened by the clock
worker, spend no request when a differently bound session is supplied, retain
the last valid health evidence after transport or semantic failure, and treat
firmware `Unsupported` as an explicit observation rather than fabricated zero
measurements.

`ControlWorkerRuntime` now schedules this seam beside successful heartbeats,
serializes it into strict schema-V6 `DeviceSessionSnapshot` values, and renders
integer facts in the live-device explorer. The window-free client remains
independently testable for native and `wasm32-unknown-unknown` without building
the CAD/CAM application or moving Hyper dependencies. Localhost browser evidence
covers the available-snapshot and lost-health-response recovery paths; no
physical MCU, ESP radio, WLAN, motor, output, or safety claim is made here.

## Authenticated capability seam

`CapabilityDownloadMachine` is a transport-independent, retry-safe assembler
for firmware's immutable `CapabilitiesGet` operation. It checks caller limits
before allocation, enforces exact status/body rules, binds every response to the
pending range and first accepted identity, retains only a contiguous prefix, and
does not expose completion until the full `ALMCAP04` document independently
decodes and hashes to that identity. Tests cover exact TinyBee reassembly,
identical ambiguous retry, identity substitution, preallocation limits, and
response-body/status rejection.

The browser adapter uses the same zero-configuration authenticated session as
clock and health. The worker publishes complete bytes once per generation only
after validation. JSON transfer is deliberately treated as untrusted: schema V7
validates the document again, and the UI validates and decodes it once more into
`BoardExplorerSnapshot`. Stale generations are rejected and disconnect removes
the admitted explorer. The localhost capability-loss run and complete evidence
are recorded in sibling
`aluminafw/docs/evidence/M10-AUTHENTICATED-CAPABILITY-WORKER-UI.md`.

## Capability-bound telemetry seam

The browser adapter drives `TelemetrySubscriptionMachine` through the same
HMAC/native-frame transport as clock, health, capability acquisition, and
waveform capture. The fixed 72-byte `ALMTPR01` poll carries the immutable
subscription reference and newest client-accepted event sequence. Empty success
means no retained event. A lost response abandons both pending layers without
advancing accepted evidence, making the next poll byte-identical.

Complete newly advanced bytes cross the worker boundary in a credential-free
`WorkerTelemetryDocument` alongside the exact subscription request. Introduced
in schema V5 and retained in current schema V7, the validator decodes both. The
worker selects resources, cadence, and encoded byte ceilings
from the authenticated `ALMDOV01` catalog rather than the graph palette. The
rendering supervisor decodes the transfer again, binds every context and
authority fact to the current live snapshot and board document, and admits only
catalogued passive stable Boolean inputs. It rejects stale/forked sequence, loss, or
snapshot-cycle histories and retains at most 64 exact events. The UI's sampled
logic plot is a passive, lossy display projection and provides no pin lease,
write, machine-arm, motion, or safety-reset mechanism.

The standalone simulator opts into deterministic overview production only for
an admitted subscription and marks every overview/sample simulated. Ordinary
fixtures and all ESP board compositions retain explicit provider policies; no
physical input sampler is inferred. An authenticated WebSocket can later carry
the same canonical events for higher-rate use, while the present polling path
is already an end-to-end authenticated live transport.

## Capability-bound waveform seam

The browser adapter now drives one `WaveformCaptureMachine` operation at a time
through the same HMAC/native-frame transport as clock, health, and capability
acquisition. Request construction requires configuration digest zero and spends
both HMAC and diagnostic pending state on ambiguity. The worker advances at most
eight waveform operations after one successful heartbeat so health and
capability work retain their independent bounds.

Complete bytes are emitted once per capture attempt as a credential-free
`WorkerWaveformDocument`. The schema validator decodes `ALMDIG01` and requires
configuration digest zero. The rendering supervisor decodes it again and binds
device ID, boot ID, capability identity, zero configuration, clock frequency,
capture ID, and every channel's stable Boolean input access to the current live
snapshot and admitted board document. A replacement capture, reboot, changed
stable identity, changed capability, generation change, or disconnect removes
stale displayed evidence. Screen projection is explicitly lossy; retained
cycles and transitions remain integers and never flow back into control.

A completed capture is not locally overwritten. The worker first drives the
old machine's retry-safe stop/status reconciliation to `Stopped`, then installs
the single pending request using the authority current at that moment. This
prevents a retained firmware record from turning a repeat UI action into a
conflicting configure and prevents stale boot or capability facts from being
copied into the replacement request.

The standalone simulator opts into a deterministic immediate-capture provider.
Ordinary tests do not receive that provider implicitly, and firmware board
compositions remain unsupported until a separately qualified hardware capture
task is attached. Physical telemetry and capture providers remain separate HIL
work even though both simulator-backed browser paths are now connected.
