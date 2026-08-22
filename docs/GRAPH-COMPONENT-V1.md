# Canonical graph component and front panel V1

`ALGC` V1 is the first reusable authoring package above `ALGW`. It embeds one
complete canonical graph workspace, gives selected internal ports a public
connector identity, and binds a bounded integer front panel to public terminals
or exact retained parameters.

## Authority boundary

Hierarchy does not weaken the existing graph boundary. The `ALGR` inside the
embedded workspace remains the only executable structural graph authority.
`ALGC` is not accepted by firmware and does not resolve an opaque node kind,
admit an implementation, flatten a component instance, allocate a resource,
prove timing, or authorize deployment. A later compiler must independently
resolve and lower a component into audited `ALGR`/graph IR before it can run.

Front-panel rectangles are signed-origin, unsigned-extent logical pixels. V1
requires nonnegative origins, nonzero extents, and bounded right/bottom edges.
They are presentation metadata only. No component API converts a rectangle into
an exact value, clock, machine coordinate, timer, or firmware quantity.

## Canonical envelope

Every fixed-width integer is little-endian. V1 contains, in order:

1. `ALGC`, version 1, and zero flags;
2. embedded component byte/count/panel-coordinate limits;
3. component revision, nonzero declared behavior version, and stable namespaced
   component name;
4. monotonic next-input, next-output, and next-panel-item identity cursors;
5. one length-delimited complete canonical `ALGW` document;
6. public inputs sorted by stable ID, each with a stable name and mapped
   internal input endpoint;
7. public outputs sorted by stable ID, each with a stable name and mapped
   internal output endpoint; and
8. front-panel items sorted by stable ID, each with a stable name, tagged exact
   binding, and integer rectangle.

Stable names are 1–64 ASCII bytes, begin with an alphabetic byte, and then use
only alphanumerics, `_`, `-`, or `.`. Public input and output names share one
connector namespace. IDs are nonzero and never duplicated. The next-ID cursors
must exceed every retained ID and may use `u32::MAX + 1` only as an exhausted
sentinel.

The first interactive policy admits at most 24 MiB total, 128 public inputs,
128 public outputs, 256 panel items, and panel edges no greater than 1,000,000
logical pixels. The separately replayed workspace retains its own 20 MiB,
256-placement, and coordinate limits. Neither embedded policy can grant itself
more authority than its caller.

`replay_graph_component` bounds the outer bytes before parsing, bounds counts
before allocation, independently replays the embedded `ALGW` and `ALGR`,
reconstructs every component invariant, rejects trailing bytes, and requires an
exact re-encoding match. SHA-256 identity is returned only after byte equality.

The initial 21-node PID/interlock component is 4,815 bytes with SHA-256
`10e6498ec36afc377f138cacb5c6afe2091c40749ea3c9e9d4bba8925a4f0228`.
It embeds the existing 3,755-byte canonical reference workspace unchanged.

## Connector semantics

A public input maps to exactly one declared internal input. That target must
have no internal wire owner: a value has one explicit source, either the
component connector or an internal wire, never both. Multiple public inputs
cannot alias one target.

A public output maps to exactly one declared internal output. It may observe an
output that also feeds internal wires, but two public outputs cannot alias the
same endpoint. Input/output direction and exact registered value type are
resolved from the embedded graph rather than copied into a second schema. The
component exposes type queries for terminals and panel items.

Public connector inputs intentionally allow an embedded workspace to be an
incomplete editor draft. Semantic admission must account for those connector
sources when hierarchy is later flattened; `ALGC` V1 itself does not claim the
draft is executable.

## Front-panel bindings

V1 has three explicit binding kinds:

- an input control supplies one public component input at runtime;
- a parameter control transactionally replaces one exact retained node
  parameter through the normal workspace edit boundary; and
- an output indicator observes one public component output.

Each binding resolves against the embedded workspace, and a binding may appear
only once on the panel. A parameter control names both stable node and
node-local parameter IDs; its type is the retained typed value's exact root
type. An input/output item inherits its terminal's resolved graph type. Panel
layout never stores a float or display-derived value.

`GraphComponentDocument::replace_workspace` advances component revision and
validates a complete candidate before mutation. Deleting a bound node, changing
a mapped port, internally wiring a public input, or invalidating any panel
binding rejects the replacement and preserves the prior component byte for
byte.

