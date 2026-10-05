# Canonical graph hierarchy, promoted parameters, and recursive flattening V3

`ALGH` V3 binds authoring-only instance nodes in a root `ALGW` and canonical
`ALGC` component definitions to exact component digests. Component definitions
may contain other component instances. The complete dependency graph is a
bounded DAG, and deterministic recursive flattening emits one ordinary
canonical workspace containing only structural `ALGR` nodes and wires.

V3 adds stable promoted component parameters and exact per-occurrence override
values. It is the only hierarchy version accepted by this source tree. The
[V2 checkpoint](GRAPH-HIERARCHY-V2.md) is retained as historical evidence; no
V2 decoder, migration shim, or compatibility path exists.

## Authority boundary

An instance is a collapsed authoring view, never opaque executable behavior.
Reserved kind `alumina.component.instance` version 2 is not registered as a
host behavior, firmware opcode, resource operation, timing claim, or safety
authority. Semantic analysis, simulation, and deployment consume only the
fully flattened ordinary graph under their existing independent registries.

`ALGH` does not admit node semantics, select an implementation, allocate a
peripheral, prove WCET, or authorize deployment. A promoted or overridden value
is typed authoring data. Firmware never receives or interprets `ALGH`, `ALGC`,
`ALGW`, or raw `ALGR` bytes.

## Canonical envelope

Every fixed-width integer is little-endian. V3 contains, in order:

1. `ALGH`, version 3, and zero flags;
2. seven embedded hierarchy limits;
3. monotonic hierarchy revision;
4. one length-delimited canonical root `ALGW`;
5. length-delimited canonical `ALGC` dependencies sorted by SHA-256 digest; and
6. scoped instance bindings in canonical scope/node/component order.

The limits independently bound complete bytes, dependency count, binding
count, depth, expanded occurrences, flattened nodes, and flattened wires. The
root workspace, every component, and every embedded graph retain their own
caller and embedded policies. Replay bounds bytes and counts before allocation,
independently replays every nested envelope, reconstructs every hierarchy and
parameter invariant, rejects trailing bytes, and requires exact re-encoding.

A binding names either the root workspace or one parent-component digest, then
one local placeholder node and one child-component digest. Root sorts before
component scopes; component scopes sort by digest and local node. Duplicate
`(scope, node)` bindings are forbidden. Every reserved placeholder has exactly
one binding, and every dependency shares the root type and clock context.

No separate override section is needed. Exact occurrence values are already
canonical node parameters inside the root or parent component's `ALGW`; they
therefore participate in the existing component, hierarchy, session, history,
and persistence identities without a parallel source of truth.

## Derived instance surface

Inputs and outputs continue to derive from stable public component connector
IDs. Inputs receive consecutive local port IDs beginning at one; outputs follow
them. Names and exact types resolve from the child `ALGC`, never from caller
copies.

The version-2 reserved instance node used by `ALGH` V3 derives its parameter
surface from the component's front-panel items whose binding is
`ParameterControl`:

- the stable `GraphFrontPanelItemId` becomes the node-local parameter ID;
- the stable panel-item name becomes the parameter name;
- the controlled node parameter supplies the exact registered type; and
- its retained exact value becomes the occurrence default.

Non-parameter input controls and output indicators do not become node
parameters. Gaps in panel-item IDs remain gaps; allocation identities are never
renumbered. Validation requires the exact parameter count, order, IDs, names,
and types, while intentionally allowing each retained value to differ from its
definition default.

## Recursive promotion

Promotion requires no new front-panel binding kind. Once a child placeholder
has derived parameters, a parent component may bind one of its own
`ParameterControl` items to that placeholder node and stable parameter ID. The
parent item then becomes a parameter on every parent occurrence. Repeating
this rule promotes an exact leaf value through any admitted hierarchy depth.

The relationship is structural and explicit at every level:

```text
root occurrence parameter
  -> wrapper panel ParameterControl
  -> child placeholder parameter
  -> leaf panel ParameterControl
  -> ordinary leaf node parameter
```

Each arrow is retained in canonical `ALGW`/`ALGC` bytes. Missing targets,
type drift, duplicate bindings, invalid names, cycles, or policy overflow reject
before the hierarchy can replace its prior state.

## Occurrence-local overrides

`GraphHierarchyDocument::set_root_instance_parameter` accepts one bound root
node, one stable front-panel item ID, and one complete `TypedGraphValue`. It
requires the exact derived parameter, delegates schema/value validation to the
cloned root workspace, reconstructs the complete hierarchy, and commits only
after every invariant succeeds. Unknown occurrences, unknown parameters,
wrong types, invalid literals, semantic rejection by a later audited caller,
or limit failure leave the complete prior hierarchy exact. Reapplying the
retained value is a byte-for-byte no-op with no revision advance.

A parameter edited on a nested placeholder inside a component definition is a
definition-level default or override shared by occurrences of that parent.
Promoting it to the parent's public parameter surface is what permits each
parent occurrence to retain a different value. This distinction prevents an
implicit path-wide mutation or hidden alias.

When a component identity evolves, placeholder refresh uses stable public
parameter IDs:

