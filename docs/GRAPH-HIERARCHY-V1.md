# Canonical graph hierarchy and flattening V1

`ALGH` V1 binds authoring-only instance nodes in one root `ALGW` to exact
canonical `ALGC` dependencies. Its compiler removes every instance and emits an
ordinary canonical workspace whose `ALGR` contains only the dependency's real
structural nodes and wires.

## Authority boundary

An instance node is a collapsed authoring view, not opaque executable behavior.
It uses reserved kind `alumina.component.instance` version 1 and is never
registered as a host behavior, firmware opcode, resource operation, or timing
claim. Semantic analysis, simulation, and deployment consume only the flattened
ordinary `ALGR` under their existing independent registries.

`ALGH` does not admit node semantics, prove that a draft is complete, select an
implementation, allocate a peripheral, prove WCET, or grant safety/deployment
authority. Firmware never receives or interprets `ALGH`, `ALGC`, `ALGW`, or raw
`ALGR` bytes.

## Canonical package

Every fixed-width integer is little-endian. V1 contains, in order:

1. `ALGH`, version 1, and zero flags;
2. embedded hierarchy byte/component/instance/flattened-count limits;
3. hierarchy revision;
4. one length-delimited canonical root `ALGW`;
5. length-delimited canonical `ALGC` dependencies sorted by SHA-256 digest; and
6. instance-node ID plus exact component digest records sorted by node ID.

The first policy admits at most 32 MiB, 64 distinct dependencies, 256
instances, 4,096 flattened nodes, and 8,192 flattened wires. The root workspace,
each component, and every embedded graph retain their own independent caller
and embedded policies. Outer replay bounds bytes/counts before allocation,
independently replays every nested envelope, reconstructs the library and
instance shapes, rejects trailing bytes, and requires exact re-encoding before
returning hierarchy identity.

Every dependency in the library—not only those currently instantiated—must use
the exact root graph type registry and clock set. Duplicate component digests,
missing dependencies, duplicate instance bindings, unbound reserved nodes, and
reserved nodes with an unknown version reject the whole package.

## Derived instance shape

The instance placeholder copies no behavior schema. Its input ports are derived
from public component inputs in canonical ID order and receive consecutive
node-local port IDs beginning at 1. Outputs follow immediately in canonical
public-output order. Names and exact value types resolve from the component's
connector pane. The placeholder is `HostExact` only as authoring placement
metadata and retains no parameters.

Any mismatch in kind/version, domain, port order/name/type, or parameter
emptiness rejects before flattening. Helper APIs resolve a public input/output
identity to its derived instance port so callers do not duplicate that mapping.

V1 deliberately admits leaf components only. A dependency containing any
reserved instance name, including an unsupported version, rejects. This hard
depth-one bound prevents hidden recursion or dependency cycles until nested
hierarchy packages have an explicit bounded dependency model.

## Deterministic flattening

Flattening works transactionally on a clone of the root workspace, in ascending
original instance-node order:

1. retain every incident root wire and the instance's integer placement;
2. delete the placeholder and its incident wires without rewinding cursors;
3. copy component nodes in canonical node-ID order using fresh monotonic root
   IDs and exact ports/parameters/domains;
4. translate component presentation coordinates relative to the component's
   minimum x/y so its top-left begins at the instance placement;
5. copy internal wires in canonical wire-ID order using fresh monotonic IDs;
6. map incoming instance ports to unowned public-input targets and outgoing
   ports to public-output sources; and
7. reconnect retained root wires through the normal typed workspace edit
   boundary.

Wires between two instances remain valid regardless of direction: flattening
the first endpoint reconnects to the still-collapsed second endpoint, and the
later pass resolves that endpoint. A self-wire maps both ends in one pass.
Final node/wire counts are proved before work begins and checked again after all
instances disappear. Identifier exhaustion, type drift, target ownership,
coordinate overflow, graph byte limits, or any edit error discards the cloned
candidate and returns no flattened workspace.

The result carries canonical `ALGW` bytes plus an audit report mapping every
component-local node ID to its new root ID. It also binds the source `ALGH`
digest, so diagnostics can relate flattened structure back to the exact
library/instance package.

## First visible proof

The control workspace constructs a one-node hierarchy around
`control.reference_pid`. The 5,008-byte `ALGH` has SHA-256
`d9a0d5cbe0b7694b506711f48d85ac2a904874261fd0d2d86244da06cf2e5f64`.
It deterministically expands one collapsed instance to 19 ordinary nodes and
22 wires. The 3,396-byte flattened `ALGW` has SHA-256
`a5e0abfd4f1e8642a78244b7c14f91150665faadef4898c49ddc256c88a98277`
and passes the existing audited HostExact registry. Its identity differs from
the hand-authored reference workspace because fresh monotonic node IDs and
translated presentation positions are intentional flattened facts.

The UI displays source hierarchy and flattened-workspace identity/counts beside
the live canonical component panel. It does not substitute the collapsed
placeholder into the executable editor canvas or claim the instance itself can
run.

## Deliberately open

Nested component dependencies, recursive depth/cycle analysis beyond the V1
leaf-only rejection, editable instance creation/deletion on the main canvas,
hierarchy-aware undo/redo and file/browser persistence, component overrides,
generic parameter promotion, library signatures/permissions, dependency locks,
incremental flattening, and source-level trace remapping remain open. `ALGH` V1
grants no semantic, implementation, resource, timing, safety, firmware, or
physical-output authority.
