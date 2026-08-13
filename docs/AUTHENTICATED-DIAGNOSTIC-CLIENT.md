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
resource list, and advance both sequence and cumulative loss monotonically. An
exact replay of the newest event is identified as a duplicate without changing
progress. A same-sequence fork or counter regression is rejected.

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

## Offline transport evidence

The tests drive these state machines through three progressively wider seams:

1. the native service dispatcher in memory;
2. the production-format origin-bound HMAC HTTP fixture; and
3. an ephemeral `127.0.0.1` TCP listener carrying real HTTP bytes.

The tests deliberately lose mutation/range responses, substitute identity,
replace latest-only events, drop a live chunk, and then recover the authoritative
512-byte deterministic TinyBee capture through four 168-byte-or-smaller ranges.
The same client crate is checked for `wasm32-unknown-unknown`.

This does not yet connect the diagnostic machines to the visible worker/UI or a
WebSocket event stream. The existing board explorer still displays the
explicitly simulated fixture directly. No physical Wi-Fi, serial device, GPIO,
or output authority is exercised.