`add_panel_item`, `update_panel_item`, and `remove_panel_item` apply the same
clone-and-reconstruct rule to panel metadata. Addition allocates only from the
monotonic panel-item cursor. Deletion never rewinds it. Update replaces the
stable name, exact binding, and integer rectangle together; a byte-identical
replacement is an exact no-op. Unknown items, exhausted identity space,
duplicate names/bindings, unresolved targets, negative/empty/overflowing
rectangles, and document-limit failures retain the complete prior `ALGC`.

## First editor workflow

The native/WASM control workspace constructs `control.reference_pid` version 1
around its current canonical workspace. The autonomous reference fixture has
seven public Stream outputs, eight exact parameter controls (P/I/D gains, clamp
minimum/maximum, safe output, and inclusive interlock minimum/maximum), and seven
exact replay indicators: four exact-rational values and three Boolean interlock
states. Controls
use the same Hyperreal parsing, typed-value validation, canonical `ALGR` edit,
history, and persistence path as the selected-node inspector.

Every accepted workspace edit rebuilds and encodes the selected component. A
compatible replacement is remapped into the existing hierarchy by old and new
exact digest, preserving authored root instances and unrelated component
dependencies. If an otherwise valid draft deletes or changes a referenced
endpoint, the front panel and hierarchy detach visibly without rejecting the
`ALGW` edit. Undo or another restoring edit reattaches them after complete
validation. Indicators never relabel stale trace bytes: once the embedded graph
digest differs from the reference replay, they report that the exact replay is
detached.

The visible panel editor lists every exact public input, retained parameter,
and public output not already owned by another item. It can add one binding
under a validated stable name, select/remove an item, replace its name,
binding, and integer x/y/width/height, or drag its header with one cumulative
pointer delta rounded only at commit. Every accepted edit replaces the selected
`ALGC` dependency, remaps its exact hierarchy bindings, freshly validates and
flattens `ALGH`, regenerates `ALGM`, reruns ordinary-node semantic admission,
and records the prior complete `ALGS` before committing. Panel-only changes
retain the embedded control `ALGW`, connector pane, root workspace, unrelated
dependencies, probes, and cached-job workspace exactly. Exact no-ops record no
history.

The visible connector editor follows the exact dependency selected in the
`ALGH` library panel. It offers bounded endpoint choices and all six stable-ID
input/output add, metadata-update, and reference-safe removal operations for
the control component or another selected dependency. Every accepted library
edit recursively rebuilds affected parents, root placeholders, `ALGH`, and
`ALGM`, then admits and records one complete `ALGS`. A non-authoritative edit
retains the selected control `ALGC` and its embedded control `ALGW` exactly. If
the edited dependency is beneath that control authority and would force the
authority itself to change identity, the UI rejects without mutation until a
coordinated control-workspace replacement exists.

The adjacent selected-definition editor opens the embedded `ALGW` of any
selected non-authoritative dependency as its own canonical structural canvas.
It supports the audited palette, stable node creation/deletion, exact label,
domain, and parameter edits, integer placement drag, and monotonic typed-wire
connect/disconnect. Transient node, wire, drag, and text-draft state is scoped
separately from the main control canvas. It can add another dependency already
present in the `ALGH` library as a fresh child occurrence; the parent-local
placeholder and scoped `GraphComponentInstance` binding commit atomically.
`ALGH`-owned placeholders remain visible and wireable but cannot be deleted by
the ordinary node action. A dedicated occurrence action removes the
placeholder, incident wires, and scoped binding together, unless a public
connector or panel item still binds that node. Each accepted edit uses the same
full recursive replacement report and complete-session transaction as
connector authoring. Exact no-ops create no revision or history state; invalid
connectors/panel references, cycles, semantic failures, and indirect
control-authority changes reject atomically. Deletion preserves the monotonic
node cursor.

The ALGM flattened-source browser can enter this editor without authoring. It
revalidates a final node or wire's complete occurrence path and exact component
digest, selects the matching library definition, and focuses the local node or
distinctly highlights the exact local wire while retaining its target node in
the existing inspector. An authoritative component source instead opens the
main control canvas, while a root source highlights the structural root canvas.
This source-navigation state is deliberately absent from `ALGC` and `ALGS`,
records no history, and does not mark persistence pending.

The reviewed fixture supplies the initial panel metadata. Once edited, the
canonical `ALGC` inside `ALGS` and `.algc` exchange is authoritative; later
compatible `ALGW` edits replace its embedded workspace while retaining the
authored panel metadata. A standalone `.algw` file has no panel metadata and
cannot silently reconstruct or overwrite it. The component-library panel
exports the exact selected `ALGC` and imports a bounded canonical standalone
leaf after full component/workspace/graph replay, exact re-encoding, audited
ordinary-node semantics, and current `ALGH` context validation. Importing an
exact duplicate is a no-op; importing a new identity or later removing that
unreferenced identity is a complete-session historical edit. Nested
definitions require scoped `ALGH` bindings that standalone `ALGC` does not
carry, so this first file workflow deliberately admits leaf packages only.

