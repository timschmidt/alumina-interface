# Canonical graph workspace V1

`ALGW` V1 is the first saved editing envelope around an exact `ALGR` graph. It
exists because executable structure and human canvas state have different
authority. The workspace embeds the complete canonical graph bytes, then adds
only bounded presentation metadata and monotonic editor identity cursors.

## Authority boundary

The embedded `ALGR` remains the sole input to semantic analysis, exact
simulation, deployment lowering, and trace identity. `ALGW` coordinates are
signed integer logical pixels. They can position a node on a canvas, but no API
converts them into a graph literal, exact geometry, a machine lattice, a timer,
or a firmware value.

Browser pointer motion is a lossy presentation input. A drag is rounded once at
commit into the integer canvas lattice and then admitted by the workspace
coordinate bound. The UI projects those bounded integers back to `f32` only for
egui painting. This two-way presentation conversion never crosses into the
embedded graph.

## Canonical envelope

Every fixed-width integer is little-endian. V1 contains, in order:

1. `ALGW`, version 1, and zero flags;
2. embedded workspace byte/count/coordinate limits;
3. a monotonic workspace revision;
4. monotonic next-node and next-wire identity cursors;
5. a length-delimited complete canonical `ALGR` document; and
6. node ID plus signed `i32` x/y for every graph node, sorted by node ID.

The next-ID cursors must be strictly greater than every retained ID. They may
equal `u32::MAX + 1` only as an explicit exhausted sentinel, so deleting the
largest wire cannot silently reuse its stable identity. Every graph node has
exactly one placement; missing, duplicate, zero, or foreign placements reject.

The first UI admission policy is 20 MiB total bytes, 256 placements, and an
absolute coordinate magnitude of 1,000,000 logical pixels. The embedded policy
cannot grant itself more memory than the caller's admission policy. Replay
bounds the outer bytes before reading lengths, independently replays the
embedded `ALGR`, reconstructs all invariants, re-encodes every byte, and assigns
SHA-256 identity only after exact byte equality.

The representative 21-node/25-wire PID/interlock workspace is 3,755 bytes with
SHA-256
`bf5135c39b67c46a3a5908d4d0d8a1d13d065b59231890e8fbdda818f064ae16`.
It embeds graph identity
`96a3348264a9b65d267b45f9a6419a44ee60473fd961abcf4436295e10b3735f`.

## Transactional editing

`GraphWorkspaceDocument` exposes eight transactional edits:

- move one node, advancing only workspace revision;
- create one complete node prototype and placement with the monotonic node ID;
- delete one node, its placement, and every incident wire without rewinding
  either ID cursor;
- connect one typed output to one unowned input with the monotonic wire ID;
- disconnect one existing wire without rewinding the ID cursor;
- replace one bounded canonical UTF-8 node label while preserving kind, ports,
  parameters, domain, placement, and stable identities;
- replace one concrete execution domain while preserving every other node and
  workspace fact; and
- replace one exact parameter value while preserving its stable parameter ID,
  name, and registered root type.

Each operation constructs and validates a complete candidate before replacing
the prior document. Rejected coordinates, missing IDs, exhausted counters,
wrong port direction/type, duplicate target ownership, invalid label/domain,
parameter type drift, or revision overflow leave the workspace byte-for-byte
unchanged. Node, wire, label, domain, and parameter edits also advance the
embedded graph revision and therefore change its canonical digest. Labels are
canonical human metadata, but are never matched as behavior identity. The core
domain replacement is deliberately structural: allowed kind/domain families,
clock relationships, wire crossings, implementations, target capabilities, and
deployment remain separate audited obligations.

The native/WASM control workspace initializes one canonical `ALGW` from the
audited deterministic layout. Its 13-entry palette is derived from the fixed
simulation registry: kind/version, ports, and parameter contracts come from
the audited node schema, while each exact initial parameter value comes from
the lowest-ID reviewed representative instance of that kind. A kind without a
fixed implementation or reviewed default prevents the palette from opening.
The palette never manufactures an implicit resource, device domain, parameter,
or port.

Nodes can be created, selected, deleted, and dragged; an output then input can
be clicked to connect, and an input can be secondary-clicked to disconnect.
The selected-node inspector edits the bounded canonical label and offers only
execution-domain families admitted by that node's reviewed schema. Concrete
device identities are collected from existing device-cycle clocks and node
placements in the graph, sorted canonically, and never accepted as raw text.
The complete candidate is rerun through the fixed semantic registry before it
can commit. A graph with no reviewed device identity consequently cannot invent
a Service or Realtime target.

The shared parameter surface accepts bounded schema-directed Boolean,
exact-rational, measurement-interval, canonical signed/unsigned lattice-count,
text, byte, array, record, option, and result literals. Every current
representative-control parameter is exact rational. Hyperreal parses rational
text exactly, composite parsing follows the registered schema, and canonical
`ALGR` encoding stores the normalized value without a floating-point
conversion. Resource/job handles and runtime Event/Stream shapes remain
visibly read-only in the text editor. The separate capability-derived target
draft can replace a resource handle only through exact catalog selection.

