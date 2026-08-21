# Canonical graph hierarchy source map V1

`ALGM` V1 is a canonical, bounded sidecar for one deterministic `ALGH` V2
flattening. It maps every node and wire in the resulting ordinary `ALGW` to
exactly one authoring origin: either an identity in the root workspace or a
local identity in one exact component occurrence.

The map exists for audit, diagnostics, and UI correlation. It does not contain
node behavior, grant graph semantics, select an implementation or resource,
authorize a deployment, or carry a firmware command. Firmware never receives
or interprets `ALGM`, `ALGH`, `ALGC`, `ALGW`, or raw `ALGR` bytes.

## Stable origins

A root origin is the original non-placeholder root node or root wire ID. A
component origin contains:

- the occurrence source path, beginning with the root placeholder ID and
  continuing with each nested component-local placeholder ID;
- the SHA-256 identity of the exact `ALGC` definition containing the item; and
- the node or wire ID local to that definition.

Placeholder nodes do not survive flattening and therefore have no final-node
record. Every surviving ordinary node does. When flattening reconnects a wire
through one or more public connector panes, the wire keeps its authoring
origin even though each replacement receives a fresh monotonic final wire ID.
This applies both to root wires and to wires authored in a parent component
that cross a nested child placeholder.

The in-memory flattening report retains records in final node/wire ID order.
It supports final-to-origin and origin-to-final lookup. Duplicate origins,
duplicate final identities, missing final items, stale deleted items, empty
component paths, and unsorted final identities fail as noncanonical.

## Canonical bytes

All fixed-width integers are little-endian. V1 contains, in order:

1. `ALGM`, version 1, and zero flags;
2. four embedded `u64` limits: maximum complete bytes, node records, wire
   records, and source-path depth;
3. the 32-byte SHA-256 identity of the complete source `ALGH`;
4. the 32-byte SHA-256 identity of the resulting flattened `ALGW`;
5. a `u32` node-record count and records in strictly increasing final-node
   order; and
6. a `u32` wire-record count and records in strictly increasing final-wire
   order.

Every record starts with an origin tag. Tag 0 carries a root-local `u32` item
ID. Tag 1 carries a `u32` path length, that many nonzero `u32` placeholder
IDs, the 32-byte component digest, and a nonzero component-local `u32` item ID.
The record then ends with the nonzero final `u32` node or wire ID. Unknown tags,
zero identities, an empty component path, duplicate origins, non-increasing
final IDs, and trailing bytes reject.

The interactive policy admits at most 4 MiB, 4,096 final-node records, 8,192
final-wire records, and path depth 32. Embedded limits are part of identity and
must fit the caller's independent admission policy. Encoding computes the
complete byte length with checked arithmetic and rejects the byte ceiling
before allocating the exact output vector.

## Independent replay

An imported map is never an independent provenance authority. Replay:

1. bounds outer bytes under the caller's policy;
2. validates the header and bounds every embedded policy field;
3. canonically encodes the caller-supplied complete hierarchy and requires the
   embedded source identity to match;
4. freshly flattens that hierarchy, rechecks its source identity, and requires
   the embedded flattened-workspace identity to match;
5. bounds and validates every node and wire record without accepting partial
   output; and
6. regenerates `ALGM` from the fresh total provenance under the embedded policy
   and requires byte-for-byte equality.

Supplying another valid hierarchy, another valid flattened workspace, a map
encoded under a different policy, or altered record bytes cannot substitute
provenance. Replay returns only the fresh flattening and the exact regenerated
encoding.

## Visible recursive proof

The reference UI hierarchy remains the two-level wrapper-to-PID proof described
in [`GRAPH-HIERARCHY-V2.md`](GRAPH-HIERARCHY-V2.md): 21 final nodes and 25 final
wires under leaf occurrence path `[1/1]`. Its 2,550-byte `ALGM` has SHA-256
`dbfaf69255a1a4159329761523fbe120d8b956548adb4bbf1958e73ab014bb77`.
For example, leaf-local node `n8` correlates to final node `n10`, and endpoint
`n8.p1` correlates to `n10.p1`.

The component panel displays the map digest and total node/wire origins and
offers bounded `.algm` download/open controls. Import performs fresh replay and
does not mutate the graph, hierarchy, component, probe sidecar, trace, or
deployment state. The selected-node inspector and exact trace cursor prefix
component-local endpoints with their occurrence paths and final identities.
At most four matching occurrences are expanded in one label; the complete map
remains retained and any additional occurrence count is explicit.

The core golden also exercises both origin tags: two ordinary root nodes, one
ordinary root wire, and a nested leaf yield 23 node and 26 wire records. The
2,577-byte encoding has SHA-256
`885eaed4357bdc7c3d97d2eff76321732260d6147afbacdb5e004bbd23c291e3`.

## Deliberately open

Canonical [`ALGS` V1](GRAPH-AUTHORING-SESSION-V1.md) now persists the selected
component, complete ALGH, and regenerated ALGM together in browser storage and
`.algs` files. Editable hierarchy construction, hierarchy-aware undo/redo,
interactive traversal from a flattened item into nested editable component
canvases, parameter promotion, signed dependency manifests, live device trace
correlation, and firmware execution evidence remain separate work. `ALGM` V1
closes only deterministic total source correlation for one complete,
validated, freshly flattened hierarchy.
