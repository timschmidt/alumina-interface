# Canonical graph authoring session V1

`ALGS` V1 is the bounded, canonical persistence and file-exchange envelope for
one exact graph-authoring session. It binds the editable control `ALGW`, its
exact `ALGP` sidecar, the catalog-managed cached-job `ALGW`, and an optional
complete `ALGH`/`ALGM` hierarchy branch under one SHA-256 identity.

`ALGS` is host authoring state. It grants no graph semantics, deployment,
resource ownership, job preparation, arming, start, firmware, timing, motion,
or safety authority. Firmware does not receive or interpret it.

## Bound artifacts

One session contains:

1. the editable control `ALGW`;
2. one `ALGP` whose workspace digest is exactly that control `ALGW` identity;
3. one inert cached-job `ALGW` whose job-handle leaves still require the
   application's independently reconstructed catalog during UI admission; and
4. either no hierarchy branch, or all of:
   - the digest of the selected `ALGC` dependency;
   - the complete `ALGH` containing that dependency; and
   - the complete `ALGM` regenerated from that exact hierarchy.

The selected component must be present in the hierarchy and its embedded
workspace must equal the control `ALGW` as both a document and canonical
identity. A detached component draft is represented by the absent hierarchy
tag. Restore does not invent a representative component when that tag is
absent.

History stacks, open file paths, text-field drafts, selected graph nodes,
pending wires, drag gestures, and replay/deployment authority are deliberately
ephemeral and are not serialized.

## Canonical wire layout

All integers are unsigned little-endian. Lengths count bytes. V1 is exactly:

1. four magic bytes `ALGS`;
2. `u16` version `1`;
3. `u16` zero flags;
4. six `u64` embedded limits, in order:
   - complete session bytes;
   - control `ALGW` bytes;
   - `ALGP` bytes;
   - cached-job `ALGW` bytes;
   - `ALGH` bytes;
   - `ALGM` bytes;
5. `u32` control length and the complete canonical control `ALGW`;
6. `u32` probe length and the complete canonical `ALGP`;
7. `u32` cached-job length and its complete canonical `ALGW`;
8. one hierarchy tag:
   - `0`: the document ends immediately;
   - `1`: a 32-byte selected-component digest, `u32` hierarchy length and
     complete canonical `ALGH`, then `u32` source-map length and complete
     canonical `ALGM`.

No padding, alternate field order, unknown flag, alternate hierarchy tag, or
trailing byte is canonical. The encoder computes the complete checked size
before allocation, allocates that exact capacity, and requires the final byte
count to match.

## First interactive policy

The embedded and caller-admitted first-release ceilings are:

| Section | Maximum bytes |
| --- | ---: |
| complete `ALGS` | 8 MiB |
| control `ALGW` | 2 MiB |
| `ALGP` | 2 MiB |
| cached-job `ALGW` | 2 MiB |
| complete `ALGH` | 4 MiB |
| complete `ALGM` | 4 MiB |

Nested artifacts also remain subject to their own graph, workspace, probe,
component, hierarchy, and source-map admission policies. An embedded limit may
be stricter than the caller's policy but never wider. Zero limits fail closed.

## Replay and cross-artifact invariants

Replay performs these operations before returning any document:

1. bounds the outer bytes and validates magic, version, flags, embedded limits,
   lengths, hierarchy tag, and exact end of input;
2. independently replays both `ALGW` documents and the `ALGP` against the
   replayed control workspace;
3. when present, independently replays the complete `ALGH`, freshly flattens
   it, and replays/regenerates the complete `ALGM`;
4. locates the selected component by canonical digest and proves its embedded
   workspace equals the control workspace;
5. reconstructs a new session document from the replayed artifacts; and
6. re-encodes the whole `ALGS` byte-for-byte.

Any nested corruption, foreign probe binding, missing selected component,
workspace substitution, stale source map, policy widening, or noncanonical
outer encoding rejects the whole session.

The application adds two semantic gates after core replay and before mutation:
the control and flattened hierarchy graphs must use the audited UI registry,
and every cached-job handle leaf must be a member of the freshly reconciled
catalog. Only after every check succeeds does it replace the control workspace,
probe package, cached-job workspace, selected component, complete hierarchy,
and source map together. Failed restore leaves all prior authoring state intact.

## Browser persistence and file exchange

Browser local storage uses the greenfield key
`alumina.graph-authoring-session.algs.v1`. Its value is `algs1:` followed by one
lowercase hexadecimal encoding of the complete canonical `ALGS` bytes. The
prefix is a storage wrapper, not part of the canonical artifact.

The retired `algwb1:` three-section value and its storage key are not read,
migrated, or shimmed. An unsupported prefix loads the canonical reference
session and marks persistence pending.

Native and browser UI expose the same complete bytes as `.algs`. Import is
bounded at 8 MiB and uses the same atomic replay/admission transaction. The
separate `.algw`, `.algp`, and `.algm` tools remain useful for focused artifact
inspection, but they are not substitutes for complete-session persistence.

## Golden fixture

The core fixture contains a 21-node/25-wire exact control workspace, an empty
but exactly bound probe sidecar, the same workspace in the cached-job slot, one
selected component, a complete one-instance hierarchy, and total node/wire
provenance. Its canonical `ALGS` is 14,794 bytes with SHA-256:

`09161f22343464aef962c513be9778b0bbe5b465ebef7423006349b56accc07c`

Tests require byte-for-byte replay, hierarchy-present and hierarchy-absent
round trips, limit and structural corruption rejection, selected-workspace
binding, exact UI hierarchy/source-map preservation, retired-prefix rejection,
foreign cached-job rejection, and atomic failure for corrupted `ALGH`/`ALGM`.

The complete browser reference session is 14,770 bytes with SHA-256
`d7a5fba83da9f254eb0d50eab301129f933016a400c9d154c5f2d97d8029cf9d`.
It contains:

- 3,755-byte control `ALGW`, SHA-256
  `bf5135c39b67c46a3a5908d4d0d8a1d13d065b59231890e8fbdda818f064ae16`;
- 407-byte `ALGP`, SHA-256
  `50955da7b4464a02f6e3eace1d51a3e9d08f56cdfa9c661259c33195b15ef223`;
- 825-byte cached-job `ALGW`, SHA-256
  `493b293ac9f83f96b6b91d4fba05597f7c7e280cdfd4f462c82d443e1f38bf58`;
- selected `ALGC`
  `10e6498ec36afc377f138cacb5c6afe2091c40749ea3c9e9d4bba8925a4f0228`;
- 7,124-byte `ALGH`, SHA-256
  `f9751073015828f20154a5536d8b217a7d3843e2d63d3b5689fcfb8c2379806a`;
  and
- 2,550-byte `ALGM`, SHA-256
  `dbfaf69255a1a4159329761523fbe120d8b956548adb4bbf1958e73ab014bb77`.

Chromium replay retained the valid hierarchy-absent 5,056-byte prefix
byte-for-byte (SHA-256
`93b65ea6b313cb37f759c105dd0f8feaa33a22b6b8ca5f879844d734a2fcd975`),
rejected a selected-component digest substitution and rewrote the exact
canonical fallback, ignored the retired storage key without migration, and
downloaded a `.algs` exactly equal to the canonical local-storage bytes.
