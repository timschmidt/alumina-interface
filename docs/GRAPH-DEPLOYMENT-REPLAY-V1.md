# Graph deployment replay evidence V1

`ALGRREP1` is a canonical, bounded input/output transcript for offline execution
of one lowered Realtime-only `ALGRIR02` package through the portable fixed-memory
firmware actors. It lets native and WASM callers retain and independently replay
the exact actor behavior used to admit an interface edit without treating a hash,
an expected truth table, or a host-side graph interpreter as execution evidence.

This boundary is deliberately non-operational. It does not discover or
authenticate an MCU, open a firmware session, install a package on hardware,
acquire a resource, configure a pin, start a job, or grant safety authority. The
target tuple, lowered deployment report, and static firmware image palettes are
independent caller-supplied authorities.

## Canonical encoding

All integers are unsigned little-endian. Counts and resource selectors are four
bytes. Input samples are sorted by the canonical typed `ResourceId`; provider
reads retain the order in which the firmware actor actually requested them.

| Field | Encoding |
| --- | --- |
| Domain | 8-byte `ALGRREP1` magic |
| Target | 16-byte device ID, then 32-byte capability and configuration digests |
| Implementation | 32-byte implementation-registry digest |
| Package | 32-byte graph-IR content digest, then 32-byte complete package digest |
| Run | transaction ID, run ID, and start cycle as three `u64` values |
| Declared policy | maximum releases, resource inputs, and provider reads as three `u32` values |
| Requested inputs | `u32` count followed by the input records below |
| Actor results | `u32` count followed by the result records below |

Each requested input record contains:

1. exact device cycle as `u64`;
2. safety admission as canonical `0` or `1`;
3. resource-sample count as `u32`; and
4. each sorted four-byte resource identity followed by `0` for unavailable,
   `1` for false, or `2` for true.

Each actor result contains its exact cycle, provider-read count, provider IDs in
actual call order, and one outcome:

- tag `1`: completed release cycle, release tick, nodes executed, items
  consumed, items emitted, sink items, and canonical optional last sink value;
- tag `2`: first-cause fault generation, stable fault code, detail, and canonical
  optional expected/received release cycles.

The SHA-256 digest covers every byte, including declared limits and both input
and output evidence. Reordering caller samples cannot change the artifact;
changing provider order, a completion count, a fault, or any identity must.

## Bounds and verification

The absolute V1 ceilings are 4,096 requested releases, 256 distinct input
resources per release, 64 retained provider reads per release, and 6,525,156
encoded bytes. Interactive policy is narrower: 1,024 releases, 128 inputs, and
64 reads. Invalid zero or over-hard limits reject before actor setup. Raw import
checks the byte ceiling before hashing or allocation based on encoded counts.

Verification proceeds in this order:

1. validate the caller's admission limits and the absolute byte ceiling;
2. require the claimed SHA-256 digest to match the supplied bytes;
3. decode the canonical authority and input prefix;
4. require exact device, capability, configuration, implementation, content,
   and package identities;
5. reject artifact-declared limits above either hard limits or the caller's
   narrower admission;
6. reject truncated fields, invalid input tags, unsorted/duplicate resources,
   empty or excessive release sets, and nonmonotonic cycles;
7. install and start fresh `FixedGraphServiceActor` and
   `FixedGraphRealtimeActor` instances with the supplied image palettes;
8. rerun every requested release until completion or the first terminal fault;
   and
9. require the newly generated complete artifact and digest to match the
   imported artifact byte-for-byte.

The imported result suffix is not trusted as a decoded report. Fresh actors
regenerate it. Any malformed, noncanonical, trailing, omitted, or invented
result bytes therefore fail the final exact comparison even when a caller
supplies their recomputed digest.

## TinyBee UI proof

The TinyBee executable target-I/O draft commits its `ALGW`, lowered `ALGRIR02`,
and two actor transcripts as one UI transaction:

- four conjunction releases, proving `00→0`, `01→0`, `10→0`, and `11→1` with
  both provider calls in stable field order; and
- one release with the first resource unavailable, proving the ordered reads,
  terminal `ResourceUnavailable` fault, and absence of a completed sink report.

The default GPIO22/GPIO32 success artifact is 528 bytes with digest
`a26b41965997461d109d2aeec4b562eb0d2c7c3dfe61870139c2bd2293f12147`.
Its 278-byte fault artifact has digest
`5e122937631516da3b57fd9f3e1f1d39a473ada6bf7dbad9c04b5121a4b89f6c`.
Rebinding the second input to GPIO35 produces success digest
`cf2215451f222f8b402b85285ae889ffd67c06e7de8eb3d9c03c8d075dc2a8df`
and fault digest
`e99110e8980e319389b8fe7731a6087375a465e4151289c37edaec0ed133b176`.

Native and browser UI rows export the exact current `.algrrep` bytes. Import is
verification-only: it accepts only the current artifact identity, performs a
fresh actor replay, and never changes the graph draft, lowering report, file
contents, firmware session, or physical state.

These are deterministic software results against the current sibling firmware
and Hyper working trees. They are not connected-board, GPIO, motor, Wi-Fi, or
safety-chain qualification.
