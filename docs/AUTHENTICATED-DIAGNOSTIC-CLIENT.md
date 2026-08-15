# Authenticated diagnostic client

`alumina-interface-client::diagnostics` owns transport-independent telemetry and
digital-capture reconciliation. It uses the same canonical bodies in native
tests, browser/WASM builds, the authenticated host fixture, and eventual device
traffic. It does not preserve an older Alumina interface or G-code API.

## Subscription state

`TelemetrySubscriptionMachine` owns the complete canonical `ALMTLS01` request
and its SHA-256 reference. It has explicit subscribing, status-reconciliation,
active, unsubscribing, and terminal phases. A lost subscribe/unsubscribe result
is never guessed: the next operation is `TelemetryStatus` with the exact session
reference.

An accepted event must pass envelope and complete `ALMOVW01` validation, match
the request's device/boot/capability/configuration/clock context and exact
resource list, and advance sequence, cumulative loss, and snapshot cycle
monotonically. Consecutive accepted snapshots must also respect the
subscription's minimum device-cycle period. An exact replay of the newest event
is identified as a duplicate without changing progress. A same-sequence fork,
counter regression, cycle regression, or early sample is rejected.

While active, `TelemetryPoll` carries the exact subscription reference and only
the newest event sequence that the client has completely validated. The service
retains the current latest-only event until that exact sequence is acknowledged.
A lost response therefore repeats the same request without consuming unseen
evidence. Sequence zero makes no acknowledgement claim, allowing a reconstructed
page/worker to reattach to the same subscription and receive its retained event.

## Capture state

`WaveformCaptureMachine` owns the complete canonical `ALMWCF01` configuration
and its SHA-256 reference. Configure, arm, completion polling, range download,
and stop have explicit reconciliation phases. An ambiguous mutation becomes an
exact status query. An ambiguous range read repeats the same digest/offset/
maximum-length request.

Only a contiguous range at the expected offset is appended. Every range digest
and complete-record identity must match status. After the final byte, the client
independently decodes the full `ALMDIG01`, binds it back to the configuration,
and verifies its SHA-256 before exposing `record()`. A final validation failure
clears partial bytes and restarts at offset zero rather than retaining ambiguous
evidence.

## Live target-context prerequisite

The production worker now acquires the immutable `ALMCAP02` target document
before later live resource diagnostics can be admitted. Its separate
`CapabilityDownloadMachine` requests only contiguous, at-most-240-byte ranges,
uses a zero expected digest for discovery, freezes the returned identity, and
repeats the identical range after ambiguous transport loss. It bounds declared
length before allocation and exposes bytes only after complete canonical decode
and SHA-256 validation.

Strict schema v5 carries progress separately from complete capability,
telemetry, and capture documents.
The rendering realm revalidates the document and identity before constructing
the board-name-independent explorer. This supplies immutable resource context;
the worker may subsequently select only capability-admitted stable Boolean
inputs for passive telemetry or capture. Descriptive resources still do not
become allowed operations, and neither path obtains a diagnostic lease or
output authority.

## Live browser telemetry

The WASM adapter provides window- and worker-scope one-step drivers for the
subscription lifecycle and authenticated event poll. Both require the existing
zero-configuration HMAC session. Request construction, fetch ambiguity, device
status, canonical decoding, and event-order failures remain typed and isolated
from the clock, health, capability, and capture state machines.

After public identity, authenticated boot/clock evidence, and the complete
capability document agree, the production worker chooses the first four sorted
`StableBooleanInput` resources. It constructs a latest-only subscription at a
requested ten updates per second using the exact device, boot, capability,
zero-configuration, and frequency context. The UI cannot supply raw pins,
context, subscription identity, event bounds, or acknowledgement progress.

Schema v5 transfers each newly advanced complete `ALMTEV01` event together with
the immutable `ALMTLS01` request used to validate it. The schema validator
decodes both; the rendering supervisor decodes them again and binds the event to
the live connection generation, public device identity, authenticated boot and
frequency, admitted capability digest, subscription ID/digest, and stable
Boolean input access. It keeps at most 64 exact events. Reboot, generation,
identity, capability, subscription, lifecycle, or disconnect changes erase
incompatible evidence.

The live panel shows exact lifecycle, subscription reference, sequence/drop
progress, provenance, quality, captured cycle, and sample age. It also projects
the bounded Boolean history into sampled logic lanes. Only screen coordinates
are lossy; canonical event bytes and device cycles remain integers and cannot
flow back into control.

## Live browser capture

The WASM adapter provides both window- and worker-scope one-step drivers for the
capture state machine. They require the existing zero-configuration HMAC
session, preserve typed request/session/fetch/diagnostic failures, abandon both
pending transport and diagnostic operations after ambiguity, and otherwise
admit only an authenticated canonical response.

The production worker constructs the complete configuration itself after
reconciling strict public identity with the signed capability document and
authenticated boot/clock facts. UI input is limited to one through four ordered
resource selectors and a bounded duration. Every selector must be present in
the exact capability graph palette as `StableBooleanInput`; the worker supplies
all device, boot, capability, configuration, frequency, capture-ID, retention,
range, trigger, and deadline fields. It explicitly requests diagnostic arm,
which is distinct from and cannot create machine-arm, resource-lease, or output
authority.

Schema v5 transfers complete capture bytes once per attempt. Both the schema
validator and rendering supervisor decode `ALMDIG01`; the supervisor then binds
the record again to current device, boot, capability, zero configuration,
frequency, capture ID, and resource authority. Reboot, device/capability change,
new capture identity, generation replacement, and disconnect remove stale
rendering evidence.

Firmware retains a completed record until an exact stop. If the operator asks
again, the worker retains at most one bounded pending request, drives the old
machine's retry-safe `WaveformStop`/status reconciliation until it reaches
`Stopped`, and only then rebuilds the replacement configuration from current
identity, capability, boot, and clock authority. A reboot or stable-device
change drops both active and pending requests rather than carrying authority
across epochs.

## Offline transport evidence

The tests drive these state machines through three progressively wider seams:

1. the native service dispatcher in memory;
2. the production-format origin-bound HMAC HTTP fixture; and
3. an ephemeral `127.0.0.1` TCP listener carrying real HTTP bytes.

The tests deliberately lose mutation, event-poll, and range responses,
substitute identity, replace latest-only events, drop a live chunk, and then
recover both exact telemetry and the authoritative 512-byte deterministic
TinyBee capture through four 168-byte-or-smaller ranges. The same client crate
is checked for `wasm32-unknown-unknown`.

The capture machine is now connected to the visible worker and to an opt-in
deterministic immediate provider in the standalone simulator. A loopback
Chromium run completed the configure/diagnostic-arm/status/range lifecycle and
rendering-realm admission for a 544-byte simulated record. The telemetry
machine is likewise connected through canonical authenticated polling; two
same-boot Chromium workers proved retained-event reattachment and subsequent
progress with exact 176-byte requests and 432-byte events. A future
authenticated WebSocket may carry the same event contract at higher rates but
is not required for this polling checkpoint. No physical Wi-Fi, serial device,
GPIO, measurement, lease, machine arm, or output authority is exercised.