A file is not required to start a new leaf package. The same library panel can
construct a named version-1 empty `ALGC` whose embedded `ALGW` and `ALGR` use
the current control graph's exact schema and clocks. Component, workspace,
node, wire, connector, and panel-item cursors all begin at one; connectors,
panel items, nodes, wires, and placements are empty. The new dependency is
selected immediately for ordinary definition editing. Invalid or overlong
names and a stable name already bound to another digest reject without
mutation; requesting the byte-identical empty component again is a
selection-only no-op with no history or persistence write.

Optimized Chromium qualification dragged panel item `#14`
`combined_permit_indicator` by exactly `(+80, +30)` logical pixels, from
`(460, 404, 240, 54)` to `(540, 434, 240, 54)`. The selected `ALGC` identity
changed from
`10e6498ec36afc377f138cacb5c6afe2091c40749ea3c9e9d4bba8925a4f0228`
to
`c607de51199369cc1ff7fb40b377309511d5fcee07df1822475efe4c0383c2fd`;
the complete `ALGH` changed to
`93567a3cf1b27c6c14acb6ef95eeb37d16fc1aab8a62e69900ad5ccca214bcb6`
and `ALGM` changed to
`2bd200edb3f2be3a84e0c6b5251f0043a2be0a0f4da8dce293b02c262e4ffbd8`.
The embedded control workspace, connector pane, root workspace, wrapper
dependency, other fourteen panel items, probes, and cached-job workspace
remained byte-identical. Visible Undo restored the exact reference `ALGS`;
visible Redo and a fresh reload restored the exact 14,770-byte moved `ALGS`
with SHA-256
`530db6a4d7cec74fcb3d36736c8de51cd024b4ed70b3b9c282234c48b5eaf666`.

The deterministic-creation Chromium qualification began from the canonical
14,770-byte reference `ALGS`
(`d7a5fba83da9f254eb0d50eab301129f933016a400c9d154c5f2d97d8029cf9d`)
and used the visible name field and create button to add
`user.browser_component`. The resulting 15,510-byte `ALGS`
(`991bb737edcd0a3ad1f45b5f0e05d657c4e2557deddec46288c6512f168fe0fe`)
contains a 736-byte revision-1 `ALGC`
`264505540b35ca004697783b0136ec2fb8f09230768355d46627af3e3c07f36f`.
Its 612-byte `ALGW` has revision one, node/wire cursors one, no placements,
and embeds a 548-byte revision-1 `ALGR` with no nodes or wires. A byte-prefix
comparison through the clock section proved that `ALGR` inherited the exact
current schema and clocks. Re-entering the same name selected that dependency
without changing a byte or adding history.

The visible selected-definition editor then created ordinary node `1`. The
replacement revision-2 `ALGC` is 851 bytes with identity
`b46e9dc604882ce77eebec1ebb8195fa7f4d7159558b4e4f00dcace6e6225198`;
the complete session is 15,625 bytes with SHA-256
`88bf2e0c9bc04d11112b3a8de865e242fd5beadbf57ab09cd4b82d235122ffa1`.
The control workspace, probes, cached-job workspace, root workspace, control
authority, flattened workspace, and all 21-node/25-wire provenance records
remained exact. Visible Undo restored the empty-component session, visible
Redo restored the one-node session, and a fresh reload retained those exact
bytes and allowed the populated component to be selected again.

## Deliberately open

The separate canonical [`ALGH` V2 hierarchy](GRAPH-HIERARCHY-V2.md) now binds
scoped component instances by exact digest, rejects dependency cycles, bounds
recursive expansion, and deterministically flattens a component DAG to ordinary
`ALGW`/`ALGR`. Its dedicated UI now creates and deletes exact root occurrences
from the embedded dependency library and preserves them across compatible
selected-component edits. Nested definition import with separately supplied
scoped bindings,
component rename/version evolution, package signatures/permissions, locked
dependency manifests, child rebinding,
overlapping/grouped/responsive panel layout policies, panel value injection
during simulation or execution, probes, and groups/comments remain open.
`ALGC` V1 grants no semantic, implementation, resource, timing, safety,
firmware, or physical-output authority.
