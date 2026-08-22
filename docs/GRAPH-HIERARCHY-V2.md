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

## Transactional authoring operations

`GraphHierarchyDocument` exposes eight bounded clone-and-validate mutations:

- `add_component` canonically encodes one standalone definition and adds its
  exact digest to the dependency library; an already-present byte-identical
  encoding is a no-op;
- `remove_component` removes only a known dependency that is named by neither
  a child binding nor a component parent scope;
- `add_root_instance` resolves an existing exact dependency, derives its
  placeholder shape, and allocates the root node through the root `ALGW`'s
  monotonic cursor;
- `remove_root_instance` deletes one exact root binding, its placeholder, and
  all incident root wires without removing dependencies or rewinding node and
  wire cursors;
- `move_root_instance` changes only the exact integer root-workspace placement
  of a known bound root placeholder and treats an identical placement as a
  no-op;
- `connect_root_wire` delegates exact direction, type, and single-input
  ownership checks to the cloned root `ALGW` and consumes its monotonic wire
  identity only in the accepted candidate;
- `disconnect_root_wire` removes one known root wire without rewinding the
  root wire cursor; and
- `replace_component` substitutes one exact library dependency, refreshes
  affected placeholder ports and public endpoints by stable connector ID, and
  recursively remaps child-component references and parent-scope references
  through every changed ancestor digest; and
- `replace_component_with_report` performs that same transaction and returns
  the requested old/new identity plus every directly or recursively rebuilt
  dependency in canonical old-digest order. Its exact no-op report contains no
  remaps.

Each operation constructs a complete candidate and reruns all hierarchy
invariants before replacing the prior document. An unknown dependency or
instance, incompatible connector shape, digest collision, limit failure,
integer overflow, or any nested validation error therefore leaves the original
hierarchy byte-for-byte intact. Replacing a dependency with its exact current
encoding is a no-op and does not advance revision.

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

The adjacent component-library panel selects dependencies already present in
the canonical hierarchy. It exports the selected dependency byte for byte,
imports a bounded canonical standalone leaf only after replay and audited
ordinary-node semantic admission, and removes only an unreferenced dependency
that is not the selected control authority. Every complete hierarchy admission
also audits the ordinary-node draft of every dependency, including
unreferenced definitions; structural instance placeholders are validated by
`ALGH` and do not acquire semantic authority from that audit.

Adding a selected dependency places a new root placeholder to the right of the
current root layout; deleting a selected root occurrence also deletes only its
incident root wires. The UI then freshly encodes `ALGH`, flattens it,
regenerates `ALGM`, admits the flattened draft through the audited semantic
registry, constructs the complete `ALGS`, records the previous complete
session, and commits all artifacts together. Library imports/removals and
direct occurrence authoring therefore participate in the same exact undo/redo
and persistence timeline as control, probe, and cached-job edits. An exact
duplicate import changes only the transient selection and records no history.

The adjacent structural root canvas projects only the root `ALGW`'s bounded
integer placements, placeholder connectors, and wires. A bound component
header can be dragged; the pointer delta is rounded once and committed through
`move_root_instance`. Selecting an output and then a type-compatible input
calls `connect_root_wire`; secondary-clicking an owned input calls
`disconnect_root_wire`. Ordinary unbound structural nodes cannot be moved by
the hierarchy operation. Every accepted canvas action reruns complete `ALGH`
validation, recursive flattening, `ALGM` regeneration, ordinary-node semantic
admission, and canonical `ALGS` history/persistence before any visible state
changes. A rejected or exact no-op action retains the complete prior session.

Optimized Chromium qualification dragged root wrapper `n1` by exactly
`(+130, +60)` logical pixels, changing its canonical placement from `(28, 28)`
to `(158, 88)`. The root `ALGW` identity changed from
`3d7775b323bfa9442ba5eee800cf0020eac3e3f046f67693dc7154359fbe0fd6`
to
`01a7ff6369a960824e090e4a8cd7d33093620f3fba8cfb365cbe5b76d58be82a`;
the complete `ALGH` changed from
`f9751073015828f20154a5536d8b217a7d3843e2d63d3b5689fcfb8c2379806a`
to
`02eed6a0b7d8b2c8b29b1af3f25bc37bb9cb612798a4a49ba2087831c680d405`.
The control `ALGW`, bound `ALGP`, cached-job `ALGW`, selected `ALGC`, and both
dependency encodings remained byte-identical. Visible Undo restored the exact
14,770-byte reference `ALGS`; visible Redo and a fresh page reload restored the
exact moved `ALGS` with SHA-256
`90167e8ee1e404caa24b4827d05bd9b6959f5196168ee980ed75bf1701937ac5`.

When an attached control workspace changes compatibly, the UI constructs the
replacement selected `ALGC` and calls `replace_component`; authored root
instances, root IDs/placements/wires, and unrelated dependency encodings remain
unchanged while every old selected-component binding moves to the new digest.
An incompatible component edit follows the existing visible detached-hierarchy
path and remains recoverable through complete-session undo.

Front-panel-only edits use that same replacement path while retaining the
selected component's embedded workspace and connector pane exactly. Because
placeholder shape is unchanged, root and nested instance nodes/wires remain
byte-identical. The replacement `ALGC`, complete `ALGH`, and source-bound
`ALGM` identities change together and enter the ordinary complete-session
history; the flattened ordinary `ALGW` remains identical.

The connector-pane editor follows the dependency chosen in the adjacent
library selector, rather than being fixed to the complete-session control
component. An accepted input/output add, update, or removal replaces that exact
dependency and uses the complete replacement report to retain the logical UI
selection across its new digest. Editing the root-reachable wrapper refreshes
the root placeholder shape while preserving root node identity, placement,
wires, and allocation cursors; the selected control `ALGC`, control `ALGW`,
probes, and cached-job workspace remain exact. If editing a non-authoritative
descendant would recursively rebuild the `ALGC` that supplies the complete
session's exact control workspace, the UI rejects the candidate atomically.
Such a change requires a future coordinated control-workspace transaction; it
cannot silently change the `ALGS` authority.

## Deliberately open

Direct root-instance creation/deletion is implemented through the dedicated
library panel, and exact root placement/wiring is implemented on its structural
canvas. Nested definition editing and import with separately supplied nested
bindings, general library creation, parameter promotion/overrides, package
signatures and permissions, locked dependency manifests, incremental
flattening, interactive traversal from final items into nested editable
canvases, coordinated descendant/control-authority replacement, and executable
front-panel inputs remain open.
Canonical [`ALGS` V1](GRAPH-AUTHORING-SESSION-V1.md) now persists one selected
component with its complete ALGH/ALGM branch atomically, and unified undo/redo
restores direct root-instance edits with all other authoring state. `ALGH` V2
and `ALGM` V1 grant no
semantic, implementation, resource, timing, safety, firmware, or
physical-output authority.
