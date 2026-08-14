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
- a bounded contiguous `CapabilityDownloadMachine` and one-time publication
  state;
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
The rendering realm reconstructs the native AHLT/ASWM and capability
relationships and rejects an invalid snapshot before inserting it.

## Lifecycle and recovery

The worker opens authentication discovery with configuration digest zero for
the clock operation, immediately begins sampling, and then probes automatically
at the declared interval. One operation per device may be in flight. Atomic
replacement and disconnect use a generation marker: a late asynchronous result
from an erased/replaced session cannot restore itself.

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
labeled diagnostic-only. There are no arm, motion, process energy, or
safety-reset controls in this checkpoint. Neither a clock-qualified snapshot,
stack watermark, nor immutable capability document proves safety-chain
freshness, physical timing, transient stack depth, or sufficient production
sizing.

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
a schema-v3 health snapshot reports the exact expected queues, executor domains,
samples, and fresh RT witness, and exactly one complete capability-document
event matches the snapshot identity. The extra post-completion heartbeat makes
a duplicate document event an observable failure. Separate expectations retain
a clock-qualified snapshot with an isolated health error after a deliberately
lost health response, or a clean clock/health snapshot with an isolated
capability error after a deliberately lost first range, then require bounded
recovery. This makes headless-browser evidence inspectable without reading
pixels from the canvas.

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
serializes it into strict schema-v3 `DeviceSessionSnapshot` values, and renders
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
after validation. JSON transfer is deliberately treated as untrusted: schema v3
validates the document again, and the UI validates and decodes it once more into
`BoardExplorerSnapshot`. Stale generations are rejected and disconnect removes
the admitted explorer. The localhost capability-loss run and complete evidence
are recorded in sibling
`aluminafw/docs/evidence/M10-AUTHENTICATED-CAPABILITY-WORKER-UI.md`.