- a value equal to the old definition default follows the new default;
- a non-default value is preserved when the stable ID and type remain;
- removing a parameter with a retained non-default value rejects atomically;
- changing the type of a retained non-default value rejects atomically; and
- a new public parameter begins at its new definition default.

Name changes follow the stable ID and refresh the derived node contract. A
wrapper panel item that still binds a removed child parameter independently
rejects through normal `ALGC` validation. Thus component evolution never
silently drops or reinterprets an authored override.

## Deterministic flattening

Root bindings are visited in canonical local-node order; each component subtree
is visited depth first in pre-order. For each occurrence, flattening:

1. validates and captures the exact placeholder parameters and placement;
2. clones the bound component workspace;
3. resolves every instance parameter through its stable panel item and applies
   the exact value to the controlled node parameter in that clone;
4. removes the placeholder while retaining incident connector wires;
5. copies component nodes, parameters, domains, ports, placements, and wires
   with fresh monotonic root identities;
6. reconnects public terminals by stable connector identity; and
7. recursively flattens copied nested placeholders, which now carry any value
   promoted through the parent occurrence.

This ordering is what makes two occurrences of the same leaf definition
flatten to different exact hardware-facing literals without cloning or
mutating the leaf `ALGC`. Final node and wire counts, removal of every reserved
placeholder, canonical workspace encoding, and audited ordinary-node semantics
remain independent acceptance boundaries.

The flattening report and canonical `ALGM` source map retain stable root-to-leaf
occurrence paths and exact component/local-node origins. The final ordinary
node retains the applied typed value; path plus node/parameter identity is
sufficient to audit which occurrence received it. Parameter values do not
alter source ownership or grant execution authority.

## Component replacement and package exchange

Connector and parameter refresh compose with recursive component replacement.
Changed ancestor identities, parent scopes, root bindings, and source maps are
rebuilt in one candidate. Root placement, wires, monotonic allocators, unrelated
dependencies, and explicit compatible overrides remain exact.

Canonical `ALCP` V1 needs no override sidecar. A definition closure already
contains nested placeholder defaults or definition-level overrides inside its
`ALGC` bytes. Root-occurrence overrides remain properties of the importing
machine hierarchy and are intentionally not smuggled into a reusable
definition package.

## Browser authoring and session behavior

The root-occurrence selector exposes schema-directed exact editors for every
promoted parameter, reports the promoted/override counts, and commits through
fresh flattening, `ALGM` regeneration, semantic audit, complete `ALGS`
construction, unified history, and browser persistence. Invalid text or an
invalid complete candidate mutates nothing. Exact no-ops record no history or
persistence write.

The selected-definition editor uses the same parameter surface on nested
placeholders. Existing front-panel authoring can bind those parameters on a
wrapper, making recursive promotion explicit and visually inspectable. The
root canvas shows the promoted-parameter count but remains an authoring view;
it cannot arm, deploy, or command firmware.

## Qualified reference proof

The current native suite proves two occurrences of the same wrapper at paths
`[1/1]` and `[2/1]`, exact values `7/3` and `11/5` on stable public parameter
ID 1, distinct flattened leaf literals, exact no-op behavior, invalid `1/0`
atomicity, recursive default evolution, explicit-override preservation,
parameter removal/type-drift rejection, complete history, and persistence.

Optimized Chromium independently parsed the canonical persisted envelopes,
used the visible editors to make those two edits, and exercised visible Undo,
Redo, and fresh reload. The one-occurrence V3 reference is 15,750-byte `ALGS`
`e473839dd8b1708fe699a8749d79b8f37672699e6ea203bff9efae1b58b30c08`,
containing 8,104-byte `ALGH`
`ecd9ab11557bf6c3a565af4563cb145bb6f1e28efc73d99ccc74f7354755f198`.
The final two-occurrence session is 18,805 bytes at
`248f2e340eae9ad724253a5bc0f50b7c832da78356d2918abb5acdf3176e19ba`;
its 8,721-byte `ALGH`
`e6545ea05712ce9a86b202086b5654ad1cb9e9f9f5a3a94ed7c203fd39f6a464`
retains canonical root values `7/3` and `11/5`, while its regenerated
flattened-workspace identity is
`902396a53f158fd0ac359d8a8874b8809e21dd0b800626b4b8fbfed4d51e45ae`.
Control `ALGW`, `ALGP`, cached-job `ALGW`, and selected `ALGC` remained exact.

The retained 69,617-byte proof record has SHA-256
`bf60d7ca8607eccea7a2d9cb02e15c629c2079bda9e996dce54c0693fb4d6f69`.
Its qualified reload screenshot selects root occurrence 2, visibly shows one
occurrence-local override and Hyperreal's mixed-number `2 1/5` rendering, and
has SHA-256
`bb21c2522b57e77f5218d20401953d497ea04fbe0a4546cf32fccc2eac3ed711`.
That transient selection retained the final `ALGS` byte-for-byte.

## Deliberately separate

Runtime values for `InputControl` and executable host front-panel programs now
belong to canonical [`ALFR` V1](GRAPH-FRONT-PANEL-RUN-V1.md), not `ALGH`.
Package signatures and permissions, locked dependency manifests,
collaboration journals, incremental flattening, and direct firmware
interpretation remain outside V3.
