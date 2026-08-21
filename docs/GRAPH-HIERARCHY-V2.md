# Canonical graph hierarchy and recursive flattening V2

`ALGH` V2 binds authoring-only instance nodes in a root `ALGW` and canonical
`ALGC` component definitions to exact component digests. Component definitions
may contain other component instances. The complete dependency graph must be a
bounded directed acyclic graph (DAG), and deterministic recursive flattening
emits an ordinary canonical workspace containing only structural `ALGR` nodes
and wires.

V2 is the only hierarchy version accepted by this green-field implementation.
There is no V1 decoder or compatibility path.

## Authority boundary

An instance node is a collapsed authoring view, not opaque executable behavior.
It uses reserved kind `alumina.component.instance` version 1 and is never
registered as a host behavior, firmware opcode, resource operation, or timing
claim. Semantic analysis, simulation, and deployment consume only the fully
flattened ordinary `ALGR` under their existing independent registries.

`ALGH` does not admit node semantics, prove that a draft is complete, select an
implementation, allocate a peripheral, prove WCET, or grant safety/deployment
authority. Firmware never receives or interprets `ALGH`, `ALGC`, `ALGW`, or raw
`ALGR` bytes.

## Canonical package

Every fixed-width integer is little-endian. V2 contains, in order:

1. `ALGH`, version 2, and zero flags;
2. seven embedded hierarchy limits;
3. hierarchy revision;
4. one length-delimited canonical root `ALGW`;
5. length-delimited canonical `ALGC` dependencies sorted by SHA-256 digest; and
6. scoped instance bindings in canonical scope/node/component order.

A binding begins with a one-byte scope tag. Tag 0 names the root workspace. Tag
1 is followed by the 32-byte digest of the containing component. Both forms
then carry the placeholder node ID and exact child-component digest. Unknown
tags fail closed. Root sorts before component scopes; component scopes sort by
digest, then bindings sort by local node ID. Duplicate `(scope, node)` pairs are
forbidden.

The interactive policy admits at most 32 MiB, 64 distinct dependencies, 4,096
binding records, nesting depth 32, 4,096 reachable expanded component
occurrences, 4,096 flattened nodes, and 8,192 flattened wires. The root
workspace, each component, and every embedded graph retain their own
independent caller and embedded policies. Outer replay bounds bytes and counts
before allocation, independently replays every nested envelope, reconstructs
all invariants, rejects trailing bytes, and requires exact re-encoding before
returning a hierarchy identity.

Every dependency in the library, including an unreferenced one, must use the
exact root graph type registry and clock set. Every reserved placeholder in the
root and every dependency has exactly one binding. Both its parent scope and
child digest must exist in the exact library. Duplicate component digests,
unknown scopes or children, normal nodes used as instances, unbound reserved
nodes, and unknown instance versions reject the complete package.

## Derived instance shape

The placeholder copies no behavior schema. Its input ports derive from public
component inputs in canonical ID order and receive consecutive node-local port
IDs beginning at 1. Outputs follow immediately in canonical public-output
order. Names and exact value types resolve from the child component's connector
pane. The placeholder is `HostExact` only as authoring placement metadata and
retains no parameters.

Any mismatch in kind/version, domain, port order/name/type, or parameter
emptiness rejects before flattening. Helper APIs resolve public input/output
identities to derived instance ports so callers do not duplicate that mapping.
The same check applies at every component scope, so a binding cannot silently
substitute a different connector shape.

## Bounded dependency analysis

Validation treats component digests as definition identities and scoped
bindings as dependency edges. A memoized depth-first analysis rejects a cycle
at the exact digest it re-enters and derives, with checked arithmetic, each
component's recursive depth, expanded occurrence count, surviving node count,
and wire count. It checks the depth of every retained definition and the total
root-reachable expansion against the embedded policy and the root graph and
placement capacities before allocating a flattened candidate.

This is structural recursion in the UI compiler only. Recursive component
definitions are rejected; no recursion or dynamic component dispatch reaches
the simulator or firmware.

## Deterministic recursive flattening

