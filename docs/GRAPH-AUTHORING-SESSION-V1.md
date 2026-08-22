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

The application adds semantic gates after core replay and before mutation: the
control graph, every dependency's ordinary-node draft, and the flattened
hierarchy graph must use the audited UI registry, while every cached-job handle
leaf must be a member of the freshly reconciled catalog. Structural component
placeholders remain the responsibility of `ALGH` validation and are removed
from the per-definition ordinary-node audit. Only after every check succeeds
does the application replace the control workspace, probe package, cached-job
workspace, selected component, complete hierarchy, and source map together.
Failed restore leaves all prior authoring state intact.

## Ephemeral complete-session history

The editor's undo/redo carrier is a bounded stack of complete canonical `ALGS`
byte strings. It is not a second wire format and is never nested into `ALGS`.
Control-workspace, probe/trigger, focused import, cached-job,
selected-component, component-library creation/import/removal, direct
root-instance, selected-component identity evolution, scoped child-occurrence
rebinding, hierarchy, and source-map state—including root instance placement
and typed root wiring—share one timeline rather than independent histories that
could be navigated into a mismatched combination.

The first interactive policy retains at most 16 snapshots in each direction
and 64 MiB of canonical `ALGS` bytes across both stacks. The current session is
held separately and does not count against that budget. A successful canonical
change records its exact prior session and clears the abandoned redo branch;
an exact no-op records nothing. Oldest complete snapshots are evicted until
both bounds hold, without splitting any nested artifact.

Creating a named empty component commits its new canonical dependency through
this same complete-session boundary. Invalid, overlong, or conflicting names
leave the current `ALGS`, both history stacks, and pending-persistence state
unchanged. Selecting an already-present byte-identical empty component changes
only ephemeral UI scope and records no history.

Renaming a selected dependency or retaining/increasing its declared behavior
version uses the same complete-session boundary. Recursive digest replacement
updates every affected hierarchy binding and source map before commit; an exact
name/version no-op records nothing, while version regression, noncanonical
decimal input, invalid or conflicting names, and any recursive admission
failure preserve the current session, both history stacks, and persistence
state byte-for-byte. Undo, redo, and persisted restore retain the evolved
identity without an alias to the former digest.

Rebinding one selected-definition child occurrence also uses this boundary.
Stable-ID-compatible parent wiring and public endpoints survive, while a
missing or incompatible live connector, cycle, bound violation, unknown
identity, or indirect control-authority replacement preserves the current
session and both history stacks exactly. Rebinding to the current target is an
exact no-op. A successful same-shape binding-only change and a successful
recursive public-shape replacement each record exactly one prior `ALGS`; Undo,
Redo, and persisted restore therefore cannot separate the placeholder from its
scoped binding or regenerated source map.

Undo and redo replay the target through the complete core boundary, then rerun
the application's audited graph and cached-job catalog admission before
changing either authoring state or history stacks. Byte corruption, tighter
replay limits, unknown semantics, or stale/foreign cached-job identities fail
without mutation. Navigation restores all six bound artifact roles together
and clears or reconciles only transient UI selections and text/drag drafts.
Fresh startup, complete-session import, and persistence restore begin with
empty history.

Flattened-to-source navigation is also transient. Selecting an `ALGM` final
node or wire, resolving its exact occurrence path, focusing a source editor,
and scrolling to that editor neither encodes a field in `ALGS` nor pushes a
history state nor marks browser persistence pending. A later canonical edit
reconciles the retained origin against the regenerated map and clears it when
the exact final/origin pair no longer exists.

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
separate `.algw`, `.algp`, `.algc`, and `.algm` tools remain useful for focused
artifact inspection, but they are not substitutes for complete-session
persistence. A valid new `.algc` leaf is inserted into the embedded hierarchy
library only after exact replay and semantic/context admission, then commits a
new complete `ALGS`; a byte-identical duplicate records no history. Removal is
available only for an unreferenced non-authoritative dependency and likewise
commits through complete-session history.

## Golden fixture

The core fixture contains a 21-node/25-wire exact control workspace, an empty
but exactly bound probe sidecar, the same workspace in the cached-job slot, one
selected component, a complete one-instance hierarchy, and total node/wire
provenance. Its canonical `ALGS` is 14,794 bytes with SHA-256:

`09161f22343464aef962c513be9778b0bbe5b465ebef7423006349b56accc07c`

Tests require byte-for-byte replay, hierarchy-present and hierarchy-absent
round trips, limit and structural corruption rejection, selected-workspace
binding, exact UI hierarchy/source-map preservation, retired-prefix rejection,
foreign cached-job rejection, atomic failure for corrupted `ALGH`/`ALGM`,
bounded complete-session history eviction, mixed graph/probe/cached-job
navigation, exact component-library import/no-op/in-use/remove/persistence,
root placement and typed disconnect/reconnect with monotonic wire identity,
front-panel add/update/remove with monotonic item identity and exact layout
history, authored-panel retention across later workspace edits, stable
connector authoring on both the selected control component and another chosen
library dependency, recursive dependency-selection/root-placeholder refresh,
selected-library definition node/placement/wire authoring, control-authority
isolation, atomic scoped child-occurrence add/remove/re-add with monotonic node
identity, exact child history/persistence, rejection of live public bindings,
cycles, ordinary placeholder deletion, and an indirect authority rewrite,
exact flattened node/wire source navigation with distinct component-local wire
highlighting, retained target-node inspection, no session/history/persistence
mutation, and stale-origin reconciliation, deterministic empty component
creation with exact schema/clock inheritance, identity-cursor initialization,
duplicate no-op selection, immediate definition editing, history/persistence,
and invalid/conflicting-name atomicity, monotonic selected-component
name/version evolution with recursive binding replacement, exact no-op,
stale-source reconciliation, history/persistence, and invalid, regressing, or
conflicting metadata atomicity, exact scoped child-occurrence rebinding with
stable-ID endpoint preservation, same-target no-op, recursive shape refresh,
history/persistence, and atomic rejection of missing connectors, cycles,
unknown targets, or authority replacement, abandoned-redo clearing, and
transactional failure for corrupt,
semantically-unreviewed, or catalog-inadmissible inputs and history targets.

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

Optimized Chromium also exercised a same-shape scoped child rebind through the
visible selected-definition inspector. The complete 16,413-byte `ALGS` changed
from A-bound identity
`7579d044c131d3098b3bfd60a8455becda9e1d98f17c4a57ce5f1f7e6d9bc5bb`
to B-bound identity
`129c1617f9f33af2374db8e294703699ad455cdd8a71dc50d0dc9fb4851f9ff4`
while the parent `ALGC`, parent `ALGW`, placement, allocation cursors, and
flattened workspace remained byte-identical. Reapplying B was an exact no-op;
visible Undo restored A, and Redo plus fresh browser restore recovered B
byte-for-byte. The selected placeholder itself correctly remained ephemeral
across reload while its canonical scoped binding persisted.
