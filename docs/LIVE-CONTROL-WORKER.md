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
clock model or treats GPU/render time as a machine timestamp.

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
It is deliberately labeled diagnostic-only. There are no arm, motion, process
energy, or safety-reset controls in this checkpoint. A clock-qualified snapshot
does not prove safety-chain freshness or physical timing.

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