Flattening works transactionally on a clone of the root workspace. Root
bindings are visited in canonical local-node order, and each component subtree
is visited depth first in pre-order:

1. retain every incident wire and the instance's integer placement;
2. delete the placeholder and its incident wires without rewinding cursors;
3. copy component nodes, including transient nested placeholders, in canonical
   node-ID order using fresh monotonic root IDs and exact
   ports/parameters/domains;
4. translate component presentation coordinates relative to the component's
   minimum x/y so its top-left begins at the instance placement;
5. copy internal wires in canonical wire-ID order using fresh monotonic IDs;
6. reconnect retained wires through the component's exact public connectors;
   and
7. recursively flatten each nested placeholder using the private local-to-root
   map produced by that copy.

Wires between component instances remain valid regardless of direction or
nesting: flattening one endpoint reconnects to the still-collapsed endpoint,
and the later visit resolves it. A self-wire maps both ends in one pass. Final
node/wire/occurrence counts are checked again, every reserved placeholder must
be gone, and the result is canonically encoded. Identifier exhaustion, type
drift, target ownership, coordinate overflow, graph byte limits, or any edit
error discards the cloned candidate.

The result carries canonical `ALGW` bytes and a depth-first audit record for
every expanded occurrence. Each record includes the transient removed node,
the exact component digest, mappings for surviving component-local nodes, and a
stable source path consisting of the root instance node followed by nested
component-local instance nodes. Wrapper-only occurrences may therefore have an
empty surviving-node map while retaining an unambiguous source path. The
result also binds the source `ALGH` digest.

The flattening additionally retains total provenance for every final node and
wire. An ordinary root item maps to its original root ID. A copied item maps to
its stable occurrence source path, exact component digest, and component-local
ID. Wires retain that origin while public-connector replacement gives them
fresh final IDs, including repeated root-wire replacement and a parent
component wire crossing a nested child. Both final-to-origin and
origin-to-final lookup are available. The separate canonical
[`ALGM` V1 source map](GRAPH-HIERARCHY-SOURCE-MAP-V1.md) serializes those total
mappings and admits them only by freshly flattening the exact complete `ALGH`.

## First visible recursive proof

The control workspace constructs a root instance of
`control.reference_pid_wrapper`; that wrapper contains one exact
`control.reference_pid` instance. The 7,124-byte `ALGH` has SHA-256
`f9751073015828f20154a5536d8b217a7d3843e2d63d3b5689fcfb8c2379806a`.
It expands two occurrences at depth two to 21 ordinary nodes and 25 wires. The
audit paths are `[1]` for the wrapper and `[1, 1]` for the PID leaf.

The 3,755-byte flattened `ALGW` has SHA-256
`6804b964535d08b9ceead3d43891c3ae4c5aa5ce38b015c34b4b385bfa3257d4`
and passes the existing audited HostExact registry. Its identity differs from
the hand-authored reference workspace because fresh monotonic node IDs and
translated presentation positions are intentional flattened facts.

The UI displays source hierarchy identity, expanded-occurrence count, actual
depth, flattened counts, and flattened-workspace identity beside the live
canonical component panel. It also displays the 2,550-byte canonical `ALGM`,
its SHA-256
`dbfaf69255a1a4159329761523fbe120d8b956548adb4bbf1958e73ab014bb77`,
and its 21 node/25 wire origins. The selected-node inspector and exact trace
cursor correlate leaf-local endpoints such as `[1/1]:n8.p1` with the final
`n10.p1`; bounded `.algm` import regenerates every byte without mutation. It
does not substitute a collapsed placeholder into the executable editor canvas
or claim that an instance itself can run.

## Deliberately open

Editable component-instance creation/deletion on the main canvas,
hierarchy-aware undo/redo and file/browser persistence, general library
authoring, parameter promotion/overrides, package signatures and permissions,
locked dependency manifests, incremental flattening, interactive traversal
from final items into nested editable canvases, and executable front-panel
inputs remain open. `ALGH` V2 and `ALGM` V1 grant no
semantic, implementation, resource, timing, safety, firmware, or
physical-output authority.
