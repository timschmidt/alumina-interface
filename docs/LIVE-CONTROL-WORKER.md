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
- one bounded public device identity, retained only after strict schema decode;
- a bounded contiguous `CapabilityDownloadMachine` and one-time publication
  state;
- at most one capability-selected `WaveformCaptureMachine`, its exact canonical
  configuration, and one-time retained-record publication state;
- the explicit Wi-Fi sampling/error policy; and
- at most 64 accepted causal heartbeat records.

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
separate bounded capture failure state. The rendering realm reconstructs the
native AHLT/ASWM, capability, identity, and capture relationships and rejects an
invalid snapshot before inserting it.

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
can request repeated 2 ms captures, show exact integer cycle/range progress,
render an edge trace, and inspect each channel level at a hover cycle. Channel
labels come from the same board capability used for authorization. There are no
machine-arm, motion, output, process-energy, or safety-reset controls in this
checkpoint. Neither a clock-qualified snapshot, stack watermark, immutable
capability document, nor simulated trace proves safety-chain freshness, physical
timing, transient stack depth, or sufficient production sizing.

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
a schema-v4 health snapshot reports the exact expected queues, executor domains,
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

The fixture can deterministically add clock drift and request/response delay,
drop one selected control request, drop an initial run of control requests, or
reboot before a selected control request. The harness has separate qualified
and conservative-rejection expectations so an excessive causal interval must
remain unusable instead of being mistaken for successful recovery.

At `alumina-interface` commit `0e0a53e` and sibling `aluminafw` simulator commit
`ba888db`, Chromium runs passed nominal, selected-response-loss, finite-outage,
reboot, and bounded-delay qualification cases. A deliberately excessive-delay
case passed only by retaining zero accepted samples and no estimate after three
round-trip rejections. The complete commands, observations, artifact hashes,
and deliberately closed claims are recorded in the sibling
`aluminafw/docs/evidence/M7-BROWSER-AUTH-HTTP-SIM.md` evidence file.

On 2026-08-14 the finalized release bundle and standalone simulator passed the
`expect=waveform-repeat` case in Chromium 147 over loopback only. The worker
reported seven accepted clock samples, the exact 3,435-byte TinyBee capability,
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
serializes it into strict schema-v4 `DeviceSessionSnapshot` values, and renders
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
does not expose completion until the full `ALMCAP02` document independently
decodes and hashes to that identity. Tests cover exact TinyBee reassembly,
identical ambiguous retry, identity substitution, preallocation limits, and
response-body/status rejection.

The browser adapter uses the same zero-configuration authenticated session as
clock and health. The worker publishes complete bytes once per generation only
after validation. JSON transfer is deliberately treated as untrusted: schema v4
validates the document again, and the UI validates and decodes it once more into
`BoardExplorerSnapshot`. Stale generations are rejected and disconnect removes
the admitted explorer. The localhost capability-loss run and complete evidence
are recorded in sibling
`aluminafw/docs/evidence/M10-AUTHENTICATED-CAPABILITY-WORKER-UI.md`.

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
task is attached. Live telemetry still requires an event transport such as the
planned authenticated WebSocket path and is not part of this checkpoint.