The UI rebuilds semantic layering and reruns audited analysis after each
candidate. A newly created or disconnected required input is retained as an
explicit semantic blocker rather than hidden or repaired. A current-tick
combinational cycle that cannot be laid out is rejected without mutation. An
empty draft remains renderable and can accept a new palette node.

The existing `ALGT` reference trace remains visible after placement-only edits
because its embedded `ALGR` digest is unchanged. Any node, wire, label, domain,
or parameter edit detaches and hides that trace: trace bytes are never relabeled
as evidence for a graph they did not simulate. Reset reconstructs the reviewed
reference graph and layout.

## Canonical history

Undo and redo retain complete pairs of independent canonical `ALGW` and bound
`ALGP` encodings, not mutable UI deltas, inverse operations, or the browser text
envelope. The default policy keeps at most 32 pair snapshots in each direction
and at most 64 MiB of combined ALGW-plus-ALGP bytes across both stacks; the
separately held current pair does not count against that budget. Both artifacts
in the oldest snapshot are evicted together. A successful graph, probe
name/capture-policy, trigger, or ALGP-import edit records the exact prior pair
and discards the abandoned redo branch. An exact no-op records nothing.

Every navigation target first replays ALGW under the same 20 MiB workspace and
interactive graph limits used for an imported file, then replays ALGP against
that reconstructed workspace under its 2 MiB policy. The UI also rebuilds its
layout and reruns audited semantic admission before history state changes. A
corrupt, noncanonical, unregistered, unbound, or otherwise inadmissible target
leaves both current documents and both history stacks unchanged. Navigation
clears transient wire, drag, parameter-text, and probe-metadata fields, but
preserves a selection only when that node still exists and resets the cursor
from the restored trigger resolution.

History is intentionally ephemeral and is not nested inside `ALGW`, `ALGP`, or
browser storage. This avoids recursive snapshots, keeps both document
identities independent of an editing session, and means a restored or freshly
opened pair begins with empty undo and redo stacks. Importing a different valid
workspace or bound sidecar is itself one undoable pair edit.

## Persistence and file exchange

The browser stores the current canonical workspace and its exact graph-probe
sidecar together in one origin-local storage value after a successful graph,
history, probe, or trigger edit. The application envelope is version-tagged
`algwp1:`, followed by lowercase hexadecimal `ALGW`, one `:`, and lowercase
hexadecimal `ALGP`. One `setItem` replaces the complete pair; there is no
two-key state in which a newly stored workspace can be mistaken for an older
sidecar. Decoding rejects the wrong tag, missing separator, odd length,
uppercase or non-hex text, and either artifact over its 2 MiB browser
persistence ceiling before allocating its bytes.

Restore first replays and admits the candidate `ALGW`, then replays `ALGP`
against that exact candidate identity. Only after both replays and byte-for-byte
canonical checks succeed does either document replace the in-memory reference.
A malformed sidecar, a valid sidecar for another workspace, or an invalid
workspace therefore rejects the pair atomically. The old ALGW-only storage key
and text representation are intentionally not compatibility inputs.

Browser download writes the byte-for-byte canonical `ALGW` or currently bound
`ALGP` encoding to a corresponding `.algw` or `.algp` Blob. Browser upload
checks the advertised file size, bounds the materialized `ArrayBuffer`, and
forwards only bytes within that artifact's policy. The native shell exposes
independent explicit paths with bounded reads and exact, synchronized writes.
Neither platform bridge parses an artifact. An opened `.algp` is replayed
against the exact current workspace and cannot mutate the graph; an identity
mismatch leaves both graph and prior sidecar unchanged. Explicit ALGW files
retain the 20 MiB workspace admission ceiling; ALGP files use their canonical
2 MiB document ceiling.

All opened ALGW bytes first pass canonical replay, embedded-limit checks, exact
re-encoding, UI layout admission, and the fixed audited semantic registry. A
required input may remain disconnected as a visible editor draft blocker;
other semantic failures, including unknown node kinds, reject without mutation.
Thus core `ALGW` can still preserve opaque future nodes, while this particular
control UI never claims it can safely interpret or edit them.

## Current exclusions

Cache-derived job-handle selection, selection sets, groups/comments,
collaborative diffs, and conflict-aware shared persistence remain later slices.
The separate canonical
[`ALGC` V1 component package](GRAPH-COMPONENT-V1.md) now embeds an unchanged
`ALGW` and adds a connector pane plus exact front-panel bindings; those facts
are deliberately not smuggled into this workspace format. `ALGW` grants no
semantic admission, implementation, Service/Realtime opcode, resource,
deployment, safety, or physical-output authority.
