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

The representative 19-node/22-wire PID workspace is 3,396 bytes with SHA-256
`d7d4ef9e27359a474b59f48cdbcb604b3d4d16f2a768a65f12c95dde8aee9799`.
It embeds graph identity
`fb173fb30bc5e04269caea439dea8fa455050142fac3a4afc78f5fd16e7ac59a`.

## Transactional editing

`GraphWorkspaceDocument` exposes six transactional edits:

- move one node, advancing only workspace revision;
- create one complete node prototype and placement with the monotonic node ID;
- delete one node, its placement, and every incident wire without rewinding
  either ID cursor;
- connect one typed output to one unowned input with the monotonic wire ID;
- disconnect one existing wire without rewinding the ID cursor; and
- replace one exact parameter value while preserving its stable parameter ID,
  name, and registered root type.

Each operation constructs and validates a complete candidate before replacing
the prior document. Rejected coordinates, missing IDs, exhausted counters,
wrong port direction/type, duplicate target ownership, parameter type drift, or
revision overflow leave the workspace byte-for-byte unchanged. Node, wire, and
parameter edits also advance the embedded graph revision and therefore change
its canonical digest.

The native/WASM control workspace initializes one canonical `ALGW` from the
audited deterministic layout. Its 11-entry palette is derived from the fixed
simulation registry: kind/version, ports, and parameter contracts come from
the audited node schema, while each exact initial parameter value comes from
the lowest-ID reviewed representative instance of that kind. A kind without a
fixed implementation or reviewed default prevents the palette from opening.
The palette never manufactures an implicit resource, device domain, parameter,
or port.

Nodes can be created, selected, deleted, and dragged; an output then input can
be clicked to connect, and an input can be secondary-clicked to disconnect.
The first parameter surface accepts bounded Boolean, exact-rational,
measurement-interval, canonical signed/unsigned lattice-count, and text
literals. Every current representative-control parameter is exact rational.
Hyperreal parses the bounded text exactly, the graph schema validates the
result, and canonical `ALGR` encoding stores the normalized value without a
floating-point conversion. Composite and identity-bearing literal shapes
remain visibly read-only.

The UI rebuilds semantic layering and reruns audited analysis after each
candidate. A newly created or disconnected required input is retained as an
explicit semantic blocker rather than hidden or repaired. A current-tick
combinational cycle that cannot be laid out is rejected without mutation. An
empty draft remains renderable and can accept a new palette node.

The existing `ALGT` reference trace remains visible after placement-only edits
because its embedded `ALGR` digest is unchanged. Any node, wire, or parameter
edit detaches and hides that trace: trace bytes are never relabeled as evidence
for a graph they did not simulate. Reset reconstructs the reviewed reference
graph and layout.

## Current exclusions

The workspace is currently in memory. File download/upload, browser
persistence, undo/redo history, node label/domain editing, composite and
identity-bearing parameter editors, selection sets, groups/comments,
subgraphs/components, front panels, and collaborative diffs remain later
slices. `ALGW` grants no semantic admission, implementation, Service/Realtime
opcode, resource, deployment, safety, or physical-output authority.
