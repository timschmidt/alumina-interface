# Canonical nested component package V1

`ALCP` V1 is the bounded, immutable exchange format for one reusable `ALGC`
component together with its complete transitive component dependency closure.
It exists because a standalone `ALGC` carries its embedded `ALGW` but cannot
carry the component-scoped instance bindings owned by `ALGH`.

V1 is the only accepted version. This green-field implementation has no legacy
decoder, alias table, or compatibility wrapper.

## Authority boundary

An `ALCP` is authoring data. It grants no node semantics, implementation,
peripheral/resource access, timing, memory, safety, firmware, deployment, or
physical-output authority. Import performs structural replay and the UI then
reruns its ordinary audited component-library, recursive flattening, semantic,
source-map, and complete-session admission before changing visible state.

Firmware never receives or decodes `ALCP`, `ALGC`, `ALGH`, `ALGW`, or raw
`ALGR` authoring bytes.

## Canonical bytes

All fixed-width integers are little-endian. The encoding contains, in order:

1. ASCII magic `ALCP`;
2. version `1` as `u16`;
3. zero flags as `u16`;
4. seven `u64` embedded limits: package bytes, component count, scoped-binding
   count, nesting depth, expanded occurrences, flattened nodes, and flattened
   wires;
5. the 32-byte SHA-256 identity of the root `ALGC`;
6. a `u32` component count followed by each length-prefixed canonical `ALGC`,
   sorted by its SHA-256 identity; and
7. a `u32` binding count followed by records sorted by parent digest, local
   node ID, and child digest. Each record is the 32-byte parent `ALGC` digest,
   a nonzero `u32` parent-local placeholder node, and the 32-byte child `ALGC`
   digest.

There is no mutable package revision. The SHA-256 digest of the complete bytes
is the package identity. Every embedded `ALGC`, `ALGW`, and `ALGR` retains its
own exact revisions, monotonic cursors, and independent admission limits.

The interactive policy admits at most 32 MiB, 64 component definitions, 4,096
component-scoped bindings, nesting depth 32, 4,096 expanded component
occurrences, 4,096 flattened ordinary nodes, and 8,192 flattened ordinary
wires. Replay checks caller limits before allocation, requires every embedded
limit to fit them, bounds each nested envelope independently, rejects trailing
bytes, reconstructs every invariant, and requires byte-for-byte canonical
re-encoding.

## Exact closure invariants

The root digest must name exactly one embedded component. Every embedded
component must be reachable from that root through component-scoped bindings;
unrelated library entries are forbidden. Root-workspace bindings are forbidden
because an `ALCP` represents a reusable definition, not a placement in a
particular machine graph.

Every reserved `alumina.component.instance` placeholder in every embedded
component has exactly one binding. Parent scopes and children exist in the
package, placeholder kind/version/ports/parameter emptiness match the bound
child, all components share the root's exact type registry and clocks, and the
dependency graph is acyclic and within all depth/expansion/flattening bounds.
Component digests and stable names are both unique within the package.

Export begins at one selected dependency in an already admitted `ALGH`, walks
its exact child bindings transitively, and omits root bindings, ancestors,
siblings, and unrelated dependencies. A leaf therefore produces a valid
one-component, zero-binding `ALCP`; `.algc` remains the smaller leaf-only
exchange when no closure is required.

## Transactional library merge

Import never replaces or aliases an existing definition:

- an incoming digest already present must have byte-identical `ALGC` bytes;
- an incoming stable name already owned by another digest rejects;
- an incoming `(parent digest, local node ID)` already bound to the same child
  is an exact duplicate, while a different child rejects;
- new definitions and new scoped bindings are accumulated on a clone; and
- the complete destination `ALGH` is rebuilt and validated once at its next
  monotonic revision before commit.

Consequently a successful changing import advances the hierarchy exactly once,
even when it adds many definitions and bindings. A byte-identical duplicate
changes no canonical hierarchy byte or revision. It may update only transient
UI selection to the package root and creates no undo or persistence record.
All errors leave the destination hierarchy byte-for-byte unchanged.

The browser exposes selected-root `.alcp` download/open controls beside the
existing `.algc` controls. An accepted changing import regenerates flattened
`ALGW`, `ALGM`, and complete `ALGS`, runs audited ordinary-node admission for
the whole resulting library, records one unified history state, and selects the
imported package root. Undo, redo, and reload therefore restore the entire
closure atomically.

## Deliberately absent

`ALCP` V1 has no signatures, permissions, locked manifest, external lookup,
partial closure, merge heuristic, identity alias, instance parameter override,
control-authority replacement, or executable front-panel input. Those require
separate exact formats and transactions; they are not inferred during import.
