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

## Present evidence boundary

Native tests exercise HMAC request/response interoperability, strict challenge
decoding, counter loss, chunk/finalize loss, corrupted progress and chunks, and
the partition-before-manifest sequence for every representative participant.
The complete client and application compile under strict native and WASM
Clippy. This is code-level browser transport evidence, not a claim that a real
browser, radio, AP/STA network, SD card, or MCU has exchanged these requests.

Still open are credential entry/storage UX, device discovery and identity
binding, UI progress/error panels, multi-device concurrency limits, browser
integration tests against simulated HTTP firmware, live TinyBee/T-Deck Pro
captures, clock fitting, prepare/commit/confirm, and physical cached-start
qualification.
